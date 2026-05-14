use capture_metadata::{
    parse_website_rule, title_rule_is_valid, BrowserTitleRule, BrowserTitleRuleMatchType,
    ExcludedAppEntry, PrivacySettings, WebsiteRule,
};
use capture_types::{
    CaptureErrorResponse, PrivacyRedactionSourceDto, PrivacyRedactionSourceKind,
    PrivacyRedactionSourceResolutionDto, PrivacyRedactionSourceRestorePayload,
    PrivacyRedactionSourceStatus, RecordingSettings,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{Emitter, Manager};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

const PRIVACY_REDACTION_SOURCES_FILE_NAME: &str = "privacy-redaction-sources.json";
pub const PRIVACY_REDACTION_SOURCES_CHANGED_EVENT: &str = "privacy_redaction_sources_changed";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct PrivacyRedactionSourcesFile {
    schema_version: u32,
    #[serde(default)]
    sources: BTreeMap<String, PrivacyRedactionSourceRecord>,
}

impl Default for PrivacyRedactionSourcesFile {
    fn default() -> Self {
        Self {
            schema_version: 1,
            sources: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct PrivacyRedactionSourceRecord {
    source_kind: PrivacyRedactionSourceKind,
    label: Option<String>,
    detail: Option<String>,
    fingerprint: Option<String>,
    #[serde(default)]
    label_forgotten: bool,
    restore_payload: Option<PrivacyRedactionSourceRestorePayload>,
    created_at: String,
    updated_at: String,
    deleted_at: Option<String>,
}

impl PrivacyRedactionSourceRecord {
    fn status(&self) -> PrivacyRedactionSourceStatus {
        if self.label_forgotten {
            PrivacyRedactionSourceStatus::Forgotten
        } else if self.deleted_at.is_some() {
            PrivacyRedactionSourceStatus::Deleted
        } else {
            PrivacyRedactionSourceStatus::Active
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct PrivacyRedactionSourcesRuntime {
    file: PrivacyRedactionSourcesFile,
}

pub type PrivacyRedactionSourcesState = Mutex<PrivacyRedactionSourcesRuntime>;

fn err(code: &str, message: impl Into<String>) -> CaptureErrorResponse {
    CaptureErrorResponse {
        code: code.to_string(),
        message: message.into(),
    }
}

fn now_rfc3339() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .expect("UTC timestamp should format")
}

fn new_source_id(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut hash = Sha256::new();
    hash.update(prefix.as_bytes());
    hash.update(nanos.to_le_bytes());
    let hex = format!("{:x}", hash.finalize());
    format!(
        "{prefix}-{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

fn source_prefix(kind: PrivacyRedactionSourceKind) -> &'static str {
    match kind {
        PrivacyRedactionSourceKind::ExcludedApp => "excluded-app",
        PrivacyRedactionSourceKind::WebsiteRule => "website",
        PrivacyRedactionSourceKind::TitleRule => "title",
    }
}

fn history_file_path(app_handle: &tauri::AppHandle) -> PathBuf {
    if let Ok(config_dir) = app_handle.path().app_config_dir() {
        return config_dir.join(PRIVACY_REDACTION_SOURCES_FILE_NAME);
    }
    PathBuf::from(crate::native_capture::settings::default_save_directory())
        .join(PRIVACY_REDACTION_SOURCES_FILE_NAME)
}

fn fingerprint(parts: &[&str]) -> String {
    let mut hash = Sha256::new();
    for part in parts {
        hash.update(part.as_bytes());
        hash.update([0]);
    }
    format!("sha256:{:x}", hash.finalize())
}

fn app_fingerprint(bundle_id: &str) -> String {
    fingerprint(&[
        "excluded_app",
        &bundle_id.trim().to_ascii_lowercase(),
    ])
}

fn website_descriptor(rule: &WebsiteRule) -> String {
    let parsed;
    let rule = if rule.host.is_none() {
        parsed = parse_website_rule(rule.id.clone(), rule.enabled, &rule.pattern);
        &parsed
    } else {
        rule
    };
    let Some(host) = rule.host.as_deref() else {
        return rule.pattern.trim().to_string();
    };
    let mut label = if rule.include_subdomains {
        format!("*.{host}")
    } else {
        host.to_string()
    };
    if let Some(port) = rule.port {
        label.push(':');
        label.push_str(&port.to_string());
    }
    if let Some(path) = rule.path_prefix.as_deref() {
        label.push_str(path);
    }
    label
}

fn website_fingerprint(rule: &WebsiteRule) -> String {
    let parsed;
    let rule = if rule.host.is_none() {
        parsed = parse_website_rule(rule.id.clone(), rule.enabled, &rule.pattern);
        &parsed
    } else {
        rule
    };
    let Some(host) = rule.host.as_deref() else {
        return fingerprint(&["website_rule", "invalid", rule.pattern.trim()]);
    };
    fingerprint(&[
        "website_rule",
        host,
        if rule.include_subdomains {
            "subdomains"
        } else {
            "exact"
        },
        rule.path_prefix.as_deref().unwrap_or(""),
        &rule.port.map(|port| port.to_string()).unwrap_or_default(),
    ])
}

fn title_fingerprint(match_type: BrowserTitleRuleMatchType, pattern: &str) -> String {
    let normalized = match match_type {
        BrowserTitleRuleMatchType::Substring => pattern.trim().to_ascii_lowercase(),
        BrowserTitleRuleMatchType::Regex => pattern.trim().to_string(),
    };
    fingerprint(&[
        "title_rule",
        match match_type {
            BrowserTitleRuleMatchType::Substring => "substring",
            BrowserTitleRuleMatchType::Regex => "regex",
        },
        &normalized,
    ])
}

fn record_for_app(app: &ExcludedAppEntry, now: &str) -> PrivacyRedactionSourceRecord {
    PrivacyRedactionSourceRecord {
        source_kind: PrivacyRedactionSourceKind::ExcludedApp,
        label: Some(app.display_name.clone()),
        detail: Some(app.bundle_id.clone()),
        fingerprint: Some(app_fingerprint(&app.bundle_id)),
        label_forgotten: false,
        restore_payload: Some(PrivacyRedactionSourceRestorePayload::ExcludedApp {
            bundle_id: app.bundle_id.clone(),
            display_name: app.display_name.clone(),
            enabled: app.enabled,
        }),
        created_at: now.to_string(),
        updated_at: now.to_string(),
        deleted_at: None,
    }
}

fn record_for_website(rule: &WebsiteRule, now: &str) -> PrivacyRedactionSourceRecord {
    PrivacyRedactionSourceRecord {
        source_kind: PrivacyRedactionSourceKind::WebsiteRule,
        label: Some(website_descriptor(rule)),
        detail: None,
        fingerprint: Some(website_fingerprint(rule)),
        label_forgotten: false,
        restore_payload: Some(PrivacyRedactionSourceRestorePayload::WebsiteRule {
            pattern: rule.pattern.clone(),
            enabled: rule.enabled,
        }),
        created_at: now.to_string(),
        updated_at: now.to_string(),
        deleted_at: None,
    }
}

fn record_for_title(rule: &BrowserTitleRule, now: &str) -> PrivacyRedactionSourceRecord {
    PrivacyRedactionSourceRecord {
        source_kind: PrivacyRedactionSourceKind::TitleRule,
        label: Some(rule.pattern.clone()),
        detail: Some(match rule.match_type {
            BrowserTitleRuleMatchType::Substring => "Substring title rule".to_string(),
            BrowserTitleRuleMatchType::Regex => "Regex title rule".to_string(),
        }),
        fingerprint: Some(title_fingerprint(rule.match_type, &rule.pattern)),
        label_forgotten: false,
        restore_payload: Some(PrivacyRedactionSourceRestorePayload::TitleRule {
            match_type: rule.match_type,
            pattern: rule.pattern.clone(),
            enabled: rule.enabled,
        }),
        created_at: now.to_string(),
        updated_at: now.to_string(),
        deleted_at: None,
    }
}

fn source_to_dto(source_id: &str, record: &PrivacyRedactionSourceRecord) -> PrivacyRedactionSourceDto {
    let restore_enabled = record.restore_payload.as_ref().map(|payload| match payload {
        PrivacyRedactionSourceRestorePayload::ExcludedApp { enabled, .. }
        | PrivacyRedactionSourceRestorePayload::WebsiteRule { enabled, .. }
        | PrivacyRedactionSourceRestorePayload::TitleRule { enabled, .. } => *enabled,
    });
    PrivacyRedactionSourceDto {
        source_id: source_id.to_string(),
        source_kind: record.source_kind,
        status: record.status(),
        label: record.label.clone(),
        detail: record.detail.clone(),
        label_forgotten: record.label_forgotten,
        restorable: record.deleted_at.is_some()
            && !record.label_forgotten
            && record.restore_payload.is_some(),
        restore_enabled,
        created_at: record.created_at.clone(),
        updated_at: record.updated_at.clone(),
        deleted_at: record.deleted_at.clone(),
    }
}

fn source_to_resolution(
    source_id: &str,
    record: Option<&PrivacyRedactionSourceRecord>,
) -> PrivacyRedactionSourceResolutionDto {
    match record {
        Some(record) => PrivacyRedactionSourceResolutionDto {
            source_id: source_id.to_string(),
            source_kind: record.source_kind,
            status: record.status(),
            label: record.label.clone(),
            detail: record.detail.clone(),
        },
        None => PrivacyRedactionSourceResolutionDto {
            source_id: source_id.to_string(),
            source_kind: kind_from_source_id(source_id),
            status: PrivacyRedactionSourceStatus::Unknown,
            label: None,
            detail: None,
        },
    }
}

fn kind_from_source_id(source_id: &str) -> PrivacyRedactionSourceKind {
    if source_id.starts_with("website-") {
        PrivacyRedactionSourceKind::WebsiteRule
    } else if source_id.starts_with("title-") {
        PrivacyRedactionSourceKind::TitleRule
    } else {
        PrivacyRedactionSourceKind::ExcludedApp
    }
}

fn load_history_from_path(path: &Path) -> PrivacyRedactionSourcesFile {
    if !path.exists() {
        return PrivacyRedactionSourcesFile::default();
    }
    let Ok(raw) = std::fs::read_to_string(path) else {
        return PrivacyRedactionSourcesFile::default();
    };
    match serde_json::from_str::<PrivacyRedactionSourcesFile>(&raw) {
        Ok(file) if file.schema_version == 1 => file,
        _ => {
            let backup = path.with_extension(format!("json.corrupt-{}", now_rfc3339().replace(':', "-")));
            let _ = std::fs::rename(path, backup);
            PrivacyRedactionSourcesFile::default()
        }
    }
}

fn persist_history_to_path(
    path: &Path,
    file: &PrivacyRedactionSourcesFile,
) -> Result<(), CaptureErrorResponse> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| err("io_error", format!("Failed to create privacy history directory: {error}")))?;
    }
    let serialized = serde_json::to_string_pretty(file)
        .map_err(|error| err("serialization_error", format!("Failed to serialize privacy history: {error}")))?;
    std::fs::write(path, serialized)
        .map_err(|error| err("io_error", format!("Failed to persist privacy history: {error}")))
}

fn active_fingerprint_exists(
    history: &PrivacyRedactionSourcesFile,
    kind: PrivacyRedactionSourceKind,
    fingerprint: &str,
    except_source_id: Option<&str>,
) -> bool {
    history.sources.iter().any(|(source_id, record)| {
        Some(source_id.as_str()) != except_source_id
            && record.source_kind == kind
            && record.deleted_at.is_none()
            && !record.label_forgotten
            && record.fingerprint.as_deref() == Some(fingerprint)
    })
}

fn reconcile_privacy_sources(
    history: &mut PrivacyRedactionSourcesFile,
    privacy: &mut PrivacySettings,
) -> bool {
    let now = now_rfc3339();
    let mut changed = false;
    let mut active_ids = std::collections::BTreeSet::new();

    for app in &mut privacy.excluded_apps {
        let fp = app_fingerprint(&app.bundle_id);
        let valid_existing = history.sources.get(&app.id).is_some_and(|record| {
            record.source_kind == PrivacyRedactionSourceKind::ExcludedApp
                && !record.label_forgotten
                && record.fingerprint.as_deref() == Some(fp.as_str())
        });
        if !valid_existing {
            app.id = new_source_id(source_prefix(PrivacyRedactionSourceKind::ExcludedApp));
            changed = true;
        }
        let record = history
            .sources
            .entry(app.id.clone())
            .or_insert_with(|| record_for_app(app, &now));
        *record = PrivacyRedactionSourceRecord {
            created_at: record.created_at.clone(),
            ..record_for_app(app, &now)
        };
        active_ids.insert(app.id.clone());
    }

    for rule in &mut privacy.excluded_website_rules {
        let normalized = parse_website_rule(rule.id.clone(), rule.enabled, &rule.pattern);
        *rule = normalized;
        let fp = website_fingerprint(rule);
        let valid_existing = history.sources.get(&rule.id).is_some_and(|record| {
            record.source_kind == PrivacyRedactionSourceKind::WebsiteRule
                && !record.label_forgotten
                && record.fingerprint.as_deref() == Some(fp.as_str())
        });
        if !valid_existing {
            rule.id = new_source_id(source_prefix(PrivacyRedactionSourceKind::WebsiteRule));
            changed = true;
        }
        let record = history
            .sources
            .entry(rule.id.clone())
            .or_insert_with(|| record_for_website(rule, &now));
        *record = PrivacyRedactionSourceRecord {
            created_at: record.created_at.clone(),
            ..record_for_website(rule, &now)
        };
        active_ids.insert(rule.id.clone());
    }

    for rule in &mut privacy.browser_title_rules {
        let fp = title_fingerprint(rule.match_type, &rule.pattern);
        let valid_existing = history.sources.get(&rule.id).is_some_and(|record| {
            record.source_kind == PrivacyRedactionSourceKind::TitleRule
                && !record.label_forgotten
                && record.fingerprint.as_deref() == Some(fp.as_str())
        });
        if !valid_existing {
            rule.id = new_source_id(source_prefix(PrivacyRedactionSourceKind::TitleRule));
            changed = true;
        }
        let record = history
            .sources
            .entry(rule.id.clone())
            .or_insert_with(|| record_for_title(rule, &now));
        *record = PrivacyRedactionSourceRecord {
            created_at: record.created_at.clone(),
            ..record_for_title(rule, &now)
        };
        active_ids.insert(rule.id.clone());
    }

    for (source_id, record) in &mut history.sources {
        if record.deleted_at.is_none()
            && !record.label_forgotten
            && !active_ids.contains(source_id)
        {
            record.deleted_at = Some(now.clone());
            record.updated_at = now.clone();
            changed = true;
        }
    }

    changed
}

pub fn initialize(app_handle: &tauri::AppHandle) {
    let settings_state = app_handle.state::<crate::native_capture::RecordingSettingsState>();
    let history_state = app_handle.state::<PrivacyRedactionSourcesState>();
    let history_path = history_file_path(app_handle);

    let mut settings_runtime = settings_state
        .lock()
        .expect("recording settings state poisoned");
    let mut history = load_history_from_path(&history_path);
    let changed = reconcile_privacy_sources(&mut history, &mut settings_runtime.settings.privacy);

    if changed {
        if let Err(error) = persist_history_to_path(&history_path, &history) {
            eprintln!("failed to persist privacy redaction history during startup: {error:?}");
        }
        if let Err(error) = crate::native_capture::settings::persist_recording_settings(
            app_handle,
            &settings_runtime.settings,
        ) {
            eprintln!("failed to persist reconciled recording settings during startup: {error:?}");
        }
    }

    history_state
        .lock()
        .expect("privacy redaction source state poisoned")
        .file = history;
}

fn persist_privacy_mutation(
    app_handle: &tauri::AppHandle,
    settings: RecordingSettings,
    history: PrivacyRedactionSourcesFile,
    clear_website_holds: bool,
) -> Result<RecordingSettings, CaptureErrorResponse> {
    let history_path = history_file_path(app_handle);
    persist_history_to_path(&history_path, &history)?;
    crate::native_capture::settings::persist_recording_settings(app_handle, &settings)?;

    app_handle
        .state::<crate::native_capture::RecordingSettingsState>()
        .lock()
        .expect("recording settings state poisoned")
        .settings = settings.clone();
    app_handle
        .state::<PrivacyRedactionSourcesState>()
        .lock()
        .expect("privacy redaction source state poisoned")
        .file = history;

    crate::native_capture::emit_recording_settings_changed(app_handle, &settings);
    let _ = app_handle.emit(PRIVACY_REDACTION_SOURCES_CHANGED_EVENT, ());
    if clear_website_holds {
        if let Some(metadata_state) =
            app_handle.try_state::<crate::native_capture::CaptureMetadataState>()
        {
            crate::native_capture::metadata::clear_website_privacy_state(metadata_state.inner());
        }
    }
    crate::status_bar::refresh(app_handle);
    Ok(settings)
}

fn with_privacy_mutation<F>(
    app_handle: tauri::AppHandle,
    clear_website_holds: bool,
    mutate: F,
) -> Result<RecordingSettings, CaptureErrorResponse>
where
    F: FnOnce(&mut RecordingSettings, &mut PrivacyRedactionSourcesFile) -> Result<(), CaptureErrorResponse>,
{
    let mut settings = app_handle
        .state::<crate::native_capture::RecordingSettingsState>()
        .lock()
        .expect("recording settings state poisoned")
        .settings
        .clone();
    let mut history = app_handle
        .state::<PrivacyRedactionSourcesState>()
        .lock()
        .expect("privacy redaction source state poisoned")
        .file
        .clone();
    mutate(&mut settings, &mut history)?;
    settings.privacy = crate::native_capture::settings::validate_privacy_settings(settings.privacy)?;
    persist_privacy_mutation(&app_handle, settings, history, clear_website_holds)
}

#[tauri::command]
pub fn add_privacy_excluded_app(
    bundle_id: String,
    display_name: String,
    app_handle: tauri::AppHandle,
) -> Result<RecordingSettings, CaptureErrorResponse> {
    with_privacy_mutation(app_handle, false, |settings, history| {
        let bundle_id = bundle_id.trim().to_string();
        let display_name = display_name.trim().to_string();
        if bundle_id.is_empty() || display_name.is_empty() {
            return Err(err("invalid_privacy_rule", "App bundle and display name are required"));
        }
        if settings
            .privacy
            .excluded_apps
            .iter()
            .any(|app| app.bundle_id == bundle_id)
        {
            return Ok(());
        }
        let source_id = new_source_id("excluded-app");
        let app = ExcludedAppEntry {
            id: source_id.clone(),
            enabled: true,
            bundle_id,
            display_name,
        };
        history
            .sources
            .insert(source_id, record_for_app(&app, &now_rfc3339()));
        settings.privacy.excluded_apps.push(app);
        Ok(())
    })
}

#[tauri::command]
pub fn add_privacy_website_rule(
    pattern: String,
    app_handle: tauri::AppHandle,
) -> Result<RecordingSettings, CaptureErrorResponse> {
    with_privacy_mutation(app_handle, true, |settings, history| {
        let rule = parse_website_rule(new_source_id("website"), true, &pattern);
        if rule.host.is_none() {
            return Err(err("invalid_privacy_rule", "Website rule must include a valid host"));
        }
        let fp = website_fingerprint(&rule);
        if active_fingerprint_exists(history, PrivacyRedactionSourceKind::WebsiteRule, &fp, None) {
            return Err(err("duplicate_privacy_rule", "An equivalent website rule already exists"));
        }
        history
            .sources
            .insert(rule.id.clone(), record_for_website(&rule, &now_rfc3339()));
        settings.privacy.excluded_website_rules.push(rule);
        Ok(())
    })
}

#[tauri::command]
pub fn update_privacy_website_rule(
    source_id: String,
    pattern: String,
    app_handle: tauri::AppHandle,
) -> Result<RecordingSettings, CaptureErrorResponse> {
    with_privacy_mutation(app_handle, true, |settings, history| {
        let index = settings
            .privacy
            .excluded_website_rules
            .iter()
            .position(|rule| rule.id == source_id)
            .ok_or_else(|| err("privacy_source_not_found", "Privacy source not found"))?;
        let enabled = settings.privacy.excluded_website_rules[index].enabled;
        let new_rule = parse_website_rule(new_source_id("website"), enabled, &pattern);
        if new_rule.host.is_none() {
            return Err(err("invalid_privacy_rule", "Website rule must include a valid host"));
        }
        let fp = website_fingerprint(&new_rule);
        if active_fingerprint_exists(history, PrivacyRedactionSourceKind::WebsiteRule, &fp, Some(&source_id)) {
            return Err(err("duplicate_privacy_rule", "An equivalent website rule already exists"));
        }
        if let Some(old) = history.sources.get_mut(&source_id) {
            old.deleted_at = Some(now_rfc3339());
            old.updated_at = now_rfc3339();
        }
        history
            .sources
            .insert(new_rule.id.clone(), record_for_website(&new_rule, &now_rfc3339()));
        settings.privacy.excluded_website_rules[index] = new_rule;
        Ok(())
    })
}

#[tauri::command]
pub fn add_privacy_title_rule(
    match_type: BrowserTitleRuleMatchType,
    pattern: String,
    app_handle: tauri::AppHandle,
) -> Result<RecordingSettings, CaptureErrorResponse> {
    with_privacy_mutation(app_handle, false, |settings, history| {
        let rule = BrowserTitleRule {
            id: new_source_id("title"),
            enabled: true,
            match_type,
            pattern: pattern.trim().to_string(),
        };
        if rule.pattern.is_empty() || (rule.match_type == BrowserTitleRuleMatchType::Regex && !title_rule_is_valid(&rule)) {
            return Err(err("invalid_privacy_rule", "Title rule is invalid"));
        }
        let fp = title_fingerprint(rule.match_type, &rule.pattern);
        if active_fingerprint_exists(history, PrivacyRedactionSourceKind::TitleRule, &fp, None) {
            return Err(err("duplicate_privacy_rule", "An equivalent title rule already exists"));
        }
        history
            .sources
            .insert(rule.id.clone(), record_for_title(&rule, &now_rfc3339()));
        settings.privacy.browser_title_rules.push(rule);
        Ok(())
    })
}

#[tauri::command]
pub fn update_privacy_title_rule(
    source_id: String,
    match_type: BrowserTitleRuleMatchType,
    pattern: String,
    app_handle: tauri::AppHandle,
) -> Result<RecordingSettings, CaptureErrorResponse> {
    with_privacy_mutation(app_handle, false, |settings, history| {
        let index = settings
            .privacy
            .browser_title_rules
            .iter()
            .position(|rule| rule.id == source_id)
            .ok_or_else(|| err("privacy_source_not_found", "Privacy source not found"))?;
        let enabled = settings.privacy.browser_title_rules[index].enabled;
        let rule = BrowserTitleRule {
            id: new_source_id("title"),
            enabled,
            match_type,
            pattern: pattern.trim().to_string(),
        };
        if rule.pattern.is_empty() || (rule.match_type == BrowserTitleRuleMatchType::Regex && !title_rule_is_valid(&rule)) {
            return Err(err("invalid_privacy_rule", "Title rule is invalid"));
        }
        let fp = title_fingerprint(rule.match_type, &rule.pattern);
        if active_fingerprint_exists(history, PrivacyRedactionSourceKind::TitleRule, &fp, Some(&source_id)) {
            return Err(err("duplicate_privacy_rule", "An equivalent title rule already exists"));
        }
        if let Some(old) = history.sources.get_mut(&source_id) {
            old.deleted_at = Some(now_rfc3339());
            old.updated_at = now_rfc3339();
        }
        history
            .sources
            .insert(rule.id.clone(), record_for_title(&rule, &now_rfc3339()));
        settings.privacy.browser_title_rules[index] = rule;
        Ok(())
    })
}

#[tauri::command]
pub fn set_privacy_source_enabled(
    source_id: String,
    enabled: bool,
    app_handle: tauri::AppHandle,
) -> Result<RecordingSettings, CaptureErrorResponse> {
    with_privacy_mutation(app_handle, source_id.starts_with("website-"), |settings, history| {
        let mut found = false;
        for app in &mut settings.privacy.excluded_apps {
            if app.id == source_id {
                app.enabled = enabled;
                if let Some(record) = history.sources.get_mut(&source_id) {
                    record.restore_payload = Some(PrivacyRedactionSourceRestorePayload::ExcludedApp {
                        bundle_id: app.bundle_id.clone(),
                        display_name: app.display_name.clone(),
                        enabled,
                    });
                    record.updated_at = now_rfc3339();
                }
                found = true;
            }
        }
        for rule in &mut settings.privacy.excluded_website_rules {
            if rule.id == source_id {
                rule.enabled = enabled;
                if let Some(record) = history.sources.get_mut(&source_id) {
                    record.restore_payload = Some(PrivacyRedactionSourceRestorePayload::WebsiteRule {
                        pattern: rule.pattern.clone(),
                        enabled,
                    });
                    record.updated_at = now_rfc3339();
                }
                found = true;
            }
        }
        for rule in &mut settings.privacy.browser_title_rules {
            if rule.id == source_id {
                rule.enabled = enabled;
                if let Some(record) = history.sources.get_mut(&source_id) {
                    record.restore_payload = Some(PrivacyRedactionSourceRestorePayload::TitleRule {
                        match_type: rule.match_type,
                        pattern: rule.pattern.clone(),
                        enabled,
                    });
                    record.updated_at = now_rfc3339();
                }
                found = true;
            }
        }
        if found { Ok(()) } else { Err(err("privacy_source_not_found", "Privacy source not found")) }
    })
}

#[tauri::command]
pub fn remove_privacy_source(
    source_id: String,
    app_handle: tauri::AppHandle,
) -> Result<RecordingSettings, CaptureErrorResponse> {
    with_privacy_mutation(app_handle, source_id.starts_with("website-"), |settings, history| {
        let before = settings.privacy.excluded_apps.len()
            + settings.privacy.excluded_website_rules.len()
            + settings.privacy.browser_title_rules.len();
        settings.privacy.excluded_apps.retain(|rule| rule.id != source_id);
        settings.privacy.excluded_website_rules.retain(|rule| rule.id != source_id);
        settings.privacy.browser_title_rules.retain(|rule| rule.id != source_id);
        let after = settings.privacy.excluded_apps.len()
            + settings.privacy.excluded_website_rules.len()
            + settings.privacy.browser_title_rules.len();
        if before == after {
            return Err(err("privacy_source_not_found", "Privacy source not found"));
        }
        let record = history
            .sources
            .get_mut(&source_id)
            .ok_or_else(|| err("privacy_source_not_found", "Privacy source not found"))?;
        record.deleted_at = Some(now_rfc3339());
        record.updated_at = now_rfc3339();
        Ok(())
    })
}

#[tauri::command]
pub fn restore_privacy_redaction_source(
    source_id: String,
    app_handle: tauri::AppHandle,
) -> Result<RecordingSettings, CaptureErrorResponse> {
    with_privacy_mutation(app_handle, source_id.starts_with("website-"), |settings, history| {
        let record = history
            .sources
            .get(&source_id)
            .cloned()
            .ok_or_else(|| err("privacy_source_not_found", "Privacy source not found"))?;
        if record.deleted_at.is_none() {
            return Err(err("privacy_source_already_active", "Privacy source is already active"));
        }
        if record.label_forgotten {
            return Err(err("privacy_source_not_restorable", "Privacy source label was forgotten"));
        }
        let payload = record
            .restore_payload
            .clone()
            .ok_or_else(|| err("privacy_source_not_restorable", "Privacy source is not restorable"))?;
        match payload {
            PrivacyRedactionSourceRestorePayload::ExcludedApp { bundle_id, display_name, enabled } => {
                let fp = app_fingerprint(&bundle_id);
                if active_fingerprint_exists(history, PrivacyRedactionSourceKind::ExcludedApp, &fp, Some(&source_id)) {
                    return Err(err("duplicate_privacy_rule", "An equivalent app rule already exists"));
                }
                settings.privacy.excluded_apps.push(ExcludedAppEntry { id: source_id.clone(), enabled, bundle_id, display_name });
            }
            PrivacyRedactionSourceRestorePayload::WebsiteRule { pattern, enabled } => {
                let rule = parse_website_rule(source_id.clone(), enabled, &pattern);
                let fp = website_fingerprint(&rule);
                if active_fingerprint_exists(history, PrivacyRedactionSourceKind::WebsiteRule, &fp, Some(&source_id)) {
                    return Err(err("duplicate_privacy_rule", "An equivalent website rule already exists"));
                }
                settings.privacy.excluded_website_rules.push(rule);
            }
            PrivacyRedactionSourceRestorePayload::TitleRule { match_type, pattern, enabled } => {
                let fp = title_fingerprint(match_type, &pattern);
                if active_fingerprint_exists(history, PrivacyRedactionSourceKind::TitleRule, &fp, Some(&source_id)) {
                    return Err(err("duplicate_privacy_rule", "An equivalent title rule already exists"));
                }
                settings.privacy.browser_title_rules.push(BrowserTitleRule { id: source_id.clone(), enabled, match_type, pattern });
            }
        }
        let record = history.sources.get_mut(&source_id).expect("record exists");
        record.deleted_at = None;
        record.updated_at = now_rfc3339();
        Ok(())
    })
}

#[tauri::command]
pub fn set_private_browser_exclusion_enabled(
    enabled: bool,
    app_handle: tauri::AppHandle,
) -> Result<RecordingSettings, CaptureErrorResponse> {
    with_privacy_mutation(app_handle, false, |settings, _history| {
        settings.privacy.private_browser_exclusion_enabled = enabled;
        Ok(())
    })
}

#[tauri::command]
pub fn forget_privacy_redaction_source_label(
    source_id: String,
    app_handle: tauri::AppHandle,
) -> Result<Vec<PrivacyRedactionSourceDto>, CaptureErrorResponse> {
    let history_path = history_file_path(&app_handle);
    let mut changed = false;
    let sources = {
        let state = app_handle.state::<PrivacyRedactionSourcesState>();
        let mut history_state = state
            .lock()
            .expect("privacy redaction source state poisoned");
        let original_history = history_state.file.clone();
        let record = history_state
            .file
            .sources
            .get_mut(&source_id)
            .ok_or_else(|| err("privacy_source_not_found", "Privacy source not found"))?;
        if record.deleted_at.is_none() {
            return Err(err(
                "cannot_forget_active_source",
                "Active privacy sources cannot be forgotten",
            ));
        }
        if !record.label_forgotten {
            record.label = None;
            record.detail = None;
            record.fingerprint = None;
            record.restore_payload = None;
            record.label_forgotten = true;
            record.updated_at = now_rfc3339();
            if let Err(error) = persist_history_to_path(&history_path, &history_state.file) {
                history_state.file = original_history;
                return Err(error);
            }
            changed = true;
        }
        manageable_sources(&history_state.file)
    };
    if changed {
        let _ = app_handle.emit(PRIVACY_REDACTION_SOURCES_CHANGED_EVENT, ());
    }
    Ok(sources)
}

fn manageable_sources(history: &PrivacyRedactionSourcesFile) -> Vec<PrivacyRedactionSourceDto> {
    history
        .sources
        .iter()
        .filter(|(_, record)| record.deleted_at.is_some() && !record.label_forgotten)
        .map(|(source_id, record)| source_to_dto(source_id, record))
        .collect()
}

#[tauri::command]
pub fn list_manageable_privacy_redaction_sources(
    state: tauri::State<'_, PrivacyRedactionSourcesState>,
) -> Vec<PrivacyRedactionSourceDto> {
    manageable_sources(
        &state
            .lock()
            .expect("privacy redaction source state poisoned")
            .file,
    )
}

#[tauri::command]
pub fn resolve_privacy_redaction_sources(
    source_ids: Vec<String>,
    state: tauri::State<'_, PrivacyRedactionSourcesState>,
) -> Vec<PrivacyRedactionSourceResolutionDto> {
    let history = state
        .lock()
        .expect("privacy redaction source state poisoned");
    source_ids
        .into_iter()
        .map(|source_id| source_to_resolution(&source_id, history.file.sources.get(&source_id)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "privacy-redaction-sources-{label}-{}",
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("time should be valid")
                    .as_nanos()
            ));
            std::fs::create_dir_all(&path).expect("test dir should be created");
            Self { path }
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn missing_history_file_first_run_returns_empty_history() {
        let dir = TestDir::new("missing");
        let history = load_history_from_path(&dir.path.join("privacy-redaction-sources.json"));
        assert_eq!(history.schema_version, 1);
        assert!(history.sources.is_empty());
    }

    #[test]
    fn corrupt_history_file_is_backed_up_and_recreated() {
        let dir = TestDir::new("corrupt");
        let path = dir.path.join("privacy-redaction-sources.json");
        std::fs::write(&path, "{not-json").expect("corrupt file should be written");

        let history = load_history_from_path(&path);

        assert!(history.sources.is_empty());
        assert!(!path.exists());
        let backups = std::fs::read_dir(&dir.path)
            .expect("dir should list")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains("corrupt"))
            .count();
        assert_eq!(backups, 1);
    }

    #[test]
    fn resolver_returns_active_deleted_forgotten_and_unknown_entries() {
        let now = now_rfc3339();
        let mut history = PrivacyRedactionSourcesFile::default();
        history.sources.insert(
            "website-active".to_string(),
            PrivacyRedactionSourceRecord {
                source_kind: PrivacyRedactionSourceKind::WebsiteRule,
                label: Some("*.bank.com".to_string()),
                detail: None,
                fingerprint: Some("sha256:a".to_string()),
                label_forgotten: false,
                restore_payload: None,
                created_at: now.clone(),
                updated_at: now.clone(),
                deleted_at: None,
            },
        );
        history.sources.insert(
            "title-deleted".to_string(),
            PrivacyRedactionSourceRecord {
                source_kind: PrivacyRedactionSourceKind::TitleRule,
                label: Some("secret".to_string()),
                detail: Some("Substring title rule".to_string()),
                fingerprint: Some("sha256:b".to_string()),
                label_forgotten: false,
                restore_payload: None,
                created_at: now.clone(),
                updated_at: now.clone(),
                deleted_at: Some(now.clone()),
            },
        );
        history.sources.insert(
            "excluded-app-forgotten".to_string(),
            PrivacyRedactionSourceRecord {
                source_kind: PrivacyRedactionSourceKind::ExcludedApp,
                label: None,
                detail: None,
                fingerprint: None,
                label_forgotten: true,
                restore_payload: None,
                created_at: now.clone(),
                updated_at: now.clone(),
                deleted_at: Some(now),
            },
        );

        assert_eq!(
            source_to_resolution("website-active", history.sources.get("website-active")).status,
            PrivacyRedactionSourceStatus::Active
        );
        assert_eq!(
            source_to_resolution("title-deleted", history.sources.get("title-deleted")).status,
            PrivacyRedactionSourceStatus::Deleted
        );
        assert_eq!(
            source_to_resolution(
                "excluded-app-forgotten",
                history.sources.get("excluded-app-forgotten")
            )
            .status,
            PrivacyRedactionSourceStatus::Forgotten
        );
        assert_eq!(
            source_to_resolution("website-missing", None).status,
            PrivacyRedactionSourceStatus::Unknown
        );
    }

    #[test]
    fn website_fingerprint_distinguishes_unparseable_patterns() {
        let first = parse_website_rule("website-a", true, "not a url");
        let second = parse_website_rule("website-b", true, "also invalid!");

        assert!(first.host.is_none());
        assert!(second.host.is_none());
        assert_ne!(website_fingerprint(&first), website_fingerprint(&second));
    }

    #[test]
    fn website_fingerprint_keeps_valid_canonical_equivalence() {
        let shorthand = parse_website_rule("website-a", true, "example.com/private");
        let explicit = parse_website_rule("website-b", true, "https://example.com/private");

        assert_eq!(shorthand.host.as_deref(), Some("example.com"));
        assert_eq!(explicit.host.as_deref(), Some("example.com"));
        assert_eq!(
            website_fingerprint(&shorthand),
            website_fingerprint(&explicit)
        );
    }
}
