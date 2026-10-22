//! Write-time near-duplicate guard for distilled **Conclusion** values.
//!
//! The distiller is shown each candidate subject's conclusions and asked to cite
//! `reinforces_id` (ADR 0043), but on real data it keeps re-emitting the same
//! belief reworded as new. Before a NEW belief is inserted, this guard compares it
//! with the existing conclusions of the SAME subject only (case-insensitive, like
//! every store query) and, on a near-duplicate, hands back that conclusion's id so
//! the caller reinforces it through the ordinary `reinforces_id` path instead.
//!
//! Two rules, either one suffices, both behind
//! [`app_infra::user_context::conclusion_dedup::statements_compatible`]
//! (negation / numbers / time words must agree):
//! - the lexical rule app-infra owns (also the startup heal's rule), always on;
//! - embedding cosine ≥ [`duplicate_cosine_threshold`] when the selected
//!   Semantic Search model is installed AND has a calibrated threshold. The model
//!   loads lazily (only once some belief gets past the lexical rule) and holds the
//!   background embed slot for the rest of the pass.

use std::collections::HashMap;
use std::sync::Arc;

use app_infra::user_context::conclusion_dedup::{find_lexical_duplicate, statements_compatible};
use app_infra::UserContextStore;
use semantic_search::{EmbedKind, SemanticSearchModelDescriptor};
use tauri::Manager;

use crate::semantic_search_worker::{
    effective_semantic_search_settings, load_embedder, resolve_selected_descriptor,
    selected_model_available, LoadedEmbedder, BACKGROUND_EMBED_SLOT,
};

/// Cosine at/above which two same-subject statements are one belief, per model.
/// Only models measured on paraphrase vs distinct-belief fixtures get one; any
/// other model falls back to the lexical rule alone.
///
/// nomic-embed-text-v1.5, `search_document:` prefix, measured 2026-10-14 (see the
/// fixtures in `app_infra::user_context::conclusion_dedup` tests): true paraphrases
/// 0.89–0.99 (most ≥ 0.95); distinct beliefs of one subject ≤ 0.89 except the
/// one-word swaps — "dark mode" vs "light mode" 0.947, weekends/weekdays 0.968,
/// "3×" vs "5×/week" 0.966 — the last two of which the qualifier guard blocks.
/// 0.955 sits above the antonym case.
/// ponytail: one calibrated model; measure the others the same way before adding them.
pub(crate) fn duplicate_cosine_threshold(model_id: &str) -> Option<f32> {
    (model_id == "nomic-embed-text-v1.5").then_some(0.955)
}

/// How many of a subject's strongest existing conclusions the embedding leg
/// compares against. The lexical leg always scans them all.
/// ponytail: a subject past this cap only gets lexical coverage on its tail.
const EMBED_COMPARE_CAP: usize = 200;

enum Embedding {
    /// Not loaded yet; `None` descriptor = no usable model this pass.
    Pending(Option<(SemanticSearchModelDescriptor, std::path::PathBuf, f32)>),
    Ready {
        embedder: Arc<LoadedEmbedder>,
        threshold: f32,
        _slot: Option<tokio::sync::SemaphorePermit<'static>>,
    },
    Off,
}

pub(crate) struct DuplicateGuard {
    embedding: Embedding,
    /// Statement vectors by conclusion id. A conclusion's statement is frozen once
    /// written, so the id is a stable key for the pass.
    vectors: HashMap<i64, Vec<f32>>,
}

impl DuplicateGuard {
    /// Resolve (but do not load) the embedding model for this pass.
    pub(crate) fn new(app_handle: &tauri::AppHandle) -> Self {
        let settings = effective_semantic_search_settings(app_handle);
        let model = app_handle.path().app_data_dir().ok().and_then(|dir| {
            if !selected_model_available(&dir, &settings) {
                return None;
            }
            let descriptor = resolve_selected_descriptor(&settings)?;
            let threshold = duplicate_cosine_threshold(&descriptor.model_id)?;
            Some((descriptor, dir, threshold))
        });
        Self {
            embedding: Embedding::Pending(model),
            vectors: HashMap::new(),
        }
    }

    /// The id of an existing conclusion of `subject` that `statement` restates,
    /// if any. `exclude` (the belief's own `supersedes_id`) is never a match: a
    /// correction must not reinforce the row it retires.
    pub(crate) async fn find_duplicate(
        &mut self,
        store: &UserContextStore,
        subject: &str,
        statement: &str,
        exclude: Option<i64>,
    ) -> Result<Option<i64>, String> {
        let existing: Vec<(i64, String)> = store
            .list_live_statements_for_subject(subject)
            .await
            .map_err(|error| error.to_string())?
            .into_iter()
            .filter(|(id, _)| Some(*id) != exclude)
            .collect();
        if existing.is_empty() {
            return Ok(None);
        }
        if let Some(id) =
            find_lexical_duplicate(statement, existing.iter().map(|(id, s)| (*id, s.as_str())))
        {
            return Ok(Some(id));
        }

        let candidates: Vec<&(i64, String)> = existing
            .iter()
            .take(EMBED_COMPARE_CAP)
            .filter(|(_, other)| statements_compatible(statement, other))
            .collect();
        if candidates.is_empty() {
            return Ok(None);
        }
        let Some((embedder, threshold)) = self.ready().await else {
            return Ok(None);
        };

        // Embed the new statement plus every candidate not cached yet, in one batch.
        let missing: Vec<(i64, String)> = candidates
            .iter()
            .filter(|(id, _)| !self.vectors.contains_key(id))
            .map(|(id, s)| (*id, s.clone()))
            .collect();
        let mut texts: Vec<String> = vec![statement.to_string()];
        texts.extend(missing.iter().map(|(_, s)| s.clone()));
        let embedded = tauri::async_runtime::spawn_blocking(move || {
            let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
            embedder.embedder.embed_texts(&refs, EmbedKind::Document)
        })
        .await;
        let mut results = match embedded {
            Ok(results) => results.into_iter(),
            Err(error) => return Ok(self.disable(format!("embed task failed: {error}"))),
        };
        let probe = match results.next() {
            Some(Ok(vector)) => vector,
            Some(Err(error)) => return Ok(self.disable(format!("embed failed: {error}"))),
            None => return Ok(None),
        };
        for ((id, _), result) in missing.iter().zip(results) {
            if let Ok(vector) = result {
                self.vectors.insert(*id, vector);
            }
        }

        // Vectors are L2-normalized by the embedder, so the dot product is cosine.
        let best = candidates
            .iter()
            .filter_map(|(id, _)| {
                let vector = self.vectors.get(id)?;
                let cosine: f32 = probe.iter().zip(vector).map(|(a, b)| a * b).sum();
                (cosine >= threshold).then_some((*id, cosine))
            })
            .max_by(|a, b| a.1.total_cmp(&b.1));
        Ok(best.map(|(id, _)| id))
    }

    /// Load the embedder on first use; `None` when no calibrated model is usable.
    async fn ready(&mut self) -> Option<(Arc<LoadedEmbedder>, f32)> {
        if let Embedding::Pending(model) = &mut self.embedding {
            let Some((descriptor, dir, threshold)) = model.take() else {
                self.embedding = Embedding::Off;
                return None;
            };
            // Same slot as the other background embed paths (see BACKGROUND_EMBED_SLOT).
            let slot = BACKGROUND_EMBED_SLOT.acquire().await.ok();
            let loaded =
                tauri::async_runtime::spawn_blocking(move || load_embedder(&dir, &descriptor))
                    .await;
            self.embedding = match loaded {
                Ok(Ok(embedder)) => Embedding::Ready {
                    embedder,
                    threshold,
                    _slot: slot,
                },
                Ok(Err(error)) => {
                    self.disable(format!("model load failed: {error}"));
                    return None;
                }
                Err(error) => {
                    self.disable(format!("model load task failed: {error}"));
                    return None;
                }
            };
        }
        match &self.embedding {
            Embedding::Ready {
                embedder,
                threshold,
                ..
            } => Some((Arc::clone(embedder), *threshold)),
            _ => None,
        }
    }

    /// Fall back to lexical-only for the rest of the pass.
    fn disable(&mut self, reason: String) -> Option<i64> {
        crate::native_capture::debug_log::log_warn(format!(
            "conclusion duplicate guard: {reason}; using the lexical rule only this pass"
        ));
        self.embedding = Embedding::Off;
        None
    }
}

/// Deferred-startup heal: collapse paraphrase copies already stored (see
/// `UserContextStore::dedupe_near_duplicate_conclusions`). Idempotent and cheap
/// when clean, so it runs every launch; a failure is retried on the next one.
pub(crate) fn run_dedup_startup_pass(infra: &::app_infra::AppInfra) {
    match tauri::async_runtime::block_on(infra.user_context().dedupe_near_duplicate_conclusions()) {
        Ok(summary) if summary.conclusions_removed > 0 => {
            crate::native_capture::debug_log::log_info(format!(
                "user context dedup removed {} near-duplicate conclusion(s) across {} subject(s)",
                summary.conclusions_removed, summary.subjects_touched
            ));
        }
        Ok(_) => {}
        Err(error) => crate::native_capture::debug_log::log_warn(format!(
            "user context dedup startup pass failed; retrying next launch: {error}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use app_infra::NewConclusion;

    async fn seed(store: &UserContextStore, subject: &str, statement: &str) -> i64 {
        store
            .upsert_conclusion(NewConclusion {
                subject: subject.to_string(),
                statement: statement.to_string(),
                confidence: 0.9,
                formed_at_ms: 1,
                last_supported_at_ms: 1,
            })
            .await
            .expect("seed")
    }

    /// No model installed (the prod default): the lexical rule alone turns a
    /// reworded belief into a reinforcement, and leaves a distinct one, another
    /// subject's row, and the belief's own supersede target alone.
    #[tokio::test]
    async fn guard_merges_a_paraphrase_and_keeps_a_distinct_belief() {
        let dir = std::env::temp_dir().join(format!(
            "conclusion-guard-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("dir");
        let infra = app_infra::AppInfra::initialize(&dir).await.expect("infra");
        let store = infra.user_context();
        let unlock = seed(
            store,
            "Mac unlock routine",
            "Unlocks the Mac at the login window several times a day",
        )
        .await;
        seed(
            store,
            "Login window",
            "Uses the login window for fast user switching",
        )
        .await;
        let mut guard = DuplicateGuard {
            embedding: Embedding::Off,
            vectors: HashMap::new(),
        };

        let paraphrase = "Frequently unlocks their Mac at the login window throughout the day";
        let found = guard
            .find_duplicate(store, "mac unlock ROUTINE", paraphrase, None)
            .await
            .expect("guard");
        assert_eq!(
            found,
            Some(unlock),
            "paraphrase reinforces the existing row"
        );

        let distinct = "Uses Touch ID rather than a password to unlock the Mac";
        let found = guard
            .find_duplicate(store, "Mac unlock routine", distinct, None)
            .await
            .expect("guard");
        assert_eq!(found, None, "a different belief about the subject inserts");

        let found = guard
            .find_duplicate(store, "Mac unlock routine", paraphrase, Some(unlock))
            .await
            .expect("guard");
        assert_eq!(
            found, None,
            "never reinforce the row this belief supersedes"
        );

        let found = guard
            .find_duplicate(store, "Login window", paraphrase, None)
            .await
            .expect("guard");
        assert_eq!(found, None, "never matched across subjects");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_calibrated_models_get_an_embedding_threshold() {
        assert_eq!(
            duplicate_cosine_threshold("nomic-embed-text-v1.5"),
            Some(0.955)
        );
        assert_eq!(duplicate_cosine_threshold("bge-m3"), None);
    }
}
