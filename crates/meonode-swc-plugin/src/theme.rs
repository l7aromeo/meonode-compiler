//! Build-time `theme.*` -> `var(--meonode-theme-*)` rewriting.
//!
//! `@meonode/ui` already performs this exact conversion at runtime, on every
//! render, for every node: `replaceThemeTokensWithCssVars` walks each node's
//! `elementProps` on the server, and the client reaches the same `var(...)`
//! form via `ThemeUtil.resolveObjWithTheme(..., { themeStringsMode: 'vars' })`.
//! So this module changes *when* the conversion happens, never *what* it
//! produces — which is what makes the transform safe to apply unconditionally.
//!
//! ## Why it is worth doing at build time
//!
//! The runtime memoizes conversions in a `WeakMap` keyed by object identity,
//! but (per its own doc comment) that only helps an object "defined outside a
//! render body". Every inline call site — `Div({ padding: 'theme.spacing.md' })`
//! — allocates a fresh props object per render, so the cache never hits and the
//! full copy-on-write walk re-runs every time.
//!
//! Measured by `e2e/bench-theme-tokens.mjs`, which drives this actual plugin
//! over a 156-node tree at the docs site's real token density (~0.8 token
//! strings per compiled call site) under `renderToPipeableStream` with
//! production React/Emotion builds:
//!
//! ```text
//! partitioning only (tokens stripped):   1.4305 -> 1.3124 ms/render   8.3% faster
//! partitioning + this rewrite:           1.5426 -> 1.3084 ms/render  15.2% faster
//! ```
//!
//! So this roughly doubles the compiler's end-to-end SSR gain, adding ~7 points
//! on top of prop partitioning's 8.3%. Note the two compiled timings are
//! effectively identical (1.3084 vs 1.3124) while the uncompiled ones differ by
//! 0.11 ms — that gap *is* the runtime token cost, and the rewrite recovers
//! essentially all of it.
//!
//! An earlier estimate of "14% incremental" came from modelling compiled output
//! with hand-written marker props rather than running the plugin; it overstated
//! the incremental share. The numbers above supersede it.
//!
//! Once rewritten, values read `var(--meonode-theme-spacing-md)`, which does not
//! contain the substring `theme.`, so the runtime's per-string
//! `input.includes('theme.')` guard short-circuits and no replacement work
//! happens. The object *walk* still runs — eliminating that needs a separate
//! "this call site is token-free" marker the runtime can act on, which is
//! deliberately not part of this change.
//!
//! ## Fidelity requirements
//!
//! Both functions below must mirror `@meonode/ui`'s implementations exactly,
//! quirks included, or compiled and uncompiled call sites would disagree:
//!
//! - The scan pattern is `/theme\.([a-zA-Z0-9_.-]+)/g`, with **no word
//!   boundary** before `theme`. `'mytheme.primary'` therefore matches at offset
//!   2 and yields `'myvar(--meonode-theme-primary)'`. That is what the runtime
//!   does today; mirroring it is correct, "improving" on it is not.
//! - The captured path is greedy over `[a-zA-Z0-9_.-]`, so `.` and `-` are
//!   consumed. `'theme.spacing.md.'` captures `spacing.md.` and produces a
//!   trailing `-`, matching the runtime.
//! - A path that names nothing in the theme still becomes a `var()` reference.
//!   `buildThemeVariablesCss` only walks `theme.system`, so e.g. `'theme.mode'`
//!   compiles to `var(--meonode-theme-mode)`, which no `:root` rule defines.
//!   That is a pre-existing runtime quirk, reproduced here rather than fixed,
//!   because diverging would make compiled output behave differently from
//!   uncompiled output.
//!
//! ## What callers must not rewrite
//!
//! Only string **values** are eligible. Object *keys* holding theme tokens
//! (e.g. `'@media (max-width: theme.breakpoint.md)'`) must keep their tokens
//! and resolve to concrete values at runtime, because CSS variables are invalid
//! inside media features and selector text. `@meonode/ui` documents this same
//! invariant on `replaceThemeTokensWithCssVars` and defers key resolution to
//! `resolveObjWithTheme`, which has the live theme. `partition.rs` upholds it by
//! only ever passing prop *values* here.

/// Mirrors `@meonode/ui`'s `toThemeVarName`:
/// `--meonode-theme-${path.replace(/[^\w.-]/g, '-').replace(/\./g, '-')}`.
///
/// `\w` in JavaScript (without the `u` flag) is exactly `[A-Za-z0-9_]`, so the
/// first replacement maps anything outside `[A-Za-z0-9_.-]` to `-`, and the
/// second collapses `.` to `-`. Both steps are implemented even though callers
/// only ever pass paths already restricted to that set (making step one a
/// no-op), so this stays a faithful port if the caller's pattern ever widens.
pub fn to_theme_var_name(path: &str) -> String {
    let mut out = String::with_capacity(path.len() + 17);
    out.push_str("--meonode-theme-");
    for ch in path.chars() {
        let mapped = if ch == '.' {
            // Step two: `.` -> `-`.
            '-'
        } else if is_js_word_char(ch) || ch == '-' {
            // Kept by step one, untouched by step two.
            ch
        } else {
            // Step one: anything outside `[\w.-]` -> `-`.
            '-'
        };
        out.push(mapped);
    }
    out
}

/// JavaScript's `\w` character class: `[A-Za-z0-9_]`. Deliberately ASCII-only —
/// JS `\w` does not match non-ASCII letters without the `u` flag, and the
/// runtime's regex has no `u` flag.
fn is_js_word_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

/// Characters accepted by the runtime's `([a-zA-Z0-9_.-]+)` capture group.
fn is_theme_path_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_' || ch == '.' || ch == '-'
}

/// Replaces every `theme.<path>` occurrence in `input` with
/// `var(--meonode-theme-<path>)`, mirroring `/theme\.([a-zA-Z0-9_.-]+)/g`.
///
/// Returns `None` when nothing matched, so callers can leave the AST untouched
/// (and thus preserve spans/formatting) in the common no-token case.
///
/// A bare `theme.` with no following path character does **not** match, since
/// the capture group requires one or more characters — matching the runtime,
/// where `'theme.'` is left alone.
pub fn rewrite_theme_tokens(input: &str) -> Option<String> {
    // Cheap reject first: the overwhelming majority of prop values contain no
    // token at all, and this mirrors the runtime's own `includes` guard.
    if !input.contains("theme.") {
        return None;
    }

    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0usize;
    let mut matched = false;

    while i < bytes.len() {
        // `theme.` is pure ASCII, so byte-wise search is safe on UTF-8 input:
        // a multi-byte character's continuation bytes are all >= 0x80 and can
        // never equal any byte of the ASCII needle.
        if input[i..].starts_with("theme.") {
            let path_start = i + "theme.".len();
            let path_end = input[path_start..]
                .char_indices()
                .find(|(_, ch)| !is_theme_path_char(*ch))
                .map_or(bytes.len(), |(offset, _)| path_start + offset);

            if path_end > path_start {
                out.push_str("var(");
                out.push_str(&to_theme_var_name(&input[path_start..path_end]));
                out.push(')');
                matched = true;
                i = path_end;
                continue;
            }
        }

        // Advance one *character*, not one byte, so multi-byte content is
        // copied intact.
        let ch = input[i..].chars().next().expect("index is a char boundary");
        out.push(ch);
        i += ch.len_utf8();
    }

    if matched {
        Some(out)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn var_name_collapses_dots_to_hyphens() {
        assert_eq!(
            to_theme_var_name("spacing.md"),
            "--meonode-theme-spacing-md"
        );
        assert_eq!(
            to_theme_var_name("base.content"),
            "--meonode-theme-base-content"
        );
    }

    #[test]
    fn var_name_keeps_word_chars_and_hyphens() {
        assert_eq!(
            to_theme_var_name("my_token-2"),
            "--meonode-theme-my_token-2"
        );
    }

    #[test]
    fn var_name_maps_non_word_chars_to_hyphens() {
        // Not reachable from the current capture group, but the port must still
        // behave like the JS `[^\w.-]` replacement if a caller ever widens.
        assert_eq!(to_theme_var_name("a b"), "--meonode-theme-a-b");
        assert_eq!(to_theme_var_name("café"), "--meonode-theme-caf-");
    }

    #[test]
    fn rewrites_a_bare_token() {
        assert_eq!(
            rewrite_theme_tokens("theme.primary").as_deref(),
            Some("var(--meonode-theme-primary)")
        );
    }

    #[test]
    fn rewrites_a_token_embedded_in_a_shorthand_value() {
        assert_eq!(
            rewrite_theme_tokens("1px solid theme.base.deep").as_deref(),
            Some("1px solid var(--meonode-theme-base-deep)")
        );
    }

    #[test]
    fn rewrites_every_occurrence() {
        assert_eq!(
            rewrite_theme_tokens("theme.spacing.sm theme.spacing.md").as_deref(),
            Some("var(--meonode-theme-spacing-sm) var(--meonode-theme-spacing-md)")
        );
    }

    #[test]
    fn returns_none_when_no_token_present() {
        assert_eq!(rewrite_theme_tokens("16px"), None);
        assert_eq!(rewrite_theme_tokens(""), None);
        assert_eq!(rewrite_theme_tokens("#007ACC"), None);
    }

    #[test]
    fn bare_theme_dot_with_no_path_does_not_match() {
        // The capture group requires at least one path character.
        assert_eq!(rewrite_theme_tokens("theme."), None);
        assert_eq!(rewrite_theme_tokens("theme. "), None);
    }

    #[test]
    fn stops_the_path_at_the_first_disallowed_character() {
        assert_eq!(
            rewrite_theme_tokens("theme.primary, theme.base").as_deref(),
            Some("var(--meonode-theme-primary), var(--meonode-theme-base)")
        );
        assert_eq!(
            rewrite_theme_tokens("calc(theme.spacing.md + 2px)").as_deref(),
            Some("calc(var(--meonode-theme-spacing-md) + 2px)")
        );
    }

    #[test]
    fn greedy_path_consumes_trailing_dot_like_the_runtime() {
        // `[a-zA-Z0-9_.-]+` includes `.`, so a trailing dot is captured and
        // becomes a trailing `-`. Matches @meonode/ui, quirk included.
        assert_eq!(
            rewrite_theme_tokens("theme.spacing.md.").as_deref(),
            Some("var(--meonode-theme-spacing-md-)")
        );
    }

    #[test]
    fn matches_without_a_word_boundary_like_the_runtime() {
        // No `\b` in the runtime regex, so this is a real match at offset 2.
        // Reproduced deliberately: diverging would make compiled output differ
        // from uncompiled output.
        assert_eq!(
            rewrite_theme_tokens("mytheme.primary").as_deref(),
            Some("myvar(--meonode-theme-primary)")
        );
    }

    #[test]
    fn preserves_multibyte_content_around_a_token() {
        assert_eq!(
            rewrite_theme_tokens("→ theme.primary ←").as_deref(),
            Some("→ var(--meonode-theme-primary) ←")
        );
    }

    #[test]
    fn multibyte_immediately_after_a_token_terminates_the_path() {
        assert_eq!(
            rewrite_theme_tokens("theme.primary→").as_deref(),
            Some("var(--meonode-theme-primary)→")
        );
    }

    #[test]
    fn token_at_end_of_input_is_rewritten() {
        assert_eq!(
            rewrite_theme_tokens("solid theme.base").as_deref(),
            Some("solid var(--meonode-theme-base)")
        );
    }

    #[test]
    fn already_rewritten_value_is_left_alone() {
        // Idempotence matters: `var(--meonode-theme-x)` has no `theme.`
        // substring, so a second pass is a no-op. This is also exactly why the
        // runtime's own `includes('theme.')` guard short-circuits on compiled
        // output.
        assert_eq!(rewrite_theme_tokens("var(--meonode-theme-primary)"), None);
    }
}
