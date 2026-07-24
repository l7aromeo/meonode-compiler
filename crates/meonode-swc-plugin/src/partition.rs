//! Task 9: prop partitioning + call-site key emission.
//!
//! Rewrites every `Decision::Compilable` call site's props object literal
//! (see `detect.rs`) into `@meonode/ui`'s pre-partitioned marker-prop shape:
//!
//! ```text
//! Div({ padding: '20px', width, onClick: handler, css: {...}, children: [A, B] })
//! // becomes:
//! Div({
//!   __meo$: 1,
//!   c: { padding: '20px', width },
//!   d: { onClick: handler },
//!   k: 'm1a2b3c',
//!   dyn: ['width', 'onClick'],
//!   css: {...},
//!   children: [A, B],
//! })
//! ```
//!
//! Detection (structural validity + effect-safety of every prop value) has
//! already happened by the time this module runs — see `detect.rs` and
//! `effect.rs`. This module only decides *where each already-accepted prop
//! goes* and builds the replacement object literal; it never re-derives
//! bail decisions.

use std::collections::HashMap;
use std::mem;

use swc_core::common::Span;
use swc_core::ecma::ast::*;
use swc_core::ecma::atoms::Atom;
use swc_core::ecma::visit::{VisitMut, VisitMutWith};

use crate::css_props::is_css_prop;
use crate::detect::{self, Decision};
use crate::effect::is_static_literal;

const MARKER_KEY: &str = "__meo$";

/// Prop keys that always stay top-level, untouched by `c`/`d` bucketing, no
/// matter what `is_css_prop` says about them (some, like `css` itself, would
/// otherwise look unrelated to CSS but still need special runtime handling;
/// `theme`/`as`/`disableEmotion` are meonode-specific config, not DOM props).
const SPECIAL_KEYS: &[&str] = &[
    "css",
    "props",
    "ref",
    "key",
    "children",
    "as",
    "theme",
    "disableEmotion",
];

fn is_special_key(name: &str) -> bool {
    SPECIAL_KEYS.contains(&name)
}

/// FNV-1a 64-bit hash (the standard offset basis / prime for the 64-bit
/// variant). Chosen over a crate dependency since it's a handful of lines and
/// we don't need cryptographic properties — just a fast, stable, well-mixed
/// hash for a compact call-site id.
fn fnv1a64(bytes: &[u8]) -> u64 {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    bytes.iter().fold(OFFSET_BASIS, |hash, &b| {
        (hash ^ b as u64).wrapping_mul(PRIME)
    })
}

const BASE36_DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";

fn to_base36(mut n: u64) -> String {
    if n == 0 {
        return "0".to_string();
    }
    let mut buf = Vec::with_capacity(13); // u64::MAX is 13 base-36 digits
    while n > 0 {
        buf.push(BASE36_DIGITS[(n % 36) as usize]);
        n /= 36;
    }
    buf.reverse();
    // Safety: every byte pushed above comes from BASE36_DIGITS, which is
    // pure ASCII.
    String::from_utf8(buf).expect("base36 digits are always valid ASCII/UTF-8")
}

/// Computes the deterministic `k` value for a call site: `m` followed by the
/// base36 encoding of the FNV-1a 64-bit hash of `filename:span_lo:span_hi`.
///
/// **Determinism scope**: stable across repeated compiles of the same file
/// content, since it's keyed on the file's own byte offsets rather than on
/// any incremental/global counter. It is *not* stable across edits that move
/// this call site within the file (or rename the file) — a shifted span or
/// changed filename changes `k`. This is by design: `k` only needs to be
/// stable for a given piece of source text, not across arbitrary edits.
fn call_site_key(filename: &str, span: Span) -> String {
    let input = format!("{filename}:{}:{}", span.lo.0, span.hi.0);
    format!("m{}", to_base36(fnv1a64(input.as_bytes())))
}

fn ident_key(name: &str) -> PropName {
    PropName::Ident(IdentName::new(Atom::from(name), swc_core::common::DUMMY_SP))
}

fn kv_prop(name: &str, value: Expr) -> PropOrSpread {
    PropOrSpread::Prop(Box::new(Prop::KeyValue(KeyValueProp {
        key: ident_key(name),
        value: Box::new(value),
    })))
}

fn marker_prop() -> PropOrSpread {
    kv_prop(
        MARKER_KEY,
        Expr::Lit(Lit::Num(Number {
            span: swc_core::common::DUMMY_SP,
            value: 1.0,
            raw: None,
        })),
    )
}

fn bucket_prop(name: &str, props: Vec<PropOrSpread>) -> PropOrSpread {
    kv_prop(
        name,
        Expr::Object(ObjectLit {
            span: swc_core::common::DUMMY_SP,
            props,
        }),
    )
}

fn key_prop(filename: &str, span: Span) -> PropOrSpread {
    kv_prop(
        "k",
        Expr::Lit(Lit::Str(Str::from(Atom::from(call_site_key(
            filename, span,
        ))))),
    )
}

fn dyn_prop(names: Vec<Atom>) -> PropOrSpread {
    let elems = names
        .into_iter()
        .map(|name| {
            Some(ExprOrSpread {
                spread: None,
                expr: Box::new(Expr::Lit(Lit::Str(Str::from(name)))),
            })
        })
        .collect();
    kv_prop(
        "dyn",
        Expr::Array(ArrayLit {
            span: swc_core::common::DUMMY_SP,
            elems,
        }),
    )
}

/// Extracts the name of an (already-validated) prop key. By the time a call
/// site reaches this module it's `Decision::Compilable`, which means
/// `detect::validate_object` has already rejected computed/numeric/bigint
/// keys and non-identifier-like string keys — only `Ident` and identifier-
/// like `Str` keys survive.
fn prop_name_atom(key: &PropName) -> Atom {
    match key {
        PropName::Ident(id) => id.sym.clone(),
        PropName::Str(s) => Atom::from(s.value.as_str().unwrap_or_default()),
        _ => unreachable!(
            "non-ident/str prop keys are rejected by detect::validate_object \
             before a call site becomes Decision::Compilable"
        ),
    }
}

/// Partitions `obj`'s props into `__meo$`/`c`/`d`/`k`/`dyn` plus any special
/// keys, and rewrites `obj.props` in place. `span` is the *call expression's*
/// span (not the object literal's), matching Task 9's spec for `k`.
///
/// Emit order: `__meo$`, `c` (omitted if empty), `d` (omitted if empty), `k`,
/// `dyn` (omitted if empty), then every special-key prop in its original
/// source order. Bucket membership: a non-special key goes to `c` if
/// `css_props::is_css_prop` recognizes it, otherwise `d`. A bucketed prop's
/// name is added to `dyn` unless its value is a "plain literal" per
/// `effect::is_static_literal` (shorthand props are always dynamic, since
/// their implicit value is an identifier).
/// Appends `name` to `dyn_names` unless it's already there. A source object
/// can legally repeat a key (`{ onClick: a, onClick: b }` — last one wins at
/// runtime, same as any JS object literal, including once both copies land
/// in the same `d`/`c` bucket object here), and each occurrence independently
/// qualifies as dynamic, but the runtime only needs the *name* once to know
/// to look it up — a duplicate entry would just be redundant, not wrong.
/// First-occurrence position is preserved (later duplicates are dropped, not
/// reordered) so this stays consistent with "preserve source order" for the
/// non-duplicate case.
fn push_dyn_name(dyn_names: &mut Vec<Atom>, name: Atom) {
    if !dyn_names.contains(&name) {
        dyn_names.push(name);
    }
}

fn rewrite_object(obj: &mut ObjectLit, filename: &str, span: Span) {
    let old_props = mem::take(&mut obj.props);

    let mut c_props: Vec<PropOrSpread> = Vec::new();
    let mut d_props: Vec<PropOrSpread> = Vec::new();
    let mut special_props: Vec<PropOrSpread> = Vec::new();
    let mut dyn_names: Vec<Atom> = Vec::new();

    for prop_or_spread in old_props {
        let PropOrSpread::Prop(prop) = prop_or_spread else {
            unreachable!("spread props are rejected by detect::validate_object");
        };

        match &*prop {
            Prop::Shorthand(ident) => {
                let name = ident.sym.clone();
                if is_special_key(name.as_ref()) {
                    special_props.push(PropOrSpread::Prop(prop));
                    continue;
                }
                // Shorthand's implicit value is the identifier itself, which
                // is always dynamic (see `effect::is_static_literal`).
                push_dyn_name(&mut dyn_names, name.clone());
                if is_css_prop(name.as_ref()) {
                    c_props.push(PropOrSpread::Prop(prop));
                } else {
                    d_props.push(PropOrSpread::Prop(prop));
                }
            }
            Prop::KeyValue(kv) => {
                let name = prop_name_atom(&kv.key);
                if is_special_key(name.as_ref()) {
                    special_props.push(PropOrSpread::Prop(prop));
                    continue;
                }
                if !is_static_literal(&kv.value) {
                    push_dyn_name(&mut dyn_names, name.clone());
                }
                if is_css_prop(name.as_ref()) {
                    c_props.push(PropOrSpread::Prop(prop));
                } else {
                    d_props.push(PropOrSpread::Prop(prop));
                }
            }
            _ => unreachable!("only Shorthand/KeyValue props survive detect::validate_object"),
        }
    }

    let mut new_props = Vec::with_capacity(
        3 + c_props.is_empty() as usize
            + d_props.is_empty() as usize
            + dyn_names.is_empty() as usize
            + special_props.len(),
    );

    new_props.push(marker_prop());
    if !c_props.is_empty() {
        new_props.push(bucket_prop("c", c_props));
    }
    if !d_props.is_empty() {
        new_props.push(bucket_prop("d", d_props));
    }
    new_props.push(key_prop(filename, span));
    if !dyn_names.is_empty() {
        new_props.push(dyn_prop(dyn_names));
    }
    new_props.extend(special_props);

    obj.props = new_props;
}

fn unwrap_parens_mut(mut expr: &mut Expr) -> &mut Expr {
    while let Expr::Paren(paren) = expr {
        expr = &mut paren.expr;
    }
    expr
}

/// A `VisitMut` pass that rewrites every call site recorded as
/// `Decision::Compilable` by a prior `detect::detect` run, keyed by call
/// span (byte offsets) since decisions are computed before this pass starts
/// mutating anything.
struct Rewriter<'a> {
    filename: &'a str,
    /// Compilable call spans (as `(lo, hi)` byte offset pairs — `Span`
    /// itself needn't implement `Hash`/`Eq` for this to work) to their
    /// props argument index.
    compilable: HashMap<(u32, u32), usize>,
}

impl VisitMut for Rewriter<'_> {
    fn visit_mut_call_expr(&mut self, call: &mut CallExpr) {
        // Recurse first: nested factory calls (e.g. inside a `children`
        // array that's itself a plain identifier reference, or arguments of
        // an unrelated call) get their own independent rewrite; order
        // doesn't matter for correctness since matching is by span, not by
        // structure, but bottom-up is the natural traversal order.
        call.visit_mut_children_with(self);

        let span = call.span;
        let Some(&props_arg_idx) = self.compilable.get(&(span.lo.0, span.hi.0)) else {
            return;
        };
        let Some(arg) = call.args.get_mut(props_arg_idx) else {
            return;
        };
        let Expr::Object(obj) = unwrap_parens_mut(&mut arg.expr) else {
            return;
        };
        rewrite_object(obj, self.filename, span);
    }
}

/// Runs detection over `program`, then rewrites every `Decision::Compilable`
/// call site's props object literal into the pre-partitioned marker-prop
/// shape (see module docs). `filename` is used to compute each call site's
/// `k` value — pass the real source filename in production (see
/// `lib.rs::process_transform`) or a fixed name in tests.
///
/// No-op (and skips the rewrite pass entirely) if there are no compilable
/// call sites, so files untouched by @meonode/ui factories pay no additional
/// traversal cost beyond detection itself.
pub fn transform_program(program: &mut Program, filename: &str) {
    let compilable: HashMap<(u32, u32), usize> = detect::detect(&*program)
        .into_iter()
        .filter_map(|d| match d.decision {
            Decision::Compilable { props_arg_idx } => {
                Some(((d.span.lo.0, d.span.hi.0), props_arg_idx))
            }
            Decision::Bail(_) => None,
        })
        .collect();

    if compilable.is_empty() {
        return;
    }

    let mut rewriter = Rewriter {
        filename,
        compilable,
    };
    program.visit_mut_with(&mut rewriter);
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
    use swc_core::ecma::visit::{Visit, VisitWith};

    /// Parses `src`, runs the resolver (mirroring the real plugin host),
    /// runs `transform_program`, then collects the object literal from the
    /// first argument of every call expression, in traversal order. AST
    /// inspection (rather than round-tripping through codegen text) keeps
    /// these assertions independent of formatting/whitespace choices.
    fn transformed_objects(src: &str, filename: &str) -> Vec<ObjectLit> {
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

            transform_program(&mut program, filename);

            struct Collector {
                found: Vec<ObjectLit>,
            }
            impl Visit for Collector {
                fn visit_call_expr(&mut self, call: &CallExpr) {
                    if let Some(arg) = call.args.first() {
                        if let Expr::Object(obj) = &*arg.expr {
                            self.found.push(obj.clone());
                        }
                    }
                    call.visit_children_with(self);
                }
            }
            let mut collector = Collector { found: Vec::new() };
            program.visit_with(&mut collector);
            collector.found
        })
    }

    fn transformed_object(src: &str, filename: &str) -> ObjectLit {
        let mut objs = transformed_objects(src, filename);
        assert_eq!(
            objs.len(),
            1,
            "expected exactly one call with an object arg"
        );
        objs.remove(0)
    }

    fn find_prop<'a>(obj: &'a ObjectLit, name: &str) -> &'a Expr {
        obj.props
            .iter()
            .find_map(|p| {
                let PropOrSpread::Prop(prop) = p else {
                    return None;
                };
                let Prop::KeyValue(kv) = &**prop else {
                    return None;
                };
                let PropName::Ident(id) = &kv.key else {
                    return None;
                };
                (id.sym.as_ref() == name).then_some(&*kv.value)
            })
            .unwrap_or_else(|| panic!("expected prop `{name}` to exist in {obj:#?}"))
    }

    fn has_prop(obj: &ObjectLit, name: &str) -> bool {
        obj.props.iter().any(|p| {
            let PropOrSpread::Prop(prop) = p else {
                return false;
            };
            match &**prop {
                Prop::KeyValue(kv) => {
                    matches!(&kv.key, PropName::Ident(id) if id.sym.as_ref() == name)
                }
                Prop::Shorthand(id) => id.sym.as_ref() == name,
                _ => false,
            }
        })
    }

    fn str_lit_value(expr: &Expr) -> String {
        let Expr::Lit(Lit::Str(s)) = expr else {
            panic!("expected a string literal, got {expr:?}");
        };
        s.value.as_str().unwrap().to_string()
    }

    fn dyn_names(obj: &ObjectLit) -> Vec<String> {
        let Expr::Array(arr) = find_prop(obj, "dyn") else {
            panic!("expected `dyn` to be an array literal");
        };
        arr.elems
            .iter()
            .map(|e| {
                let elem = e.as_ref().expect("no elisions expected in dyn");
                str_lit_value(&elem.expr)
            })
            .collect()
    }

    #[test]
    fn dyn_list_preserves_source_order() {
        let obj = transformed_object(
            r#"
            import { Div } from '@meonode/ui';
            Div({ onClick: handler, padding: '1px', width, color: someColor });
            "#,
            "test.tsx",
        );
        // `padding` is a static literal ('1px'), so it's excluded from
        // `dyn` even though it's bucketed; the rest, in source order.
        assert_eq!(dyn_names(&obj), vec!["onClick", "width", "color"]);
    }

    #[test]
    fn k_is_deterministic_for_identical_input() {
        let src = r#"
            import { Div } from '@meonode/ui';
            Div({ padding: 1 });
        "#;
        let obj1 = transformed_object(src, "same.tsx");
        let obj2 = transformed_object(src, "same.tsx");
        assert_eq!(
            str_lit_value(find_prop(&obj1, "k")),
            str_lit_value(find_prop(&obj2, "k"))
        );
    }

    #[test]
    fn k_differs_for_different_spans() {
        let objs = transformed_objects(
            r#"
            import { Div } from '@meonode/ui';
            Div({ padding: 1 });
            Div({ padding: 1 });
            "#,
            "test.tsx",
        );
        assert_eq!(objs.len(), 2);
        assert_ne!(
            str_lit_value(find_prop(&objs[0], "k")),
            str_lit_value(find_prop(&objs[1], "k"))
        );
    }

    #[test]
    fn k_differs_for_different_filenames() {
        let src = r#"
            import { Div } from '@meonode/ui';
            Div({ padding: 1 });
        "#;
        let obj1 = transformed_object(src, "a.tsx");
        let obj2 = transformed_object(src, "b.tsx");
        assert_ne!(
            str_lit_value(find_prop(&obj1, "k")),
            str_lit_value(find_prop(&obj2, "k"))
        );
    }

    #[test]
    fn bucket_membership_matches_css_props_set() {
        let obj = transformed_object(
            r#"
            import { Div } from '@meonode/ui';
            Div({ backgroundColor: 'red', onClick: handler, id: 'x' });
            "#,
            "test.tsx",
        );
        assert!(is_css_prop("backgroundColor"));
        assert!(!is_css_prop("onClick"));
        assert!(!is_css_prop("id"));

        let Expr::Object(c) = find_prop(&obj, "c") else {
            panic!("expected `c` to be an object literal");
        };
        assert!(has_prop(c, "backgroundColor"));
        assert!(!has_prop(c, "onClick"));
        assert!(!has_prop(c, "id"));

        let Expr::Object(d) = find_prop(&obj, "d") else {
            panic!("expected `d` to be an object literal");
        };
        assert!(has_prop(d, "onClick"));
        assert!(has_prop(d, "id"));
        assert!(!has_prop(d, "backgroundColor"));
    }

    #[test]
    fn non_css_only_props_omit_c_bucket() {
        let obj = transformed_object(
            r#"
            import { Div } from '@meonode/ui';
            Div({ onClick: handler, id: 'x' });
            "#,
            "test.tsx",
        );
        assert!(!has_prop(&obj, "c"));
        assert!(has_prop(&obj, "d"));
    }

    #[test]
    fn all_static_values_omit_dyn() {
        let obj = transformed_object(
            r#"
            import { Div } from '@meonode/ui';
            Div({ padding: '1px', id: 'x' });
            "#,
            "test.tsx",
        );
        assert!(!has_prop(&obj, "dyn"));
    }

    #[test]
    fn emit_order_is_marker_c_d_k_dyn_then_specials() {
        let obj = transformed_object(
            r#"
            import { Div } from '@meonode/ui';
            Div({ children: ['x'], padding: '1px', onClick: handler, width, css: { color: 'red' } });
            "#,
            "test.tsx",
        );
        let order: Vec<String> = obj
            .props
            .iter()
            .map(|p| {
                let PropOrSpread::Prop(prop) = p else {
                    panic!("expected only Prop entries");
                };
                match &**prop {
                    Prop::KeyValue(kv) => match &kv.key {
                        PropName::Ident(id) => id.sym.as_ref().to_string(),
                        _ => panic!("expected ident key"),
                    },
                    _ => panic!("expected only KeyValue entries"),
                }
            })
            .collect();
        // c/d bucket order matches spec; special keys (`children`, `css`)
        // keep their *original* relative source order (children before css)
        // at the tail, after __meo$/c/d/k/dyn.
        assert_eq!(
            order,
            vec!["__meo$", "c", "d", "k", "dyn", "children", "css"]
        );
    }

    /// Coordinator-requested check for the `children`-last exception
    /// (amending `3a061c0`): `css` and `children` are both special keys, and
    /// `children` is last in source order here — confirms it stays last
    /// after rewriting, which is exactly the invariant the exception in
    /// `detect::validate_object` relies on for safety.
    #[test]
    fn css_and_children_source_final_emits_children_last() {
        let obj = transformed_object(
            r#"
            import { Div } from '@meonode/ui';
            Div({ css: { color: 'red' }, padding: '1px', children: ['x'] });
            "#,
            "test.tsx",
        );
        let special_order: Vec<String> = obj
            .props
            .iter()
            .filter_map(|p| {
                let PropOrSpread::Prop(prop) = p else {
                    return None;
                };
                let Prop::KeyValue(kv) = &**prop else {
                    return None;
                };
                let PropName::Ident(id) = &kv.key else {
                    return None;
                };
                let name = id.sym.as_ref();
                (name != "__meo$" && name != "c" && name != "d" && name != "k" && name != "dyn")
                    .then(|| name.to_string())
            })
            .collect();
        assert_eq!(special_order, vec!["css", "children"]);
        assert_eq!(special_order.last(), Some(&"children".to_string()));
    }

    /// A source-final `children` value containing nested factory calls is
    /// effectful (the calls themselves aren't effect-free) but allowed by
    /// the `children`-last exception; those nested calls are independently
    /// `Decision::Compilable` call sites in their own right, so the same
    /// `VisitMut` pass must rewrite them too — not just the outer call.
    #[test]
    fn nested_factory_calls_in_children_are_independently_rewritten() {
        // Not `transformed_object` (which requires exactly one call with an
        // object-literal first arg): the nested `Div({ color: 'red' })`
        // inside `children` is itself such a call, so there are two. Take
        // the outer one (found first, in pre-order traversal) and reach
        // into `children` directly to check the nested calls.
        let objs = transformed_objects(
            r#"
            import { Div, P } from '@meonode/ui';
            Div({ padding: '1px', onClick: h, children: [Div({ color: 'red' }), P('x', { color: 'blue' })] });
            "#,
            "test.tsx",
        );
        assert_eq!(objs.len(), 2, "expected outer Div + nested Div objects");
        let obj = &objs[0];
        // The outer call itself compiled (has __meo$/c/d/k).
        assert!(has_prop(obj, "__meo$"));

        let Expr::Array(children) = find_prop(obj, "children") else {
            panic!("expected `children` to be an array literal");
        };
        assert_eq!(children.elems.len(), 2);

        let nested_div = &children.elems[0].as_ref().unwrap().expr;
        let Expr::Call(nested_div_call) = &**nested_div else {
            panic!("expected first child to still be a call expression");
        };
        let Some(arg0) = nested_div_call.args.first() else {
            panic!("expected nested Div call to have an arg");
        };
        let Expr::Object(nested_div_props) = &*arg0.expr else {
            panic!("expected nested Div's first arg to be an object literal");
        };
        assert!(
            has_prop(nested_div_props, "__meo$"),
            "nested Div({{ color: 'red' }}) should have been rewritten independently"
        );

        let nested_p = &children.elems[1].as_ref().unwrap().expr;
        let Expr::Call(nested_p_call) = &**nested_p else {
            panic!("expected second child to still be a call expression");
        };
        // P is children-first: arg 0 is the untouched children-first
        // argument ('x'), arg 1 is the props object that got rewritten.
        let Some(Expr::Lit(Lit::Str(_))) = nested_p_call.args.first().map(|a| &*a.expr) else {
            panic!("expected nested P's arg 0 to remain an untouched string literal");
        };
        let Some(arg1) = nested_p_call.args.get(1) else {
            panic!("expected nested P call to have a props arg");
        };
        let Expr::Object(nested_p_props) = &*arg1.expr else {
            panic!("expected nested P's second arg to be an object literal");
        };
        assert!(
            has_prop(nested_p_props, "__meo$"),
            "nested P('x', {{ color: 'blue' }}) should have been rewritten independently"
        );
    }

    /// A nested factory call doesn't have to be inside `children` to get
    /// independently rewritten — any arrow-expression prop value effect-free
    /// enough to be accepted (see `effect::is_effect_free`) can itself
    /// contain further call expressions in its body, and the `VisitMut` pass
    /// walks into function bodies just like any other subtree.
    #[test]
    fn nested_factory_call_inside_arrow_prop_value_is_independently_rewritten() {
        let objs = transformed_objects(
            r#"
            import { Div } from '@meonode/ui';
            Div({ onClick: () => Div({ color: 'red' }) });
            "#,
            "test.tsx",
        );
        // The outer `Div({ onClick: ... })` doesn't itself have an object
        // literal as its *first* arg's inner nested call — both calls here
        // have their object literal as arg 0, so both get collected.
        assert_eq!(objs.len(), 2);
        let outer = &objs[0];
        assert!(has_prop(outer, "__meo$"));

        let Expr::Object(d) = find_prop(outer, "d") else {
            panic!("expected `d` bucket to exist");
        };
        let Expr::Arrow(arrow) = find_prop(d, "onClick") else {
            panic!("expected onClick's value to still be an arrow expr");
        };
        let BlockStmtOrExpr::Expr(body) = &*arrow.body else {
            panic!("expected an expression arrow body");
        };
        let Expr::Call(nested_call) = &**body else {
            panic!("expected the arrow body to still be a call expression");
        };
        let Some(arg0) = nested_call.args.first() else {
            panic!("expected the nested call to have a props arg");
        };
        let Expr::Object(nested_props) = &*arg0.expr else {
            panic!("expected the nested call's arg to be an object literal");
        };
        assert!(
            has_prop(nested_props, "__meo$"),
            "nested Div({{ color: 'red' }}) inside the arrow body should have \
             been rewritten independently"
        );
    }

    #[test]
    fn duplicate_key_dedupes_dyn_list() {
        let obj = transformed_object(
            r#"
            import { Div } from '@meonode/ui';
            Div({ onClick: a, onClick: b, width });
            "#,
            "test.tsx",
        );
        // `onClick` appears twice (last wins at runtime, same as any JS
        // object literal) but should only be listed once in `dyn`, in its
        // first-occurrence position.
        assert_eq!(dyn_names(&obj), vec!["onClick", "width"]);
    }
}
