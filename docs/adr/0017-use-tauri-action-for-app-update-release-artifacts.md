# Use Tauri Action for App Update release artifacts

Mnema V1 **App Update** release builds use `tauri-apps/tauri-action` to run the Tauri build, upload release artifacts, and generate the updater `latest.json` asset, while keeping Mnema-owned verification steps before the action runs. This follows Tauri's supported GitHub Releases updater path and avoids maintaining a repo-owned updater manifest generator, while preserving Mnema-specific release gates such as version consistency, frontend/Rust checks, and sidecar preparation.
