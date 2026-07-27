//! Shared prop-key utilities used by both `detect.rs` (bail decisions,
//! evaluation-order analysis) and `partition.rs` (the actual rewrite).
//!
//! Keeping `SPECIAL_KEYS`/[`is_special_key`] and [`key_name_atom`] here
//! (rather than duplicated in each module) guarantees both modules agree on
//! what counts as a special key and how a prop's name is derived from its
//! `PropName` — any divergence between the two would be a real compiler bug
//! (e.g. detect.rs computing an evaluation-order rank for a different key
//! than partition.rs actually buckets it under).

use swc_core::ecma::ast::PropName;
use swc_core::ecma::atoms::Atom;

/// Prop keys that always stay top-level, untouched by `c`/`d` bucketing, no
/// matter what `css_props::is_css_prop` says about them (some, like `css`
/// itself, would otherwise look unrelated to CSS but still need special
/// runtime handling; `theme`/`as`/`disableEmotion` are meonode-specific
/// config, not DOM props). `key` and `children` are here too — see the
/// module docs on `@meonode/ui`'s `createElement` spread-children/explicit-
/// key semantics, which must stay byte-identical.
pub const SPECIAL_KEYS: &[&str] = &[
    "css",
    "props",
    "ref",
    "key",
    "children",
    "as",
    "theme",
    "disableEmotion",
];

pub fn is_special_key(name: &str) -> bool {
    SPECIAL_KEYS.contains(&name)
}

/// Extracts the name of an (already-validated) prop key as an owned `Atom`.
///
/// By the time this is called (either from `detect::validate_object`'s
/// order-analysis pass, after `validate_key` has already confirmed the key is
/// `Ident`/`Str`, or from `partition::rewrite_object` on an already-
/// `Decision::Compilable` call site), computed/numeric/bigint keys have
/// already been rejected — only `Ident` and identifier-or-not `Str` keys
/// (see the v0.2 quoted-string-key rule) survive.
pub fn key_name_atom(key: &PropName) -> Atom {
    match key {
        PropName::Ident(id) => id.sym.clone(),
        PropName::Str(s) => Atom::from(s.value.as_str().unwrap_or_default()),
        _ => unreachable!(
            "non-ident/str prop keys are rejected by detect::validate_object \
             before a call site becomes Decision::Compilable, and before \
             order-analysis derives a key name from them"
        ),
    }
}

/// Special keys that are still visible to `BaseNode._getStableKey`, and so may
/// legitimately be named in `dyn`.
///
/// `ref` and `children` are destructured off in the `BaseNode` constructor, and
/// `key` in `_getStableKey` itself, before the marker props are ever inspected.
/// Naming any of those three in `dyn` would make it unresolvable at runtime —
/// tripping the debug "dyn names prop X but it isn't present" warning and
/// hashing `undefined` instead of a real value.
pub fn is_stable_key_visible_special(name: &str) -> bool {
    matches!(name, "css" | "props" | "as" | "theme" | "disableEmotion")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn special_keys_are_recognized() {
        for key in SPECIAL_KEYS {
            assert!(is_special_key(key));
        }
    }

    #[test]
    fn non_special_key_is_not_recognized() {
        assert!(!is_special_key("padding"));
        assert!(!is_special_key("onClick"));
    }
}
