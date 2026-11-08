//! Wipe User Context: engine off first, then every derived and recalled store.

use std::sync::Arc;

use capture_types::UpdateAiRuntimeSettingsRequest;

use crate::{AppCore, AppInfraState, CoreEvent};

/// **Wipe User Context** (#97, ADR 0029): the explicit, full clear of the derived
/// dossier. Because that dossier deliberately outlives the raw-capture Retention
/// Policy window, this is the disclosed control for erasing it.
///
/// These are three independent writes (engine settings, the `user_context_*`
/// tables, the conversation store) with no enclosing transaction, so a mid-failure
/// is possible. We order them so the *only* possible partial state is the safe one:
///
/// 1. Turn the **Reasoning Engine** OFF FIRST, through the normal AI-runtime
///    settings flow (`enabled = false`), so it persists to
///    `recording-settings.json` and broadcasts
///    `recording_settings_changed`/`*_domain_changed`. Wiping implies "I'm done";
///    rebuilding is a deliberate re-opt-in. (Merely toggling the engine off — the
///    separate master toggle — is NOT a wipe: it stops new derivation but leaves
///    the dossier readable.) Disabling first means a later data-wipe failure leaves
///    "engine OFF + dossier present" (consistent, retryable) rather than the unsafe
///    "engine ON + data gone" the previous ordering risked. If disabling fails we
///    bail before touching any data, leaving everything intact.
/// 2. Clear every `user_context_*` table — all derived **Activity** /
///    **Conclusion** data AND **Dismissal State** and the derivation-run ledger
///    (`wipe_all`). Raw captures and other settings are untouched.
/// 3. Clear all persistent Quick Recall / Chat conversations (issue #102), the
///    single shared store, and Quick Recall's recent queries, so the same control
///    erases every derived/recalled surface. All data wipes run even if one errors, so a single
///    failure cannot strand the other store half-cleared with no retry; the first
///    error is surfaced afterwards.
/// 4. Emit `user_context_changed` so the now-empty surface refreshes.
pub async fn wipe_user_context(core: &Arc<AppCore>, infra: &AppInfraState) -> Result<(), String> {
    // 1. Disable the engine FIRST via the same settings-update path the toggle
    //    uses, so persistence + broadcasts (recording_settings_changed /
    //    domain_changed) are identical. Doing this before any data wipe makes the
    //    unsafe "engine ON + data gone" state unreachable: if this fails we bail
    //    with everything intact; if a later data wipe fails we are left with
    //    "engine OFF + dossier present", which is consistent and retryable.
    crate::native_capture::settings_update::update_ai_runtime_settings(
        UpdateAiRuntimeSettingsRequest {
            enabled: Some(false),
            ..Default::default()
        },
        core,
    )
    .map_err(|e| e.message)?;

    // 2 + 3. Storage half: clear the whole dossier AND all persistent conversations.
    //    Run both regardless of an error in the first so a single store failure
    //    cannot leave the other half-cleared with no retry path; report the first
    //    error after attempting both.
    let wipe_context = infra.user_context().wipe_all().await;
    let wipe_conversations = infra.conversation().wipe_all().await;
    // Quick Recall's recent queries share the conversations' contract.
    let wipe_recent_queries = ::app_infra::delete_recent_queries(infra.write_pool()).await;
    wipe_context.map_err(|e| e.to_string())?;
    wipe_conversations.map_err(|e| e.to_string())?;
    wipe_recent_queries.map_err(|e| e.to_string())?;

    // 4. Refresh the (now empty) User Context surface.
    core.host().emit(CoreEvent::UserContextChanged);
    Ok(())
}
