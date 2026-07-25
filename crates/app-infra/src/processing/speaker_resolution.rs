//! Cross-segment speaker identity resolution — the pure decision layer.
//!
//! When a diarization job produces a cluster, it is compared against every
//! existing cluster **in the same session** and the outcome is decided purely
//! from cosine scores: auto-reuse an existing cluster, store a merge suggestion
//! for the user, or mint a new speaker. That decision is this module.
//!
//! It lives apart from `store.rs` because it is the tuning seam: the thresholds
//! below are the constants behind the "merge with Unknown Speaker 2, 3, 5…"
//! chain, and `scripts/diarization_bench/segment_identity_bench.py` replays this
//! exact function over dumped embeddings to measure what changing them does.
//! Keeping it pure (no DB, no async) is what makes that replay possible without
//! re-implementing — and silently drifting from — the shipped rules.

use serde::{Deserialize, Serialize};

/// Cosine score at or above which an incoming cluster silently reuses an
/// existing one, with no user-visible suggestion.
pub const SPEAKER_CLUSTER_AUTO_REUSE_THRESHOLD: f32 = 0.82;
/// Cosine score at or above which a merge suggestion is stored for the user.
pub const SPEAKER_CLUSTER_SUGGEST_MERGE_THRESHOLD: f32 = 0.68;
/// Auto-reuse is blocked when the top two candidates are within this margin.
pub const SPEAKER_CLUSTER_AMBIGUITY_MARGIN: f32 = 0.06;

/// The thresholds behind [`resolve_stable_speaker_cluster_from_candidates`].
///
/// [`Default`] is exactly the shipped behavior, so production call sites pass
/// `Default::default()` and only the benchmark harness varies them.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SpeakerResolutionTuning {
    pub auto_reuse_threshold: f32,
    pub suggest_merge_threshold: f32,
    pub ambiguity_margin: f32,
    /// When true, a near-tie between the top two candidates only blocks
    /// auto-reuse if those candidates are *different identities*. Several
    /// fragments of one voice scoring alike is evidence for merging, not
    /// against it. `false` is the shipped behavior.
    pub person_aware_ambiguity: bool,
    /// When true, a confirmed recognition steers as well as vetoes: if the
    /// top-scoring candidate belongs to a different person, fall back to the
    /// best candidate that actually *is* the recognized person rather than
    /// suggesting a merge with the wrong one. `false` is the shipped behavior.
    pub recognition_steers: bool,
}

impl Default for SpeakerResolutionTuning {
    fn default() -> Self {
        Self {
            auto_reuse_threshold: SPEAKER_CLUSTER_AUTO_REUSE_THRESHOLD,
            suggest_merge_threshold: SPEAKER_CLUSTER_SUGGEST_MERGE_THRESHOLD,
            ambiguity_margin: SPEAKER_CLUSTER_AMBIGUITY_MARGIN,
            person_aware_ambiguity: false,
            recognition_steers: false,
        }
    }
}

/// One existing cluster scored against the incoming one.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StableSpeakerClusterCandidate {
    pub id: i64,
    pub score: f32,
    pub person_id: Option<i64>,
}

/// What to do with the incoming cluster.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StableSpeakerClusterResolution {
    pub auto_merge_target_cluster_id: Option<i64>,
    pub suggested_merge_target_cluster_id: Option<i64>,
    pub suggested_merge_score: Option<f32>,
}

/// Decide the incoming cluster's fate from its scored candidates.
///
/// `candidates` is sorted in place by descending score (ties broken by lowest
/// id, so the outcome is stable regardless of row order).
pub fn resolve_stable_speaker_cluster_from_candidates(
    candidates: &mut [StableSpeakerClusterCandidate],
    recognition_person_id: Option<i64>,
    tuning: &SpeakerResolutionTuning,
) -> StableSpeakerClusterResolution {
    candidates.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.id.cmp(&right.id))
    });

    let Some(mut best) = candidates.first().copied() else {
        return StableSpeakerClusterResolution::default();
    };
    let mut second_score = candidates.get(1).map(|candidate| candidate.score);

    // A confirmed recognition can steer, not just veto: prefer the best
    // candidate that already belongs to the recognized person. Without this the
    // resolver suggests merging with whichever cluster scored highest — even
    // when recognition has already said it is somebody else.
    if tuning.recognition_steers {
        if let Some(person_id) = recognition_person_id {
            if best.person_id.is_some_and(|owner| owner != person_id) {
                if let Some(owned) = candidates
                    .iter()
                    .find(|candidate| candidate.person_id == Some(person_id))
                    .copied()
                {
                    best = owned;
                    // Re-derive the runner-up relative to the new best.
                    second_score = candidates
                        .iter()
                        .filter(|candidate| candidate.id != owned.id)
                        .map(|candidate| candidate.score)
                        .fold(None, |acc: Option<f32>, score| {
                            Some(acc.map_or(score, |current| current.max(score)))
                        });
                }
            }
        }
    }

    let near_tie = second_score.is_some_and(|score| best.score - score < tuning.ambiguity_margin);
    let ambiguous = near_tie
        && !(tuning.person_aware_ambiguity && tie_is_same_identity(candidates, &best, tuning));
    let confirmed_person_conflict = recognition_person_id.zip(best.person_id).is_some_and(
        |(incoming_person_id, existing_person_id)| incoming_person_id != existing_person_id,
    );

    if best.score >= tuning.auto_reuse_threshold && !ambiguous && !confirmed_person_conflict {
        StableSpeakerClusterResolution {
            auto_merge_target_cluster_id: Some(best.id),
            ..Default::default()
        }
    } else if best.score >= tuning.suggest_merge_threshold {
        StableSpeakerClusterResolution {
            suggested_merge_target_cluster_id: Some(best.id),
            suggested_merge_score: Some(best.score),
            ..Default::default()
        }
    } else {
        StableSpeakerClusterResolution::default()
    }
}

/// True when every candidate tied with `best` plausibly *is* `best` — same
/// assigned person, or all still unnamed. Unnamed clusters scoring alike are
/// treated as fragments of one voice; that is the whole point of the rule.
fn tie_is_same_identity(
    candidates: &[StableSpeakerClusterCandidate],
    best: &StableSpeakerClusterCandidate,
    tuning: &SpeakerResolutionTuning,
) -> bool {
    candidates
        .iter()
        .filter(|candidate| {
            candidate.id != best.id && best.score - candidate.score < tuning.ambiguity_margin
        })
        .all(|candidate| match (best.person_id, candidate.person_id) {
            // Both named: only the same person is not a conflict.
            (Some(left), Some(right)) => left == right,
            // ponytail: one named + one unnamed is treated as a conflict — the
            // unnamed one might be somebody else and auto-merging would silently
            // put a stranger under a real name. Suggest instead.
            (Some(_), None) | (None, Some(_)) => false,
            // Neither named: near-identical scores read as one over-clustered voice.
            (None, None) => true,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(id: i64, score: f32, person_id: Option<i64>) -> StableSpeakerClusterCandidate {
        StableSpeakerClusterCandidate {
            id,
            score,
            person_id,
        }
    }

    fn shipped() -> SpeakerResolutionTuning {
        SpeakerResolutionTuning::default()
    }

    #[test]
    fn auto_reuses_unambiguous_high_similarity() {
        let mut candidates = vec![candidate(1, 0.83, None), candidate(2, 0.70, None)];

        let resolution =
            resolve_stable_speaker_cluster_from_candidates(&mut candidates, None, &shipped());

        assert_eq!(resolution.auto_merge_target_cluster_id, Some(1));
        assert_eq!(resolution.suggested_merge_target_cluster_id, None);
    }

    #[test]
    fn suggests_for_ambiguous_high_similarity() {
        let mut candidates = vec![candidate(1, 0.83, None), candidate(2, 0.78, None)];

        let resolution =
            resolve_stable_speaker_cluster_from_candidates(&mut candidates, None, &shipped());

        assert_eq!(resolution.auto_merge_target_cluster_id, None);
        assert_eq!(resolution.suggested_merge_target_cluster_id, Some(1));
    }

    #[test]
    fn suggests_for_medium_similarity() {
        let mut candidates = vec![candidate(1, 0.70, None)];

        let resolution =
            resolve_stable_speaker_cluster_from_candidates(&mut candidates, None, &shipped());

        assert_eq!(resolution.auto_merge_target_cluster_id, None);
        assert_eq!(resolution.suggested_merge_target_cluster_id, Some(1));
        assert_eq!(resolution.suggested_merge_score, Some(0.70));
    }

    #[test]
    fn creates_independent_for_low_similarity() {
        let mut candidates = vec![candidate(1, 0.67, None)];

        let resolution =
            resolve_stable_speaker_cluster_from_candidates(&mut candidates, None, &shipped());

        assert_eq!(resolution.auto_merge_target_cluster_id, None);
        assert_eq!(resolution.suggested_merge_target_cluster_id, None);
    }

    #[test]
    fn has_no_match_without_candidates() {
        let mut candidates = vec![];

        let resolution =
            resolve_stable_speaker_cluster_from_candidates(&mut candidates, None, &shipped());

        assert_eq!(resolution.auto_merge_target_cluster_id, None);
        assert_eq!(resolution.suggested_merge_target_cluster_id, None);
    }

    #[test]
    fn confirmed_person_conflict_blocks_auto_reuse() {
        let mut candidates = vec![candidate(1, 0.90, Some(10))];

        let resolution =
            resolve_stable_speaker_cluster_from_candidates(&mut candidates, Some(20), &shipped());

        assert_eq!(resolution.auto_merge_target_cluster_id, None);
        assert_eq!(resolution.suggested_merge_target_cluster_id, Some(1));
    }

    // --- the behaviors the harness exists to measure --------------------------

    #[test]
    fn shipped_rules_suggest_merging_with_the_wrong_person() {
        // Recognition says "this is person 20", but person 10's cluster scores
        // highest. Shipped behavior blocks the auto-merge and then suggests
        // merging with person 10 anyway.
        let mut candidates = vec![candidate(1, 0.90, Some(10)), candidate(2, 0.75, Some(20))];

        let resolution =
            resolve_stable_speaker_cluster_from_candidates(&mut candidates, Some(20), &shipped());

        assert_eq!(resolution.suggested_merge_target_cluster_id, Some(1));
    }

    #[test]
    fn recognition_steers_to_the_recognized_person() {
        let mut candidates = vec![candidate(1, 0.90, Some(10)), candidate(2, 0.75, Some(20))];
        let tuning = SpeakerResolutionTuning {
            recognition_steers: true,
            ..Default::default()
        };

        let resolution =
            resolve_stable_speaker_cluster_from_candidates(&mut candidates, Some(20), &tuning);

        assert_eq!(resolution.suggested_merge_target_cluster_id, Some(2));
        assert_eq!(resolution.auto_merge_target_cluster_id, None);
    }

    #[test]
    fn person_aware_ambiguity_merges_fragments_of_one_voice() {
        // Three unnamed fragments of the same speaker, all scoring alike.
        // Shipped rules call this ambiguous and refuse; person-aware rules
        // read it as one over-clustered voice.
        let mut candidates = vec![
            candidate(1, 0.86, None),
            candidate(2, 0.84, None),
            candidate(3, 0.83, None),
        ];
        let tuning = SpeakerResolutionTuning {
            person_aware_ambiguity: true,
            ..Default::default()
        };

        assert_eq!(
            resolve_stable_speaker_cluster_from_candidates(
                &mut candidates.clone(),
                None,
                &shipped()
            )
            .auto_merge_target_cluster_id,
            None,
        );
        assert_eq!(
            resolve_stable_speaker_cluster_from_candidates(&mut candidates, None, &tuning)
                .auto_merge_target_cluster_id,
            Some(1),
        );
    }

    #[test]
    fn person_aware_ambiguity_still_blocks_two_different_people() {
        let mut candidates = vec![candidate(1, 0.86, Some(10)), candidate(2, 0.84, Some(20))];
        let tuning = SpeakerResolutionTuning {
            person_aware_ambiguity: true,
            ..Default::default()
        };

        let resolution =
            resolve_stable_speaker_cluster_from_candidates(&mut candidates, None, &tuning);

        assert_eq!(resolution.auto_merge_target_cluster_id, None);
        assert_eq!(resolution.suggested_merge_target_cluster_id, Some(1));
    }
}
