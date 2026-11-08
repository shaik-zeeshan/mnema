//! The atomic Semantic Search Model switch: rebuild the vector table, then
//! persist the selection through the recording-settings save.

use std::sync::Arc;

use semantic_search::resolve_descriptor;

use super::select_model_lock;
use crate::AppCore;

/// **Switch the Semantic Search Model**: rebuild the `vec0` table at the
/// newly-selected model's dimension, THEN persist the selection — two sequential
/// transactions ordered so the failure modes are recoverable rather than
/// permanently stuck.
///
/// This replaces a worse two-step ordering (persist `model_id`, *then* re-index)
/// that was the root of three faces of one bug (H1/H2 + the re-index race): if the
/// re-index failed after the persist, the table stayed at the old dimension while
/// the selection named a new-dimension model — every search hard-failed and the
/// backfill worker error-looped forever, with no recovery by re-selecting the same
/// model (the UI early-returns on an unchanged pick). Here the order is reversed,
/// so each failure leaves a consistent-or-self-healing state:
///   - **A failed rebuild never advances the selection.** The recreate runs in its
///     own transaction and rolls back, so the old model stays selected against the
///     intact old-dim table and the frontend surfaces the error.
///   - **A failed persist leaves a new-dim table that startup reconciliation
///     re-aligns.** The table is already at the new dimension but the selection
///     still names the old model; the error is reported, and the next startup's
///     [`reconcile_semantic_search_index_on_startup`] rebuilds the table back to
///     whatever model remained selected — so there is no permanently-stuck state.
/// The two writes are NOT one atomic operation; the ordering plus the startup
/// reconciler is what keeps table-dim and persisted-id from disagreeing for long.
///
/// The rebuild+persist pair is serialized process-wide by [`SELECT_MODEL_LOCK`] so
/// two concurrent invocations cannot interleave their recreate+persist and leave
/// the table dimension and persisted id disagreeing.
///
/// Different **Semantic Search Model Tier**s produce incomparable vectors and
/// `vec0` is a fixed-dim table, so a switch re-derives every **Search Result
/// Anchor** (ADR 0036); a switch can also change the dimension (768-dim `nomic` →
/// 1024-dim `bge-m3`), so the table is rebuilt, not merely cleared. Recreating it
/// re-exposes every `direct` anchor to the **Semantic Index Backfill** worker,
/// which re-derives each under the new model (newest-first), progress living
/// entirely in the DB. Returns the number of vectors discarded.
///
/// The caller (Settings UI) gates the switch behind a `@tauri-apps/plugin-dialog`
/// confirm; this command owns the persist+rebuild ordering so the frontend no
/// longer makes two separate invokes that could leave state half-applied.
pub async fn select_semantic_search_model(
    model_id: String,
    core: &Arc<AppCore>,
    infra: &crate::AppInfraState,
    settings_state: &crate::recording_settings::RecordingSettingsState,
) -> Result<u64, String> {
    // Serialize the whole rebuild+persist against any concurrent switch, so two
    // invocations cannot interleave recreate + persist and leave the table
    // dimension disagreeing with the persisted id. Held until the command returns.
    let _switch_guard = select_model_lock().lock().await;

    // Resolve the target model's dimension up front. An unresolvable id (legacy /
    // not enumerated) aborts BEFORE touching the table or the persisted selection,
    // so a bad pick is a clean no-op.
    let current = crate::semantic_search_worker::effective_semantic_search_settings(core);
    let descriptor = resolve_descriptor(&current.provider, &model_id)
        .ok_or_else(|| format!("unknown semantic search model '{model_id}'"))?;
    let dimension = descriptor.dimension;

    // Rebuild the table FIRST (the step that can fail under the worker's
    // concurrent writes). Only on success do we persist the new selection, so the
    // persisted `model_id` and the live table dimension stay in lockstep.
    let cleared = infra
        .semantic_search()
        .recreate_vectors_table(&model_id, dimension)
        .await
        .map_err(|error| format!("failed to rebuild semantic search vectors: {error}"))?;

    // Persist the selection now that the table matches. A persist failure here is
    // reported to the caller; the table is already at the new dimension, and the
    // next startup reconciliation would re-align it to whatever model remained
    // selected, so there is no permanently-stuck state.
    crate::native_capture::settings_update::persist_semantic_search_settings(
        core,
        settings_state,
        capture_types::UpdateSemanticSearchSettingsRequest {
            enabled: None,
            provider: None,
            model_id: Some(Some(model_id.clone())),
        },
    )
    .map_err(|error| {
        format!(
            "rebuilt the semantic search index but failed to persist the model selection: {}",
            error.message
        )
    })?;

    // Hardening assert (F15): the rebuild + persist are two separate transactions
    // under the in-process `SELECT_MODEL_LOCK`, relying on startup reconciliation to
    // re-align a half-applied switch. Confirm the live vec0 table now names the
    // model we just selected, so a half-applied switch (a concurrent worker
    // DROP+CREATE landing between, or a recreate that did not take) is DETECTED and
    // logged loudly rather than silently degrading search to keyword-only. The
    // selection is already persisted, so a mismatch is reported but not rolled back —
    // the next startup reconciliation self-heals it.
    //
    // The check is on the MODEL STAMP, not the column width: with same-dimension
    // models in the catalog (gte-modernbert / granite-r2 are 768 like nomic,
    // granite-small is 384 like multilingual-e5-small), a width compare would read
    // green on exactly the switches most in need of detection.
    // Compared against the composite INDEX EPOCH (`model_id@recipe`), not the bare
    // id: the stamp also records the embedding recipe, so a stale-recipe table must
    // read as a mismatch here too.
    let expected_epoch = ::app_infra::vectors_index_epoch(&model_id);
    match infra.semantic_search().live_vector_model().await {
        Ok(Some(live)) if live == expected_epoch => {}
        Ok(other) => {
            crate::debug_log::log_warn(format!(
                "semantic search model switch to '{model_id}': the live vec0 table is stamped {other:?}, NOT the selected model, after rebuild+persist (half-applied switch — startup reconciliation will re-align it; search stays keyword-only until then)"
            ));
        }
        Err(error) => {
            crate::debug_log::log_warn(format!(
                "semantic search model switch to '{model_id}': could not read the live vec0 model stamp to confirm it matches the selection after rebuild+persist: {error}"
            ));
        }
    }

    crate::debug_log::log_info(format!(
        "semantic search model switched to '{model_id}': rebuilt vector table at {dimension} dims, discarded {cleared} vector(s); the backfill worker will re-derive every anchor under the new model"
    ));
    Ok(cleared)
}
