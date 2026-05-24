# Release Process

Mnema currently ships macOS builds as internal/test artifacts. These builds are ad hoc signed and not notarized because there is no Apple Developer account configured yet.

## Prepare a Release

1. Bump the desktop app version in:
   - `apps/desktop/src-tauri/tauri.conf.json`
   - `apps/desktop/package.json`
   - `apps/desktop/src-tauri/Cargo.toml`
2. Open and merge the version bump PR after normal CI passes.

## Build a macOS Release

1. In GitHub, open **Actions**.
2. Run the **macOS Release** workflow manually.
3. Enter the version, for example `0.1.0` or `v0.1.0`.

The workflow verifies that all desktop app version fields match, runs frontend and Tauri Rust checks, builds a universal macOS app with `APPLE_SIGNING_IDENTITY="-"`, stages release artifacts, writes `SHA256SUMS`, uploads workflow artifacts, and creates or updates a draft prerelease.

## Review and Publish

Before publishing the draft GitHub Release:

1. Download the DMG from the draft release or workflow artifact.
2. Install it on a macOS machine.
3. Confirm the expected Gatekeeper warning for an ad hoc signed app is documented in release notes.
4. Launch Mnema and smoke-test onboarding, settings, recording start/stop, and any CLI-dependent path that uses the bundled sidecar.
5. Edit generated release notes, then publish the GitHub prerelease.

## Future Developer ID Release

When an Apple Developer account is available, keep the same workflow shape but replace ad hoc signing with Developer ID signing and notarization secrets in a protected `macos-release` GitHub environment.
