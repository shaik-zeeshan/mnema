//! The audio transcription (and speaker analysis) backfill pass, and the
//! admissions it and the reprocess paths freeze onto queued jobs.

use std::path::Path;

use capture_types::{AudioTranscriptionSettings, SpeakerAnalysisSettings};

use crate::AppCore;

pub async fn run_audio_transcription_backfill_after_model_install(
    infra: &::app_infra::AppInfra,
    core: &AppCore,
) {
    run_audio_transcription_backfill_pass(infra, core, "post-download").await;
}

pub async fn run_audio_transcription_backfill_pass(
    infra: &::app_infra::AppInfra,
    core: &AppCore,
    reason: &str,
) {
    let admission = audio_transcription_admission_for_current_settings(core);
    if !admission.enabled || !admission.provider_available || admission.payload_json.is_none() {
        crate::debug_log::log(
            format!(
                "{reason} audio transcription backfill skipped because transcription is disabled or the selected model is unavailable"
            ),
        );
        return;
    }

    match infra
        .backfill_missing_audio_transcription_jobs(&admission)
        .await
    {
        Ok(enqueued_count) => {
            crate::debug_log::log_info(format!(
                "{reason} audio transcription backfill completed (enqueued={enqueued_count})"
            ));
        }
        Err(error) => {
            crate::debug_log::log_warn(format!(
                "{reason} audio transcription backfill failed: {error}"
            ));
        }
    }

    let speaker_admission = speaker_analysis_admission_for_current_settings(core);
    if speaker_admission.enabled
        && speaker_admission.provider_available
        && speaker_admission.payload_json.is_some()
    {
        match infra
            .backfill_missing_speaker_analysis_jobs(&speaker_admission)
            .await
        {
            Ok(enqueued_count) => {
                crate::debug_log::log_info(format!(
                    "{reason} speaker analysis backfill completed (enqueued={enqueued_count})"
                ));
            }
            Err(error) => {
                crate::debug_log::log_warn(format!(
                    "{reason} speaker analysis backfill failed: {error}"
                ));
            }
        }
    }
}

pub fn audio_transcription_admission_for_current_settings(
    core: &AppCore,
) -> ::app_infra::AudioSegmentTranscriptionAdmission {
    let (transcription_settings, speaker_settings) = match core
        .state::<crate::recording_settings::RecordingSettingsState>()
        .lock()
    {
        Ok(settings) => (
            settings.settings.transcription.clone(),
            settings.settings.speaker_analysis.clone(),
        ),
        Err(_) => {
            crate::debug_log::log_error(
                "failed to read recording settings for audio transcription backfill admission",
            );
            return ::app_infra::AudioSegmentTranscriptionAdmission::disabled();
        }
    };

    if !transcription_settings.enabled || !transcription_settings.microphone_enabled {
        return ::app_infra::AudioSegmentTranscriptionAdmission::disabled();
    }

    let app_data_dir = &core.paths().config;

    audio_transcription_admission_for_settings(
        app_data_dir,
        &transcription_settings,
        Some(&speaker_settings),
    )
}

pub fn speaker_analysis_admission_for_current_settings(
    core: &AppCore,
) -> ::app_infra::AudioSegmentSpeakerAnalysisAdmission {
    let speaker_settings = match core
        .state::<crate::recording_settings::RecordingSettingsState>()
        .lock()
    {
        Ok(settings) => settings.settings.speaker_analysis.clone(),
        Err(_) => {
            crate::debug_log::log_error(
                "failed to read recording settings for speaker analysis admission",
            );
            return ::app_infra::AudioSegmentSpeakerAnalysisAdmission::disabled();
        }
    };
    speaker_analysis_admission_for_settings(core, &speaker_settings)
}

fn speaker_analysis_admission_for_settings(
    core: &AppCore,
    speaker_settings: &SpeakerAnalysisSettings,
) -> ::app_infra::AudioSegmentSpeakerAnalysisAdmission {
    if !speaker_settings.separate_speakers {
        return ::app_infra::AudioSegmentSpeakerAnalysisAdmission::disabled();
    }

    let app_data_dir = &core.paths().config;
    let models_dir = speaker_analysis::speaker_analysis_models_dir(app_data_dir);
    let provider = speaker_settings.provider.clone();
    let model_id = speaker_settings.model_id.clone();
    let manifest = speaker_analysis::builtin_model_manifest();
    let Some(descriptor) =
        speaker_analysis::find_model_descriptor(&manifest, &provider, model_id.as_deref())
    else {
        return ::app_infra::AudioSegmentSpeakerAnalysisAdmission::unavailable();
    };
    match speaker_analysis::detect_model_status(&models_dir, descriptor) {
        Ok(status) if status.status == speaker_analysis::ModelStatusKind::Installed => {
            let mut payload =
                ::app_infra::SpeakerAnalysisJobPayload::new(provider, model_id.clone());
            payload.normalize_model_selection();
            payload.recognize_people = speaker_settings.recognize_saved_people;
            payload.auto_label_owner = speaker_settings.auto_label_owner;
            insert_speaker_analysis_timeout_option(&mut payload, speaker_settings.timeout_seconds);
            match serde_json::to_string(&payload) {
                Ok(payload_json) => {
                    ::app_infra::AudioSegmentSpeakerAnalysisAdmission::available(payload_json)
                }
                Err(error) => {
                    crate::debug_log::log_error(format!(
                        "failed to serialize speaker analysis payload: {error}"
                    ));
                    ::app_infra::AudioSegmentSpeakerAnalysisAdmission::unavailable()
                }
            }
        }
        Ok(_) => ::app_infra::AudioSegmentSpeakerAnalysisAdmission::unavailable(),
        Err(error) => {
            crate::debug_log::log_error(format!(
                "failed to inspect selected speaker analysis model: {error}"
            ));
            ::app_infra::AudioSegmentSpeakerAnalysisAdmission::unavailable()
        }
    }
}

pub fn audio_transcription_admission_for_settings(
    app_data_dir: &Path,
    transcription_settings: &AudioTranscriptionSettings,
    speaker_settings: Option<&SpeakerAnalysisSettings>,
) -> ::app_infra::AudioSegmentTranscriptionAdmission {
    if !transcription_settings.enabled || !transcription_settings.microphone_enabled {
        return ::app_infra::AudioSegmentTranscriptionAdmission::disabled();
    }

    match crate::audio_transcription_models::selected_audio_transcription_model_available(
        app_data_dir,
        transcription_settings,
    ) {
        Ok(true) => {
            let provider = crate::audio_transcription_models::provider_id_for_settings(
                transcription_settings.provider,
            );
            let mut payload = ::app_infra::AudioTranscriptionJobPayload::new(
                provider,
                transcription_settings.model_id.clone(),
                transcription_settings.language.clone(),
            );
            payload.options =
                crate::audio_transcription_models::transcription_request_options_for_settings(
                    transcription_settings,
                );
            attach_speaker_analysis_payload(&mut payload, app_data_dir, speaker_settings);
            match serde_json::to_string(&payload) {
                Ok(payload_json) => {
                    ::app_infra::AudioSegmentTranscriptionAdmission::available(payload_json)
                }
                Err(error) => {
                    crate::debug_log::log_error(format!(
                        "failed to serialize audio transcription backfill payload: {error}"
                    ));
                    ::app_infra::AudioSegmentTranscriptionAdmission::unavailable()
                }
            }
        }
        Ok(false) => ::app_infra::AudioSegmentTranscriptionAdmission::unavailable(),
        Err(error) => {
            crate::debug_log::log_error(format!(
                "failed to inspect selected audio transcription model for backfill: {error}"
            ));
            ::app_infra::AudioSegmentTranscriptionAdmission::unavailable()
        }
    }
}

fn attach_speaker_analysis_payload(
    payload: &mut ::app_infra::AudioTranscriptionJobPayload,
    app_data_dir: &Path,
    speaker_settings: Option<&SpeakerAnalysisSettings>,
) {
    let Some(speaker_settings) = speaker_settings else {
        return;
    };
    if !speaker_settings.separate_speakers {
        return;
    }
    let models_dir = speaker_analysis::speaker_analysis_models_dir(app_data_dir);
    let manifest = speaker_analysis::builtin_model_manifest();
    let Some(descriptor) = speaker_analysis::find_model_descriptor(
        &manifest,
        &speaker_settings.provider,
        speaker_settings.model_id.as_deref(),
    ) else {
        return;
    };
    match speaker_analysis::detect_model_status(&models_dir, descriptor) {
        Ok(status) if status.status == speaker_analysis::ModelStatusKind::Installed => {}
        Ok(_) => return,
        Err(error) => {
            crate::debug_log::log_error(format!(
                "failed to inspect selected speaker analysis model for transcription payload: {error}"
            ));
            return;
        }
    }
    let mut speaker_payload = ::app_infra::SpeakerAnalysisJobPayload::new(
        speaker_settings.provider.clone(),
        speaker_settings.model_id.clone(),
    );
    speaker_payload.normalize_model_selection();
    speaker_payload.recognize_people = speaker_settings.recognize_saved_people;
    speaker_payload.auto_label_owner = speaker_settings.auto_label_owner;
    insert_speaker_analysis_timeout_option(&mut speaker_payload, speaker_settings.timeout_seconds);
    if let Ok(value) = serde_json::to_value(speaker_payload) {
        payload.options.insert(
            ::app_infra::SPEAKER_ANALYSIS_PAYLOAD_OPTION_KEY.to_string(),
            value,
        );
    }
}

fn insert_speaker_analysis_timeout_option(
    payload: &mut ::app_infra::SpeakerAnalysisJobPayload,
    timeout_seconds: u64,
) {
    let timeout_seconds = timeout_seconds.clamp(60, 3600);
    payload.options.insert(
        ::app_infra::HELPER_TIMEOUT_SECONDS_OPTION.to_string(),
        serde_json::json!(timeout_seconds),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use audio_transcription::providers::PARAKEET_TDT_0_6B_V3_ONNX_INT8_MODEL_ID;
    use std::fs;

    #[test]
    fn audio_transcription_admission_for_settings_reflects_selected_model_availability() {
        let dir = tempfile::tempdir().unwrap();
        let settings = AudioTranscriptionSettings::default();

        let missing = audio_transcription_admission_for_settings(dir.path(), &settings, None);
        assert!(missing.enabled);
        assert!(!missing.provider_available);
        assert_eq!(missing.payload_json, None);

        // Parakeet, not the default Whisper: app-core's own build has no `local-whisper`, so an
        // installed Whisper model is refused here (see `audio_transcription_models` tests).
        let settings = AudioTranscriptionSettings {
            provider: capture_types::AudioTranscriptionProvider::Parakeet,
            model_id: Some(PARAKEET_TDT_0_6B_V3_ONNX_INT8_MODEL_ID.to_string()),
            ..settings
        };
        let models_dir = audio_transcription::audio_transcription_models_dir(dir.path());
        let install_dir = audio_transcription::model_install_dir(
            &models_dir,
            audio_transcription::PARAKEET_PROVIDER_ID,
            PARAKEET_TDT_0_6B_V3_ONNX_INT8_MODEL_ID,
        )
        .expect("model install directory");
        fs::create_dir_all(&install_dir).expect("model install directory should be created");
        for file in [
            "config.json",
            "nemo128.onnx",
            "encoder-model.int8.onnx",
            "decoder_joint-model.int8.onnx",
            "vocab.txt",
        ] {
            fs::write(install_dir.join(file), b"model").expect("model file should be written");
        }
        audio_transcription::write_installed_marker(
            &models_dir,
            audio_transcription::PARAKEET_PROVIDER_ID,
            PARAKEET_TDT_0_6B_V3_ONNX_INT8_MODEL_ID,
        )
        .expect("installed marker should be written");

        let available = audio_transcription_admission_for_settings(dir.path(), &settings, None);
        assert!(available.enabled);
        assert!(available.provider_available);
        let payload: ::app_infra::AudioTranscriptionJobPayload = serde_json::from_str(
            available
                .payload_json
                .as_deref()
                .expect("available model should freeze payload"),
        )
        .expect("payload should deserialize");
        assert_eq!(payload.provider, audio_transcription::PARAKEET_PROVIDER_ID);
        assert_eq!(
            payload.model_id.as_deref(),
            Some(PARAKEET_TDT_0_6B_V3_ONNX_INT8_MODEL_ID)
        );
        assert_eq!(payload.language, "auto");
    }
}
