//! The **Semantic Index Backfill** worker (issue #123): a single self-healing
//! sweep-loop on the `mnema-deferred-startup` seam that derives a **Semantic
//! Search Vector** for every `direct` **Search Result Anchor** that has
//! searchable text but no vector yet.
//!
//! One query covers everything (ADR 0036): live capture and historical backfill
//! drain through the same `anchors_missing_vector` select, ordered newest-first
//! so fresh capture preempts the backlog. Progress lives entirely in the DB (the
//! presence/absence of a `vec0` row), so the sweep is resumable across restarts
//! with no in-memory cursor — a restart mid-backfill continues from DB state, and
//! a reprocessed anchor (delete + reinsert with a new id, old vector dropped by
//! the slice-1 `AFTER DELETE` trigger) reappears automatically.
//!
//! Like local transcription/OCR, the feature is **default-on but model-gated**:
//! with no **Semantic Search Model** installed the worker is a silent no-op
//! (logged once at INFO, never an error, never blocking capture, never
//! auto-downloading). It mirrors `spawn_user_context_worker` /
//! `spawn_retention_cleanup_worker`: one `tauri::async_runtime::spawn` loop,
//! tracked for graceful shutdown, that selects between an idle sleep and the
//! shutdown watch.
//!
//! CPU placement: the fastembed/ort embed is blocking model work, so it runs on a
//! blocking thread (never the tokio reactor, never the capture hot path). DB
//! reads/writes stay on the async loop.

use std::sync::Arc;
use std::time::Duration;

use capture_types::{default_semantic_search_settings, SemanticSearchSettings};
use semantic_search::{
    detect_model_status, model_install_dir, resolve_descriptor, semantic_search_models_dir, Pooling,
    SemanticSearchEmbedder, SemanticSearchModelDescriptor,
};
use tauri::Manager;

use crate::app_infra::{shutdown_aware_sleep, AppInfraState, BackgroundWorkersState};

/// How many anchors to drain per batch. A bounded batch keeps the worker
/// responsive to shutdown between batches and caps the blocking-thread hop cost,
/// while still amortizing the per-batch DB round-trips.
const SWEEP_BATCH_SIZE: i64 = 16;

/// Idle poll interval when there is nothing to embed (caught up, or the model is
/// not installed). Kept modest so the worker notices freshly captured anchors and
/// a just-installed model promptly, but it does no work on these ticks.
const IDLE_POLL_INTERVAL: Duration = Duration::from_secs(20);

/// Backoff after a batch error (a DB hiccup or an embed failure). Embedding never
/// blocks capture, so a failure just retries later rather than surfacing.
const ERROR_RETRY_INTERVAL: Duration = Duration::from_secs(30);

/// The outcome of one sweep pass, deciding the loop's next sleep.
enum SweepPass {
    /// At least one anchor was embedded + stored this pass; loop immediately to
    /// drain the rest (fresh capture preempts; backlog drains back-to-back).
    DidWork,
    /// No anchors needed a vector (caught up) OR the model is not installed
    /// (silent no-op): sleep the idle interval.
    Idle,
    /// A recoverable error this pass: sleep the error-retry interval.
    Error,
}

/// Spawn the **Semantic Index Backfill** worker. Mirrors
/// `spawn_user_context_worker`: tracks the handle for graceful shutdown and
/// selects between an idle sleep and the shutdown watch. The embedder is loaded
/// lazily on the first pass that has work AND an installed model, and is reused
/// across passes; it is dropped (and reloaded next time) if the selection becomes
/// unavailable, so a model switch from Settings is picked up without a restart.
pub fn spawn_semantic_index_backfill_worker(
    infra: AppInfraState,
    app_handle: tauri::AppHandle,
    background_workers: BackgroundWorkersState,
) {
    let mut shutdown_rx = background_workers.subscribe();
    crate::native_capture::debug_log::log_info(format!(
        "starting semantic index backfill worker (batch={}, idle_poll_ms={}, error_retry_ms={})",
        SWEEP_BATCH_SIZE,
        IDLE_POLL_INTERVAL.as_millis(),
        ERROR_RETRY_INTERVAL.as_millis(),
    ));

    let handle = tauri::async_runtime::spawn(async move {
        let infra = Arc::clone(&infra);
        // The loaded embedder, reused across passes. `None` until the first pass
        // that needs it with an installed model. A `LoadedEmbedder` remembers which
        // model it is, so a Settings model switch reloads it.
        let mut embedder: Option<LoadedEmbedder> = None;
        // Log the "no model installed" no-op only once per inert stretch, so a
        // default/Anthropic-only user does not see a per-tick log line.
        let mut logged_no_model = false;

        loop {
            if *shutdown_rx.borrow() {
                break;
            }

            let pass = run_sweep_pass(&infra, &app_handle, &mut embedder, &mut logged_no_model).await;
            let sleep = match pass {
                SweepPass::DidWork => {
                    // Drain the rest of the backlog back-to-back, but still poll
                    // the shutdown watch between batches (yield via a zero-ish
                    // sleep so a quit mid-backfill is honored promptly).
                    Duration::from_millis(0)
                }
                SweepPass::Idle => IDLE_POLL_INTERVAL,
                SweepPass::Error => ERROR_RETRY_INTERVAL,
            };

            if shutdown_aware_sleep(&mut shutdown_rx, sleep).await {
                break;
            }
        }

        crate::native_capture::debug_log::log_info("stopped semantic index backfill worker");
    });
    background_workers.track(handle);
}

/// A loaded **Semantic Search Model**, tagged with the provider/model id it was
/// loaded for so a Settings model switch triggers a reload. Shared with the query
/// path (`semantic_search_query.rs`), which loads the same model to embed the
/// query string for **Hybrid Search**.
pub(crate) struct LoadedEmbedder {
    pub(crate) provider: String,
    pub(crate) model_id: String,
    pub(crate) embedder: SemanticSearchEmbedder,
}

/// Run one sweep pass: gate on the installed model, drain up to one batch of
/// anchors newest-first, embed each on a blocking thread, and store the vectors.
/// Never panics; any error is logged and turned into [`SweepPass::Error`].
async fn run_sweep_pass(
    infra: &AppInfraState,
    app_handle: &tauri::AppHandle,
    embedder_slot: &mut Option<LoadedEmbedder>,
    logged_no_model: &mut bool,
) -> SweepPass {
    // The user's selected **Semantic Search Model Tier** (Settings slice #125):
    // the worker reloads the embedder when the provider/model id changes
    // (`embedder_matches`), so a model switch from Settings is picked up live with
    // no restart. A model-tier change to a non-768 dim is preceded by the Settings
    // re-index, which clears the vec0 table; the worker then re-derives every
    // anchor under the new model.
    let settings = effective_semantic_search_settings(app_handle);

    let app_data_dir = match app_handle.path().app_data_dir() {
        Ok(dir) => dir,
        Err(error) => {
            crate::native_capture::debug_log::log_error(format!(
                "semantic index backfill could not resolve app data dir: {error}"
            ));
            return SweepPass::Error;
        }
    };

    // Model-gating: silent no-op when no model is installed (matches the
    // transcription backfill skip). `Err` only on a corrupt marker — treat it as
    // unavailable, never a capture-blocking failure.
    let available = selected_model_available(&app_data_dir, &settings);
    if !available {
        // Drop any previously-loaded embedder so a model that was uninstalled (or a
        // selection turned off) stops holding the session, and the next install
        // reloads cleanly.
        *embedder_slot = None;
        if !*logged_no_model {
            crate::native_capture::debug_log::log_info(
                "semantic index backfill skipped: no Semantic Search Model installed (silent no-op)",
            );
            *logged_no_model = true;
        }
        return SweepPass::Idle;
    }
    *logged_no_model = false;

    // Peek the backlog before paying for an embedder load: if nothing needs a
    // vector we idle without touching the model.
    let batch = match infra.semantic_search().anchors_missing_vector(SWEEP_BATCH_SIZE).await {
        Ok(batch) => batch,
        Err(error) => {
            crate::native_capture::debug_log::log_error(format!(
                "semantic index backfill failed to read anchors missing a vector: {error}"
            ));
            return SweepPass::Error;
        }
    };
    if batch.is_empty() {
        return SweepPass::Idle;
    }

    // Resolve the catalog descriptor (dimension/window/pooling + install path) for
    // the selected model, then load the embedder if not already loaded for it.
    let Some(descriptor) = resolve_selected_descriptor(&settings) else {
        // Availability said yes but the descriptor vanished — defensive; treat as
        // unavailable for this pass.
        *embedder_slot = None;
        return SweepPass::Idle;
    };
    if !embedder_matches(embedder_slot, &descriptor) {
        *embedder_slot = match load_embedder(&app_data_dir, &descriptor) {
            Ok(loaded) => Some(loaded),
            Err(error) => {
                crate::native_capture::debug_log::log_error(format!(
                    "semantic index backfill failed to load model '{}/{}': {error}",
                    descriptor.provider, descriptor.model_id
                ));
                return SweepPass::Error;
            }
        };
    }

    // Embed the batch on a blocking thread (fastembed/ort is CPU model work), then
    // store each vector. The embedder is moved into the blocking task and back out
    // so it survives across passes.
    let loaded = embedder_slot.take().expect("embedder loaded above");
    let texts: Vec<(i64, String)> = batch
        .iter()
        .map(|anchor| (anchor.anchor_id, anchor.body_text.clone()))
        .collect();

    let (loaded, embedded) = match tauri::async_runtime::spawn_blocking(move || {
        let mut loaded = loaded;
        let mut out: Vec<(i64, std::result::Result<Vec<f32>, String>)> =
            Vec::with_capacity(texts.len());
        for (anchor_id, body_text) in texts {
            let result = loaded
                .embedder
                .embed_text(&body_text)
                .map_err(|error| error.to_string());
            out.push((anchor_id, result));
        }
        (loaded, out)
    })
    .await
    {
        Ok(pair) => pair,
        Err(error) => {
            crate::native_capture::debug_log::log_error(format!(
                "semantic index backfill embed task panicked/cancelled: {error}"
            ));
            // The embedder was moved into the failed task; it will be reloaded.
            return SweepPass::Error;
        }
    };
    // Restore the embedder for the next pass.
    *embedder_slot = Some(loaded);

    let mut stored = 0u64;
    let mut errors = 0u64;
    for (anchor_id, result) in embedded {
        match result {
            Ok(vector) => {
                // Re-check just before storing so a vector derived from text that
                // was deleted (retention / Delete Recent) or already replaced by a
                // reprocess mid-embed is never inserted as an orphan.
                match infra.semantic_search().anchor_still_missing_vector(anchor_id).await {
                    Ok(true) => match infra.semantic_search().store_vector(anchor_id, &vector).await {
                        Ok(()) => stored += 1,
                        Err(error) => {
                            errors += 1;
                            crate::native_capture::debug_log::log_error(format!(
                                "semantic index backfill failed to store vector for anchor {anchor_id}: {error}"
                            ));
                        }
                    },
                    Ok(false) => {
                        // The anchor was deleted or reprocessed mid-embed; skip it
                        // (the new anchor, if any, is picked up next pass).
                    }
                    Err(error) => {
                        errors += 1;
                        crate::native_capture::debug_log::log_error(format!(
                            "semantic index backfill failed to re-check anchor {anchor_id}: {error}"
                        ));
                    }
                }
            }
            Err(error) => {
                errors += 1;
                crate::native_capture::debug_log::log_error(format!(
                    "semantic index backfill failed to embed anchor {anchor_id}: {error}"
                ));
            }
        }
    }

    if stored > 0 {
        // Backlog count after this batch, so the log shows historical-backfill
        // progress draining toward zero. Best-effort: a count failure never
        // affects the sweep.
        let backlog = infra
            .semantic_search()
            .count_anchors_missing_vector()
            .await
            .unwrap_or(-1);
        crate::native_capture::debug_log::log_info(format!(
            "semantic index backfill embedded {stored} anchor(s) (batch={}, errors={errors}, backlog={backlog})",
            batch.len()
        ));
        return SweepPass::DidWork;
    }
    if errors > 0 {
        return SweepPass::Error;
    }
    // The batch was non-empty but every anchor was skipped by the re-check (all
    // deleted/reprocessed mid-embed): treat as work so we loop and pick up the
    // replacements, but it is effectively idle if nothing remains.
    SweepPass::DidWork
}

/// The effective **Semantic Search** selection for the worker and the query
/// embedder: the user's `RecordingSettings.semantic_search`. Falls back to the
/// default-on English-tier selection when the settings state is not yet managed
/// (early startup) so historical capture still gets vectored once a model is
/// installed.
pub(crate) fn effective_semantic_search_settings(
    app_handle: &tauri::AppHandle,
) -> SemanticSearchSettings {
    match app_handle.try_state::<crate::native_capture::RecordingSettingsState>() {
        Some(state) => {
            crate::native_capture::read_recording_settings(state.inner()).semantic_search
        }
        None => default_semantic_search_settings(),
    }
}

/// Resolve the catalog descriptor for the selected model, or `None` when the
/// selection is disabled / unset / unknown.
///
/// Goes through the shared resolver, so a **Custom**-picked fastembed model
/// (synthesized from fastembed's `ModelInfo`) resolves to a descriptor with the
/// correct dimension/window/layout — not just the 3 guided manifest tiers. This
/// is what lets the worker load + embed under a Custom model and the query path
/// embed the search text with the same model.
pub(crate) fn resolve_selected_descriptor(
    settings: &SemanticSearchSettings,
) -> Option<SemanticSearchModelDescriptor> {
    if !settings.enabled {
        return None;
    }
    let model_id = settings.model_id.as_deref()?;
    resolve_descriptor(&settings.provider, model_id)
}

/// Whether the selected **Semantic Search Model** (manifest tier OR Custom pick)
/// is installed on disk — the worker/query model-gate.
///
/// Mirrors the crate's `selected_semantic_search_model_available` but routes
/// through [`resolve_selected_descriptor`] so a Custom model outside the manifest
/// is recognized once downloaded. Returns `false` (a silent no-op, never an error)
/// when disabled / unselected / unresolvable / not yet installed; a corrupt marker
/// is treated as unavailable too.
pub(crate) fn selected_model_available(
    app_data_dir: &std::path::Path,
    settings: &SemanticSearchSettings,
) -> bool {
    let Some(descriptor) = resolve_selected_descriptor(settings) else {
        return false;
    };
    detect_model_status(semantic_search_models_dir(app_data_dir), &descriptor)
        .map(|status| status.is_available())
        .unwrap_or(false)
}

/// Whether the currently-loaded embedder is for `descriptor`'s exact
/// provider/model id (so a Settings model switch reloads it).
fn embedder_matches(
    slot: &Option<LoadedEmbedder>,
    descriptor: &SemanticSearchModelDescriptor,
) -> bool {
    slot.as_ref().is_some_and(|loaded| {
        loaded.provider == descriptor.provider && loaded.model_id == descriptor.model_id
    })
}

/// Load the embedder for `descriptor` from its install directory under
/// `semantic_search_models/{provider}/{model_id}/`.
pub(crate) fn load_embedder(
    app_data_dir: &std::path::Path,
    descriptor: &SemanticSearchModelDescriptor,
) -> Result<LoadedEmbedder, String> {
    let models_dir = semantic_search_models_dir(app_data_dir);
    let install_dir = model_install_dir(&models_dir, &descriptor.provider, &descriptor.model_id)
        .map_err(|error| error.to_string())?;
    let embedder = SemanticSearchEmbedder::load_from_dir(
        &install_dir,
        descriptor.max_tokens,
        pooling_for_model(&descriptor.model_id),
        &descriptor.expected_layout,
        // The guided tiers (nomic / e5 / bge) all use fastembed's default
        // mean/CLS-pooled output; no model in the catalog names a specific output
        // tensor, so `None` is correct. (A future Custom model that needs a named
        // output would carry it through here from its `ModelInfo.output_key`.)
        None,
    )
    .map_err(|error| error.to_string())?;
    Ok(LoadedEmbedder {
        provider: descriptor.provider.clone(),
        model_id: descriptor.model_id.clone(),
        embedder,
    })
}

/// The fastembed pooling strategy for a model: Mean for nomic/e5 sentence
/// models, Cls for BGE (per the `semantic-search` runtime doc). Defaults to Mean,
/// the common case for the guided tiers.
fn pooling_for_model(model_id: &str) -> Pooling {
    if model_id.starts_with("bge") {
        Pooling::Cls
    } else {
        Pooling::Mean
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use semantic_search::builtin_model_manifest;

    #[test]
    fn pooling_is_mean_for_nomic_and_e5_cls_for_bge() {
        assert!(matches!(pooling_for_model("nomic-embed-text-v1.5"), Pooling::Mean));
        assert!(matches!(pooling_for_model("multilingual-e5-small"), Pooling::Mean));
        assert!(matches!(pooling_for_model("bge-m3"), Pooling::Cls));
    }

    #[test]
    fn resolve_descriptor_honors_enabled_and_known_model() {
        let mut settings = default_semantic_search_settings();
        // Default English tier resolves to the nomic 768-dim descriptor.
        let descriptor = resolve_selected_descriptor(&settings).expect("default descriptor");
        assert_eq!(descriptor.model_id, "nomic-embed-text-v1.5");
        assert_eq!(descriptor.dimension, 768);

        // Disabled => no descriptor (the worker idles).
        settings.enabled = false;
        assert!(resolve_selected_descriptor(&settings).is_none());

        // Unknown model => no descriptor.
        settings.enabled = true;
        settings.model_id = Some("not-a-real-model".to_string());
        assert!(resolve_selected_descriptor(&settings).is_none());

        // No model selected => no descriptor.
        settings.model_id = None;
        assert!(resolve_selected_descriptor(&settings).is_none());
    }

    #[test]
    fn embedder_matches_only_the_same_provider_and_model() {
        let manifest = builtin_model_manifest();
        let nomic = manifest
            .models
            .iter()
            .find(|m| m.model_id == "nomic-embed-text-v1.5")
            .expect("nomic descriptor");

        // No loaded embedder => never matches.
        assert!(!embedder_matches(&None, nomic));
    }
}
