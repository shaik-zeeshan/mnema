use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum BrowserFamily {
    Safari,
    Chromium,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum BrowserSecureEntryState {
    Active,
    Clear,
    Unavailable,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum BrowserSecureEntryReason {
    FocusedPasswordControl,
    FocusedRelatedCredentialControl,
    FocusedAutocompleteCredentialControl,
    NoFocusedCredentialControl,
    ExtensionNotInstalled,
    ExtensionNotPaired,
    NativeMessagingUnavailable,
    WebsitePermissionUnavailable,
    BrowserUnsupported,
    PageUnsupported,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrowserSecureEntrySignalV1 {
    pub version: u8,
    pub kind: String,
    pub browser_family: BrowserFamily,
    pub state: BrowserSecureEntryState,
    pub reason: BrowserSecureEntryReason,
    pub observed_at_unix_ms: u64,
    pub sequence: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum BrowserMetadataState {
    Available,
    Unavailable,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum BrowserMetadataReason {
    ActiveTab,
    MetadataDisabled,
    UrlModeOff,
    ExtensionNotInstalled,
    ExtensionNotPaired,
    NativeMessagingUnavailable,
    WebsitePermissionUnavailable,
    BrowserUnsupported,
    PageUnsupported,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrowserMetadataSignalV1 {
    pub version: u8,
    pub kind: String,
    pub browser_family: BrowserFamily,
    pub state: BrowserMetadataState,
    pub reason: BrowserMetadataReason,
    pub observed_at_unix_ms: u64,
    pub sequence: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum BrowserIntegrationCoverageState {
    Reliable,
    Partial,
    Unavailable,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum BrowserIntegrationPairingState {
    Unpaired,
    Pairing,
    Paired,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum BrowserMetadataSource {
    BrowserExtension,
    NativeBrowserUrlProbe,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrowserFamilyIntegrationStatus {
    pub browser_family: BrowserFamily,
    pub pairing_state: BrowserIntegrationPairingState,
    pub coverage_state: BrowserIntegrationCoverageState,
    pub secure_entry_state: BrowserSecureEntryState,
    pub secure_entry_reason: BrowserSecureEntryReason,
    pub metadata_state: BrowserMetadataState,
    pub metadata_reason: BrowserMetadataReason,
    pub last_observed_at_unix_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrowserIntegrationStatus {
    pub native_apps: BrowserIntegrationCoverageState,
    pub safari: BrowserFamilyIntegrationStatus,
    pub chromium: BrowserFamilyIntegrationStatus,
    pub metadata_source: BrowserMetadataSource,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrowserIntegrationPairingAction {
    pub browser_family: BrowserFamily,
    pub pairing_state: BrowserIntegrationPairingState,
    pub setup_url: Option<String>,
    pub expires_at_unix_ms: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_secure_entry_signal_roundtrips_camel_case_snake_values() {
        let signal = BrowserSecureEntrySignalV1 {
            version: 1,
            kind: "browser_secure_entry_signal".to_string(),
            browser_family: BrowserFamily::Safari,
            state: BrowserSecureEntryState::Active,
            reason: BrowserSecureEntryReason::FocusedPasswordControl,
            observed_at_unix_ms: 1_700_000_000_000,
            sequence: 7,
        };

        let json = serde_json::to_value(&signal).expect("serialize");
        assert_eq!(json["browserFamily"], "safari");
        assert_eq!(json["state"], "active");
        assert_eq!(json["reason"], "focused_password_control");
        let parsed: BrowserSecureEntrySignalV1 =
            serde_json::from_value(json).expect("deserialize");
        assert_eq!(parsed, signal);
    }

    #[test]
    fn browser_metadata_signal_roundtrips_without_url_when_unavailable() {
        let signal = BrowserMetadataSignalV1 {
            version: 1,
            kind: "browser_metadata_signal".to_string(),
            browser_family: BrowserFamily::Chromium,
            state: BrowserMetadataState::Unavailable,
            reason: BrowserMetadataReason::UrlModeOff,
            observed_at_unix_ms: 1,
            sequence: 2,
            url: None,
        };

        let json = serde_json::to_value(&signal).expect("serialize");
        assert!(json.get("url").is_none());
        let parsed: BrowserMetadataSignalV1 =
            serde_json::from_value(json).expect("deserialize");
        assert_eq!(parsed, signal);
    }
}
