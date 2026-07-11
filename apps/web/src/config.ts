export const GITHUB_OWNER = "shaik-zeeshan";
export const GITHUB_REPO = "mnema";
export const GITHUB_URL = `https://github.com/${GITHUB_OWNER}/${GITHUB_REPO}`;
export const PLATFORM_LABEL = "macOS · Apple Silicon";

// Releases are distributed from Cloudflare R2 behind release.mnema.day, not
// GitHub, so downloads keep working even if the source repo goes private.
// The promote workflow maintains these objects; see docs/release-process.md.
export const RELEASES_BASE_URL = "https://release.mnema.day";
export const STABLE_FEED_URL = `${RELEASES_BASE_URL}/stable/latest.json`;
export const DOWNLOAD_URL = `${RELEASES_BASE_URL}/stable/Mnema.dmg`;
