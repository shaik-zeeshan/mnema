//! Candidate **Subject handle** selection for Conclusion distillation (slices 5/6).
//!
//! Feeds the distillation prompt a small "KNOWN SUBJECTS — reuse these handles"
//! set so the engine reuses an existing handle (which then reinforces the
//! canonical row via the subject-only upsert) instead of coining a reworded
//! duplicate. Two sourcing modes, with the LLM as the matcher in BOTH — only the
//! candidate source differs:
//!
//! - **Mode 1 (semantic):** when a **Semantic Search Model** is installed, embed
//!   the window's Activities (as queries) and KNN them against the stored Subject
//!   Vectors, then union/dedup/floor/cap into a small relevant candidate set.
//! - **Mode 2 (fallback):** no model — the caller falls back to the full distinct
//!   handle set, recency-ordered and capped (see
//!   [`super::derivation`]'s assembly). This module only produces Mode 1; an empty
//!   return is the signal to fall back.
//!
//! Graceful degradation is load-bearing: prod ships zero embedding model, so the
//! no-model path must no-op cleanly to Mode 2.

use std::collections::HashMap;

use app_infra::SubjectVectorStore;
use semantic_search::EmbedKind;
use tauri::Manager;

use crate::semantic_search_worker::{
    effective_semantic_search_settings, load_embedder, resolve_selected_descriptor,
    selected_model_available,
};

/// How many nearest Subject Vectors to pull per Activity in Mode 1. Tunable —
/// kept small so the per-Activity candidate lists stay relevant before the union.
const K_PER_ACTIVITY: usize = 5;

/// Loose cosine floor below which a KNN hit is dropped from the candidate set
/// (Mode 1). A starting point to calibrate in slice 7 — deliberately permissive so
/// the LLM (the real matcher) still sees plausibly-related handles, while obviously
/// unrelated subjects are pruned before they reach the prompt.
const SUBJECT_CANDIDATE_COSINE_FLOOR: f32 = 0.3;

/// Cap on the number of distinct candidate handles surfaced from Mode 1, after the
/// union/dedup/floor. Tunable. Bounds the prompt growth from a wide window.
const SUBJECT_CANDIDATE_CAP: usize = 40;

/// Mode 1 candidate selection: embed each Activity (title + summary) as a Query,
/// KNN it against the stored Subject Vectors, and union/dedup/floor/cap the hits
/// into a small relevant set of handles for the KNOWN SUBJECTS prompt block.
///
/// Returns `vec![]` (the caller falls back to Mode 2) when **Semantic Search** is
/// inert: no model installed, the selection is disabled/unknown, no Activities, or
/// a load/embed failure. A `vec![]` never surfaces an error — distillation simply
/// uses the recency-ordered fallback, the same "no usable runtime → degrade
/// gracefully" shape as the rest of the semantic path.
///
/// The model load + embed run on a blocking thread because the candle forward is
/// synchronous model work (Metal GPU on macOS / candle-CPU elsewhere) that must
/// stay off the tokio reactor (ADR 0037). Distillation is an infrequent slow beat,
/// so the embedder is loaded once per pass and dropped at the end (no caching) —
/// the simpler robust option.
pub(crate) async fn select_semantic_subject_candidates(
    app_handle: &tauri::AppHandle,
    subject_vectors: &SubjectVectorStore,
    activities: &[capture_types::Activity],
) -> Vec<String> {
    if activities.is_empty() {
        return Vec::new();
    }

    let settings = effective_semantic_search_settings(app_handle);
    let app_data_dir = match app_handle.path().app_data_dir() {
        Ok(dir) => dir,
        Err(error) => {
            crate::native_capture::debug_log::log_error(format!(
                "subject-candidate select could not resolve app data dir: {error}"
            ));
            return Vec::new();
        }
    };

    // Model-gate → Mode 1 vs Mode 2: a silent no-op (empty set) when no model is
    // installed / the selection is disabled / unknown, so the caller falls back to
    // the recency-ordered handle set. Never an error, never an auto-download.
    if !selected_model_available(&app_data_dir, &settings) {
        return Vec::new();
    }
    let Some(descriptor) = resolve_selected_descriptor(&settings) else {
        return Vec::new();
    };
    // The active model's identity string (`provider/model_id`): the KNN only ranks
    // Subject Vectors embedded under THIS model, so a stale-model vector never
    // produces a garbage cosine. Computed before `descriptor` moves into the
    // blocking embed task below.
    let active_model = format!("{}/{}", descriptor.provider, descriptor.model_id);

    // Build one query text per Activity, mirroring what `build_distillation_prompt`
    // shows the engine (title + summary). Empty texts are dropped.
    let texts: Vec<String> = activities
        .iter()
        .filter_map(|activity| {
            let title = activity.title.trim();
            let summary = activity.summary.trim();
            let text = if summary.is_empty() {
                title.to_string()
            } else if title.is_empty() {
                summary.to_string()
            } else {
                format!("{title}\n{summary}")
            };
            (!text.is_empty()).then_some(text)
        })
        .collect();
    if texts.is_empty() {
        return Vec::new();
    }

    // Embed all Activity texts as QUERIES (they are queries against the stored
    // Document-kind Subject corpus) on a blocking thread. Load the embedder once
    // per pass and drop it when the task returns.
    let embed_result = tauri::async_runtime::spawn_blocking(move || {
        let loaded = match load_embedder(&app_data_dir, &descriptor) {
            Ok(loaded) => loaded,
            Err(error) => {
                crate::native_capture::debug_log::log_error(format!(
                    "subject-candidate select failed to load model '{}/{}': {error}",
                    descriptor.provider, descriptor.model_id
                ));
                return None;
            }
        };
        let bodies: Vec<&str> = texts.iter().map(|text| text.as_str()).collect();
        Some(loaded.embedder.embed_texts(&bodies, EmbedKind::Query))
    })
    .await;

    let results = match embed_result {
        Ok(Some(results)) => results,
        Ok(None) => return Vec::new(),
        Err(error) => {
            crate::native_capture::debug_log::log_error(format!(
                "subject-candidate select embed task panicked/cancelled: {error}"
            ));
            return Vec::new();
        }
    };

    // KNN each finite query vector against the stored Subject Vectors, collecting
    // one (subject, cosine) list per Activity for the union/dedup/floor/cap below.
    let mut per_activity: Vec<Vec<(String, f32)>> = Vec::new();
    for result in results {
        let vector = match result {
            Ok(vector) => vector,
            Err(error) => {
                crate::native_capture::debug_log::log_error(format!(
                    "subject-candidate select embed failed for an activity: {error}"
                ));
                continue;
            }
        };
        // Mirror the query-path guard: a NaN/inf component would yield a
        // non-deterministic KNN ordering, so skip this activity's vector.
        if vector.iter().any(|component| !component.is_finite()) {
            continue;
        }
        match subject_vectors
            .subject_vector_knn(&vector, &active_model, K_PER_ACTIVITY)
            .await
        {
            Ok(hits) => per_activity.push(hits),
            Err(error) => {
                crate::native_capture::debug_log::log_error(format!(
                    "subject-candidate select KNN failed: {error}"
                ));
            }
        }
    }

    merge_candidate_handles(per_activity)
}

/// How many of the newest Subject handles are ALWAYS surfaced in the KNOWN
/// SUBJECTS block, ahead of (and in addition to) the semantic candidates.
///
/// Load-bearing for dedup: a Subject created by a recent distillation has NOT
/// been embedded into the Subject Vectors yet — the backfill worker embeds it
/// only *after* it is created (empirically ~tens of seconds later, and never
/// before the very next distillation that overlaps the same activity). So the
/// semantic KNN (Mode 1) structurally cannot surface the freshest Subjects, which
/// are exactly the ones the next distillation is most likely to re-derive and
/// split into a reworded duplicate (recency bias: the user is still doing the same
/// thing). Leading the candidate list with this recency floor guarantees those are
/// offered to the LLM for verbatim reuse regardless of embedding state.
pub(crate) const KNOWN_SUBJECTS_RECENCY_FLOOR: usize = 30;

/// Assemble the final KNOWN SUBJECTS candidate list by UNIONING the recency-ordered
/// handle set (newest first; always available) with the `related` candidates — the
/// lexical-overlap leg followed by the semantic KNN leg (the latter empty when no
/// embedding model is installed). The order is deliberate:
///
/// 1. the newest [`KNOWN_SUBJECTS_RECENCY_FLOOR`] handles — the dup-prone fresh
///    Subjects the embedding backfill cannot have reached yet;
/// 2. the `related` handles — Subjects relevant to the current window that may be
///    OLDER than the recency floor (a reworded lexical duplicate, or a long-tail
///    subject the user returned to that semantic search surfaced);
/// 3. the remaining (older) recency handles, filling whatever prompt budget is left.
///
/// Deduped case-insensitively, first occurrence wins (a handle already in the
/// recency floor is not repeated by a later related hit). The char cap in
/// `build_known_subjects_block` bounds the final size; leading with the recency
/// floor + related candidates guarantees the freshest and most-relevant Subjects
/// survive truncation. With no model and no lexical hits, `related` is empty and
/// this collapses to exactly the recency list (the prior model-free behavior).
///
/// Replaces the prior either/or (`semantic OR recency`), whose gap was the live
/// duplication bug: a non-empty semantic set suppressed the recency fallback, so a
/// just-created-not-yet-embedded Subject was invisible to the LLM and got reworded
/// into a near-duplicate.
pub(crate) fn merge_known_subjects(recency: Vec<String>, related: Vec<String>) -> Vec<String> {
    let floor = recency.len().min(KNOWN_SUBJECTS_RECENCY_FLOOR);
    let mut ordered: Vec<String> = Vec::with_capacity(recency.len() + related.len());
    ordered.extend(recency[..floor].iter().cloned());
    ordered.extend(related);
    ordered.extend(recency[floor..].iter().cloned());

    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    ordered
        .into_iter()
        .filter(|handle| {
            let key = handle.trim().to_ascii_lowercase();
            !key.is_empty() && seen.insert(key)
        })
        .collect()
}

/// Pure union/dedup/floor/cap over the per-Activity KNN results — factored out so
/// it is unit-testable without an embedder or DB.
///
/// Unions handles across the window, deduping CASE-INSENSITIVELY and keeping the
/// MAX cosine similarity seen for each handle; drops any hit below
/// [`SUBJECT_CANDIDATE_COSINE_FLOOR`]; orders by best similarity descending (ties
/// broken by handle for a stable order); and caps the result at
/// [`SUBJECT_CANDIDATE_CAP`] distinct handles. The first-seen original casing of a
/// handle is preserved.
fn merge_candidate_handles(per_activity: Vec<Vec<(String, f32)>>) -> Vec<String> {
    // key = lowercased handle → (original-cased handle, best similarity seen).
    let mut best: HashMap<String, (String, f32)> = HashMap::new();
    for hits in per_activity {
        for (handle, similarity) in hits {
            let handle = handle.trim();
            if handle.is_empty() || similarity < SUBJECT_CANDIDATE_COSINE_FLOOR {
                continue;
            }
            let key = handle.to_ascii_lowercase();
            best.entry(key)
                .and_modify(|(_, existing)| {
                    if similarity > *existing {
                        *existing = similarity;
                    }
                })
                .or_insert_with(|| (handle.to_string(), similarity));
        }
    }

    let mut ranked: Vec<(String, f32)> = best.into_values().collect();
    ranked.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    ranked.truncate(SUBJECT_CANDIDATE_CAP);
    ranked.into_iter().map(|(handle, _)| handle).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_dedups_case_insensitively_keeping_max_similarity() {
        // "Apple" appears twice across activities with different casing + sims; it
        // must collapse to ONE handle carrying the MAX similarity (0.9), and order
        // by descending similarity.
        let merged = merge_candidate_handles(vec![
            vec![("Apple".to_string(), 0.6), ("Rust".to_string(), 0.95)],
            vec![("apple".to_string(), 0.9)],
        ]);
        assert_eq!(merged, vec!["Rust".to_string(), "Apple".to_string()]);
    }

    #[test]
    fn merge_drops_hits_below_the_cosine_floor() {
        // 0.2 is below SUBJECT_CANDIDATE_COSINE_FLOOR (0.3) → dropped; 0.3 is at the
        // floor (not below) → kept.
        let merged = merge_candidate_handles(vec![vec![
            ("Below".to_string(), 0.2),
            ("AtFloor".to_string(), 0.3),
        ]]);
        assert_eq!(merged, vec!["AtFloor".to_string()]);
    }

    #[test]
    fn merge_caps_the_candidate_count() {
        // More distinct handles than the cap → only SUBJECT_CANDIDATE_CAP survive,
        // and they are the highest-similarity ones (descending order).
        let hits: Vec<(String, f32)> = (0..(SUBJECT_CANDIDATE_CAP + 10))
            .map(|i| (format!("subject-{i:03}"), 0.4 + (i as f32) * 0.001))
            .collect();
        let merged = merge_candidate_handles(vec![hits]);
        assert_eq!(merged.len(), SUBJECT_CANDIDATE_CAP);
        // The single highest-similarity handle is the last index, ranked first.
        let top = format!("subject-{:03}", SUBJECT_CANDIDATE_CAP + 9);
        assert_eq!(merged.first(), Some(&top));
    }

    #[test]
    fn merge_empty_input_is_empty() {
        assert!(merge_candidate_handles(vec![]).is_empty());
        assert!(merge_candidate_handles(vec![vec![]]).is_empty());
    }

    #[test]
    fn known_subjects_no_model_is_just_recency() {
        // No embedding model → semantic is empty → the union is exactly the
        // recency list, order preserved (the prior model-free behavior).
        let recency = vec!["A".to_string(), "B".to_string(), "C".to_string()];
        assert_eq!(merge_known_subjects(recency.clone(), vec![]), recency);
    }

    #[test]
    fn known_subjects_leads_with_recency_floor_then_semantic() {
        // The fresh handle "Marvel Rivals / gaming" leads (it is in the recency
        // floor) even though it has no vector and is absent from the semantic set;
        // semantic adds an OLDER relevant handle ("Apple") after the floor.
        let recency = vec![
            "Marvel Rivals / gaming".to_string(),
            "async communication".to_string(),
        ];
        let semantic = vec!["Apple".to_string()];
        assert_eq!(
            merge_known_subjects(recency, semantic),
            vec![
                "Marvel Rivals / gaming".to_string(),
                "async communication".to_string(),
                "Apple".to_string(),
            ]
        );
    }

    #[test]
    fn known_subjects_dedups_case_insensitively_first_wins() {
        // A semantic hit that duplicates a recency-floor handle (different casing)
        // collapses to the recency occurrence; original casing preserved.
        let recency = vec!["Apple".to_string(), "Rust".to_string()];
        let semantic = vec!["apple".to_string(), "Vim".to_string()];
        assert_eq!(
            merge_known_subjects(recency, semantic),
            vec!["Apple".to_string(), "Rust".to_string(), "Vim".to_string()]
        );
    }

    #[test]
    fn known_subjects_floor_caps_the_recency_lead_then_appends_rest_after_semantic() {
        // With more recency handles than the floor, the lead is the newest FLOOR
        // handles, then the semantic candidates, then the older recency tail. This
        // is what keeps a relevant OLD semantic hit ahead of the older recency tail
        // under the prompt char cap.
        let recency: Vec<String> = (0..(KNOWN_SUBJECTS_RECENCY_FLOOR + 3))
            .map(|i| format!("recent-{i:03}"))
            .collect();
        let semantic = vec!["semantic-old".to_string()];
        let merged = merge_known_subjects(recency.clone(), semantic);
        // First FLOOR entries are the newest recency handles, in order.
        assert_eq!(&merged[..KNOWN_SUBJECTS_RECENCY_FLOOR], &recency[..KNOWN_SUBJECTS_RECENCY_FLOOR]);
        // The semantic hit lands immediately after the floor, ahead of the tail.
        assert_eq!(merged[KNOWN_SUBJECTS_RECENCY_FLOOR], "semantic-old");
        assert_eq!(&merged[KNOWN_SUBJECTS_RECENCY_FLOOR + 1..], &recency[KNOWN_SUBJECTS_RECENCY_FLOOR..]);
    }

    #[test]
    fn known_subjects_drops_blank_handles() {
        let recency = vec!["  ".to_string(), "Real".to_string()];
        let semantic = vec!["".to_string()];
        assert_eq!(merge_known_subjects(recency, semantic), vec!["Real".to_string()]);
    }
}
