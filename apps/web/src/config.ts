export const GITHUB_OWNER = "shaik-zeeshan";
export const GITHUB_REPO = "mnema";
export const GITHUB_URL = `https://github.com/${GITHUB_OWNER}/${GITHUB_REPO}`;
export const GITHUB_RELEASES_API_URL = `https://api.github.com/repos/${GITHUB_OWNER}/${GITHUB_REPO}/releases?per_page=10`;
export const PLATFORM_LABEL = "macOS · Apple Silicon";

// Static fallback for crawlers, disabled JavaScript, and GitHub API failures.
// Browser runtime upgrades these links to the latest published macOS artifact.
export const RELEASE_URL = `${GITHUB_URL}/releases`;
export const DOWNLOAD_URL = RELEASE_URL;
