//! The **Hybrid Search** query-embedding seam (issue #124): embeds the search
//! query string into a **Semantic Search Vector** so the app-infra search path
//! can fuse a `vec0` KNN with the FTS5 **Text Search** ranking by RRF.
//!
//! app-infra takes no embedding-runtime dependency (the same boundary that keeps
//! `ai-runtime` out of it for User Context), so the *query* is embedded here, in
//! the desktop layer, and passed into `search_capture` as a pre-computed
//! `query_embedding`. When no **Semantic Search Model** is installed — the
//! default/Anthropic-only case — this returns `None` and search stays
//! keyword-only with no regression, exactly the **default-on but model-gated**
//! shape of local transcription/OCR. Mnema never auto-downloads a model.
//!
//! The loaded model is cached in [`SemanticQueryEmbedderState`] so repeated
//! searches (every keystroke) reuse one embedder rather than reloading the model;
//! a Settings model switch reloads it because the cache remembers which
//! provider/model it holds. The embed runs on a blocking thread because the candle
//! forward is synchronous model work (Metal GPU on macOS / candle-CPU elsewhere)
//! that must stay off the tokio reactor (ADR 0037).

use std::sync::Mutex;

use tauri::Manager;

use crate::semantic_search_worker::{
    effective_semantic_search_settings, load_embedder, resolve_selected_descriptor,
    selected_model_available, LoadedEmbedder,
};

/// The cached query embedder, shared as Tauri managed state across `search_capture`
/// calls. `None` until the first search runs with an installed model. A model
/// switch from Settings reloads it (the cache remembers its provider/model id).
#[derive(Default)]
pub struct SemanticQueryEmbedderState {
    cached: Mutex<Option<LoadedEmbedder>>,
}

impl SemanticQueryEmbedderState {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Embed the search `query` into a **Semantic Search Vector** for **Hybrid
/// Search**, or `None` when **Semantic Search** is inert: no model installed,
/// the selection is disabled/unknown, an empty query, or a model load/embed
/// failure. A `None` never surfaces an error to the caller — search simply runs
/// keyword-only, the same "no usable runtime → feature unavailable" shape as
/// local transcription.
///
/// `query` is the operator-stripped residual body (see
/// `::app_infra::semantic_search_residual_query`), not the raw query — so
/// `app:`/`before:`/`source:` operators and quoted phrases never pollute the
/// meaning vector, and it matches what FTS ranks on. An all-operators query has an
/// empty residual, which the trim/empty-guard below resolves to `None`
/// (keyword-only).
///
/// The model load and the embed both run on a blocking thread because the candle
/// forward is synchronous model work (Metal GPU on macOS / candle-CPU elsewhere)
/// that must stay off the tokio reactor (and off the capture hot path — this is
/// user-initiated search, not capture).
pub async fn embed_search_query(
    app_handle: &tauri::AppHandle,
    state: &tauri::State<'_, SemanticQueryEmbedderState>,
    query: &str,
) -> Option<Vec<f32>> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return None;
    }

    let settings = effective_semantic_search_settings(app_handle);
    let app_data_dir = match app_handle.path().app_data_dir() {
        Ok(dir) => dir,
        Err(error) => {
            crate::native_capture::debug_log::log_error(format!(
                "semantic query embed could not resolve app data dir: {error}"
            ));
            return None;
        }
    };

    // Model-gating: silent no-op when no model is installed (no error, no
    // auto-download). Routes through the resolver so a Custom-picked model is
    // recognized once installed; a corrupt marker is treated as unavailable.
    if !selected_model_available(&app_data_dir, &settings) {
        return None;
    }

    let descriptor = resolve_selected_descriptor(&settings)?;

    // Take the cached embedder out of the mutex for the blocking task (so we never
    // hold a std Mutex across an await), reloading if the selection changed, then
    // put it back. The state is per-process and search is serialized enough that
    // the brief None window between take and restore is acceptable.
    let cached = {
        let mut guard = state.cached.lock().unwrap_or_else(|poison| poison.into_inner());
        match guard.take() {
            Some(loaded)
                if loaded.provider == descriptor.provider
                    && loaded.model_id == descriptor.model_id =>
            {
                Some(loaded)
            }
            // A different model (Settings switch) or nothing cached: drop it and
            // reload below.
            _ => None,
        }
    };

    let query = trimmed.to_string();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let loaded = match cached {
            Some(loaded) => loaded,
            None => match load_embedder(&app_data_dir, &descriptor) {
                Ok(loaded) => loaded,
                Err(error) => {
                    crate::native_capture::debug_log::log_error(format!(
                        "semantic query embed failed to load model '{}/{}': {error}",
                        descriptor.provider, descriptor.model_id
                    ));
                    return None;
                }
            },
        };
        // The query embed runs at the blocking thread's DEFAULT QoS. The retired
        // backfill per-thread background-QoS downclock is gone under candle (ADR
        // 0037), so there is no QoS to deliberately leave un-backgrounded here; this
        // is simply user-initiated, latency-sensitive search work. `embed_text` is
        // `&self`-immutable, so the embedder needs no `mut`.
        let vector = loaded
            .embedder
            .embed_text(&query)
            .map_err(|error| error.to_string());
        Some((loaded, vector))
    })
    .await;

    let (loaded, vector) = match result {
        Ok(Some((loaded, vector))) => (loaded, vector),
        Ok(None) => return None,
        Err(error) => {
            crate::native_capture::debug_log::log_error(format!(
                "semantic query embed task panicked/cancelled: {error}"
            ));
            return None;
        }
    };

    // Restore the embedder for the next search.
    {
        let mut guard = state.cached.lock().unwrap_or_else(|poison| poison.into_inner());
        *guard = Some(loaded);
    }

    match vector {
        // Mirror the write-path guard (`store_vector` rejects non-finite
        // components): a NaN/inf component would yield non-deterministic KNN
        // ordering, so drop the vector and fall back to the keyword-only path the
        // function already takes when there is no usable semantic vector.
        Ok(vector) if vector.iter().any(|component| !component.is_finite()) => {
            crate::native_capture::debug_log::log_error(
                "semantic query embed produced a non-finite component; \
                 falling back to keyword-only search"
                    .to_string(),
            );
            None
        }
        Ok(vector) => Some(vector),
        Err(error) => {
            crate::native_capture::debug_log::log_error(format!(
                "semantic query embed failed: {error}"
            ));
            None
        }
    }
}
