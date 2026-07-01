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

## Combined macOS + Windows Release

The macOS-only workflow above is retired to a manual fallback. Real releases now
run through one combined pipeline that builds both platforms from a single tag.

- `.github/workflows/macos-release.yml` no longer triggers on `v*` tags
  (`workflow_dispatch` only). It stays as a manual fallback until the combined
  path is proven on a preview tag, then it is deleted (see the operator runbook).
- `.github/workflows/release.yml` owns the `v*` tag trigger (and a manual
  `workflow_dispatch(version)`).

### Combined flow

Pushing a `v*` tag (or dispatching **Release**) runs three jobs:

1. `build-macos` (macos-15, `aarch64-apple-darwin`) builds the DMG,
   `.app.tar.gz`, and `.sig` — build-only, uploaded as artifacts, no release.
2. `build-windows` (windows-latest, `x86_64-pc-windows-msvc`) builds the NSIS
   `-setup.exe` and its `.exe.sig` — build-only, under `msvc-dev-cmd`, with a
   secret-gated Authenticode signing stub.
3. `assemble` (ubuntu, `needs` both) hand-builds ONE two-platform `latest.json`
   (`darwin-aarch64` + `windows-x86_64`) via
   `scripts/assemble-release-manifest.mjs`, checksums every asset into
   `SHA256SUMS`, and creates the DRAFT GitHub Release with all assets.

Because the single manifest is a build output of the `assemble` job (not
patched into an existing release by two racing per-OS jobs), manifest generation
is race-free. After promotion the stable feed
`releases/latest/download/latest.json` is a single two-platform manifest.

After review, **Promote Release** (`.github/workflows/release-promote.yml`,
`workflow_dispatch(version)`) validates the draft carries the DMG,
`.app.tar.gz`, `.app.tar.gz.sig`, the NSIS installer, its `.exe.sig`, and
`latest.json`, then publishes it: stable versions publish non-draft/non-prerelease
(the GitHub stable feed becomes live); preview versions publish as prerelease and
deploy that release's `latest.json` to GitHub Pages `updates/preview/latest.json`.

### Windows specifics

- NSIS-only, per-user installer (`nsis.installMode: currentUser`). No MSI.
- Passive auto-update (`updater.windows.installMode: passive`) via the NSIS updater.
- No CLI sidecar in the Windows bundle (deferred until the named-pipe broker lands).
- ORT (`onnxruntime.dll` and providers) DLLs are auto-staged next to the exe at
  build time by `scripts/prepare-ort-dylibs.mjs`.
- Updater minisign signing works on Windows through the shared
  `TAURI_SIGNING_PRIVATE_KEY`, exactly as on macOS.

### Unsigned-preview posture (Windows)

There is no Authenticode certificate yet, so the `build-windows` signing step is
a **no-op stub** whenever the cert secret is absent. Consequences:

- Windows SmartScreen shows an "unknown publisher" warning on both first install
  and on each auto-update.
- This is acceptable for preview / smoke builds only.
- Treat code signing (intended: **Azure Trusted Signing**) as a gate before
  inviting non-technical users onto the stable channel.

### Windows on-device smoke checklist

Operator, before promoting a version to stable:

1. The NSIS `-setup.exe` installs (accept the SmartScreen "unknown publisher"
   warning).
2. WebView2 bootstrap downloads and installs if the runtime is missing.
3. The app launches.
4. Onboarding and settings work; recording starts and stops.
5. First-run model downloads succeed (OCR / transcription / speaker).
6. An in-app update from a prior build applies via the passive NSIS updater
   (SmartScreen warning expected again while unsigned).

### Operator runbook / migration steps

These are the manual gate steps that cannot be run from CI:

1. Cut a throwaway `-preview.N` tag. Confirm:
   - both `build-macos` and `build-windows` legs build,
   - `assemble` produces a two-platform `latest.json`,
   - the draft release carries the NSIS `.exe` + `.exe.sig` + `latest.json` +
     the macOS DMG / `.app.tar.gz` / `.sig` assets,
   - **Promote Release** deploys the preview feed to Pages.
2. Regression check: confirm the macOS leg still yields the same artifacts the
   old `macos-release.yml` produced.
3. Only after both pass: delete `macos-release.yml` (and optionally
   `macos-release-promote.yml`). Its `v*` trigger is already disabled.
4. The Authenticode signing step is asserted to be a no-op while no cert secret
   is set; do not test real signing until a certificate (Azure Trusted Signing)
   is provisioned.

## Future Developer ID Release

When an Apple Developer account is available, keep the same release shape but replace ad hoc signing with Developer ID signing and notarization in the protected `macos-release` environment. Do not rotate the Tauri updater signing key unless you are prepared to handle updater key migration for existing installations.
