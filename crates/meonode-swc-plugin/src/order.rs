//! Evaluation-order safety analysis — the v0.2 rule (see the design doc's
//! "Evaluation-order safety (v0.2 rule)" section, which supersedes v1's
//! blanket "every prop value must be effect-free" bail).
//!
//! `partition.rs` physically reorders a compiled call's props: everything
//! moves into `c`/`d` buckets (all of `c` before all of `d`), and every
//! special key (`children`, `css`, `key`, ...) moves to the tail, in its
//! *original relative order among other special keys*. A leading `...spread`
//! (see Change 2 in the v0.2 doc) stays exactly where it was written, right
//! after `__meo$`.
//!
//! Effect-free values (literals, identifiers, arrow/function expressions,
//! nested effect-free literals — see `effect::is_effect_free`) can't observe
//! *or* cause side effects, so moving them anywhere is unobservable. Only the
//! relative order of *effectful* values matters: if compiling would emit two
//! effectful values in a different relative order than they appear in
//! source, that's an observable behavior change (whichever runs first could
//! matter — e.g. two calls with side effects, or evaluation order affecting
//! which throws first) and the call must bail.
//!
//! This module is the shared, standalone core of that check: given the
//! sequence of emitted "ranks" for a call's effectful values (in source
//! order), does that sequence stay non-decreasing? It has no AST knowledge at
//! all — `detect.rs` is responsible for walking the object literal, deciding
//! which values are effectful (`effect::is_effect_free`) and which
//! [`EmitRank`] each lands in (spread / `c` / `d` / special, using
//! `css_props::is_css_prop` and `keys::is_special_key`), then handing the
//! resulting sequence to [`order_preserved`].

/// Where a prop (or spread) lands in the emitted object, for evaluation-order
/// purposes. Lower variants are emitted earlier; the `Ord` derive gives us
/// exactly the comparison [`order_preserved`] needs.
///
/// Two items with the *same* rank never need special handling here: neither
/// `partition.rs`'s bucketing nor its special-key tail reorders same-rank
/// entries relative to each other (they're pushed in source order and kept
/// there), so only a rank *decrease* across source order can indicate an
/// inversion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EmitRank {
    /// A leading `...spread` — stays literally in place at the front of the
    /// emitted object, immediately after `__meo$`. Always ranks first: v0.2's
    /// leading-spread rule (Change 2) only accepts spreads that already
    /// precede every static prop in source, so a spread's source position is
    /// never later than any `Css`/`Data`/`Special` item's anyway.
    ///
    /// Also used for a non-special prop that stays flat (unbucketed)
    /// *because* a spread is present and its value isn't a static literal —
    /// see `detect::rank_for_prop` and the stable-key-hazard doc comment on
    /// `detect::validate_object`. Such a prop is never bucketed into `c`/`d`
    /// when a spread is present, so it shares the spread's emitted position
    /// class exactly.
    Spread,
    /// A non-special key recognized by `css_props::is_css_prop` — bucketed
    /// into `c`.
    Css,
    /// A non-special key not recognized as CSS — bucketed into `d`.
    Data,
    /// A special key (`children`, `css`, `key`, `ref`, `props`, `as`,
    /// `theme`, `disableEmotion`) — left top-level at the tail, in original
    /// relative source order among themselves.
    Special,
}

/// Returns `true` if the emitted-rank sequence of a call's effectful prop
/// values — read in source order — is non-decreasing, i.e. compiling this
/// call site would not reorder any effectful value relative to any other
/// effectful value.
///
/// Trivially `true` for 0 or 1 elements — the case that covers the dominant
/// real-world pattern (`key: item.id`, a single dynamic `backgroundColor`, a
/// ternary on one prop) per the v0.2 design doc.
pub fn order_preserved(
    effectful_ranks_in_source_order: impl IntoIterator<Item = EmitRank>,
) -> bool {
    let mut prev: Option<EmitRank> = None;
    for rank in effectful_ranks_in_source_order {
        if let Some(prev_rank) = prev {
            if rank < prev_rank {
                return false;
            }
        }
        prev = Some(rank);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use EmitRank::*;

    #[test]
    fn zero_effectful_values_is_trivially_preserved() {
        assert!(order_preserved(Vec::<EmitRank>::new()));
    }

    #[test]
    fn one_effectful_value_is_trivially_preserved() {
        assert!(order_preserved([Special]));
        assert!(order_preserved([Css]));
        assert!(order_preserved([Data]));
        assert!(order_preserved([Spread]));
    }

    #[test]
    fn two_effectful_values_same_rank_preserved() {
        // Same bucket: relative source order is preserved by construction
        // (partition.rs never reorders within a bucket), so equal ranks are
        // always fine regardless of which bucket.
        assert!(order_preserved([Css, Css]));
        assert!(order_preserved([Data, Data]));
        assert!(order_preserved([Special, Special]));
    }

    #[test]
    fn two_effectful_values_increasing_rank_preserved() {
        assert!(order_preserved([Spread, Css]));
        assert!(order_preserved([Css, Data]));
        assert!(order_preserved([Data, Special]));
        assert!(order_preserved([Spread, Special]));
    }

    #[test]
    fn two_effectful_values_decreasing_rank_violates_order() {
        // A `d`-bucket effectful value in source before a `c`-bucket
        // effectful value: emission puts `c` first, so this is exactly the
        // reordering v0.2 must reject.
        assert!(!order_preserved([Data, Css]));
        assert!(!order_preserved([Special, Data]));
        assert!(!order_preserved([Special, Css]));
        assert!(!order_preserved([Css, Spread]));
    }

    #[test]
    fn three_effectful_values_all_non_decreasing_preserved() {
        assert!(order_preserved([Spread, Css, Css]));
        assert!(order_preserved([Css, Data, Special]));
        assert!(order_preserved([Spread, Data, Special]));
    }

    #[test]
    fn three_effectful_values_single_inversion_in_middle_violates_order() {
        // Css, Special, Data: the middle-to-last step (Special -> Data) is a
        // decrease, even though the first step (Css -> Special) increases.
        assert!(!order_preserved([Css, Special, Data]));
    }

    #[test]
    fn three_effectful_values_last_pair_inverted_violates_order() {
        assert!(!order_preserved([Spread, Data, Css]));
    }

    #[test]
    fn many_same_rank_values_are_fine() {
        assert!(order_preserved([Data, Data, Data, Data, Data]));
    }
}
