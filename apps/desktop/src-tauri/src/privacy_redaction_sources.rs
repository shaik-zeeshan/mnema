use capture_types::{CaptureErrorResponse, RecordingSettings};
use tauri::Manager;

fn err(code: &str, message: &str) -> CaptureErrorResponse {
    CaptureErrorResponse {
        code: code.to_string(),
        message: message.to_string(),
    }
}

fn new_app_source_id() -> String {
    format!(
        "excluded-app-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default()
    )
}

fn with_app_exclusion_mutation(
    app_handle: tauri::AppHandle,
    mutate: impl FnOnce(&mut RecordingSettings) -> Result<(), CaptureErrorResponse>,
) -> Result<RecordingSettings, CaptureErrorResponse> {
    let settings = {
        let settings_state = app_handle.state::<crate::native_capture::RecordingSettingsState>();
        let mut settings_runtime = settings_state
            .lock()
            .expect("recording settings state poisoned");
        let mut settings = settings_runtime.settings.clone();

        mutate(&mut settings)?;
        settings.privacy =
            crate::native_capture::settings::validate_privacy_settings(settings.privacy)?;
        crate::native_capture::settings::persist_recording_settings(&app_handle, &settings)?;

        settings_runtime.settings = settings.clone();
        settings
    };

    crate::native_capture::emit_recording_settings_changed(&app_handle, &settings);
    crate::native_capture::privacy::request_privacy_filter_refresh(
        &app_handle,
        crate::native_capture::privacy::PrivacyRefreshReason::StaticAppRuleMutation,
    );
    Ok(settings)
}

#[tauri::command]
pub fn add_privacy_excluded_app(
    bundle_id: String,
    display_name: String,
    app_handle: tauri::AppHandle,
) -> Result<RecordingSettings, CaptureErrorResponse> {
    with_app_exclusion_mutation(app_handle, |settings| {
        let bundle_id = crate::native_capture::settings::canonicalize_app_bundle_id(&bundle_id);
        let display_name = display_name.trim().to_string();
        if bundle_id.is_empty() || display_name.is_empty() {
            return Err(err(
                "invalid_privacy_rule",
                "App bundle and display name are required",
            ));
        }
        if settings.privacy.excluded_apps.iter().any(|app| {
            crate::native_capture::settings::canonicalize_app_bundle_id(&app.bundle_id) == bundle_id
        }) {
            return Ok(());
        }
        settings
            .privacy
            .excluded_apps
            .push(capture_metadata::ExcludedAppEntry {
                id: new_app_source_id(),
                enabled: true,
                bundle_id,
                display_name,
            });
        Ok(())
    })
}

#[tauri::command]
pub fn set_privacy_excluded_app_enabled(
    source_id: String,
    enabled: bool,
    app_handle: tauri::AppHandle,
) -> Result<RecordingSettings, CaptureErrorResponse> {
    with_app_exclusion_mutation(app_handle, |settings| {
        let Some(app) = settings
            .privacy
            .excluded_apps
            .iter_mut()
            .find(|app| app.id == source_id)
        else {
            return Err(err("privacy_source_not_found", "Privacy app exclusion not found"));
        };
        app.enabled = enabled;
        Ok(())
    })
}

#[tauri::command]
pub fn remove_privacy_excluded_app(
    source_id: String,
    app_handle: tauri::AppHandle,
) -> Result<RecordingSettings, CaptureErrorResponse> {
    with_app_exclusion_mutation(app_handle, |settings| {
        let before = settings.privacy.excluded_apps.len();
        settings
            .privacy
            .excluded_apps
            .retain(|app| app.id != source_id);
        if settings.privacy.excluded_apps.len() == before {
            return Err(err("privacy_source_not_found", "Privacy app exclusion not found"));
        }
        Ok(())
    })
}
