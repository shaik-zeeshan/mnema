export const GITHUB_OWNER = "shaik-zeeshan";
export const GITHUB_REPO = "mnema";
export const GITHUB_URL = `https://github.com/${GITHUB_OWNER}/${GITHUB_REPO}`;
export const GITHUB_RELEASES_API_URL = `https://api.github.com/repos/${GITHUB_OWNER}/${GITHUB_REPO}/releases?per_page=10`;
export const PLATFORM_LABEL = "macOS · Apple Silicon";

// Static fallback for crawlers, disabled JavaScript, and GitHub API failures.
// Browser runtime upgrades these links to the latest published macOS artifact.
export const RELEASE_URL = `${GITHUB_URL}/releases`;
export const DOWNLOAD_URL = RELEASE_URL;

// Public Polar checkout link for the one-time Mnema License ($69).
// Set PUBLIC_CHECKOUT_URL at build time (e.g. the live link in prod deploys);
// falls back to the sandbox link (must match apps/desktop/src/lib/licensing.ts).
export const CHECKOUT_URL =
  import.meta.env.PUBLIC_CHECKOUT_URL ??
  "https://sandbox-api.polar.sh/v1/checkout-links/polar_cl_YHKNSVQFLu5jQdlQvAlupGMvOoH2a5axMrJti4NOEIu/redirect";
