use std::sync::Arc;
use tauri::State;

pub(crate) const DEEPGRAM_KEY_ACCOUNT: &str = "transcription.deepgram";

/// THE Deepgram key loader for [`audio_transcription::providers::DeepgramProvider`].
/// `Ok(None)` = no key configured; `Err(message)` = the secret vault could not be read
/// (denied) — kept distinct per ADR 0048's amendment so the provider parks the job as
/// transient liveness instead of reporting "no key configured".
pub(crate) fn load_deepgram_key() -> Result<Option<String>, String> {
    app_infra::load_ai_provider_key(DEEPGRAM_KEY_ACCOUNT).map_err(|error| error.to_string())
}

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
    tokio::task::spawn_blocking(move || {
        app_infra::store_ai_provider_key(DEEPGRAM_KEY_ACCOUNT, &key)
    })
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
        Arc::new(load_deepgram_key),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Denied ≠ missing at the wiring seam (ADR 0048 amendment): the loader keeps
    /// "no key stored" as `Ok(None)` and never collapses a store error into it.
    /// One sequential test (not several) because the vault slot and the Deepgram
    /// account are process-shared.
    #[test]
    fn load_deepgram_key_distinguishes_missing_from_stored() {
        crate::secret_vault_test_support::install_shared_test_secret_vault();

        app_infra::delete_ai_provider_key(DEEPGRAM_KEY_ACCOUNT).expect("delete should succeed");
        assert_eq!(
            load_deepgram_key(),
            Ok(None),
            "no key stored must be Ok(None)"
        );

        app_infra::store_ai_provider_key(DEEPGRAM_KEY_ACCOUNT, "dg-key").expect("store");
        assert_eq!(load_deepgram_key(), Ok(Some("dg-key".to_string())));

        app_infra::delete_ai_provider_key(DEEPGRAM_KEY_ACCOUNT).expect("cleanup");
        assert_eq!(load_deepgram_key(), Ok(None));
    }

    /// The denied error the loader forwards must be user-worthy and must never read
    /// like "no key configured" — it becomes the job's park reason and the Settings
    /// failure message verbatim.
    #[test]
    fn secret_vault_denied_maps_to_a_user_worthy_message() {
        let message = app_infra::AppInfraError::SecretVaultDenied("user denied prompt".to_string())
            .to_string();
        assert!(message.contains("keychain"), "got: {message}");
        assert!(message.contains("access denied"), "got: {message}");
        assert!(message.contains("Settings"), "got: {message}");
        assert!(
            !message.to_lowercase().contains("no key"),
            "denied must not read as missing: {message}"
        );
    }
}
