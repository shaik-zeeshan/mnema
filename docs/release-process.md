# Release Process

Mnema currently ships Apple Silicon macOS builds. These builds are ad hoc signed and not notarized until a Developer ID certificate is available. Tauri updater signing is separate from Developer ID signing: updater signing verifies update archives inside Mnema, while Developer ID signing/notarization controls macOS Gatekeeper trust.

The first update-capable build must still be installed manually by existing users. Only builds that already contain the App Update Service can receive later updates in-app.

## Distribution Bucket

Public distribution lives on Cloudflare R2 behind `https://release.mnema.day` (bucket `mnema-release`), never on GitHub, so downloads and update feeds keep working if the source repository goes private. GitHub Releases remain as an internal build-staging area and record. Bucket layout:

- `releases/v{version}/` — immutable artifacts (DMG, `.app.tar.gz`, `.app.tar.gz.sig`).
- `stable/latest.json` and `preview/latest.json` — the per-channel updater feeds, with URLs rewritten to the R2 artifacts.
- `stable/Mnema.dmg` — fixed download URL used by the website, replaced on each stable promotion.

The promote workflow authenticates with three repository secrets: `R2_ACCOUNT_ID`, `R2_ACCESS_KEY_ID`, `R2_SECRET_ACCESS_KEY` (an R2 API token with Object Read & Write on the bucket). The website's footer version chip reads `stable/latest.json` cross-origin, which requires a CORS rule on the bucket allowing `GET` from the site's origins.

Transition note: installs of v0.1.12 and earlier still poll the old GitHub feeds. To migrate them while the old repo is public, upload the next release's rewritten `latest.json` to the old repo's newest release: `gh release upload v0.1.12 latest.json --clobber`. Once no such installs remain, this can stop.

## Update Channels

- Stable Update is the default channel. It reads `https://release.mnema.day/stable/latest.json`.
- Preview Update is explicit opt-in. It uses prerelease versions such as `0.3.0-preview.1` and reads `https://release.mnema.day/preview/latest.json`.
- Draft releases are smoke-test staging only and must not be update-visible on either channel.
- Preview builds may be less stable and may show macOS security warnings while builds remain ad hoc signed and non-notarized.

## Updater Signing Key

Use one Tauri updater signing keypair for stable and preview channels.

- The public key is committed in `apps/desktop/src-tauri/tauri.conf.json`.
- Store the private key as `TAURI_SIGNING_PRIVATE_KEY` in the protected `macos-release` GitHub environment.
- Store `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` only if the generated key uses a password.
- Do not commit private signing keys or derived secret files.

## Prepare a Release

Use `scripts/release.sh` from a clean release branch.

Examples:

```sh
scripts/release.sh patch
scripts/release.sh minor
scripts/release.sh 0.3.0-preview.1
scripts/release.sh v0.3.0 --yes
```

The script updates:

- `apps/desktop/src-tauri/tauri.conf.json`
- `apps/desktop/package.json`
- `apps/desktop/src-tauri/Cargo.toml`
- `bun.lock`
- `Cargo.lock`

Plain `patch`, `minor`, and `major` bumps only produce stable `X.Y.Z` versions. If the current version is a prerelease, provide the next version explicitly.

## Build a Draft macOS Release

Pushing a `v*` tag or manually running **macOS Release** starts `.github/workflows/macos-release.yml`.

The workflow:

1. Verifies desktop version consistency with `scripts/verify-desktop-release-version.sh`.
2. Derives the channel from the version: `X.Y.Z` is stable, `X.Y.Z-preview.N` is preview.
3. Runs `bun run check`.
4. Prepares the debug Mnema CLI sidecar for Rust verification.
5. Runs `cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml --locked`.
6. Runs `tauri-apps/tauri-action@v0` for `--target aarch64-apple-darwin`.
7. Creates or updates a draft GitHub Release with the DMG, `.app.tar.gz`, `.app.tar.gz.sig`, and `latest.json`.
8. Uploads staged workflow artifacts and `SHA256SUMS` for smoke testing convenience.

The release remains draft after this workflow. Draft releases are not visible to the stable updater feed and should not be copied to the preview feed.

## Smoke Test the Draft

Before promotion:

1. Download the DMG from the draft release or workflow artifact.
2. Install it on an Apple Silicon Mac.
3. Confirm the expected Gatekeeper warning is acceptable for the current ad hoc build state.
4. Launch Mnema and smoke-test onboarding, settings, recording start/stop, update settings, and bundled CLI-dependent paths.
5. Confirm the draft release has a DMG, `.app.tar.gz`, `.app.tar.gz.sig`, and `latest.json`.

## Promote a Release

**Promote macOS Release** (`.github/workflows/macos-release-promote.yml`) runs automatically when the draft GitHub release is published (draft → published in the GitHub UI), or manually via workflow dispatch with the reviewed version.

For both channels the workflow:

- Confirms the release is still draft (unless `republish=true`, which backfills R2 from an already-published release) and has the required assets.
- Downloads the assets, rewrites `latest.json`'s platform URLs to `https://release.mnema.day/releases/v{version}/…`, and uploads artifacts plus the channel feed (`stable/latest.json` or `preview/latest.json`) to R2. Stable promotions also replace the fixed `stable/Mnema.dmg` download.
- Verifies the public URLs serve the promoted version, then marks the GitHub release published (stable: non-prerelease; preview: prerelease) as the internal record.

## Future Developer ID Release

When an Apple Developer account is available, keep the same release shape but replace ad hoc signing with Developer ID signing and notarization in the protected `macos-release` environment. Do not rotate the Tauri updater signing key unless you are prepared to handle updater key migration for existing installations.
