use std::sync::Arc;
use tauri::State;

pub(crate) const DEEPGRAM_KEY_ACCOUNT: &str = "transcription.deepgram";

/// Managed-state wrapper over the shared cell the Deepgram provider writes on an API-key rejection.
/// Newtype (not the bare Arc) so Tauri's type-keyed state can't collide with another managed value.
pub struct DeepgramAuthStatusState(pub audio_transcription::providers::DeepgramAuthStatus);

#[tauri::command]
pub async fn transcription_set_deepgram_key(
    key: String,
    auth: State<'_, DeepgramAuthStatusState>,
) -> Result<(), String> {
    let key = key.trim().to_string();
    if key.is_empty() {
        return Err("an API key is required".to_string());
    }
    let cell = auth.0.clone();
    tokio::task::spawn_blocking(move || app_infra::store_ai_provider_key(DEEPGRAM_KEY_ACCOUNT, &key))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())?;
    // A fresh key invalidates any prior rejection surfaced in Settings.
    if let Ok(mut status) = cell.lock() {
        *status = None;
    }
    Ok(())
}

#[tauri::command]
pub async fn transcription_has_deepgram_key() -> Result<bool, String> {
    tokio::task::spawn_blocking(|| app_infra::has_ai_provider_key(DEEPGRAM_KEY_ACCOUNT))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn transcription_clear_deepgram_key(
    auth: State<'_, DeepgramAuthStatusState>,
) -> Result<(), String> {
    let cell = auth.0.clone();
    tokio::task::spawn_blocking(|| app_infra::delete_ai_provider_key(DEEPGRAM_KEY_ACCOUNT))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())?;
    if let Ok(mut status) = cell.lock() {
        *status = None;
    }
    Ok(())
}

/// Result of a Deepgram API-key health check — the transcription-side analogue of the AI runtime's
/// connection test. `ok` = the key was accepted by Deepgram; `message` is a human-readable status.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeepgramProbeResult {
    ok: bool,
    message: String,
}

/// Validate the saved key against Deepgram (`GET /v1/auth/token` — no audio, no billing). Runs on
/// Save and behind a "Check" button so a bad/revoked key surfaces immediately instead of only when
/// a real segment later fails. A 401/403 also sets the auth-status cell (same path as a live job),
/// so the rejection shows in the status line.
#[tauri::command]
pub async fn transcription_test_deepgram(
    auth: State<'_, DeepgramAuthStatusState>,
) -> Result<DeepgramProbeResult, String> {
    let provider = audio_transcription::providers::DeepgramProvider::new(
        Arc::new(|| {
            app_infra::load_ai_provider_key(DEEPGRAM_KEY_ACCOUNT)
                .ok()
                .flatten()
        }),
        auth.0.clone(),
    );
    Ok(match provider.check_health().await {
        Ok(()) => DeepgramProbeResult {
            ok: true,
            message: "Deepgram API key verified.".to_string(),
        },
        Err(error) => DeepgramProbeResult {
            ok: false,
            message: error.to_string(),
        },
    })
}

/// The last Deepgram API-key rejection message (or None). ADR 0048: liveness-requeued jobs are
/// silent, so a revoked key surfaces here for the Settings status line.
#[tauri::command]
pub async fn transcription_deepgram_auth_status(
    auth: State<'_, DeepgramAuthStatusState>,
) -> Result<Option<String>, String> {
    Ok(auth.0.lock().ok().and_then(|status| status.clone()))
}
