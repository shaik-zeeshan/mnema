export const GITHUB_URL = "https://github.com/shaik-zeeshan/mnema";
export const APP_VERSION = "0.1.0";
export const PLATFORM_LABEL = "macOS · Apple Silicon";

// Direct download of the macOS (Apple Silicon) build. Tauri publishes the DMG as
// `mnema_{version}_aarch64.dmg` under the matching `v{version}` release tag, so
// bumping APP_VERSION keeps the download link pointed at the right artifact.
export const DOWNLOAD_URL = `${GITHUB_URL}/releases/download/v${APP_VERSION}/mnema_${APP_VERSION}_aarch64.dmg`;
export const RELEASE_URL = `${GITHUB_URL}/releases/tag/v${APP_VERSION}`;
