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

// Public Polar checkout link for the one-time Mnema License ($69).
// Set PUBLIC_CHECKOUT_URL at build time (e.g. the live link in prod deploys);
// falls back to the sandbox link (must match apps/desktop/src/lib/licensing.ts).
// `||` not `??`: CI vars that are unset arrive as "" — treat empty as unset.
export const CHECKOUT_URL =
  import.meta.env.PUBLIC_CHECKOUT_URL ||
  "https://sandbox-api.polar.sh/v1/checkout-links/polar_cl_lMoTLnM0OegXGCtfDMzfFi54ZZ41zhfSL8mvP1BpK1L/redirect";
