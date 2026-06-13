//! Confidence Policy (issue #95) — the **pure**, unit-tested math governing how
//! a **Conclusion**'s **Confidence** forms, fades, hides, and resurfaces.
//!
//! This is the **fixed product policy** from `docs/user-context/CONTEXT.md`
//! ("Confidence Policy"): the user has **no decay sliders**. The constants below
//! are tuning — the *decision* is the bias toward **stability**, so the dossier
//! reads as a considered judgment, not a mood ring. Every function is pure and
//! takes an explicit `now_ms`, so the worker and the tests share the same math
//! without wall-clock nondeterminism.
//!
//! Confidence is the recency-weighting of evidence: recent supporting
//! **Activity** values push it up, silence lets it sink on its own (a quiet fade,
//! even with nothing contradicting), and contradicting Activities push it down
//! *faster*. One half-life rule yields both the quiet fade and the active
//! reversal — the evidence links' recency *is* the confidence.

use capture_types::ConclusionStatus;

// === Fixed-as-policy constants (stability-biased; values are tuning) ========

/// **Formation bar.** A Conclusion needs at least this many supporting
/// Activities before it forms — no flimsy one-afternoon conclusions. Stability:
/// repeated evidence before anything appears in the dossier.
pub const FORMATION_BAR_EVIDENCE: usize = 2;

/// **Display floor.** Below this confidence a Conclusion leaves the *visible*
/// dossier (status `faded`) but its **Confidence History** is kept, so the
/// Subject page can still show the faded arc. Faded is **not** deleted.
pub const DISPLAY_FLOOR: f64 = 0.15;

/// **Fade half-life (days).** The slow silence fade: with no fresh supporting
/// evidence, confidence halves every this-many days. 30 days is deliberately
/// long — a quiet stretch must not erase a trait (that is what **Pin** protects).
pub const FADE_HALF_LIFE_DAYS: f64 = 30.0;

/// **Contradiction drop.** Each contradicting Activity subtracts this much
/// confidence — far more than an equivalent stretch of silence would, so an
/// active reversal moves faster than a quiet fade (CONTEXT.md: "contradicting
/// Activity values push it down faster").
pub const CONTRADICTION_DROP: f64 = 0.35;

/// **Resurface multiplier.** Overturning a **Dismiss** takes substantially more
/// fresh evidence than forming the conclusion took: at least this multiple of the
/// evidence that the dismissed conclusion was originally built on. (Consumed by
/// the Dismissal/resurface slice #99; implemented here so the policy is complete.)
pub const RESURFACE_EVIDENCE_MULTIPLIER: f64 = 2.0;

/// Base confidence a brand-new Conclusion starts from before its supporting
/// evidence lifts it. Low so a freshly-formed Conclusion is provisional.
const INITIAL_BASE: f64 = 0.30;

/// Confidence added per supporting Activity at formation time.
const INITIAL_SUPPORT_INCREMENT: f64 = 0.12;

/// Upper bound a freshly-formed Conclusion may reach: confidence earns its way
/// toward 1.0 over time (with sustained support), it is not granted at birth.
const INITIAL_CAP: f64 = 0.90;

/// Milliseconds in a day, for the silence-elapsed → days conversion.
const MS_PER_DAY: f64 = 24.0 * 60.0 * 60.0 * 1000.0;

/// Clamp a confidence value into the valid `[0.0, 1.0]` band.
fn clamp(confidence: f64) -> f64 {
    confidence.clamp(0.0, 1.0)
}

/// Initial confidence for a freshly-distilled Conclusion. Rises with supporting
/// evidence, is lowered by any contradictions, and is clamped to a sane starting
/// band so a Conclusion is never *born* near-certain (it earns certainty over
/// time). `support_count` is expected to already meet [`meets_formation_bar`].
pub fn initial_confidence(support_count: usize, contradict_count: usize) -> f64 {
    let raw = INITIAL_BASE + INITIAL_SUPPORT_INCREMENT * support_count as f64
        - CONTRADICTION_DROP * contradict_count as f64;
    // Cap the formation value below 1.0 (earn-it-over-time), but never below 0.
    clamp(raw.min(INITIAL_CAP))
}

/// Exponential half-life decay over elapsed **silence** (days since
/// `last_supported_at_ms`). With no fresh support, confidence halves every
/// [`FADE_HALF_LIFE_DAYS`]. A **pinned** Conclusion is exempt — it returns
/// `current` unchanged (Pin protects against the quiet fade). The `now < last`
/// guard returns `current` (clock skew / out-of-order call must not *raise*
/// confidence).
pub fn decay(current: f64, last_supported_at_ms: i64, now_ms: i64, pinned: bool) -> f64 {
    if pinned {
        return clamp(current);
    }
    if now_ms <= last_supported_at_ms {
        return clamp(current);
    }
    let elapsed_ms = (now_ms - last_supported_at_ms) as f64;
    let elapsed_days = elapsed_ms / MS_PER_DAY;
    // current * 0.5 ^ (elapsed_days / half_life)
    let factor = 0.5_f64.powf(elapsed_days / FADE_HALF_LIFE_DAYS);
    clamp(current * factor)
}

/// Apply contradiction pressure: each contradicting Activity drops confidence by
/// [`CONTRADICTION_DROP`], which is far steeper than an equivalent silence fade,
/// so an active reversal outruns the quiet fade. Clamped to `[0.0, 1.0]`.
pub fn apply_contradiction(current: f64, contradiction_count: usize) -> f64 {
    clamp(current - CONTRADICTION_DROP * contradiction_count as f64)
}

/// Re-derivation **reinforcement** (#9/#10): fold a fresh distillation window's
/// support/contradiction into an *existing* Conclusion's confidence so it
/// **rises with fresh supporting evidence, fades slowly in silence, and drops
/// faster on contradiction** — rather than being overwritten by whatever the
/// latest window alone justifies.
///
/// The previous code set `confidence = initial_confidence(...)` on every
/// re-derivation, resetting a long-standing well-supported Conclusion to the
/// current window's formation value (often *lower*) and wiping the accrued
/// decay. This is the ratchet that earns confidence upward over time:
///
/// 1. **Decay the existing value to now** over silence since `decay_anchor_ms`
///    (`COALESCE(last_decayed_at_ms, last_supported_at_ms)`), so a stale
///    Conclusion does not keep an unearned-high baseline.
/// 2. **Ratchet up** to `max(decayed_existing, newly_justified)`, where
///    `newly_justified = initial_confidence(support, 0)` — fresh support can
///    only *raise* (or hold) confidence, never silently drop a supported
///    Conclusion to a lower formation value because this window saw less support.
/// 3. **Apply contradiction** on top via [`apply_contradiction`]: each fresh
///    contradicting Activity drops [`CONTRADICTION_DROP`], so an active reversal
///    moves the value DOWN faster than silence — and can pull it below the
///    ratcheted-up value (the contradiction path is wired into the persist path,
///    not only formation; this is what made the dead `apply_contradiction` live).
///
/// The result still respects the formation cap implicitly: `newly_justified`
/// is capped at [`INITIAL_CAP`] by `initial_confidence`, and `decayed_existing`
/// can only have come from prior reinforcement under the same rule (or formation),
/// so the ratchet never *grants* above what sustained support earned. `pinned`
/// is honored by the decay step (a pinned Conclusion does not fade).
pub fn reinforce(
    existing: f64,
    decay_anchor_ms: i64,
    now_ms: i64,
    support_count: usize,
    contradict_count: usize,
    pinned: bool,
) -> f64 {
    // 1. Fade the prior value to now (silence since the last support/decay).
    let decayed_existing = decay(existing, decay_anchor_ms, now_ms, pinned);
    // 2. Fresh support can only raise (or hold) — never reset to a lower window.
    let newly_justified = initial_confidence(support_count, 0);
    let ratcheted = decayed_existing.max(newly_justified);
    // 3. Contradiction actively reverses, faster than silence, on top of the ratchet.
    apply_contradiction(ratcheted, contradict_count)
}

/// Whether enough supporting evidence has accumulated for a Conclusion to form
/// (the **formation bar**: ≥ [`FORMATION_BAR_EVIDENCE`] supporting Activities).
pub fn meets_formation_bar(support_count: usize) -> bool {
    support_count >= FORMATION_BAR_EVIDENCE
}

/// Whether a confidence value sits below the **display floor** (and so the
/// Conclusion should leave the visible dossier as `faded`, history retained).
pub fn below_display_floor(confidence: f64) -> bool {
    confidence < DISPLAY_FLOOR
}

/// The visibility [`ConclusionStatus`] for a confidence value. Below the display
/// floor and **not pinned** → `Faded` (leaves the visible dossier, keeps its
/// history); otherwise `Visible`. A pinned Conclusion never fades. This **never**
/// returns `Dismissed` — dismissal is a user action (#99), not a confidence
/// outcome.
pub fn status_for(confidence: f64, pinned: bool) -> ConclusionStatus {
    if !pinned && below_display_floor(confidence) {
        ConclusionStatus::Faded
    } else {
        ConclusionStatus::Visible
    }
}

/// Whether *fresh* supporting evidence clears the high **resurface bar** for a
/// previously-dismissed Conclusion: the fresh support must be at least
/// [`RESURFACE_EVIDENCE_MULTIPLIER`]× the evidence the dismissed conclusion was
/// built on, so overturning a Dismiss takes substantially more than forming it
/// did (a correction never feels ignored). Consumed by the Dismissal slice (#99).
pub fn meets_resurface_bar(fresh_support_count: usize, prior_dismissal_evidence_count: i64) -> bool {
    // A non-positive prior count means there is effectively no bar to clear (no
    // evidence was recorded against the dismissal) — any fresh support resurfaces.
    if prior_dismissal_evidence_count <= 0 {
        return fresh_support_count > 0;
    }
    fresh_support_count as f64
        >= RESURFACE_EVIDENCE_MULTIPLIER * prior_dismissal_evidence_count as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY_MS: i64 = 24 * 60 * 60 * 1000;

    #[test]
    fn fresh_evidence_raises_confidence_over_none() {
        // More supporting evidence yields a higher initial confidence.
        let none = initial_confidence(0, 0);
        let some = initial_confidence(2, 0);
        let more = initial_confidence(4, 0);
        assert!(some > none, "support should raise initial confidence");
        assert!(more > some, "more support should raise it further");
        // Always in band.
        assert!((0.0..=1.0).contains(&none));
        assert!((0.0..=1.0).contains(&more));
    }

    #[test]
    fn initial_confidence_is_lowered_by_contradiction_and_capped() {
        // A contradiction lowers the formation value vs the same support alone.
        assert!(initial_confidence(3, 1) < initial_confidence(3, 0));
        // Never born above the formation cap (earn certainty over time).
        assert!(initial_confidence(100, 0) <= INITIAL_CAP);
        // Never below 0 even with heavy contradiction.
        assert!(initial_confidence(0, 100) >= 0.0);
    }

    #[test]
    fn silence_decays_more_with_more_days() {
        let start = 1_000_000_000_000;
        let current = 0.8;
        let after_15d = decay(current, start, start + 15 * DAY_MS, false);
        let after_30d = decay(current, start, start + 30 * DAY_MS, false);
        let after_60d = decay(current, start, start + 60 * DAY_MS, false);
        // Monotonically lower as silence lengthens.
        assert!(after_15d < current);
        assert!(after_30d < after_15d);
        assert!(after_60d < after_30d);
        // One half-life (30 days) ≈ half the confidence.
        assert!((after_30d - current * 0.5).abs() < 1e-9);
    }

    #[test]
    fn split_interval_decay_telescopes_to_one_half_life() {
        // Regression for the worker's chained-decay bug (#1): the decay beat fires
        // many times over a silence. If each pass anchors on the PREVIOUS pass's
        // decay time (the `decay_anchor_ms` the worker now passes) rather than the
        // fixed `last_supported_at_ms`, splitting one 30-day interval into two
        // 15-day passes must telescope to EXACTLY one 30-day half-life — not
        // re-apply the full elapsed factor each pass (which compounded to ~quadratic
        // decay). decay(15d) then decay(15d-from-the-new-anchor) == decay(30d).
        let start = 1_000_000_000_000;
        let c0 = 0.8;
        // Pass 1: 15 days of silence, anchored at the last support time.
        let mid = start + 15 * DAY_MS;
        let after_pass1 = decay(c0, start, mid, false);
        // Pass 2: another 15 days, anchored at the PREVIOUS decay time (`mid`), not
        // back at `start`. This is the per-pass-delta anchoring the fix introduces.
        let after_pass2 = decay(after_pass1, mid, mid + 15 * DAY_MS, false);
        // One-shot 30-day decay from the original anchor.
        let one_shot_30d = decay(c0, start, start + 30 * DAY_MS, false);
        assert!(
            (after_pass2 - one_shot_30d).abs() < 1e-9,
            "two 15d passes ({after_pass2}) must equal one 30d decay ({one_shot_30d})"
        );
        // And concretely ≈ C0 * 0.5 (one half-life).
        assert!((after_pass2 - c0 * 0.5).abs() < 1e-9);
    }

    #[test]
    fn no_elapsed_silence_leaves_confidence_unchanged() {
        let now = 2_000_000_000_000;
        // now == last and now < last both leave confidence unchanged (guard).
        assert_eq!(decay(0.6, now, now, false), 0.6);
        assert_eq!(decay(0.6, now, now - DAY_MS, false), 0.6);
    }

    #[test]
    fn pinned_conclusion_does_not_decay() {
        let start = 1_500_000_000_000;
        // A pinned Conclusion keeps its confidence across an arbitrary silence.
        let decayed = decay(0.7, start, start + 365 * DAY_MS, true);
        assert_eq!(decayed, 0.7, "Pin exempts a Conclusion from decay");
    }

    #[test]
    fn contradiction_drops_faster_than_equivalent_silence() {
        let start = 1_000_000_000_000;
        let current = 0.8;
        // A contradiction is an *instantaneous* reversal; silence only erodes
        // confidence as time passes. So over the SAME elapsed stretch (here a
        // week — a realistic re-derivation cadence), the contradiction drops far
        // more than the equivalent silence. This is the "faster" of CONTEXT.md.
        let by_contradiction = apply_contradiction(current, 1);
        let by_silence_7d = decay(current, start, start + 7 * DAY_MS, false);
        assert!(
            by_contradiction < by_silence_7d,
            "a contradiction must drop confidence faster than a comparable stretch of silence"
        );
        // Concretely: one CONTRADICTION_DROP (0.35) far exceeds a week of the
        // 30-day half-life fade (~15% of the value), so the gap is large.
        assert!(
            (current - by_contradiction) > (current - by_silence_7d),
            "the contradiction's drop magnitude exceeds the silence drop"
        );
        // Multiple contradictions clamp at the floor, never below 0.
        assert_eq!(apply_contradiction(0.2, 10), 0.0);
    }

    #[test]
    fn below_floor_maps_to_faded_history_idea() {
        // A below-floor, unpinned Conclusion fades (leaves the visible dossier);
        // its Confidence History is kept by the store, so the arc survives.
        assert!(below_display_floor(0.10));
        assert!(!below_display_floor(DISPLAY_FLOOR));
        assert_eq!(status_for(0.10, false), ConclusionStatus::Faded);
        // A pinned below-floor Conclusion stays visible (Pin protects it).
        assert_eq!(status_for(0.10, true), ConclusionStatus::Visible);
        // Above the floor is always visible.
        assert_eq!(status_for(0.5, false), ConclusionStatus::Visible);
        // status_for never dismisses.
        assert_ne!(status_for(0.0, false), ConclusionStatus::Dismissed);
        assert_ne!(status_for(1.0, true), ConclusionStatus::Dismissed);
    }

    #[test]
    fn resurface_needs_multiplier_times_prior_evidence() {
        // Prior dismissal built on 2 Activities → needs ≥ 4 fresh (2.0×).
        assert!(!meets_resurface_bar(2, 2), "equal evidence must not resurface");
        assert!(!meets_resurface_bar(3, 2), "below 2× must not resurface");
        assert!(meets_resurface_bar(4, 2), "exactly 2× resurfaces");
        assert!(meets_resurface_bar(5, 2), "above 2× resurfaces");
        // No recorded prior evidence → any fresh support resurfaces.
        assert!(meets_resurface_bar(1, 0));
        assert!(!meets_resurface_bar(0, 0));
    }

    #[test]
    fn reinforce_ratchets_up_and_never_resets_to_a_lower_window() {
        // A long-standing well-supported Conclusion (0.85) re-derived in a window
        // that alone would only justify a low formation value (2 supports → 0.54)
        // must NOT be reset downward (#9): fresh support holds/raises, never drops.
        let start = 1_000_000_000_000;
        let well_supported = 0.85;
        let reinforced = reinforce(well_supported, start, start, 2, 0, false);
        assert!(
            reinforced >= well_supported - 1e-9,
            "fresh support must not reset a well-supported conclusion downward"
        );

        // More fresh support than the existing value justifies ratchets it UP.
        let provisional = 0.40;
        let raised = reinforce(provisional, start, start, 4, 0, false);
        assert!(raised > provisional, "stronger fresh support raises confidence");
    }

    #[test]
    fn reinforce_fades_existing_over_silence_before_ratcheting() {
        // With NO time elapsed the prior value is preserved; with a long silence the
        // prior value decays first, so the ratchet baseline is the faded value.
        let start = 1_000_000_000_000;
        let existing = 0.80;
        let no_silence = reinforce(existing, start, start, 0, 0, false);
        let after_30d = reinforce(existing, start, start + 30 * DAY_MS, 0, 0, false);
        assert!((no_silence - existing).abs() < 1e-9);
        assert!(after_30d < existing, "silence fades the prior value first");
        assert!((after_30d - existing * 0.5).abs() < 1e-9, "≈ one half-life");
    }

    #[test]
    fn reinforce_wires_contradiction_to_drop_faster_than_silence() {
        // #10: the contradiction path is LIVE on re-derivation — a contradicting
        // Activity drops the reinforced value faster than a quiet fade would.
        let start = 1_000_000_000_000;
        let existing = 0.80;
        let with_contradiction = reinforce(existing, start, start, 2, 1, false);
        let support_only = reinforce(existing, start, start, 2, 0, false);
        assert!(
            with_contradiction < support_only,
            "a fresh contradiction actively lowers the reinforced confidence"
        );
        // And the drop is at least one CONTRADICTION_DROP below the support-only value.
        assert!((support_only - with_contradiction) >= CONTRADICTION_DROP - 1e-9);
    }

    #[test]
    fn reinforce_honors_pin_no_fade() {
        // A pinned Conclusion does not fade during the reinforce decay step.
        let start = 1_000_000_000_000;
        let existing = 0.70;
        let pinned = reinforce(existing, start, start + 365 * DAY_MS, 0, 0, true);
        assert!((pinned - existing).abs() < 1e-9, "Pin exempts the fade step");
    }

    #[test]
    fn formation_bar_gates_flimsy_conclusions() {
        assert!(!meets_formation_bar(0));
        assert!(!meets_formation_bar(1));
        assert!(meets_formation_bar(FORMATION_BAR_EVIDENCE));
        assert!(meets_formation_bar(5));
    }
}
