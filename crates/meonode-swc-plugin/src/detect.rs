//! Factory call-site detection.
//!
//! Identifies which `CallExpression`s in a program are compilable
//! `@meonode/ui` factory calls — calls whose `props` argument is a plain
//! object literal that Task 9's partitioner can safely rewrite into
//! pre-partitioned marker props. This module is detection-only: it never
//! mutates the AST. It records a [`Decision`] per relevant call site (
//! [`Decision::Compilable`] or [`Decision::Bail`] with a reason), so callers
//! (Task 9's rewrite pass) can extend this visitor to act on the results.
//!
//! ## Binding resolution
//!
//! Call-site classification is entirely binding-based, keyed by
//! `(Atom, SyntaxContext)` pairs. This requires the SWC resolver
//! (`swc_ecma_transforms_base::resolver`) to have already run over the
//! program so that every [`Ident`] carries the [`SyntaxContext`] of the
//! binding it actually refers to:
//!
//! - **In the real plugin**: the host (next-swc / `@swc/core`) runs the
//!   resolver before invoking the plugin, and the plugin never applies it
//!   itself — see `TransformPluginProgramMetadata::unresolved_mark` and
//!   `lib.rs::process_transform`. By the time `transform_program` runs, all
//!   `Ident`s in `program` already carry resolved contexts.
//! - **In fixture tests** (`tests/fixture.rs`): the resolver is chained
//!   explicitly, mirroring what the host would otherwise do (`(resolver(...),
//!   ...)` as the `Pass` given to `test_fixture`).
//! - **In this module's unit tests**: a small harness parses a snippet and
//!   runs the resolver by hand before calling [`transform_program`], since
//!   there is no host/`test_fixture` to do it for us.
//!
//! Matching on `(sym, ctxt)` rather than `sym` alone is exactly what lets us
//! tell a genuine `@meonode/ui` import apart from a same-named local that
//! shadows it (`shadowed_div` bailout) — after resolution, the shadowing
//! declaration gets a distinct `SyntaxContext`, so its call sites simply
//! fail to match the tracked binding.

use std::collections::{HashMap, HashSet};

use swc_core::common::{Span, SyntaxContext};
use swc_core::ecma::ast::*;
use swc_core::ecma::atoms::Atom;
use swc_core::ecma::visit::{Visit, VisitWith};

use crate::factories::factory_children_first;

const UI_MODULE: &str = "@meonode/ui";
const UI_MODULE_CLIENT: &str = "@meonode/ui/client";
const MARKER_KEY: &str = "__meo$";
const NODE_NAME: &str = "Node";
const CREATE_NODE_NAME: &str = "createNode";
const CREATE_CHILDREN_FIRST_NODE_NAME: &str = "createChildrenFirstNode";

/// A binding key: an identifier's symbol plus the `SyntaxContext` the
/// resolver assigned to the specific binding it refers to. Two idents with
/// the same `sym` but different `ctxt` refer to different bindings (e.g. an
/// import and a shadowing local).
type BindKey = (Atom, SyntaxContext);

/// What kind of @meonode/ui factory a resolved binding refers to, and where
/// its `props` argument lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateKind {
    /// A known HTML factory (imported directly, e.g. `Div`, `P`, or an
    /// aliased/derived binding of one).
    Html { children_first: bool },
    /// The `Node(element, props, deps)` factory.
    Node,
}

impl CandidateKind {
    /// The 0-based argument index of the `props` object for a call using
    /// this factory kind.
    fn props_arg_idx(self) -> usize {
        match self {
            CandidateKind::Html {
                children_first: false,
            } => 0,
            CandidateKind::Html {
                children_first: true,
            } => 1,
            CandidateKind::Node => 1,
        }
    }
}

/// What an import binding from `@meonode/ui`/`@meonode/ui/client` resolves
/// to. `Creator` bindings (`createNode`/`createChildrenFirstNode` imports)
/// are not call-site candidates themselves — they're only meaningful when
/// used to derive a local factory (`const X = createNode(...)`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImportBinding {
    Candidate(CandidateKind),
    Creator { children_first: bool },
}

/// Why a call site that referenced a tracked (or shadowed) factory binding
/// was not classified as compilable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BailReason {
    /// The callee's `sym` matches a known factory/creator name, but its
    /// resolved `SyntaxContext` doesn't match any tracked binding — i.e. the
    /// name is shadowed by a local declaration, or otherwise not the
    /// `@meonode/ui` import it looks like.
    ShadowedOrUnbound,
    /// The callee was a member expression (`Namespace.Div(...)`) whose
    /// object resolved to a `import * as Namespace from '@meonode/ui'`
    /// binding.
    NamespaceImport,
    /// Zero args, or the props argument position is missing entirely.
    /// Technically still "a candidate call", but there's nothing to
    /// partition, so it's treated as a bail (left untouched).
    MissingPropsArg,
    /// The props argument is present but isn't (syntactically) a plain
    /// object literal — e.g. an identifier, a call, a ternary, a member
    /// expression, or a spread argument.
    NotObjectLiteral,
    /// The object literal contains a spread property (`{ ...rest }`).
    SpreadProp,
    /// A property has a computed key (`{ [k]: v }`).
    ComputedKey,
    /// A property has a numeric (or bigint) literal key.
    NumericKey,
    /// A property has a string literal key that isn't identifier-like
    /// (e.g. `{ 'foo-bar': 1 }`).
    NonIdentifierStringKey,
    /// The object literal contains a getter or setter accessor property.
    GetterSetterProp,
    /// The object literal contains a shorthand method (`{ foo() {} }`).
    MethodProp,
    /// Any other property kind not covered above (currently only
    /// `Prop::Assign`, which is not valid in a normal object literal
    /// expression but is handled defensively).
    UnsupportedPropKind,
    /// The object literal already has a `__meo$` key (this call site has
    /// presumably already been compiled).
    ExistingMarker,
}

/// The detection outcome for a single call expression.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// This call site is a compilable @meonode/ui factory call; its props
    /// object literal is at argument index `props_arg_idx`. Task 9 consumes
    /// this to drive the actual rewrite.
    Compilable { props_arg_idx: usize },
    /// This call site is either not a factory call at all, or is one that
    /// can't (or shouldn't) be rewritten. See [`BailReason`] for why.
    Bail(BailReason),
}

/// A recorded detection outcome for one call expression, keyed by its span
/// so Task 9's rewrite pass can correlate decisions back to AST nodes.
///
/// `lib.rs::process_transform` currently discards the returned decisions
/// (Task 8 is detection-only — see module docs), so these fields have no
/// non-test reader yet; Task 9's rewrite pass is the first real consumer.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct CallSiteDecision {
    pub span: Span,
    pub decision: Decision,
}

fn is_ui_module(src: &Str) -> bool {
    src.value == UI_MODULE || src.value == UI_MODULE_CLIENT
}

/// Pass 1: collects `@meonode/ui`/`@meonode/ui/client` import bindings.
///
/// Runs as its own traversal (rather than being folded into the main
/// detector) so that local-factory derivation (pass 2) and call
/// classification (pass 3) always see the complete import table regardless
/// of where in the file the imports happen to be written.
#[derive(Default)]
struct ImportCollector {
    bindings: HashMap<BindKey, ImportBinding>,
    namespace_imports: HashSet<BindKey>,
}

impl Visit for ImportCollector {
    fn visit_import_decl(&mut self, n: &ImportDecl) {
        if n.type_only || !is_ui_module(&n.src) {
            return;
        }

        for spec in &n.specifiers {
            match spec {
                ImportSpecifier::Named(named) => {
                    if named.is_type_only {
                        continue;
                    }
                    let imported_name: &Atom = match &named.imported {
                        Some(ModuleExportName::Ident(id)) => &id.sym,
                        // A string module export name (`import { "x" as y }`)
                        // can never spell a valid JS identifier factory name;
                        // nothing in @meonode/ui is imported this way.
                        Some(ModuleExportName::Str(_)) => continue,
                        None => &named.local.sym,
                    };
                    let key = (named.local.sym.clone(), named.local.ctxt);

                    if imported_name.as_ref() == NODE_NAME {
                        self.bindings
                            .insert(key, ImportBinding::Candidate(CandidateKind::Node));
                    } else if imported_name.as_ref() == CREATE_NODE_NAME {
                        self.bindings.insert(
                            key,
                            ImportBinding::Creator {
                                children_first: false,
                            },
                        );
                    } else if imported_name.as_ref() == CREATE_CHILDREN_FIRST_NODE_NAME {
                        self.bindings.insert(
                            key,
                            ImportBinding::Creator {
                                children_first: true,
                            },
                        );
                    } else if let Some(children_first) =
                        factory_children_first(imported_name.as_ref())
                    {
                        self.bindings.insert(
                            key,
                            ImportBinding::Candidate(CandidateKind::Html { children_first }),
                        );
                    }
                }
                ImportSpecifier::Namespace(ns) => {
                    self.namespace_imports
                        .insert((ns.local.sym.clone(), ns.local.ctxt));
                }
                ImportSpecifier::Default(_) => {
                    // @meonode/ui has no default export used as a factory;
                    // nothing to track.
                }
            }
        }
    }
}

/// Pass 2: derives local factory bindings from same-file
/// `const X = createNode(...)` / `const X = createChildrenFirstNode(...)`
/// declarations, where the creator identifier itself resolves to a tracked
/// `@meonode/ui` import (pass 1's output).
struct LocalFactoryCollector<'a> {
    imports: &'a HashMap<BindKey, ImportBinding>,
    locals: HashMap<BindKey, CandidateKind>,
}

impl Visit for LocalFactoryCollector<'_> {
    fn visit_var_decl(&mut self, n: &VarDecl) {
        if n.kind == VarDeclKind::Const {
            for decl in &n.decls {
                self.collect_declarator(decl);
            }
        }
        n.visit_children_with(self);
    }
}

impl LocalFactoryCollector<'_> {
    fn collect_declarator(&mut self, decl: &VarDeclarator) {
        let Pat::Ident(binding_ident) = &decl.name else {
            return;
        };
        let Some(init) = &decl.init else {
            return;
        };
        let Expr::Call(call) = &**init else {
            return;
        };
        let Callee::Expr(callee_expr) = &call.callee else {
            return;
        };
        let Expr::Ident(callee_ident) = &**callee_expr else {
            return;
        };

        let callee_key = (callee_ident.sym.clone(), callee_ident.ctxt);
        if let Some(ImportBinding::Creator { children_first }) = self.imports.get(&callee_key) {
            let local_key = (binding_ident.id.sym.clone(), binding_ident.id.ctxt);
            self.locals.insert(
                local_key,
                CandidateKind::Html {
                    children_first: *children_first,
                },
            );
        }
    }
}

/// Pass 3: walks every call expression in the program and classifies it
/// against the merged binding table built from passes 1 and 2.
struct Detector {
    bindings: HashMap<BindKey, CandidateKind>,
    /// Every `sym` that has at least one tracked binding somewhere in the
    /// file (regardless of `ctxt`) — used to distinguish "not related to
    /// @meonode/ui at all" (no decision recorded) from "shadowed" (bail).
    tracked_syms: HashSet<Atom>,
    namespace_imports: HashSet<BindKey>,
    decisions: Vec<CallSiteDecision>,
}

impl Visit for Detector {
    fn visit_call_expr(&mut self, call: &CallExpr) {
        if let Some(decision) = self.classify_call(call) {
            self.decisions.push(CallSiteDecision {
                span: call.span,
                decision,
            });
        }
        call.visit_children_with(self);
    }
}

impl Detector {
    fn classify_call(&self, call: &CallExpr) -> Option<Decision> {
        let Callee::Expr(callee_expr) = &call.callee else {
            return None;
        };

        match &**callee_expr {
            Expr::Ident(id) => {
                let key = (id.sym.clone(), id.ctxt);
                if let Some(kind) = self.bindings.get(&key).copied() {
                    Some(classify_props(call, kind.props_arg_idx()))
                } else if self.tracked_syms.contains(&id.sym) {
                    Some(Decision::Bail(BailReason::ShadowedOrUnbound))
                } else {
                    None
                }
            }
            Expr::Member(member) => {
                if let Expr::Ident(obj_id) = &*member.obj {
                    let key = (obj_id.sym.clone(), obj_id.ctxt);
                    if self.namespace_imports.contains(&key) {
                        return Some(Decision::Bail(BailReason::NamespaceImport));
                    }
                }
                None
            }
            _ => None,
        }
    }
}

fn classify_props(call: &CallExpr, props_arg_idx: usize) -> Decision {
    let Some(arg) = call.args.get(props_arg_idx) else {
        return Decision::Bail(BailReason::MissingPropsArg);
    };
    if arg.spread.is_some() {
        return Decision::Bail(BailReason::NotObjectLiteral);
    }

    match unwrap_parens(&arg.expr) {
        Expr::Object(obj) => match validate_object(obj) {
            Some(reason) => Decision::Bail(reason),
            None => Decision::Compilable { props_arg_idx },
        },
        _ => Decision::Bail(BailReason::NotObjectLiteral),
    }
}

fn unwrap_parens(mut expr: &Expr) -> &Expr {
    while let Expr::Paren(paren) = expr {
        expr = &paren.expr;
    }
    expr
}

/// Validates an object literal as a compilable props object. Returns `None`
/// if it's clean, or the first disqualifying [`BailReason`] found.
///
/// The existing-marker check runs over every property first (independent of
/// structural validity) so a call site that already has `__meo$` is always
/// reported as [`BailReason::ExistingMarker`], even if it happens to also
/// contain some other bail-worthy shape.
fn validate_object(obj: &ObjectLit) -> Option<BailReason> {
    if obj.props.iter().any(is_marker_prop) {
        return Some(BailReason::ExistingMarker);
    }

    for prop_or_spread in &obj.props {
        match prop_or_spread {
            PropOrSpread::Spread(_) => return Some(BailReason::SpreadProp),
            PropOrSpread::Prop(prop) => match &**prop {
                Prop::Shorthand(_) => {}
                Prop::KeyValue(kv) => {
                    if let Some(reason) = validate_key(&kv.key) {
                        return Some(reason);
                    }
                }
                Prop::Getter(_) | Prop::Setter(_) => {
                    return Some(BailReason::GetterSetterProp);
                }
                Prop::Method(_) => return Some(BailReason::MethodProp),
                Prop::Assign(_) => return Some(BailReason::UnsupportedPropKind),
            },
        }
    }

    None
}

fn is_marker_prop(prop_or_spread: &PropOrSpread) -> bool {
    let PropOrSpread::Prop(prop) = prop_or_spread else {
        return false;
    };
    match &**prop {
        Prop::Shorthand(id) => id.sym.as_ref() == MARKER_KEY,
        Prop::KeyValue(kv) => match &kv.key {
            PropName::Ident(id) => id.sym.as_ref() == MARKER_KEY,
            PropName::Str(s) => s.value == MARKER_KEY,
            _ => false,
        },
        _ => false,
    }
}

fn validate_key(key: &PropName) -> Option<BailReason> {
    match key {
        PropName::Ident(_) => None,
        PropName::Str(s) => match s.value.as_str() {
            Some(value) if is_identifier_like(value) => None,
            _ => Some(BailReason::NonIdentifierStringKey),
        },
        PropName::Num(_) | PropName::BigInt(_) => Some(BailReason::NumericKey),
        PropName::Computed(_) => Some(BailReason::ComputedKey),
    }
}

fn is_identifier_like(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c == '_' || c == '$' || c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c == '_' || c == '$' || c.is_ascii_alphanumeric())
}

/// Runs the full three-pass detector over `program` and returns every
/// recorded call-site decision, in traversal order.
///
/// This is read-only: it never mutates `program`. The returned decisions are
/// consumed by Task 9's rewrite pass; for now nothing acts on them.
pub fn detect(program: &Program) -> Vec<CallSiteDecision> {
    let mut imports = ImportCollector::default();
    program.visit_with(&mut imports);

    let mut locals = LocalFactoryCollector {
        imports: &imports.bindings,
        locals: HashMap::new(),
    };
    program.visit_with(&mut locals);

    let mut bindings: HashMap<BindKey, CandidateKind> = HashMap::new();
    for (key, binding) in &imports.bindings {
        if let ImportBinding::Candidate(kind) = binding {
            bindings.insert(key.clone(), *kind);
        }
    }
    bindings.extend(locals.locals);

    let tracked_syms: HashSet<Atom> = bindings.keys().map(|(sym, _)| sym.clone()).collect();

    let mut detector = Detector {
        bindings,
        tracked_syms,
        namespace_imports: imports.namespace_imports,
        decisions: Vec::new(),
    };
    program.visit_with(&mut detector);
    detector.decisions
}

/// Public entry point, wired into `lib.rs::process_transform`.
///
/// Runs detection over `program` and returns the recorded decisions.
/// Intentionally does not mutate `program` — the rewrite that acts on these
/// decisions lands in Task 9.
pub fn transform_program(program: &Program) -> Vec<CallSiteDecision> {
    detect(program)
}

#[cfg(test)]
mod tests {
    use super::*;
    use swc_core::common::sync::Lrc;
    use swc_core::common::{FileName, Globals, Mark, SourceMap, GLOBALS};
    use swc_core::ecma::ast::EsVersion;
    use swc_core::ecma::parser::lexer::Lexer;
    use swc_core::ecma::parser::{EsSyntax, Parser, StringInput, Syntax};
    use swc_core::ecma::transforms::base::resolver;
    use swc_core::ecma::visit::VisitMutWith;

    /// Parses `src` as an ES module, runs the resolver (mirroring what the
    /// swc plugin host does before invoking the plugin), then runs detection
    /// and returns the recorded [`Decision`]s in traversal order.
    fn decisions_for(src: &str) -> Vec<Decision> {
        GLOBALS.set(&Globals::new(), || {
            let cm: Lrc<SourceMap> = Default::default();
            let fm = cm.new_source_file(Lrc::new(FileName::Anon), src.to_string());

            let lexer = Lexer::new(
                Syntax::Es(EsSyntax::default()),
                EsVersion::EsNext,
                StringInput::from(&*fm),
                None,
            );
            let mut parser = Parser::new_from(lexer);
            let mut program = parser
                .parse_program()
                .unwrap_or_else(|e| panic!("failed to parse test snippet: {e:?}\n---\n{src}"));

            let unresolved_mark = Mark::new();
            let top_level_mark = Mark::new();
            program.visit_mut_with(&mut resolver(unresolved_mark, top_level_mark, false));

            detect(&program).into_iter().map(|d| d.decision).collect()
        })
    }

    #[test]
    fn named_import_div_is_compilable_at_arg_0() {
        let decisions = decisions_for(
            r#"
            import { Div } from '@meonode/ui';
            Div({ padding: 1 });
            "#,
        );
        assert_eq!(decisions, vec![Decision::Compilable { props_arg_idx: 0 }]);
    }

    #[test]
    fn aliased_import_is_compilable() {
        let decisions = decisions_for(
            r#"
            import { Div as D } from '@meonode/ui';
            D({ padding: 1 });
            "#,
        );
        assert_eq!(decisions, vec![Decision::Compilable { props_arg_idx: 0 }]);
    }

    #[test]
    fn children_first_factory_is_compilable_at_arg_1() {
        let decisions = decisions_for(
            r#"
            import { P } from '@meonode/ui';
            P('hello', { color: 'red' });
            "#,
        );
        assert_eq!(decisions, vec![Decision::Compilable { props_arg_idx: 1 }]);
    }

    #[test]
    fn node_factory_is_compilable_at_arg_1() {
        let decisions = decisions_for(
            r#"
            import { Node } from '@meonode/ui';
            Node('div', { padding: 1 });
            "#,
        );
        assert_eq!(decisions, vec![Decision::Compilable { props_arg_idx: 1 }]);
    }

    #[test]
    fn client_subpath_import_is_tracked() {
        let decisions = decisions_for(
            r#"
            import { Div } from '@meonode/ui/client';
            Div({ padding: 1 });
            "#,
        );
        assert_eq!(decisions, vec![Decision::Compilable { props_arg_idx: 0 }]);
    }

    #[test]
    fn local_create_node_binding_is_compilable() {
        let decisions = decisions_for(
            r#"
            import { createNode } from '@meonode/ui';
            const Box = createNode('div');
            Box({ padding: 1 });
            "#,
        );
        // Two calls happen here: `createNode('div')` itself (not a
        // candidate — creators aren't callable candidates) and
        // `Box({...})` (compilable). Only one decision should be recorded.
        assert_eq!(decisions, vec![Decision::Compilable { props_arg_idx: 0 }]);
    }

    #[test]
    fn local_create_children_first_node_binding_uses_arg_1() {
        let decisions = decisions_for(
            r#"
            import { createChildrenFirstNode } from '@meonode/ui';
            const Text = createChildrenFirstNode('p');
            Text('hi', { color: 'red' });
            "#,
        );
        assert_eq!(decisions, vec![Decision::Compilable { props_arg_idx: 1 }]);
    }

    #[test]
    fn shadowed_import_bails() {
        let decisions = decisions_for(
            r#"
            import { Div } from '@meonode/ui';
            function f() {
              const Div = something();
              return Div({ padding: 1 });
            }
            "#,
        );
        assert_eq!(
            decisions,
            vec![Decision::Bail(BailReason::ShadowedOrUnbound)]
        );
    }

    #[test]
    fn namespace_import_bails() {
        let decisions = decisions_for(
            r#"
            import * as M from '@meonode/ui';
            M.Div({ padding: 1 });
            "#,
        );
        assert_eq!(decisions, vec![Decision::Bail(BailReason::NamespaceImport)]);
    }

    #[test]
    fn non_object_props_bails() {
        let decisions = decisions_for(
            r#"
            import { Div } from '@meonode/ui';
            const props = { padding: 1 };
            Div(props);
            "#,
        );
        assert_eq!(
            decisions,
            vec![Decision::Bail(BailReason::NotObjectLiteral)]
        );
    }

    #[test]
    fn spread_in_props_bails() {
        let decisions = decisions_for(
            r#"
            import { Div } from '@meonode/ui';
            const rest = {};
            Div({ ...rest, padding: 1 });
            "#,
        );
        assert_eq!(decisions, vec![Decision::Bail(BailReason::SpreadProp)]);
    }

    #[test]
    fn computed_key_bails() {
        let decisions = decisions_for(
            r#"
            import { Div } from '@meonode/ui';
            const key = 'padding';
            Div({ [key]: 1 });
            "#,
        );
        assert_eq!(decisions, vec![Decision::Bail(BailReason::ComputedKey)]);
    }

    #[test]
    fn identifier_like_string_key_is_fine() {
        let decisions = decisions_for(
            r#"
            import { Div } from '@meonode/ui';
            Div({ "padding": 1 });
            "#,
        );
        assert_eq!(decisions, vec![Decision::Compilable { props_arg_idx: 0 }]);
    }

    #[test]
    fn non_identifier_like_string_key_bails() {
        let decisions = decisions_for(
            r#"
            import { Div } from '@meonode/ui';
            Div({ "foo-bar": 1 });
            "#,
        );
        assert_eq!(
            decisions,
            vec![Decision::Bail(BailReason::NonIdentifierStringKey)]
        );
    }

    #[test]
    fn numeric_key_bails() {
        let decisions = decisions_for(
            r#"
            import { Div } from '@meonode/ui';
            Div({ 0: 1 });
            "#,
        );
        assert_eq!(decisions, vec![Decision::Bail(BailReason::NumericKey)]);
    }

    #[test]
    fn getter_setter_prop_bails() {
        let decisions = decisions_for(
            r#"
            import { Div } from '@meonode/ui';
            Div({ get padding() { return 1; } });
            "#,
        );
        assert_eq!(
            decisions,
            vec![Decision::Bail(BailReason::GetterSetterProp)]
        );
    }

    #[test]
    fn method_shorthand_key_bails() {
        let decisions = decisions_for(
            r#"
            import { Div } from '@meonode/ui';
            Div({ onClick() {} });
            "#,
        );
        assert_eq!(decisions, vec![Decision::Bail(BailReason::MethodProp)]);
    }

    #[test]
    fn function_value_prop_is_fine() {
        // A method *shorthand* bails, but a plain key/value pair whose
        // value happens to be a function expression is just a normal
        // (dynamic) prop — not a bail.
        let decisions = decisions_for(
            r#"
            import { Div } from '@meonode/ui';
            Div({ onClick: function () {} });
            "#,
        );
        assert_eq!(decisions, vec![Decision::Compilable { props_arg_idx: 0 }]);
    }

    #[test]
    fn existing_marker_bails() {
        let decisions = decisions_for(
            r#"
            import { Div } from '@meonode/ui';
            Div({ __meo$: 1, padding: 1 });
            "#,
        );
        assert_eq!(decisions, vec![Decision::Bail(BailReason::ExistingMarker)]);
    }

    #[test]
    fn missing_props_arg_bails() {
        let decisions = decisions_for(
            r#"
            import { Div } from '@meonode/ui';
            Div();
            "#,
        );
        assert_eq!(decisions, vec![Decision::Bail(BailReason::MissingPropsArg)]);
    }

    #[test]
    fn unrelated_calls_are_ignored() {
        let decisions = decisions_for(
            r#"
            import { Div } from '@meonode/ui';
            console.log('hello');
            someOtherFunction({ padding: 1 });
            "#,
        );
        assert_eq!(decisions, vec![]);
    }
}
