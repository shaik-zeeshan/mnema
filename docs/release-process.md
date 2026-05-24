# Release Process

Mnema currently ships Apple Silicon macOS builds. These builds are ad hoc signed and not notarized until a Developer ID certificate is available. Tauri updater signing is separate from Developer ID signing: updater signing verifies update archives inside Mnema, while Developer ID signing/notarization controls macOS Gatekeeper trust.

The first update-capable build must still be installed manually by existing users. Only builds that already contain the App Update Service can receive later updates in-app.

## Update Channels

- Stable Update is the default channel. It uses the latest published, non-prerelease GitHub Release at `https://github.com/shaik-zeeshan/mnema/releases/latest/download/latest.json`.
- Preview Update is explicit opt-in. It uses prerelease versions such as `0.3.0-preview.1` and reads `https://shaik-zeeshan.github.io/mnema/updates/preview/latest.json`.
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
6. Runs `tauri-apps/tauri-action@v1` for `--target aarch64-apple-darwin`.
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

Run **Promote macOS Release** (`.github/workflows/macos-release-promote.yml`) manually with the reviewed version.

For stable versions:

- The workflow confirms the release is still draft and has required assets.
- It publishes the release as non-draft and non-prerelease.
- No GitHub Pages update is made. GitHub `releases/latest/download/latest.json` becomes the stable feed.

For preview versions:

- The workflow confirms the release is still draft and has required assets.
- It publishes the release as non-draft prerelease.
- It downloads that release's `latest.json` and deploys it through GitHub Pages at `updates/preview/latest.json`.

GitHub Pages must be configured for GitHub Actions deployment. The promote workflow needs `contents: write` for release publication plus `pages: write` and `id-token: write` for preview feed deployment.

## Future Developer ID Release

When an Apple Developer account is available, keep the same release shape but replace ad hoc signing with Developer ID signing and notarization in the protected `macos-release` environment. Do not rotate the Tauri updater signing key unless you are prepared to handle updater key migration for existing installations.
