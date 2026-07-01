use capture_types::{
    CaptureErrorResponse, RecordingSettings, RecordingSettingsDomainUpdateResponse,
    SettingsOwnershipDomain,
};
use std::sync::atomic::{AtomicU64, Ordering};

fn err(code: &str, message: &str) -> CaptureErrorResponse {
    CaptureErrorResponse {
        code: code.to_string(),
        message: message.to_string(),
    }
}

static NEXT_APP_SOURCE_ID_SUFFIX: AtomicU64 = AtomicU64::new(0);

fn new_app_source_id(existing_apps: &[capture_metadata::ExcludedAppEntry]) -> String {
    loop {
        let candidate = format!(
            "excluded-app-{}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or_default(),
            NEXT_APP_SOURCE_ID_SUFFIX.fetch_add(1, Ordering::Relaxed)
        );
        if existing_apps.iter().all(|app| app.id != candidate) {
            return candidate;
        }
    }
}

#[cfg(test)]
fn test_app_source_id(prefix: &str, suffix: u64) -> String {
    format!("excluded-app-{}-{}", prefix, suffix)
}

#[cfg(test)]
fn new_app_source_id_with_generator(
    existing_apps: &[capture_metadata::ExcludedAppEntry],
    mut next_candidate: impl FnMut() -> String,
) -> String {
    loop {
        let candidate = next_candidate();
        if existing_apps.iter().all(|app| app.id != candidate) {
            return candidate;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn excluded_app(id: &str) -> capture_metadata::ExcludedAppEntry {
        capture_metadata::ExcludedAppEntry {
            id: id.to_string(),
            enabled: true,
            bundle_id: format!("com.example.{id}"),
            display_name: id.to_string(),
        }
    }

    #[test]
    fn generated_app_source_id_skips_existing_collision() {
        let existing = vec![excluded_app(&test_app_source_id("same-tick", 0))];
        let mut suffix = 0;

        let id = new_app_source_id_with_generator(&existing, || {
            let candidate = test_app_source_id("same-tick", suffix);
            suffix += 1;
            candidate
        });

        assert_eq!(id, test_app_source_id("same-tick", 1));
    }

    #[test]
    fn generated_app_source_ids_are_unique_across_rapid_calls() {
        let mut apps = Vec::new();
        for _ in 0..100 {
            let id = new_app_source_id(&apps);
            assert!(apps
                .iter()
                .all(|app: &capture_metadata::ExcludedAppEntry| app.id != id));
            apps.push(excluded_app(&id));
        }
    }
}

fn with_app_exclusion_mutation(
    app_handle: tauri::AppHandle,
    mutate: impl FnOnce(&mut RecordingSettings) -> Result<(), CaptureErrorResponse>,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    crate::native_capture::apply_recording_settings_domain_mutation_from_app_handle(
        &app_handle,
        SettingsOwnershipDomain::AppPrivacyExclusion,
        mutate,
    )
}

#[tauri::command]
pub fn add_privacy_excluded_app(
    bundle_id: String,
    display_name: String,
    app_handle: tauri::AppHandle,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
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
                id: new_app_source_id(&settings.privacy.excluded_apps),
                enabled: true,
                bundle_id,
                display_name,
            });
        Ok(())
    })
}

#[cfg(target_os = "macos")]
pub(crate) fn add_or_enable_privacy_excluded_app_from_app_handle(
    app_handle: tauri::AppHandle,
    bundle_id: String,
    display_name: String,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    with_app_exclusion_mutation(app_handle, |settings| {
        let bundle_id = crate::native_capture::settings::canonicalize_app_bundle_id(&bundle_id);
        let display_name = display_name.trim().to_string();
        if bundle_id.is_empty() || display_name.is_empty() {
            return Err(err(
                "invalid_privacy_rule",
                "App bundle and display name are required",
            ));
        }
        if let Some(existing) = settings.privacy.excluded_apps.iter_mut().find(|app| {
            crate::native_capture::settings::canonicalize_app_bundle_id(&app.bundle_id) == bundle_id
        }) {
            existing.enabled = true;
            return Ok(());
        }
        settings
            .privacy
            .excluded_apps
            .push(capture_metadata::ExcludedAppEntry {
                id: new_app_source_id(&settings.privacy.excluded_apps),
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
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    with_app_exclusion_mutation(app_handle, |settings| {
        let Some(app) = settings
            .privacy
            .excluded_apps
            .iter_mut()
            .find(|app| app.id == source_id)
        else {
            return Err(err(
                "privacy_source_not_found",
                "Privacy app exclusion not found",
            ));
        };
        app.enabled = enabled;
        Ok(())
    })
}

#[tauri::command]
pub fn remove_privacy_excluded_app(
    source_id: String,
    app_handle: tauri::AppHandle,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    with_app_exclusion_mutation(app_handle, |settings| {
        let before = settings.privacy.excluded_apps.len();
        settings
            .privacy
            .excluded_apps
            .retain(|app| app.id != source_id);
        if settings.privacy.excluded_apps.len() == before {
            return Err(err(
                "privacy_source_not_found",
                "Privacy app exclusion not found",
            ));
        }
        Ok(())
    })
}
