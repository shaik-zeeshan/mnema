use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeCaptureDebugLogStatus {
    pub enabled: bool,
    pub path: String,
    pub exists: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneralAppLogStatus {
    pub path: String,
    pub exists: bool,
    /// On-disk size in bytes; `None` when the file is missing.
    pub size_bytes: Option<u64>,
}
