# Windows 11 Storage, Access, and Release Research

_Last researched: 2026-05-26_

This note covers the Windows 11 equivalents for Mnema's current macOS **Storage, access, and release** seams. It intentionally focuses on what this repo already uses, plus the Windows APIs/tools we should use later.

## Short recommendation

For a Windows 11 bring-up:

- **Storage paths:** keep Tauri/Rust-owned path resolution, but use Windows-appropriate locations:
  - small settings/grants/audit: `app_config_dir()` / `dirs::config_dir()` -> `%APPDATA%\com.shaikzeeshan.mnema`;
  - large persistent app-owned models/state: `app_local_data_dir()` -> `%LOCALAPPDATA%\com.shaikzeeshan.mnema`;
  - generated caches: `app_cache_dir()` / `BaseDirectory::AppCache` -> `%LOCALAPPDATA%\com.shaikzeeshan.mnema` cache roots;
  - capture library/default `saveDirectory`: prefer a shallow non-roaming LocalAppData library for new Windows installs, not the current `HOME/.mnema` fallback.
- **Encrypted Capture Index key store:** use **Windows Credential Manager** generic credentials for the SQLCipher passphrase. A DPAPI-protected local file is a viable fallback, but Credential Manager is the closest Windows equivalent to macOS Keychain.
- **Broker Authorization Channel:** use a **Windows named pipe** through `tokio::net::windows::named_pipe`, with remote clients rejected and a security descriptor restricted to the current user. Do not use localhost TCP for V1 unless named pipes fail.
- **Deep links / app launch:** keep `tauri-plugin-deep-link` and the `mnema://` scheme; Windows desktop schemes must be registered in config. CLI launch/reopen should use an actual `mnema://...` URL, not a bare `mnema` command string.
- **CLI sidecar:** keep Tauri `externalBin` with target-triple `.exe` sidecars. Re-check the Windows CLI install target before shipping; `%LOCALAPPDATA%\Microsoft\WindowsApps\mnema.exe` may work only when that directory is writable and on `PATH`, while an app-local bin directory plus PATH guidance is safer for unpackaged releases.
- **Release:** start with **Windows 11 x64 + NSIS current-user installer + Tauri updater artifacts + Authenticode signing**. Keep MSI optional. Use `downloadBootstrapper` for WebView2 on normal Windows 11 distribution; use `offlineInstaller` for Microsoft Store/offline enterprise scenarios.
- **Signing:** keep Tauri updater signing as-is; add Windows Authenticode signing separately. Azure Trusted Signing, EV certs, or modern OV provider flows are the practical options.

## Current Mnema inventory

### Storage used today

| Current use | Source | Windows 11 status / action |
| --- | --- | --- |
| `recording-settings.json` under Tauri `app_config_dir()` | `apps/desktop/src-tauri/src/native_capture_settings.rs` | Good. Tauri resolves app config to `config_dir / bundle_identifier`; on Windows `config_dir` is Roaming AppData. |
| `keyboard-bindings.json`, `one-time-prompts.json`, `app-update-settings.json`, CLI grants/audit under app config | `keyboard_bindings.rs`, `one_time_prompts.rs`, `app_updates.rs`, `crates/app-infra/src/brokered_access.rs` | Good for small user settings/policy files. CLI uses `dirs::config_dir()`, which also maps to Roaming AppData on Windows. |
| Default `saveDirectory` from `HOME/.mnema` | `native_capture_settings.rs` | Not Windows-safe. `HOME` may be unset in a Windows desktop process, causing relative `.mnema`. Use `dirs::data_local_dir()` or Tauri `app_local_data_dir()` for Windows defaults. |
| SQLite / SQLCipher DB under `<saveDirectory>/db/app.sqlite3` | `crates/app-infra/src/db.rs` | Works conceptually. Keep DB in the active capture library, but keep the encryption key outside `saveDirectory`. Verify bundled SQLCipher builds on `windows-latest` MSVC; a local Windows check currently fails unless OpenSSL is configured (`OPENSSL_DIR` or an equivalent `libsqlite3-sys`/OpenSSL setup). |
| Recordings under `<saveDirectory>/recordings/YYYY/MM/DD/` | capture lifecycle / app infra | Works conceptually. Keep default path shallow to avoid Windows `MAX_PATH` surprises; test user-selected OneDrive/network locations separately. |
| Model downloads under `app_data_dir()` | `audio_transcription_models.rs`, `ocr_models.rs`, `speaker_analysis_models.rs`, `native_capture.rs` | On Windows, Tauri `app_data_dir()` maps to Roaming AppData. Large models should move to `app_local_data_dir()` for Windows so they do not roam. |
| App icon and preview caches under `BaseDirectory::AppCache` | `native_capture.rs`, `app_infra/frame_preview.rs` | Good. Cache is disposable and should remain under LocalAppData/cache roots. |
| Frontend file write capability only for `$DOWNLOAD/*` | `apps/desktop/src-tauri/capabilities/default.json` | Good. Tauri fs plugin supports Windows; if the frontend needs more Windows paths, extend capability scope explicitly. |

### Access used today

| Current use | Source | Windows 11 status / action |
| --- | --- | --- |
| Capture Index Key Store uses macOS `security` CLI | `crates/app-infra/src/capture_index_key_store.rs` | Missing on Windows. Implement Credential Manager generic credential load/store behind the existing adapter. |
| Broker Authorization Channel uses Unix domain sockets | `apps/desktop/src-tauri/src/broker_authorization_channel.rs`, `crates/cli/src/main.rs` | Missing on Windows. Add a named-pipe transport shared by desktop and CLI. |
| Brokered CLI data commands run without the app if grants and DB/key are available | `crates/app-infra/src/brokered_access.rs`, `crates/cli/src/main.rs` | Works only after Windows key store and config-dir parity exist. |
| App reopen / `open` command uses `mnema://open/<opaque_id>` | `crates/app-infra/src/brokered_access.rs` | Windows path already shells out through `cmd /C start "" <url>`; keep this for deep-link opens. |
| CLI app launch uses `cmd /C start "" "mnema"` | `crates/cli/src/main.rs` | Risky: this is not clearly a URL scheme invocation. Prefer a real `mnema://access/request` deep link or registered app path. |
| Tauri plugins: deep-link, single-instance, updater, global-shortcut, dialog, opener, fs, clipboard, log, tray | `apps/desktop/src-tauri/Cargo.toml`, `tauri.conf.json` | Tauri documents Windows support for these desktop plugins. Need behavior smoke tests, not replacement APIs. |
| Native tray/status bar | `status_bar.rs` | Tauri tray supports Windows, but macOS template-icon behavior may need a Windows-specific icon asset/test. |
| Global shortcuts | `keyboard_bindings.rs` | Tauri global-shortcut supports Windows. `CommandOrControl` should map to Control; verify default conflicts. |

### Release used today

| Current use | Source | Windows 11 status / action |
| --- | --- | --- |
| Tauri bundling with `targets: "all"`, `createUpdaterArtifacts: true`, `externalBin: ["binaries/mnema-cli"]` | `apps/desktop/src-tauri/tauri.conf.json` | Add Windows-specific bundle config. Prefer NSIS-only initially or set Tauri Action to prefer NSIS if MSI and NSIS are both built. |
| CLI sidecar target suffix script supports `.exe` | `scripts/prepare-mnema-cli-sidecar.sh` | Good shape. Run it on `windows-latest` under Git Bash/PowerShell before relying on it. |
| macOS-only release workflows and docs | `.github/workflows/macos-release*.yml`, `docs/release-process.md` | Add Windows release workflow/docs. Decide whether one multi-platform release workflow owns `latest.json` or whether a post-build step merges platform manifests. |
| App update feed stable/preview | `app_updates.rs`, `docs/release-process.md` | Tauri updater supports Windows. Static `latest.json` must include `windows-x86_64` target entries with URL and signature. |
| Incompatible update copy says “Mac” | `apps/desktop/src-tauri/src/app_updates.rs` | Change to platform-neutral copy before Windows release. |
| Windows icons already present | `apps/desktop/src-tauri/icons/icon.ico`, Square logos | Good starting point; verify installer/control panel appearance. |

## Windows choices and alternatives

### Storage paths

Use Tauri/Rust path APIs rather than hardcoded environment variables:

- Tauri `app_config_dir()` = `config_dir / ${bundle_identifier}`. On Windows, `config_dir` maps to `{FOLDERID_RoamingAppData}` / `%APPDATA%`.
- Tauri `app_data_dir()` also maps through `data_dir`, which is Roaming AppData on Windows.
- Tauri `app_local_data_dir()` maps through `local_data_dir`, which is `{FOLDERID_LocalAppData}` / `%LOCALAPPDATA%`.
- Tauri `app_cache_dir()` maps through `cache_dir`, which is LocalAppData on Windows.

Recommended Mnema split:

| Data class | Windows location |
| --- | --- |
| Settings, grants, audit metadata, update settings | `%APPDATA%\com.shaikzeeshan.mnema\*.json` |
| Large downloaded OCR/transcription/speaker models | `%LOCALAPPDATA%\com.shaikzeeshan.mnema\models\...` |
| Preview/icon caches | `%LOCALAPPDATA%\com.shaikzeeshan.mnema\cache\...` via Tauri cache APIs |
| Default capture library | `%LOCALAPPDATA%\com.shaikzeeshan.mnema\library` or another shallow local path, user-changeable |
| User exports/downloads | Tauri dialog + `$DOWNLOAD` scope when initiated by frontend |

Do not store large model directories in Roaming AppData on Windows. Do not rely on `HOME` for desktop defaults. Keep `MNEMA_APP_CONFIG_DIR` and `MNEMA_SAVE_DIRECTORY` as test/debug overrides.

Windows path gotchas to test:

- `MAX_PATH`: Windows still has legacy 260-character path constraints unless long paths are enabled and the app is long-path-aware. Mnema's date-organized recording paths are nested, so defaults should be shallow.
- Cloud/network folders: OneDrive or network shares can affect file locking, latency, and retention cleanup. Treat them as user-selected advanced storage until tested.
- Case-insensitivity and backslashes: persist paths as opaque strings, but canonicalize only at filesystem boundaries.

### Capture Index Key Store

Primary recommendation: **Windows Credential Manager generic credentials**.

Suggested mapping:

- Type: `CRED_TYPE_GENERIC`.
- Target name: `com.shaikzeeshan.mnema.capture-index/<index_id>` or equivalent stable string.
- User name: `com.shaikzeeshan.mnema` or empty/app label.
- Secret: SQLCipher passphrase bytes/string currently generated by `CaptureIndexKeyStore`.
- Persistence: `CRED_PERSIST_LOCAL_MACHINE` so the credential persists for the same Windows user on the same computer and does not roam.

Implementation options:

1. **Direct Win32 through `windows` / `windows-sys`**: call `CredReadW`, `CredWriteW`, `CredFree`; most control, small surface.
2. **`keyring` crate**: supports platform OS credential stores, including macOS Keychain and Windows Credential Manager. This could also replace the current macOS `security` CLI later.
3. **DPAPI-protected local file**: use `CryptProtectData` / `CryptUnprotectData` and store the encrypted blob in LocalAppData. This is simpler to inspect/back up as a file but less like Keychain/Credential Manager.
4. **Tauri Stronghold**: cross-platform secret vault, but it is not a platform-owned key store and usually introduces password/secret-store lifecycle decisions. Not the V1 fit for ADR 0012.

Failure semantics should match ADR 0012: if the key is missing or inaccessible for an existing encrypted index, fail closed and ask the user to restore the original Windows user/machine context, choose a new save directory, or reset the index. Do not create a plaintext fallback.

### Broker Authorization Channel

Primary recommendation: **Windows named pipes**.

Suggested shape:

- Pipe name: `\\.\pipe\mnema-com.shaikzeeshan.mnema-<current-user-sid>-cli-access` or another deterministic per-user name.
- Server: desktop app creates pipe with `tokio::net::windows::named_pipe::ServerOptions`.
- Client: CLI connects with `tokio::net::windows::named_pipe::ClientOptions`.
- Use `first_pipe_instance(true)` to prevent another server instance from owning the same endpoint.
- Keep `reject_remote_clients(true)`; Tokio disables remote clients by default, but set it explicitly.
- Use `create_with_security_attributes_raw(...)` with a `SECURITY_ATTRIBUTES` security descriptor restricted to the current user SID. Do not rely on the default named-pipe DACL because Microsoft documents default read access for Everyone/anonymous.
- Preserve the existing newline-delimited JSON request/response protocol and one-active-request-at-a-time policy.

Alternatives:

| Alternative | Why not first |
| --- | --- |
| Localhost TCP | Easier cross-platform, but needs port discovery, firewall/security decisions, and loopback binding hardening. |
| Windows AF_UNIX / Unix sockets | Windows support exists at OS level, but Rust/Tokio portability and packaging are less straightforward than named pipes. |
| File polling / request files | Already demoted to compatibility fallback by ADR 0014; worse latency and stale-state handling. |

### Deep links and single instance

Keep `tauri-plugin-deep-link` with `mnema` registered in `tauri.conf.json`. Tauri documents Windows support, but desktop deep links must be registered in config and cannot be dynamically registered at runtime.

Windows-specific follow-ups:

- Test installed `mnema://open/<opaque_id>` from PowerShell, `cmd`, browser, and the packaged CLI.
- Prefer `cmd /C start "" "mnema://..."` or Tauri opener for URLs.
- Do not use bare `mnema` to launch the app from CLI authorization; that can resolve to a command on `PATH` instead of the registered URL scheme.
- Ensure `tauri-plugin-single-instance` forwards deep-link arguments and focuses/restores the right window.

### Mnema CLI installation

Tauri sidecar packaging is mostly ready:

- `externalBin: ["binaries/mnema-cli"]` expects `mnema-cli-x86_64-pc-windows-msvc.exe` for Windows x64.
- `scripts/prepare-mnema-cli-sidecar.sh` already appends `.exe` for Windows target triples.
- `bundled_mnema_cli_path_in_dir` already searches plain and target-triple `.exe` sidecar names.

Open Windows install-path decision:

| Option | Pros | Cons |
| --- | --- | --- |
| Current `%LOCALAPPDATA%\Microsoft\WindowsApps\mnema.exe` copy/symlink | Often on user `PATH`; similar to Store app aliases | Needs verification for write permissions/conflicts; may be confused with Windows App Execution Alias behavior. |
| `%LOCALAPPDATA%\Programs\Mnema\bin\mnema.exe` + PATH guidance | App-owned, explicit, avoids WindowsApps ambiguity | Usually requires PATH update guidance/manual action. |
| MSIX AppExecutionAlias | Native Store/package behavior | Only for packaged/MSIX/Microsoft Store path, not the initial NSIS GitHub release. |

Recommendation: for unpackaged NSIS releases, prefer an app-owned LocalAppData bin directory unless WindowsApps testing proves reliable and non-conflicting. Never overwrite an unmanaged `mnema.exe`.

### Windows release pipeline

Initial target:

- OS: Windows 11 users; build on `windows-latest`.
- Arch: `x86_64-pc-windows-msvc` first. Add `aarch64-pc-windows-msvc` later if Windows-on-ARM demand exists.
- Installer: NSIS `currentUser` mode first. It avoids admin by default and is friendlier for updater installs.
- WebView2: default `downloadBootstrapper` is acceptable for Windows 11; Microsoft/Tauri note WebView2 runtime is distributed with Windows 10 April 2018+ and Windows 11. Use `offlineInstaller` for Store/offline distribution.
- Updater install mode: Tauri updater `windows.installMode = "passive"` (default/recommended) so installs show progress without requiring interaction.
- Signing: Authenticode-sign `.exe` and installer artifacts. Keep Tauri updater `.sig` signing separate.

Likely Tauri config direction:

```jsonc
{
  "bundle": {
    "targets": ["nsis"],
    "windows": {
      "webviewInstallMode": { "type": "downloadBootstrapper" },
      "nsis": { "installMode": "currentUser" }
    }
  },
  "plugins": {
    "updater": {
      "windows": { "installMode": "passive" }
    }
  }
}
```

If keeping `targets: "all"`, make the release job choose which Windows installer is used in updater JSON (`updaterJsonPreferNsis: true`) or verify Tauri Action's default MSI choice is intended.

Windows workflow checklist:

1. `bun install --frozen-lockfile`.
2. `bun run check`.
3. Install Rust stable and ensure MSVC build tools are available on `windows-latest`.
4. Build/prepare `mnema-cli-x86_64-pc-windows-msvc.exe` sidecar.
5. Install/configure native prerequisites before the Rust check: OpenSSL for SQLCipher (`OPENSSL_DIR` or equivalent), LLVM/libclang for bindgen (`LIBCLANG_PATH`), CMake/C++ tools for native crates, and Bun for frontend/Tauri commands.
6. `cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml --locked` on Windows.
7. `bun run tauri -- build --target x86_64-pc-windows-msvc` or Tauri Action equivalent.
8. Sign Windows executable/installer with Authenticode.
9. Upload NSIS installer, updater `.sig`, checksums, and `latest.json` containing `windows-x86_64`.
10. Smoke-test install, first launch, deep links, update check, update install/restart, CLI sidecar install/status, global shortcuts, tray, and uninstall.

Release feed caveat: the current macOS workflow lets Tauri Action generate `latest.json` for one target. For multi-platform releases, verify that `latest.json` is merged across macOS and Windows targets rather than overwritten by whichever job finishes last. A single matrix workflow or a final manifest-consolidation job is safer.

### Code signing choices

Separate two signatures:

- **Tauri updater signature**: already configured through `TAURI_SIGNING_PRIVATE_KEY`; verifies updater artifacts inside Mnema and is required by Tauri updater.
- **Windows Authenticode signature**: needed to reduce SmartScreen warnings, build reputation, and for Microsoft Store paths. It signs the Windows app/installer itself.

Options:

| Option | Fit |
| --- | --- |
| Azure Trusted Signing / Artifact Signing | Good modern CI option; managed HSM/certificate lifecycle. |
| EV code signing certificate | Best immediate SmartScreen reputation, higher cost/process overhead. |
| OV code signing certificate | Possible, but post-2023 storage/provider rules vary; follow certificate issuer docs. |
| Unsigned preview | Technically runnable, but expect SmartScreen/browser warnings; not acceptable for broad Windows release. |

Microsoft Store later:

- Tauri currently produces EXE/MSI installers for Store listing.
- Store-distributed installer must be offline, handle auto-updates, and be code signed.
- Use `webviewInstallMode: { "type": "offlineInstaller" }` for Store config.

## Suggested first implementation slices

1. Add Windows CI compile/check job without capture runtime support.
2. Replace Windows default `saveDirectory` with a non-roaming LocalAppData library; keep existing macOS behavior.
3. Move Windows model storage to `app_local_data_dir()` while keeping app config for small JSON files.
4. Implement Windows Credential Manager adapter for `CaptureIndexKeyStoreAdapter` and add key-store tests behind Windows cfg where possible.
5. Add Windows named-pipe Broker Authorization Channel transport in desktop and CLI, preserving the existing JSON protocol.
6. Fix CLI app-launch deep link to use `mnema://...` on Windows.
7. Decide CLI install directory and update Access Settings copy/status for Windows.
8. Add NSIS Windows release workflow with sidecar prep, signing placeholders, updater artifacts, and smoke-test docs.
9. Update `docs/release-process.md` once Windows release is real.

## Sources checked

- Tauri PathResolver docs — https://docs.rs/tauri/latest/tauri/path/struct.PathResolver.html
- `dirs` crate Windows config/data/cache/local-data mappings — https://docs.rs/dirs/latest/dirs/
- Microsoft Known Folders (`FOLDERID_RoamingAppData`, `FOLDERID_LocalAppData`) — https://learn.microsoft.com/en-us/windows/win32/shell/knownfolderid
- Microsoft maximum path length limitations — https://learn.microsoft.com/en-us/windows/win32/fileio/maximum-file-path-limitation
- Microsoft Credential Manager `CredWriteW` — https://learn.microsoft.com/en-us/windows/win32/api/wincred/nf-wincred-credwritew
- Microsoft Credential Manager `CredReadW` — https://learn.microsoft.com/en-us/windows/win32/api/wincred/nf-wincred-credreadw
- Microsoft `CREDENTIALW` persistence/type fields — https://learn.microsoft.com/en-us/windows/win32/api/wincred/ns-wincred-credentialw
- Microsoft DPAPI `CryptProtectData` — https://learn.microsoft.com/en-us/windows/win32/api/dpapi/nf-dpapi-cryptprotectdata
- Microsoft DPAPI `CryptUnprotectData` — https://learn.microsoft.com/en-us/windows/win32/api/dpapi/nf-dpapi-cryptunprotectdata
- `keyring` / `windows-native-keyring-store` crates — https://docs.rs/keyring/latest/keyring/ and https://docs.rs/windows-native-keyring-store/latest/windows_native_keyring_store/
- Microsoft named pipes, pipe names, and named-pipe security — https://learn.microsoft.com/en-us/windows/win32/ipc/named-pipes
- Tokio Windows named pipe API — https://docs.rs/tokio/latest/tokio/net/windows/named_pipe/
- Tauri deep-link plugin — https://v2.tauri.app/plugin/deep-linking/
- Tauri single-instance, global-shortcut, opener, dialog, file-system, clipboard, log, updater plugin docs — https://v2.tauri.app/plugin/
- Tauri sidecar / external binaries docs — https://v2.tauri.app/develop/sidecar/
- Tauri Windows installer docs — https://v2.tauri.app/distribute/windows-installer/
- Tauri Windows code signing docs — https://v2.tauri.app/distribute/sign/windows/
- Tauri updater docs — https://v2.tauri.app/plugin/updater/
- Tauri Action README — https://github.com/tauri-apps/tauri-action
- Tauri Microsoft Store docs — https://v2.tauri.app/distribute/microsoft-store/
- Microsoft Azure Trusted Signing / Artifact Signing overview — https://learn.microsoft.com/en-us/azure/trusted-signing/overview
