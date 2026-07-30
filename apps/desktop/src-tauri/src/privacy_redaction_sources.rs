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

    // Excluding an app before installing it: the rule is stored under its name
    // alone, and the first sighting of the installed app fills the bundle id
    // into that same row — same source id, same enabled state, no second row.
    #[test]
    fn pending_rule_resolves_in_place_on_first_sighting_of_a_matching_name() {
        let mut settings = crate::native_capture::settings::default_recording_settings();

        upsert_privacy_excluded_app(&mut settings, "", " Figma ", false)
            .expect("a name-only rule is allowed");
        assert_eq!(settings.privacy.excluded_apps.len(), 1);
        assert!(settings.privacy.excluded_apps[0].bundle_id.is_empty());
        assert_eq!(settings.privacy.excluded_apps[0].display_name, "Figma");
        let pending_id = settings.privacy.excluded_apps[0].id.clone();

        // The user strikes the pending rule before the app ever appears.
        settings.privacy.excluded_apps[0].enabled = false;

        // Same name typed again while still pending must not add a second row.
        upsert_privacy_excluded_app(&mut settings, "", "figma", false)
            .expect("a duplicate name-only rule is a no-op");
        assert_eq!(settings.privacy.excluded_apps.len(), 1);

        upsert_privacy_excluded_app(&mut settings, "com.figma.Desktop", "Figma", false)
            .expect("first sighting resolves the rule");

        assert_eq!(settings.privacy.excluded_apps.len(), 1);
        assert_eq!(settings.privacy.excluded_apps[0].id, pending_id);
        assert_eq!(
            settings.privacy.excluded_apps[0].bundle_id,
            "com.figma.Desktop"
        );
        assert!(
            !settings.privacy.excluded_apps[0].enabled,
            "resolution must not silently re-enable a struck rule"
        );
    }

    #[test]
    fn pending_rule_requires_a_display_name() {
        let mut settings = crate::native_capture::settings::default_recording_settings();

        let error = upsert_privacy_excluded_app(&mut settings, "", "  ", false)
            .expect_err("a rule with neither a bundle id nor a name is not a rule");

        assert_eq!(error.code, "invalid_privacy_rule");
        assert!(settings.privacy.excluded_apps.is_empty());
    }

    #[test]
    fn resolved_rules_still_match_on_bundle_id_not_display_name() {
        let mut settings = crate::native_capture::settings::default_recording_settings();

        upsert_privacy_excluded_app(&mut settings, "com.example.One", "Notes", false)
            .expect("first rule");
        upsert_privacy_excluded_app(&mut settings, "com.example.Two", "Notes", false)
            .expect("a different app that happens to share a name is its own rule");

        assert_eq!(settings.privacy.excluded_apps.len(), 2);
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

/// Adds an app-exclusion rule, or updates the one that already covers the app.
///
/// An empty `bundle_id` is a rule for an app that is not installed yet: it is
/// stored under its typed display name and excludes nothing until it resolves
/// (`evaluate_privacy` skips an empty bundle id, so it reaches neither the
/// screen filter nor the system-audio tap). The first sighting of an installed
/// app with a matching display name — an add from the app list, or the frontend's
/// app-list refresh — fills the bundle id into that same row, keeping its source
/// id and its enabled state.
fn upsert_privacy_excluded_app(
    settings: &mut RecordingSettings,
    bundle_id: &str,
    display_name: &str,
    enable_existing: bool,
) -> Result<(), CaptureErrorResponse> {
    let bundle_id = crate::native_capture::settings::canonicalize_app_bundle_id(bundle_id);
    let display_name = display_name.trim().to_string();
    if display_name.is_empty() {
        return Err(err("invalid_privacy_rule", "App display name is required"));
    }
    let same_name = |app: &capture_metadata::ExcludedAppEntry| {
        crate::native_capture::settings::canonicalize_app_display_name(&app.display_name)
            == crate::native_capture::settings::canonicalize_app_display_name(&display_name)
    };

    // A name-only rule matches on the name; a real bundle id matches on the id
    // first, then resolves a pending rule carrying the same name.
    let existing = settings.privacy.excluded_apps.iter_mut().find(|app| {
        if bundle_id.is_empty() {
            same_name(app)
        } else {
            crate::native_capture::settings::canonicalize_app_bundle_id(&app.bundle_id) == bundle_id
                || (app.bundle_id.trim().is_empty() && same_name(app))
        }
    });
    if let Some(existing) = existing {
        if existing.bundle_id.trim().is_empty() && !bundle_id.is_empty() {
            existing.bundle_id = bundle_id;
        }
        if enable_existing {
            existing.enabled = true;
        }
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
}

#[tauri::command]
pub fn add_privacy_excluded_app(
    bundle_id: String,
    display_name: String,
    app_handle: tauri::AppHandle,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    with_app_exclusion_mutation(app_handle, |settings| {
        upsert_privacy_excluded_app(settings, &bundle_id, &display_name, false)
    })
}

pub(crate) fn add_or_enable_privacy_excluded_app_from_app_handle(
    app_handle: tauri::AppHandle,
    bundle_id: String,
    display_name: String,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    with_app_exclusion_mutation(app_handle, |settings| {
        upsert_privacy_excluded_app(settings, &bundle_id, &display_name, true)
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

/// Whether the privacy excluded-apps list also filters the system-audio tap
/// (Settings → Privacy → "Filter system audio"). Mnema's own-process exclusion
/// is never toggleable. Routed through the same privacy-domain mutation seam as
/// the excluded-apps edits, so a mid-recording flip reaches the live tap through
/// the same refresh path a privacy-list edit takes.
#[tauri::command]
pub fn set_privacy_filter_system_audio(
    enabled: bool,
    app_handle: tauri::AppHandle,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    with_app_exclusion_mutation(app_handle, |settings| {
        settings.privacy.filter_system_audio = enabled;
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
