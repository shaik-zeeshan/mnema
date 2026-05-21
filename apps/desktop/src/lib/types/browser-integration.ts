export type BrowserFamily = "safari" | "chromium";
export type BrowserSecureEntryState = "active" | "clear" | "unavailable";
export type BrowserSecureEntryReason =
	| "focused_password_control"
	| "focused_related_credential_control"
	| "focused_autocomplete_credential_control"
	| "no_focused_credential_control"
	| "extension_not_installed"
	| "extension_not_paired"
	| "native_messaging_unavailable"
	| "website_permission_unavailable"
	| "browser_unsupported"
	| "page_unsupported";
export type BrowserMetadataState = "available" | "unavailable";
export type BrowserMetadataReason =
	| "active_tab"
	| "metadata_disabled"
	| "url_mode_off"
	| "extension_not_installed"
	| "extension_not_paired"
	| "native_messaging_unavailable"
	| "website_permission_unavailable"
	| "browser_unsupported"
	| "page_unsupported";
export type BrowserIntegrationCoverageState = "reliable" | "partial" | "unavailable";
export type BrowserIntegrationPairingState = "unpaired" | "pairing" | "paired";
export type BrowserMetadataSource =
	| "browser_extension"
	| "native_browser_url_probe"
	| "unavailable";

export interface BrowserSecureEntrySignalV1 {
	version: 1;
	kind: "browser_secure_entry_signal";
	browserFamily: BrowserFamily;
	state: BrowserSecureEntryState;
	reason: BrowserSecureEntryReason;
	observedAtUnixMs: number;
	sequence: number;
}

export interface BrowserMetadataSignalV1 {
	version: 1;
	kind: "browser_metadata_signal";
	browserFamily: BrowserFamily;
	state: BrowserMetadataState;
	reason: BrowserMetadataReason;
	observedAtUnixMs: number;
	sequence: number;
	url?: string;
}

export interface BrowserFamilyIntegrationStatus {
	browserFamily: BrowserFamily;
	pairingState: BrowserIntegrationPairingState;
	coverageState: BrowserIntegrationCoverageState;
	secureEntryState: BrowserSecureEntryState;
	secureEntryReason: BrowserSecureEntryReason;
	metadataState: BrowserMetadataState;
	metadataReason: BrowserMetadataReason;
	lastObservedAtUnixMs: number | null;
}

export interface BrowserIntegrationStatus {
	nativeApps: BrowserIntegrationCoverageState;
	safari: BrowserFamilyIntegrationStatus;
	chromium: BrowserFamilyIntegrationStatus;
	metadataSource: BrowserMetadataSource;
}

export interface BrowserIntegrationPairingAction {
	browserFamily: BrowserFamily;
	pairingState: BrowserIntegrationPairingState;
	setupUrl: string | null;
	expiresAtUnixMs: number | null;
}
