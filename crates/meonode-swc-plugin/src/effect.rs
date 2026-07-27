//! Effect and reorder-safety classifiers for prop values.
//!
//! The partitioner rewrites a compilable call's props object literal into
//! separate `__meo$c` (CSS) / `__meo$d` (DOM) buckets, so bucketed props no
//! longer evaluate in their original source positions: all `c` values run
//! before all `d` values, and special keys move to the tail.
//!
//! An earlier version demanded every prop value be side-effect-free. That was
//! both too strict (bailing common shapes) and, once relaxed to "only
//! effectful values must keep their order", unsound — effect-free is not
//! value-stable. See `order.rs` for the worked counterexample.
//!
//! This module is the single source of truth for three related judgments:
//!
//! - [`is_effect_free`] — can this expression cause an observable side effect?
//!   `detect.rs` uses it to decide whether an object contains any effectful
//!   value at all, which is what makes reordering observable in the first
//!   place.
//! - [`is_order_inert`] — is this expression's *position* unobservable?
//!   Static literals and closure definitions qualify. `detect.rs` filters
//!   these out of the order sequence it hands to `order::order_preserved`.
//! - [`is_static_literal`] — the narrowest check, used by `partition.rs` to
//!   decide `dyn` membership and flat-vs-bucketed placement. Deliberately
//!   *not* widened alongside `is_order_inert`: widening it would change what
//!   gets hashed into the stable key.

use swc_core::ecma::ast::*;

/// Returns `true` if `expr` is provably free of side effects when evaluated,
/// recursing into nested object/array literals.
///
/// Accepted:
/// - Literals: string, number, bool, null, bigint, regex
/// - Identifiers (including `undefined`) — reading a binding is never treated
///   as effectful here, even though exotic getters exist in theory
/// - Arrow / function expressions (defining a closure has no side effects)
/// - Unary `-`/`+` applied directly to a numeric literal (`-1`, `+2`)
/// - Template literals with no substitutions (a plain string in disguise)
/// - Object/array literals whose entries are all recursively effect-free; a
///   spread anywhere inside (`{ ...x }`, `[...x]`) fails the whole literal,
///   since its arity/contents aren't known here
///
/// Rejected (falls through to `_ => false`): calls, `new`, member expressions
/// (getter risk), templates with substitutions, `await`/`yield`, assignments,
/// tagged templates, conditionals (ternary) — kept simple for v1 — and
/// anything else not explicitly listed above (binary/logical/sequence
/// expressions, `this`, classes, JSX, optional chaining, etc.).
pub fn is_effect_free(expr: &Expr) -> bool {
    match unwrap_parens(expr) {
        Expr::Lit(_) => true,
        Expr::Ident(_) => true,
        Expr::Arrow(_) => true,
        Expr::Fn(_) => true,
        Expr::Unary(unary) => is_numeric_unary(unary),
        Expr::Tpl(tpl) => tpl.exprs.is_empty(),
        Expr::Object(obj) => obj.props.iter().all(is_effect_free_prop),
        Expr::Array(arr) => arr.elems.iter().all(|elem| match elem {
            None => true,
            Some(ExprOrSpread {
                spread: Some(_), ..
            }) => false,
            Some(ExprOrSpread { spread: None, expr }) => is_effect_free(expr),
        }),
        _ => false,
    }
}

fn is_effect_free_prop(prop_or_spread: &PropOrSpread) -> bool {
    match prop_or_spread {
        PropOrSpread::Spread(_) => false,
        PropOrSpread::Prop(prop) => match &**prop {
            Prop::Shorthand(_) => true,
            Prop::KeyValue(kv) => {
                !matches!(&kv.key, PropName::Computed(_)) && is_effect_free(&kv.value)
            }
            Prop::Getter(_) | Prop::Setter(_) | Prop::Method(_) | Prop::Assign(_) => false,
        },
    }
}

fn is_numeric_unary(unary: &UnaryExpr) -> bool {
    matches!(unary.op, UnaryOp::Minus | UnaryOp::Plus)
        && matches!(unwrap_parens(&unary.arg), Expr::Lit(Lit::Num(_)))
}

fn unwrap_parens(mut expr: &Expr) -> &Expr {
    while let Expr::Paren(paren) = expr {
        expr = &paren.expr;
    }
    expr
}

/// Returns `true` if `expr` is a "plain literal" for `dyn`-list purposes: a
/// value whose textual form is already fully known at compile time, so the
/// runtime never needs to resolve a binding to use it.
///
/// This is a *strict subset* of [`is_effect_free`] — every static literal is
/// effect-free, but not every effect-free value is static. Identifiers,
/// function/arrow expressions, and nested object/array literals are all
/// effect-free yet still count as dynamic here: an identifier needs a binding
/// lookup, and Task 9's spec conservatively treats nested object/array
/// literals as dynamic too (rather than trying to prove they're deeply
/// constant).
///
/// Accepted as static: string/number/bool/null/bigint/regex literals, `-x`/
/// `+x` over a numeric literal, and template literals with no substitutions.
/// Everything else is dynamic.
pub fn is_static_literal(expr: &Expr) -> bool {
    match unwrap_parens(expr) {
        Expr::Lit(_) => true,
        Expr::Unary(unary) => is_numeric_unary(unary),
        Expr::Tpl(tpl) => tpl.exprs.is_empty(),
        _ => false,
    }
}

/// Whether an expression's *position* among sibling prop values is
/// unobservable — i.e. it may be freely reordered by bucketing.
///
/// This is deliberately distinct from [`is_static_literal`], which decides
/// bucket membership and `dyn` participation in `partition.rs`. Widening that
/// one would change what gets hashed into the stable key; this predicate only
/// feeds the evaluation-order analysis in `detect::validate_object`.
///
/// Beyond static literals, a function or arrow *expression* qualifies: defining
/// a closure performs no reads and no side effects. It allocates a function
/// object capturing its bindings by reference, and nothing in the body runs
/// until it is later invoked. So moving *when the closure is constructed*
/// relative to a sibling that reads or mutates state cannot be observed — the
/// closure still resolves its captures at call time, long after the whole
/// object literal has been built.
///
/// This matters because writing a handler before style props
/// (`{ onClick: () => ..., padding: cond ? a : b }`) is an extremely common
/// shape; treating the arrow as order-sensitive would bail those call sites for
/// no reason.
pub fn is_order_inert(expr: &Expr) -> bool {
    match unwrap_parens(expr) {
        Expr::Arrow(_) | Expr::Fn(_) => true,
        other => is_static_literal(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_expr(src: &str) -> Box<Expr> {
        use swc_core::common::sync::Lrc;
        use swc_core::common::{FileName, SourceMap};
        use swc_core::ecma::ast::EsVersion;
        use swc_core::ecma::parser::lexer::Lexer;
        use swc_core::ecma::parser::{EsSyntax, Parser, StringInput, Syntax};

        let cm: Lrc<SourceMap> = Default::default();
        let fm = cm.new_source_file(Lrc::new(FileName::Anon), src.to_string());
        let lexer = Lexer::new(
            Syntax::Es(EsSyntax::default()),
            EsVersion::EsNext,
            StringInput::from(&*fm),
            None,
        );
        let mut parser = Parser::new_from(lexer);
        parser
            .parse_expr()
            .unwrap_or_else(|e| panic!("failed to parse expr: {e:?}\n---\n{src}"))
    }

    fn effect_free(src: &str) -> bool {
        is_effect_free(&parse_expr(src))
    }

    fn static_literal(src: &str) -> bool {
        is_static_literal(&parse_expr(src))
    }

    // --- is_effect_free: accepted ---

    #[test]
    fn string_literal_is_effect_free() {
        assert!(effect_free(r#""hello""#));
    }

    #[test]
    fn number_literal_is_effect_free() {
        assert!(effect_free("42"));
    }

    #[test]
    fn bool_literal_is_effect_free() {
        assert!(effect_free("true"));
    }

    #[test]
    fn null_literal_is_effect_free() {
        assert!(effect_free("null"));
    }

    #[test]
    fn bigint_literal_is_effect_free() {
        assert!(effect_free("10n"));
    }

    #[test]
    fn regex_literal_is_effect_free() {
        assert!(effect_free("/abc/g"));
    }

    #[test]
    fn identifier_is_effect_free() {
        assert!(effect_free("width"));
    }

    #[test]
    fn undefined_identifier_is_effect_free() {
        assert!(effect_free("undefined"));
    }

    #[test]
    fn arrow_expr_is_effect_free() {
        assert!(effect_free("() => 1"));
    }

    #[test]
    fn fn_expr_is_effect_free() {
        assert!(effect_free("function () {}"));
    }

    #[test]
    fn negated_numeric_literal_is_effect_free() {
        assert!(effect_free("-1"));
        assert!(effect_free("+1"));
    }

    #[test]
    fn template_without_substitutions_is_effect_free() {
        assert!(effect_free("`plain`"));
    }

    #[test]
    fn nested_object_literal_is_effect_free() {
        assert!(effect_free("({ a: 1, b: { c: 2 } })"));
    }

    #[test]
    fn nested_array_literal_is_effect_free() {
        assert!(effect_free("[1, 2, [3, 4]]"));
    }

    #[test]
    fn array_with_elision_is_effect_free() {
        assert!(effect_free("[1, , 3]"));
    }

    // --- is_effect_free: rejected ---

    #[test]
    fn call_expr_is_effectful() {
        assert!(!effect_free("f()"));
    }

    #[test]
    fn member_expr_is_effectful() {
        assert!(!effect_free("a.b"));
    }

    #[test]
    fn template_with_substitution_is_effectful() {
        assert!(!effect_free("`${x}`"));
    }

    #[test]
    fn await_expr_is_effectful() {
        // `await x` is only valid syntax inside an async function; pull the
        // `AwaitExpr` node out of an async arrow body rather than asserting
        // on the enclosing (trivially-effectful, being a call) IIFE.
        let iife = parse_expr("(async () => await x)()");
        let Expr::Call(call) = &*iife else {
            panic!("expected a call expression");
        };
        let Callee::Expr(callee) = &call.callee else {
            panic!("expected a callee expression");
        };
        // The source parenthesizes the arrow (`(async () => ...)()`), so the
        // callee is a `Paren` wrapping the `Arrow`, not the `Arrow` itself.
        let Expr::Arrow(arrow) = unwrap_parens(callee) else {
            panic!("expected an arrow callee, got {callee:?}");
        };
        let BlockStmtOrExpr::Expr(body) = &*arrow.body else {
            panic!("expected an expression arrow body");
        };
        assert!(matches!(&**body, Expr::Await(_)));
        assert!(!is_effect_free(body));
    }

    #[test]
    fn assignment_is_effectful() {
        assert!(!effect_free("(x = 1)"));
    }

    #[test]
    fn new_expr_is_effectful() {
        assert!(!effect_free("new Foo()"));
    }

    #[test]
    fn tagged_template_is_effectful() {
        assert!(!effect_free("tag`hi`"));
    }

    #[test]
    fn conditional_is_effectful() {
        assert!(!effect_free("a ? 1 : 2"));
    }

    #[test]
    fn spread_inside_nested_object_is_effectful() {
        assert!(!effect_free("({ a: { ...rest } })"));
    }

    #[test]
    fn spread_inside_nested_array_is_effectful() {
        assert!(!effect_free("[1, ...rest]"));
    }

    #[test]
    fn nested_object_with_call_value_is_effectful() {
        assert!(!effect_free("({ a: f() })"));
    }

    // --- is_static_literal ---

    #[test]
    fn string_is_static() {
        assert!(static_literal(r#""x""#));
    }

    #[test]
    fn number_is_static() {
        assert!(static_literal("1"));
    }

    #[test]
    fn negated_number_is_static() {
        assert!(static_literal("-1"));
    }

    #[test]
    fn plain_template_is_static() {
        assert!(static_literal("`plain`"));
    }

    #[test]
    fn identifier_is_dynamic() {
        assert!(!static_literal("width"));
    }

    #[test]
    fn arrow_is_dynamic() {
        assert!(!static_literal("() => 1"));
    }

    #[test]
    fn fn_expr_is_dynamic() {
        assert!(!static_literal("function () {}"));
    }

    #[test]
    fn nested_object_is_dynamic() {
        assert!(!static_literal("({ a: 1 })"));
    }

    #[test]
    fn nested_array_is_dynamic() {
        assert!(!static_literal("[1, 2]"));
    }
}
