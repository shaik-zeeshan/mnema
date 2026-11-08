//! What Settings shows besides its drafts: one answer per loader command. The
//! typed wire structs (`RecordingSettings`, `LicenseStatus`, the mic state) are
//! used as is; the rest stay JSON until their DTOs move into a crate, read
//! through [`Json`]'s small accessors.

use std::collections::{HashMap, HashSet};

use capture_types::{LicenseStatus, MicrophoneControllerState, RecordingSettings};
use serde_json::Value;

#[derive(Clone, Debug, Default)]
pub struct SettingsData {
    /// `get_recording_settings`; `None` = still loading (skeleton rows).
    pub settings: Option<RecordingSettings>,
    pub load_error: Option<String>,
    /// `get_keyboard_bindings_settings` (`KeyboardBindingsSettings` JSON).
    pub keyboard: Option<Value>,
    /// `get_microphone_controller_state`.
    pub mic: Option<MicrophoneControllerState>,
    /// `get_capture_permissions().permissions`.
    pub permissions: Option<Value>,
    /// `get_capture_support`.
    pub capture_support: Option<Value>,
    pub license: Option<LicenseStatus>,
    pub storage_usage: Option<Value>,
    pub storage_location: Option<String>,
    pub ai_status: Option<Value>,
    /// `ask_ai_availability`.
    pub ask_ai: Option<Value>,
    /// Provider instance ids with a key in the vault, and each key's last four.
    pub keys_saved: HashSet<String>,
    pub key_endings: HashMap<String, String>,
    /// MCP connector ids with a secret in the vault.
    pub mcp_secrets: HashSet<String>,
    pub user_context: Option<Value>,
    pub ocr_models: Option<Value>,
    pub transcription_models: Option<Value>,
    pub speaker_models: Option<Value>,
    pub semantic_models: Option<Value>,
    /// `get_account_owner_person_id` → that person's name (None = no voiceprint).
    pub voice_owner: Option<String>,
    pub cli_status: Option<Value>,
    pub grants: Vec<Value>,
    pub history: Vec<Value>,
    pub update: Option<Value>,
    pub log_status: Option<Value>,
    pub notices: Option<Value>,
}

/// `v.at(&["app", "version"])`-style reads over loader JSON.
pub trait Json {
    fn at(&self, path: &[&str]) -> &Value;
    fn str_at(&self, path: &[&str]) -> Option<&str> {
        self.at(path).as_str()
    }
    fn bool_at(&self, path: &[&str]) -> bool {
        self.at(path).as_bool().unwrap_or(false)
    }
    fn num_at(&self, path: &[&str]) -> Option<f64> {
        self.at(path).as_f64()
    }
}

impl Json for Value {
    fn at(&self, path: &[&str]) -> &Value {
        path.iter().fold(self, |v, k| &v[*k])
    }
}

/// One model row's facts (the OCR / transcription / speaker status DTOs).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModelView {
    pub provider: String,
    pub model_id: Option<String>,
    pub display_name: String,
    pub status: String,
    pub available: bool,
    pub os_managed: bool,
    pub missing_files: usize,
    pub download_bytes: Option<u64>,
    pub failure: Option<String>,
    pub runtime_message: Option<String>,
    pub install_path: Option<String>,
}

impl ModelView {
    pub fn from_json(v: &Value) -> Self {
        Self {
            provider: v.str_at(&["provider"]).unwrap_or_default().into(),
            model_id: v.str_at(&["modelId"]).map(Into::into),
            display_name: v.str_at(&["displayName"]).unwrap_or_default().into(),
            status: v.str_at(&["status"]).unwrap_or_default().into(),
            available: v.bool_at(&["available"]),
            os_managed: v.str_at(&["management"]) == Some("os_managed"),
            missing_files: v.at(&["missingFiles"]).as_array().map_or(0, Vec::len),
            download_bytes: v.at(&["download", "byteSize"]).as_u64(),
            failure: v.str_at(&["failureMessage"]).map(Into::into),
            runtime_message: v.str_at(&["runtimeMessage"]).map(Into::into),
            install_path: v.str_at(&["installPath"]).map(Into::into),
        }
    }
}

/// Every model of a `{ providers: [{ provider, displayName, models }] }` status.
pub fn provider_models(status: Option<&Value>) -> Vec<(String, String, Vec<ModelView>)> {
    let Some(providers) = status.and_then(|s| s.at(&["providers"]).as_array()) else {
        return Vec::new();
    };
    providers
        .iter()
        .map(|p| {
            let models = p
                .at(&["models"])
                .as_array()
                .map_or(Vec::new(), |m| m.iter().map(ModelView::from_json).collect());
            (
                p.str_at(&["provider"]).unwrap_or_default().to_string(),
                p.str_at(&["displayName"]).unwrap_or_default().to_string(),
                models,
            )
        })
        .collect()
}

/// The model a provider + id selects (the provider's first when the id is unset).
pub fn selected_model(
    status: Option<&Value>,
    provider: &str,
    model_id: Option<&str>,
) -> Option<ModelView> {
    let (_, _, models) = provider_models(status)
        .into_iter()
        .find(|(p, _, _)| p == provider)?;
    models
        .iter()
        .find(|m| model_id.is_some() && m.model_id.as_deref() == model_id)
        .or_else(|| models.first())
        .cloned()
}

/// `formatBytes`: decimal units, one decimal past bytes, "unknown size" for 0.
pub fn format_bytes(value: f64) -> String {
    if !value.is_finite() || value <= 0. {
        return "unknown size".into();
    }
    let units = ["B", "KB", "MB", "GB", "TB", "PB"];
    let (mut size, mut unit) = (value, 0);
    while size >= 1000. && unit < units.len() - 1 {
        size /= 1000.;
        unit += 1;
    }
    if unit == 0 {
        format!("{size:.0} {}", units[unit])
    } else {
        format!("{size:.1} {}", units[unit])
    }
}

/// `formatMeasuredBytes`: a measured zero reads "0 B", not "unknown size".
pub fn format_measured(value: f64) -> String {
    if value > 0. {
        format_bytes(value)
    } else {
        "0 B".into()
    }
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// Unix ms in `now`'s offset (the app clock's zone).
fn at(ms: i64, now: time::OffsetDateTime) -> Option<time::OffsetDateTime> {
    time::OffsetDateTime::from_unix_timestamp_nanos(ms as i128 * 1_000_000)
        .ok()
        .map(|t| t.to_offset(now.offset()))
}

/// "Nov 4, 2:02 PM" (`toLocaleString` month short, day, hour, minute; 12-hour).
pub fn short_datetime(ms: i64, now: time::OffsetDateTime) -> String {
    let Some(t) = at(ms, now) else {
        return String::new();
    };
    let h = t.hour() % 12;
    let h = if h == 0 { 12 } else { h };
    let ampm = if t.hour() < 12 { "AM" } else { "PM" };
    format!(
        "{} {}, {h}:{:02} {ampm}",
        MONTHS[t.month() as usize - 1],
        t.day(),
        t.minute()
    )
}

/// "Nov 4" (`month: short, day: numeric`).
pub fn short_day(ms: i64, now: time::OffsetDateTime) -> String {
    at(ms, now).map_or_else(String::new, |t| {
        format!("{} {}", MONTHS[t.month() as usize - 1], t.day())
    })
}

/// `Intl.RelativeTimeFormat(numeric: "auto")` in minutes / hours / days.
pub fn relative(ms: i64, now_ms: i64) -> String {
    let diff = ms - now_ms;
    let abs = diff.abs();
    let (n, unit) = if abs < 3_600_000 {
        ((diff as f64 / 60_000.).round() as i64, "minute")
    } else if abs < 86_400_000 {
        ((diff as f64 / 3_600_000.).round() as i64, "hour")
    } else {
        ((diff as f64 / 86_400_000.).round() as i64, "day")
    };
    match (n, unit) {
        (0, "minute") => "this minute".into(),
        (0, "hour") => "this hour".into(),
        (0, _) => "today".into(),
        (-1, "day") => "yesterday".into(),
        (1, "day") => "tomorrow".into(),
        (n, u) if n < 0 => format!("{} {u}{} ago", -n, if n == -1 { "" } else { "s" }),
        (n, u) => format!("in {n} {u}{}", if n == 1 { "" } else { "s" }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn bytes_format_like_the_web() {
        assert_eq!(format_bytes(0.), "unknown size");
        assert_eq!(format_bytes(999.), "999 B");
        assert_eq!(format_bytes(1_100_000_000.), "1.1 GB");
        assert_eq!(format_measured(0.), "0 B");
    }

    #[test]
    fn selected_model_falls_back_to_the_first() {
        let s = json!({ "providers": [{ "provider": "p", "displayName": "P", "models": [
            { "provider": "p", "modelId": "a", "displayName": "A", "status": "available", "available": true },
            { "provider": "p", "modelId": "b", "displayName": "B", "status": "missing", "available": false,
              "download": { "byteSize": 5 } }
        ]}]});
        assert_eq!(
            selected_model(Some(&s), "p", Some("b"))
                .unwrap()
                .download_bytes,
            Some(5)
        );
        assert_eq!(
            selected_model(Some(&s), "p", None).unwrap().display_name,
            "A"
        );
        assert!(selected_model(Some(&s), "q", None).is_none());
    }
}
