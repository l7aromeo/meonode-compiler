//! Task 9: prop partitioning + call-site key emission.
//!
//! Rewrites every `Decision::Compilable` call site's props object literal
//! (see `detect.rs`) into `@meonode/ui`'s pre-partitioned marker-prop shape:
//!
//! ```text
//! Div({ padding: 'theme.spacing.md', width, onClick: handler, css: {...}, children: [A, B] })
//! // becomes:
//! Div({
//!   __meo$: 2,
//!   __meo$c: { padding: 'var(--meonode-theme-spacing-md)', width },
//!   __meo$d: { onClick: handler },
//!   __meo$k: 'm1a2b3c',
//!   __meo$dyn: ['width', 'onClick'],
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
//!
//! The one value *transformation* it performs is the `theme.*` ->
//! `var(--meonode-theme-*)` rewrite (see `theme.rs`), applied to bucketed
//! string-literal values only. Scope is deliberately narrow — see
//! [`rewrite_theme_tokens_in_buckets`].

use std::collections::HashMap;
use std::mem;

use swc_core::common::Span;
use swc_core::ecma::ast::*;
use swc_core::ecma::atoms::Atom;
use swc_core::ecma::visit::{VisitMut, VisitMutWith};

use crate::config::CompileConfig;
use crate::css_props::is_css_prop;
use crate::effect::is_inline_function;
use crate::detect::{self, Decision};
use crate::effect::is_static_literal;
use crate::keys::{is_special_key, is_stable_key_visible_special, key_name_atom};
use crate::theme::rewrite_theme_tokens;

const MARKER_KEY: &str = "__meo$";
/// Schema version emitted by this compiler. Schema 1 named its buckets `c`/`d`/
/// `k`/`dyn` at the top level, which collides with real props once spreads are
/// left in place -- `d` is a valid SVG `<path>` attribute, so a spread carrying
/// it would be consumed by the runtime as the DOM bucket. Schema 2 namespaces
/// every bucket under the marker prefix, making collision impossible.
const MARKER_SCHEMA: f64 = 2.0;
const BUCKET_CSS_KEY: &str = "__meo$c";
const BUCKET_DOM_KEY: &str = "__meo$d";
const BUCKET_SITE_KEY: &str = "__meo$k";

/// Schema emitted for call sites that get a key but no prop partitioning.
const KEY_ONLY_SCHEMA: f64 = 3.0;
const BUCKET_DYN_KEY: &str = "__meo$dyn";

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
            value: MARKER_SCHEMA,
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
        BUCKET_SITE_KEY,
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
        BUCKET_DYN_KEY,
        Expr::Array(ArrayLit {
            span: swc_core::common::DUMMY_SP,
            elems,
        }),
    )
}

/// Appends the schema 3 marker and call-site key to `obj`, leaving every
/// existing prop untouched and in place.
///
/// Used for call sites that are genuine factory calls with an object-literal
/// props argument, but which cannot be partitioned — a spread after static
/// props, a computed or numeric key, an accessor or method prop, or an
/// ordering constraint. Bucketing is refused there, but the call-site key is a
/// hash of filename and span and needs no knowledge of the props, so it can
/// still be stamped.
///
/// The two props are **appended**, not prepended. Appending puts them after any
/// spread, so a spread can never shadow the marker, and two constant literals
/// evaluated last change neither evaluation order nor any value — which is what
/// makes this safe even on a call site that bailed *for* an ordering reason.
fn stamp_call_site_key(obj: &mut ObjectLit, filename: &str, span: Span) {
    obj.props.push(kv_prop(
        MARKER_KEY,
        Expr::Lit(Lit::Num(Number {
            span: swc_core::common::DUMMY_SP,
            value: KEY_ONLY_SCHEMA,
            raw: None,
        })),
    ));
    obj.props.push(key_prop(filename, span));
}

/// Partitions `obj`'s props into `__meo$`/leading-props/`c`/`d`/`k`/`dyn`
/// plus any special keys, and rewrites `obj.props` in place. `span` is the
/// *call expression's* span (not the object literal's), matching Task 9's
/// spec for `k`.
///
/// Emit order: `__meo$`, every leading `...spread` **and** (when a spread is
/// present) every non-static-literal non-special prop, in their combined
/// original relative source order (see "Leading spreads and the stable-key
/// hazard" below), `c` (omitted if empty), `d` (omitted if empty), `k` +
/// `dyn` (both omitted entirely when a spread is present — see below,
/// otherwise `dyn` itself is omitted if empty), then every special-key prop
/// in its original source order.
///
/// Bucket membership: a non-special key goes to `c` if `css_props::is_css_prop`
/// recognizes it, otherwise `d` — *unless* a spread is present and the value
/// isn't a static literal (`effect::is_static_literal`), in which case it
/// stays flat/unbucketed instead (pushed into `leading`, alongside the
/// spread(s)). When there's no spread, every non-special prop is always
/// bucketed as before, and its name is added to `dyn` unless its value is
/// static (shorthand props are always dynamic, since their implicit value is
/// an identifier).
///
/// ## Leading spreads and the stable-key hazard
///
/// A spread's own argument contributes no bucketed prop and no `dyn` name —
/// its contents aren't known until runtime, so `@meonode/ui`'s `processProps`
/// classifies them generically via its "passthrough" fast path
/// (`getCSSProps`/`getDOMProps` over whatever the spread merges in), with the
/// compiler-bucketed `c`/`d` static props applied on top — reproducing
/// plain-JS "later key wins" semantics exactly (see the v0.2 design doc's
/// Change 2, and this module's tests asserting `c`'s value wins).
///
/// But a spread's contents are equally invisible to `k`: `k` is a pure
/// function of call-site *source position*, so it's identical across every
/// evaluation of the same call site regardless of what the spread happens to
/// contain that time. If `k` (and `dyn`, which is only meaningful alongside
/// it) were still emitted, two evaluations with *different* spread contents
/// would produce an *identical* stable key — and if the node also carries a
/// `deps` array, `@meonode/ui`'s `elementCache` is keyed by that stable key,
/// so a stale cached element (built from an earlier evaluation's props)
/// could be returned instead of a fresh one reflecting the new props. So `k`
/// and `dyn` are never emitted when a spread is present; `_getStableKey`
/// falls back to its legacy `createPropSignature` path instead (see
/// `@meonode/ui`'s own test: "marker without k falls back to the legacy
/// signature path").
///
/// That legacy path hashes each *top-level* prop by value (primitives
/// inline, functions via cached `toString` hash, `css` structurally via a
/// dedicated CSS hash, arrays specially) but any other object-*valued*
/// top-level prop only by its key *names* (see
/// `NodeUtil._serializePropValue`'s generic object branch) — which is fine
/// for the spread's own contents (they land as ordinary flat top-level
/// props, hashed by value like anything else) but would silently blind the
/// signature to a *bucketed* dynamic prop's actual value (hidden one level
/// deeper inside `c`/`d`, which only contributes its own key names to the
/// hash). Restricting bucketing (when a spread is present) to static-literal
/// values only avoids this: a static literal's value never varies between
/// evaluations of the same call site, so hiding it behind `c`/`d`'s
/// structural hash loses no real information, while every value that
/// *could* vary stays flat — exactly where it would sit in genuinely
/// uncompiled code, which the legacy signature path already hashes
/// correctly. See `detect::rank_for_prop` and `detect::validate_object`'s
/// matching doc comment, which this function's bucketing must never diverge
/// from.
///
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

/// Rewrites `theme.*` tokens to `var(--meonode-theme-*)` in the string-literal
/// values of already-bucketed props (see `theme.rs` for why this is safe: the
/// runtime performs the identical conversion on every render, so this only
/// moves it earlier).
///
/// ## Why the scope is this narrow
///
/// Only **direct, bucketed, string-literal prop values** are eligible. Three
/// deliberate exclusions:
///
/// - **Object keys are never touched.** A key like
///   `'@media (max-width: theme.breakpoint.md)'` must resolve to a concrete
///   value, because CSS variables are invalid inside media features and
///   selector text. `@meonode/ui` documents this same invariant and defers key
///   resolution to `resolveObjWithTheme`, which holds the live theme. Since
///   this function only ever reads `kv.value`, the invariant holds structurally.
///
/// - **Special keys are skipped**, because they are never bucketed and so never
///   reach this function. That matters most for `theme:` — rewriting tokens
///   inside a *theme definition* would corrupt it, not optimize it — and it
///   also means `css: {...}` blocks are left to the runtime. On the real
///   `@meonode/ui` docs site only 42 of 758 token-bearing lines sit inside a
///   nested object (`css:`, `props:`, selectors, media queries), so restricting
///   to direct props still covers ~94% of tokens for a fraction of the surface.
///
/// - **No recursion into nested object/array literals**, and no template
///   literals. A no-substitution template is value-equivalent to a string, but
///   rewriting one means re-deriving its `raw` escaping to stay faithful; the
///   runtime handles any token this pass leaves behind, so skipping is free.
///
/// Rewriting cannot affect bucket membership, `dyn` membership, or the stable
/// key: a string literal stays a string literal, so `effect::is_static_literal`
/// still holds and every decision made above this call is unchanged.
fn rewrite_theme_tokens_in_buckets(buckets: [&mut Vec<PropOrSpread>; 2]) {
    for bucket in buckets {
        for prop_or_spread in bucket.iter_mut() {
            let PropOrSpread::Prop(prop) = prop_or_spread else {
                continue;
            };
            let Prop::KeyValue(kv) = &mut **prop else {
                // Shorthand props carry an identifier value, never a literal.
                continue;
            };
            let Expr::Lit(Lit::Str(str_lit)) = unwrap_parens_mut(&mut kv.value) else {
                continue;
            };
            // `Str::value` is WTF-8, which can hold lone surrogates that have no
            // `&str` form. `as_atom()` yields `Some` only for valid UTF-8, so a
            // string containing an unpaired surrogate is left untouched (the
            // runtime still handles any token in it). Computed before the
            // mutation below so the immutable borrow ends first.
            let rewritten = str_lit
                .value
                .as_atom()
                .and_then(|atom| rewrite_theme_tokens(atom));

            if let Some(rewritten) = rewritten {
                str_lit.value = rewritten.into();
                // Drop the original token so the emitted code carries the
                // rewritten value. `raw` holds the source text verbatim
                // (quotes included) and takes precedence over `value` when the
                // codegen emits, so a stale `raw` would silently undo the whole
                // rewrite.
                str_lit.raw = None;
            }
        }
    }
}

fn rewrite_object(obj: &mut ObjectLit, filename: &str, span: Span) {
    let old_props = mem::take(&mut obj.props);
    let has_spread = old_props
        .iter()
        .any(|p| matches!(p, PropOrSpread::Spread(_)));

    // Spreads themselves, plus (only when `has_spread`) any non-static-literal
    // non-special prop that must stay flat for stable-key safety — see this
    // function's doc comment. Built via a single forward scan, so their
    // combined relative source order is preserved automatically.
    let mut leading: Vec<PropOrSpread> = Vec::new();
    let mut c_props: Vec<PropOrSpread> = Vec::new();
    let mut d_props: Vec<PropOrSpread> = Vec::new();
    let mut special_props: Vec<PropOrSpread> = Vec::new();
    let mut dyn_names: Vec<Atom> = Vec::new();

    for prop_or_spread in old_props {
        let prop = match prop_or_spread {
            // Leading spread (Change 2): `detect::validate_object` already
            // guaranteed every spread precedes every static prop, so it's
            // always safe to leave it exactly where it was — right after
            // `__meo$` once every leading prop has been collected.
            PropOrSpread::Spread(spread) => {
                leading.push(PropOrSpread::Spread(spread));
                continue;
            }
            PropOrSpread::Prop(prop) => prop,
        };

        match &*prop {
            Prop::Shorthand(ident) => {
                let name = ident.sym.clone();
                if is_special_key(name.as_ref()) {
                    // Special keys stay top-level, but a shorthand's value is an
                    // identifier read -- always dynamic -- so its name must still
                    // enter `dyn` or the stable key would never see it change.
                    // The runtime resolves `dyn` names via c -> d -> top-level.
                    if !has_spread && is_stable_key_visible_special(name.as_ref()) {
                        push_dyn_name(&mut dyn_names, name.clone());
                    }
                    special_props.push(PropOrSpread::Prop(prop));
                    continue;
                }
                // Shorthand's implicit value is the identifier itself: always
                // dynamic (see `effect::is_static_literal`), so with a spread
                // present it must stay flat rather than get bucketed.
                if has_spread {
                    leading.push(PropOrSpread::Prop(prop));
                    continue;
                }
                push_dyn_name(&mut dyn_names, name.clone());
                if is_css_prop(name.as_ref()) {
                    c_props.push(PropOrSpread::Prop(prop));
                } else {
                    d_props.push(PropOrSpread::Prop(prop));
                }
            }
            Prop::KeyValue(kv) => {
                let name = key_name_atom(&kv.key);
                if is_special_key(name.as_ref()) {
                    // Special keys stay top-level and are never bucketed, but a
                    // *dynamic* special value (e.g. `css: computeCss()`) still has
                    // to enter `dyn`: on the non-spread path `k` is emitted, so
                    // `dyn` is the only thing making the stable key value-sensitive.
                    // Without this a changing `css`/`theme` value would keep a
                    // stale cached element. The runtime resolves `dyn` names via
                    // c -> d -> top-level, which covers these.
                    if !has_spread
                        && !is_static_literal(&kv.value)
                        && !is_inline_function(&kv.value)
                        && is_stable_key_visible_special(name.as_ref())
                    {
                        push_dyn_name(&mut dyn_names, name.clone());
                    }
                    special_props.push(PropOrSpread::Prop(prop));
                    continue;
                }
                let is_static = is_static_literal(&kv.value);
                if has_spread && !is_static {
                    leading.push(PropOrSpread::Prop(prop));
                    continue;
                }
                // An inline function literal is bucketed like any other dynamic
                // value, but is deliberately kept out of `dyn`: the runtime
                // hashes functions by source text, which is fixed by the call
                // site, so it would only ever contribute a constant that `k`
                // already covers. See `effect::is_inline_function`.
                if !is_static && !is_inline_function(&kv.value) {
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

    // Applied after bucketing rather than inline, so it reads as exactly what
    // it is: a value transformation over the final bucket contents, incapable
    // of influencing any placement decision made above.
    rewrite_theme_tokens_in_buckets([&mut c_props, &mut d_props]);

    // A spread present means `k`/`dyn` are never emitted (stable-key hazard
    // — see doc comment above), regardless of whether `dyn_names` ended up
    // empty anyway.
    if has_spread {
        dyn_names.clear();
    }

    let mut new_props = Vec::with_capacity(
        3 + leading.len()
            + c_props.is_empty() as usize
            + d_props.is_empty() as usize
            + dyn_names.is_empty() as usize
            + special_props.len(),
    );

    new_props.push(marker_prop());
    new_props.extend(leading);
    if !c_props.is_empty() {
        new_props.push(bucket_prop(BUCKET_CSS_KEY, c_props));
    }
    if !d_props.is_empty() {
        new_props.push(bucket_prop(BUCKET_DOM_KEY, d_props));
    }
    if !has_spread {
        new_props.push(key_prop(filename, span));
        if !dyn_names.is_empty() {
            new_props.push(dyn_prop(dyn_names));
        }
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
    /// Call sites that cannot be partitioned but can still be keyed. Same
    /// mapping shape; rewritten by `stamp_call_site_key` instead of
    /// `rewrite_object`.
    key_only: HashMap<(u32, u32), usize>,
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
        let key = (span.lo.0, span.hi.0);
        let (props_arg_idx, partition) = match self.compilable.get(&key) {
            Some(&idx) => (idx, true),
            None => match self.key_only.get(&key) {
                Some(&idx) => (idx, false),
                None => return,
            },
        };
        let Some(arg) = call.args.get_mut(props_arg_idx) else {
            return;
        };
        let Expr::Object(obj) = unwrap_parens_mut(&mut arg.expr) else {
            return;
        };
        if partition {
            rewrite_object(obj, self.filename, span);
        } else {
            stamp_call_site_key(obj, self.filename, span);
        }
    }
}

/// Runs detection over `program`, then rewrites every `Decision::Compilable`
/// call site's props object literal into the pre-partitioned marker-prop
/// shape (see module docs). `filename` is used to compute each call site's
/// `k` value — pass the real source filename in production (see
/// `lib.rs::process_transform`) or a fixed name in tests. `config` carries
/// Change 4's `factoryModules` list through to `detect::detect`.
///
/// No-op (and skips the rewrite pass entirely) if there are no compilable
/// call sites, so files untouched by @meonode/ui factories pay no additional
/// traversal cost beyond detection itself.
pub fn transform_program(program: &mut Program, filename: &str, config: &CompileConfig) {
    let mut compilable: HashMap<(u32, u32), usize> = HashMap::new();
    let mut key_only: HashMap<(u32, u32), usize> = HashMap::new();

    for d in detect::detect(&*program, config) {
        let span_key = (d.span.lo.0, d.span.hi.0);
        match d.decision {
            Decision::Compilable { props_arg_idx } => {
                compilable.insert(span_key, props_arg_idx);
            }
            Decision::KeyOnly { props_arg_idx } => {
                key_only.insert(span_key, props_arg_idx);
            }
            Decision::Bail(_) => {}
        }
    }

    if compilable.is_empty() && key_only.is_empty() {
        return;
    }

    let mut rewriter = Rewriter {
        filename,
        compilable,
        key_only,
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
    /// runs `transform_program` with the default (empty `factoryModules`)
    /// config, then collects the object literal from the first argument of
    /// every call expression, in traversal order. AST inspection (rather
    /// than round-tripping through codegen text) keeps these assertions
    /// independent of formatting/whitespace choices.
    fn transformed_objects(src: &str, filename: &str) -> Vec<ObjectLit> {
        transformed_objects_with_config(src, filename, &CompileConfig::default())
    }

    /// Like [`transformed_objects`], but with a caller-supplied
    /// [`CompileConfig`] — used to exercise Change 4's `factoryModules`
    /// option end to end through the real rewrite pass.
    fn transformed_objects_with_config(
        src: &str,
        filename: &str,
        config: &CompileConfig,
    ) -> Vec<ObjectLit> {
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

            transform_program(&mut program, filename, config);

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
        let Expr::Array(arr) = find_prop(obj, "__meo$dyn") else {
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
            str_lit_value(find_prop(&obj1, "__meo$k")),
            str_lit_value(find_prop(&obj2, "__meo$k"))
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
            str_lit_value(find_prop(&objs[0], "__meo$k")),
            str_lit_value(find_prop(&objs[1], "__meo$k"))
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
            str_lit_value(find_prop(&obj1, "__meo$k")),
            str_lit_value(find_prop(&obj2, "__meo$k"))
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

        let Expr::Object(c) = find_prop(&obj, "__meo$c") else {
            panic!("expected `c` to be an object literal");
        };
        assert!(has_prop(c, "backgroundColor"));
        assert!(!has_prop(c, "onClick"));
        assert!(!has_prop(c, "id"));

        let Expr::Object(d) = find_prop(&obj, "__meo$d") else {
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
        assert!(!has_prop(&obj, "__meo$c"));
        assert!(has_prop(&obj, "__meo$d"));
    }

    /// Compiles a single `Div({...})` call and returns its emitted props object.
    fn one_call(props_src: &str) -> ObjectLit {
        transformed_object(
            &format!("import {{ Div }} from '@meonode/ui';\nDiv({props_src});"),
            "test.tsx",
        )
    }

    /// The keys present in one emitted bucket (`__meo$c` / `__meo$d`).
    fn bucket_keys(obj: &ObjectLit, bucket: &str) -> Vec<String> {
        let Expr::Object(inner) = find_prop(obj, bucket) else {
            panic!("expected `{bucket}` to be an object literal");
        };
        inner
            .props
            .iter()
            .filter_map(|p| {
                let PropOrSpread::Prop(prop) = p else { return None };
                match &**prop {
                    Prop::KeyValue(kv) => match &kv.key {
                        PropName::Ident(id) => Some(id.sym.as_ref().to_string()),
                        PropName::Str(st) => Some(st.value.as_str().unwrap().to_string()),
                        _ => None,
                    },
                    Prop::Shorthand(id) => Some(id.sym.as_ref().to_string()),
                    _ => None,
                }
            })
            .collect()
    }

    #[test]
    fn inline_arrow_is_bucketed_but_omitted_from_dyn() {
        // The runtime hashes a function by source text, which is fixed by the
        // call site, so listing it in `dyn` only buys a constant that `k`
        // already encodes -- while costing a `toString()` and a hash per
        // render, because the runtime's memo is keyed by function identity and
        // an inline literal is a fresh object every render.
        let obj = one_call("{ padding: '8px', onClick: () => {} }");
        assert!(!has_prop(&obj, "__meo$dyn"));
        assert!(bucket_keys(&obj, "__meo$d").contains(&"onClick".to_string()));
    }

    #[test]
    fn inline_function_expression_is_also_omitted() {
        let obj = one_call("{ onClick: function () {} }");
        assert!(!has_prop(&obj, "__meo$dyn"));
        assert!(bucket_keys(&obj, "__meo$d").contains(&"onClick".to_string()));
    }

    #[test]
    fn parenthesized_inline_arrow_is_omitted() {
        let obj = one_call("{ onClick: (() => {}) }");
        assert!(!has_prop(&obj, "__meo$dyn"));
    }

    #[test]
    fn function_reference_still_enters_dyn() {
        // An identifier can resolve to a different function on a later render.
        let obj = one_call("{ onClick: handler }");
        assert_eq!(dyn_names(&obj), vec!["onClick"]);
    }

    #[test]
    fn conditional_between_functions_still_enters_dyn() {
        let obj = one_call("{ onClick: cond ? a : b }");
        assert_eq!(dyn_names(&obj), vec!["onClick"]);
    }

    #[test]
    fn call_returning_a_function_still_enters_dyn() {
        let obj = one_call("{ onClick: makeHandler(id) }");
        assert_eq!(dyn_names(&obj), vec!["onClick"]);
    }

    #[test]
    fn inline_arrow_on_a_special_key_is_omitted_too() {
        let obj = one_call("{ css: () => ({ color: 'red' }) }");
        assert!(!has_prop(&obj, "__meo$dyn"));
    }

    #[test]
    fn mixed_inline_and_referenced_handlers_keep_only_the_reference() {
        let obj = one_call("{ onClick: () => {}, onBlur: handler, padding: '8px' }");
        assert_eq!(dyn_names(&obj), vec!["onBlur"]);
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
        assert!(!has_prop(&obj, "__meo$dyn"));
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
            vec![
                "__meo$",
                "__meo$c",
                "__meo$d",
                "__meo$k",
                "__meo$dyn",
                "children",
                "css"
            ]
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
                (name != "__meo$"
                    && name != "__meo$c"
                    && name != "__meo$d"
                    && name != "__meo$k"
                    && name != "__meo$dyn")
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

        let Expr::Object(d) = find_prop(outer, "__meo$d") else {
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

    // --- Change 2: leading spreads ---

    #[test]
    fn leading_spread_stays_top_level_right_after_marker() {
        let obj = transformed_object(
            r#"
            import { Div } from '@meonode/ui';
            const extra = {};
            Div({ ...extra, padding: '8px' });
            "#,
            "test.tsx",
        );
        assert!(
            matches!(&obj.props[0], PropOrSpread::Prop(_)),
            "expected __meo$ first"
        );
        let PropOrSpread::Spread(spread) = &obj.props[1] else {
            panic!(
                "expected the spread to be the second emitted prop (right after __meo$), got {:?}",
                obj.props[1]
            );
        };
        let Expr::Ident(ident) = &*spread.expr else {
            panic!("expected the spread argument to still be the bare `extra` identifier");
        };
        assert_eq!(ident.sym.as_ref(), "extra");

        let Expr::Object(c) = find_prop(&obj, "__meo$c") else {
            panic!("expected `c` bucket to exist");
        };
        assert!(has_prop(c, "padding"));
    }

    #[test]
    fn multiple_leading_spreads_preserve_relative_order() {
        let obj = transformed_object(
            r#"
            import { Div } from '@meonode/ui';
            const a = {};
            const b = {};
            Div({ ...a, ...b, padding: '8px' });
            "#,
            "test.tsx",
        );
        let spread_names: Vec<String> = obj.props[1..3]
            .iter()
            .map(|p| {
                let PropOrSpread::Spread(s) = p else {
                    panic!("expected a spread entry, got {p:?}");
                };
                let Expr::Ident(id) = &*s.expr else {
                    panic!("expected an identifier spread argument");
                };
                id.sym.as_ref().to_string()
            })
            .collect();
        assert_eq!(spread_names, vec!["a", "b"]);
    }

    #[test]
    fn spread_only_props_emits_no_c_d_k_or_dyn() {
        let obj = transformed_object(
            r#"
            import { Div } from '@meonode/ui';
            const extra = {};
            Div({ ...extra });
            "#,
            "test.tsx",
        );
        assert!(!has_prop(&obj, "__meo$c"));
        assert!(!has_prop(&obj, "__meo$d"));
        assert!(!has_prop(&obj, "__meo$dyn"));
        assert!(
            !has_prop(&obj, "__meo$k"),
            "k must never be emitted when a spread is present — see the \
             stable-key hazard doc comment on rewrite_object"
        );
        let PropOrSpread::Spread(_) = &obj.props[1] else {
            panic!(
                "expected the spread to still be present, got {:?}",
                obj.props[1]
            );
        };
    }

    /// The stable-key hazard this test guards against: `k` is a pure
    /// function of call-site source position, so if it were emitted here,
    /// two evaluations of this exact call site with *different* `extra`
    /// contents would get an identical stable key — and, combined with a
    /// `deps` array, could return a stale cached element built from a
    /// different evaluation's props. Even when every other prop is a static
    /// literal (so `c`/`d` bucketing is otherwise fully safe), `k`/`dyn`
    /// must still be omitted whenever a spread is present.
    #[test]
    fn spread_present_omits_k_and_dyn_even_with_only_static_other_props() {
        let obj = transformed_object(
            r#"
            import { Div } from '@meonode/ui';
            const extra = {};
            Div({ ...extra, padding: '8px', id: 'x' });
            "#,
            "test.tsx",
        );
        assert!(!has_prop(&obj, "__meo$k"));
        assert!(!has_prop(&obj, "__meo$dyn"));
        // Bucketing for the static props is still fully retained.
        let Expr::Object(c) = find_prop(&obj, "__meo$c") else {
            panic!("expected `c` bucket to exist");
        };
        assert!(has_prop(c, "padding"));
        let Expr::Object(d) = find_prop(&obj, "__meo$d") else {
            panic!("expected `d` bucket to exist");
        };
        assert!(has_prop(d, "id"));
    }

    /// A non-static-literal prop (`onClick: handler` — an identifier, always
    /// dynamic) alongside a spread must stay flat/unbucketed rather than
    /// land in `d`: bucketing it would hide its actual value one level
    /// deeper behind `d`'s own structural (key-names-only) hash once `k`
    /// is omitted and `_getStableKey` falls back to the legacy signature
    /// path, silently reintroducing the same stable-key collision hazard
    /// for an ordinary dynamic prop instead of a spread. `padding` (a
    /// static literal) is unaffected and still bucketed normally.
    #[test]
    fn non_static_prop_stays_flat_when_spread_present() {
        let obj = transformed_object(
            r#"
            import { Div } from '@meonode/ui';
            const extra = {};
            Div({ ...extra, onClick: handler, padding: '8px' });
            "#,
            "test.tsx",
        );
        assert!(!has_prop(&obj, "__meo$k"));
        assert!(!has_prop(&obj, "__meo$dyn"));

        let Expr::Object(c) = find_prop(&obj, "__meo$c") else {
            panic!("expected `c` bucket to exist");
        };
        assert!(has_prop(c, "padding"));
        // `onClick` must NOT be in `d` (or `c`) — it must stay flat instead.
        assert!(
            !has_prop(&obj, "__meo$d"),
            "onClick is the only would-be-`d` candidate and must stay flat, \
             so `d` shouldn't be emitted at all"
        );

        // `onClick` should appear as a flat top-level KeyValue prop, right
        // alongside the spread (both at Change-2's "leading" position).
        let has_flat_on_click = obj.props.iter().any(|p| {
            let PropOrSpread::Prop(prop) = p else {
                return false;
            };
            let Prop::KeyValue(kv) = &**prop else {
                return false;
            };
            matches!(&kv.key, PropName::Ident(id) if id.sym.as_ref() == "onClick")
        });
        assert!(
            has_flat_on_click,
            "expected `onClick` to be present as a flat top-level prop"
        );
    }

    /// Multiple leading spreads plus a mixed static/dynamic set of other
    /// props (both spreads must stay contiguous and precede every static
    /// prop — Change 2's leading-spread rule; interleaving a spread *between*
    /// static props is a `TrailingSpread` bail, covered separately in
    /// `detect.rs`'s tests): the static prop still buckets, the dynamic one
    /// stays flat with the spreads, and their combined relative source order
    /// is preserved.
    #[test]
    fn mixed_static_and_dynamic_props_with_multiple_spreads() {
        let obj = transformed_object(
            r#"
            import { Div } from '@meonode/ui';
            const a = {};
            const b = {};
            Div({ ...a, ...b, onClick: handler, padding: '8px' });
            "#,
            "test.tsx",
        );
        assert!(!has_prop(&obj, "__meo$k"));
        assert!(!has_prop(&obj, "__meo$dyn"));

        // Leading group (everything before `c`): spread `a`, spread `b`,
        // `onClick`, in that exact source order.
        let leading_kinds: Vec<String> = obj.props[1..4]
            .iter()
            .map(|p| match p {
                PropOrSpread::Spread(s) => {
                    let Expr::Ident(id) = &*s.expr else {
                        panic!("expected identifier spread argument");
                    };
                    format!("...{}", id.sym.as_ref())
                }
                PropOrSpread::Prop(prop) => {
                    let Prop::KeyValue(kv) = &**prop else {
                        panic!("expected a KeyValue prop");
                    };
                    let PropName::Ident(id) = &kv.key else {
                        panic!("expected an ident key");
                    };
                    id.sym.as_ref().to_string()
                }
            })
            .collect();
        assert_eq!(leading_kinds, vec!["...a", "...b", "onClick"]);

        let Expr::Object(c) = find_prop(&obj, "__meo$c") else {
            panic!("expected `c` bucket to exist");
        };
        assert!(has_prop(c, "padding"));
    }

    // --- Change 3: quoted string keys ---

    #[test]
    fn quoted_non_identifier_key_is_bucketed_into_d_with_original_quoted_key() {
        let obj = transformed_object(
            r#"
            import { Div } from '@meonode/ui';
            Div({ padding: '4px', "data-parallax": "true" });
            "#,
            "test.tsx",
        );
        let Expr::Object(d) = find_prop(&obj, "__meo$d") else {
            panic!("expected `d` bucket to exist");
        };
        let found = d.props.iter().any(|p| {
            let PropOrSpread::Prop(prop) = p else {
                return false;
            };
            let Prop::KeyValue(kv) = &**prop else {
                return false;
            };
            matches!(&kv.key, PropName::Str(s) if s.value.as_str() == Some("data-parallax"))
        });
        assert!(
            found,
            "expected `data-parallax` to be bucketed into `d`, still keyed by its original quoted string"
        );
    }

    // --- Change 4: `factoryModules` plugin config ---

    #[test]
    fn factory_module_call_site_is_rewritten_when_configured() {
        let config = CompileConfig {
            factory_modules: vec!["@meonode/mui".to_string()],
        };
        let mut objs = transformed_objects_with_config(
            r#"
            import { Button } from '@meonode/mui';
            Button({ padding: '8px', onClick: handler });
            "#,
            "test.tsx",
            &config,
        );
        assert_eq!(objs.len(), 1);
        let obj = objs.remove(0);
        assert!(has_prop(&obj, "__meo$"));
        let Expr::Object(c) = find_prop(&obj, "__meo$c") else {
            panic!("expected `c` bucket to exist");
        };
        assert!(has_prop(c, "padding"));
    }

    #[test]
    fn factory_module_call_site_untouched_without_config() {
        let obj = transformed_object(
            r#"
            import { Button } from '@meonode/mui';
            Button({ padding: '8px' });
            "#,
            "test.tsx",
        );
        assert!(
            !has_prop(&obj, "__meo$"),
            "Button(...) must not be rewritten without factoryModules configured"
        );
    }
}
