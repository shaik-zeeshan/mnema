//! Voice enrollment door: a recorded clip in, a stored voiceprint or a typed
//! rejection out.
//!
//! A thin adapter. All enrollment judgment lives in
//! `speaker_analysis::embed_enrollment_clip`; all storage rules live in
//! `ProcessingStore::upsert_account_owner_voiceprint`. This file only moves
//! values between them, and turns recognition on once a voiceprint exists —
//! without that flip the voiceprint would be loaded by nothing
//! (`default_speaker_recognition_enabled()` is `false`).

use std::sync::Arc;

use capture_types::SettingsOwnershipDomain;
use serde::{Deserialize, Serialize};
use tauri::Manager;

use crate::app_infra::{AppInfraState, PersonProfileDto};

/// Name used when the caller does not supply one. The Voice screen may ask; the
/// Settings re-enroll surface does not have to.
const DEFAULT_OWNER_DISPLAY_NAME: &str = "You";

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnrollAccountOwnerVoiceRequest {
    /// Path of the clip produced by `record_bounded_microphone_clip`.
    pub clip_path: String,
    pub display_name: Option<String>,
}

/// The one thing the Voice screen renders: enrolled, or why not. Mirrors
/// `speaker_analysis::EnrollmentOutcome` — the rejections are the embedder's
/// words, not this file's.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum VoiceEnrollmentOutcomeDto {
    #[serde(rename_all = "camelCase")]
    Enrolled { profile: PersonProfileDto },
    #[serde(rename_all = "camelCase")]
    TooShort { duration_ms: u64 },
    NoSpeech,
    #[serde(rename_all = "camelCase")]
    MultipleSpeakers { speaker_count: usize },
}

/// Embed an enrollment clip and, if it is usable, store it against the account
/// owner's Person Profile and switch recognition on.
#[tauri::command]
pub async fn enroll_account_owner_voice(
    request: EnrollAccountOwnerVoiceRequest,
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, AppInfraState>,
) -> Result<VoiceEnrollmentOutcomeDto, String> {
    let app_data_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|error| format!("failed to resolve app data directory: {error}"))?;
    let models_dir = speaker_analysis::speaker_analysis_models_dir(&app_data_dir);
    let clip_path = std::path::PathBuf::from(&request.clip_path);

    // The embedder decodes and diarizes; keep it off the async runtime's worker.
    let judgment = tauri::async_runtime::spawn_blocking(move || {
        embed_enrollment_clip_for_build(&clip_path, &models_dir)
    })
    .await
    .map_err(|error| format!("voice enrollment task failed: {error}"))??;

    let (embedding, model_id) = match judgment {
        ClipJudgment::Voiceprint {
            embedding,
            model_id,
        } => (embedding, model_id),
        ClipJudgment::Rejected(outcome) => return Ok(outcome),
    };

    let display_name = request
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or(DEFAULT_OWNER_DISPLAY_NAME);

    let infra = Arc::clone(&*state);
    let profile = infra
        .upsert_account_owner_voiceprint(
            display_name,
            speaker_analysis::SPEAKRS_PROVIDER_ID,
            &model_id,
            &embedding,
        )
        .await
        .map_err(|error| format!("failed to store account owner voiceprint: {error}"))?;

    crate::native_capture::apply_recording_settings_domain_mutation_from_app_handle(
        &app_handle,
        SettingsOwnershipDomain::Processing,
        |settings| {
            enable_recognition_for_enrollment(settings);
            Ok(())
        },
    )
    .map_err(|error| format!("failed to enable speaker recognition: {}", error.message))?;

    Ok(VoiceEnrollmentOutcomeDto::Enrolled {
        profile: PersonProfileDto::from(profile),
    })
}

/// Whether an account-owner voiceprint exists — the read-out the Voice step and
/// the Settings enrollment surface both need.
#[tauri::command]
pub async fn get_account_owner_person_id(
    state: tauri::State<'_, AppInfraState>,
) -> Result<Option<i64>, String> {
    let infra = Arc::clone(&*state);
    infra
        .account_owner_person_id()
        .await
        .map_err(|error| format!("failed to read account owner profile: {error}"))
}

/// Enrolling switches recognition on. Without this the voiceprint is loaded by
/// nothing: `default_speaker_recognition_enabled()` is `false`, and only
/// `recognize_saved_people` puts enrolled people on the analysis request.
/// "Label my voice automatically" is left alone — it defaults on, and a user who
/// turned it off is not asking for it back.
fn enable_recognition_for_enrollment(settings: &mut capture_types::RecordingSettings) {
    settings.speaker_analysis.recognize_saved_people = true;
}

/// The embedder's verdict, restated without naming `EnrollmentOutcome` — that
/// type only exists when the speakrs feature is compiled in, and the command
/// body has to build either way.
enum ClipJudgment {
    Voiceprint { embedding: Vec<u8>, model_id: String },
    Rejected(VoiceEnrollmentOutcomeDto),
}

/// The one path shape this door accepts: a clip the bounded recorder itself
/// wrote, directly in the OS temp dir. `clip_path` arrives from the renderer, so
/// without this the door would open — and, below, destroy — any file the caller
/// names.
#[cfg(target_os = "macos")]
fn is_bounded_enrollment_clip(clip_path: &std::path::Path) -> bool {
    clip_path.parent() == Some(std::env::temp_dir().as_path())
        && clip_path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                name.starts_with(crate::native_capture::ENROLLMENT_CLIP_PREFIX)
                    && name.ends_with(".m4a")
            })
}

/// Judge the clip, then destroy it — on every exit: stored, rejected, or failed.
///
/// The clip is a raw recording of the user's voice sitting in the OS temp dir,
/// outside the encrypted capture store, outside retention, and outside Delete
/// Recent Capture. It exists only to be embedded, so nothing keeps it once the
/// embedder has spoken.
fn embed_enrollment_clip_for_build(
    clip_path: &std::path::Path,
    models_dir: &std::path::Path,
) -> Result<ClipJudgment, String> {
    #[cfg(target_os = "macos")]
    if !is_bounded_enrollment_clip(clip_path) {
        // Deliberately says nothing about the path it was handed.
        return Err("voice enrollment accepts only a clip it recorded".to_string());
    }
    let judgment = embed_enrollment_clip_inner(clip_path, models_dir);
    let _ = std::fs::remove_file(clip_path);
    judgment
}

#[cfg(feature = "speaker-analysis-speakrs")]
fn embed_enrollment_clip_inner(
    clip_path: &std::path::Path,
    models_dir: &std::path::Path,
) -> Result<ClipJudgment, String> {
    use speaker_analysis::EnrollmentOutcome;
    let outcome = speaker_analysis::embed_enrollment_clip(clip_path, models_dir)
        .map_err(|error| format!("voice enrollment failed: {error}"))?;
    Ok(match outcome {
        EnrollmentOutcome::Voiceprint {
            embedding,
            model_id,
        } => ClipJudgment::Voiceprint {
            embedding,
            model_id,
        },
        EnrollmentOutcome::TooShort { duration_ms } => {
            ClipJudgment::Rejected(VoiceEnrollmentOutcomeDto::TooShort { duration_ms })
        }
        EnrollmentOutcome::NoSpeech => {
            ClipJudgment::Rejected(VoiceEnrollmentOutcomeDto::NoSpeech)
        }
        EnrollmentOutcome::MultipleSpeakers { speaker_count } => {
            ClipJudgment::Rejected(VoiceEnrollmentOutcomeDto::MultipleSpeakers { speaker_count })
        }
    })
}

#[cfg(not(feature = "speaker-analysis-speakrs"))]
fn embed_enrollment_clip_inner(
    _clip_path: &std::path::Path,
    _models_dir: &std::path::Path,
) -> Result<ClipJudgment, String> {
    Err("voice enrollment is not compiled into this build".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test 11 (settings half): storing a voiceprint turns recognition on.
    #[test]
    fn enrolling_enables_recognition_and_leaves_auto_labelling_at_its_default() {
        let mut settings = crate::native_capture::settings::default_recording_settings();
        assert!(
            !settings.speaker_analysis.recognize_saved_people,
            "recognition is off until something enrolls"
        );

        enable_recognition_for_enrollment(&mut settings);

        assert!(settings.speaker_analysis.recognize_saved_people);
        assert!(
            settings.speaker_analysis.auto_label_owner,
            "label-my-voice-automatically is on by default once enrolled"
        );
    }
}
