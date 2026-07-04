# Plan: Windows Broker Authorization Channel via named pipe

Design authority: [ADR 0045](docs/adr/0045-windows-broker-authorization-channel-via-named-pipe.md) (accepted), porting the macOS Unix-socket channel of [ADR 0014](docs/adr/0014-use-app-mediated-cli-access-authorization-channel.md) to Windows. Approach settled in a grill-with-docs session (7 decisions). Closes `SUPPORTS.md` Storage-access items "Implement Broker Authorization Channel for Windows" and "Update `crates/cli` authorization request path for Windows".

## Problem

On Windows, the Mnema CLI cannot obtain a **CLI Access Grant**. The entire **Broker Authorization Channel** — the app-mediated local channel that lets `mnema search`/`timeline`/`show-text`/`open` request user approval — is `#[cfg(unix)]`: the desktop server `start()` is a no-op and the CLI's `send_authorization_request` returns `app_unavailable` unconditionally. So a Windows user with the CLI installed can never authorize access, and every first-time data command dead-ends at `authorization_required` with no way to grant. This is one of the last two unbuilt Windows subsystems.

## Solution

Port the channel transport to a **Windows named pipe** (`tokio::net::windows::named_pipe`) while keeping the newline-framed JSON protocol, grant policy, prompt UX, pending-request window, and one-active-request semantics byte-for-byte identical to macOS. The connection handler and protocol I/O become generic over `AsyncRead + AsyncWrite` and are shared single-source; only endpoint setup and the accept loop are `#[cfg]`-forked. The pipe endpoint is `\\.\pipe\{app_identifier}-{current_user_sid}-cli-access`, secured with a protected user-SID-only DACL built from SDDL. The CLI opens the pipe, maps `ERROR_FILE_NOT_FOUND`/`ERROR_PIPE_BUSY` to the existing unavailable/retry paths, and launches the app via the registered `mnema://access/request` scheme instead of a bare `mnema` command.

## User Stories

1. As a Windows CLI user, I want `mnema search` to prompt for approval in the Mnema app and create a reusable grant, so that I get the same first-run authorization flow macOS users get.
2. As a Windows CLI user, I want a denied, cancelled, or timed-out request to return the same stable exit codes and messages as on macOS, so that my scripts behave identically across platforms.
3. As a Windows user, I want the authorization endpoint reachable only by my own account, so that no other local or remote user can connect to it.
4. As a Windows user on a multi-session machine, I want my app and CLI to find each other without colliding with another logged-in user's Mnema, so that the channel is per-user correct.
5. As a maintainer, I want the protocol-parsing and framing code shared and tested on every OS, so that a Windows transport port doesn't fork the security-sensitive code.

## Implementation Decisions

Anchored in ADR 0045; the 7 grill decisions in dependency order:

- **Transport:** Windows named pipes via `tokio::net::windows::named_pipe`. No new dependency for transport — `tokio`'s `net` feature is already enabled on both `crates/cli` and the desktop crate. Localhost TCP rejected (port discovery, firewall prompt, loopback hardening).
- **Code structure:** `handle_connection`, `read_request_line`, and the `write_*` helpers become generic over `S: AsyncRead + AsyncWrite + Unpin` and are shared. Only `start()`'s endpoint setup + accept loop is `#[cfg(unix)]` / `#[cfg(windows)]`-forked. No transport trait. Helpers currently gated `#[cfg(unix)]` / `#[cfg(any(test, unix))]` (`REQUEST_MAX_BYTES`, `read_request_line`, `ActiveRequestGuard`, the `write_*` helpers) widen to `#[cfg(any(unix, windows))]`.
- **Endpoint naming:** `\\.\pipe\{app_identifier}-{current_user_sid}-cli-access`. Server reads `app.config().identifier`; CLI reads `env!("MNEMA_APP_IDENTIFIER")` (same value). Both derive the SID from their own process token (`OpenProcessToken` → `GetTokenInformation(TokenUser)` → `ConvertSidToStringSidW`). SID is in the *name* (multi-session ownership), not just the DACL.
- **Security descriptor:** protected user-SID-only DACL via SDDL `D:P(A;;GA;;;{sid})` through `ConvertStringSecurityDescriptorToSecurityDescriptorW`, passed to `ServerOptions::create_with_security_attributes_raw`. No Everyone/anonymous (the default), no SYSTEM/Administrators. `reject_remote_clients(true)` explicit; `first_pipe_instance(true)` on the first instance only. Security descriptor built once, raw pointer reused for every instance.
- **Accept model:** create first instance → `connect().await` → create the *next* instance (same security attrs, no `first_pipe_instance`) → `spawn(handle_connection(connected))` → loop. `ActiveRequestGuard` stays unchanged inside `handle_connection`, so a concurrent second client connects and gets the same fast `busy` response (parity with macOS, not a connect-blocking change).
- **Client transport:** CLI opens via `ClientOptions`. `ERROR_FILE_NOT_FOUND` → `app_unavailable_error()` (drives existing launch-and-retry); `ERROR_PIPE_BUSY` → short bounded retry (a few ~100 ms waits) then `app_unavailable`. No stale-file machinery on Windows — a named pipe is a kernel object; the ADR-0014 stale-socket-unlink rule is macOS-specific. Widen `AUTHORIZATION_TIMEOUT` and the 2 s connect timeout to `#[cfg(any(unix, windows))]`.
- **App launch:** replace the Windows `launch_mnema_app()` branch `cmd /C start "" mnema` with `cmd /C start "" "mnema://access/request"` (registered `mnema` scheme, forwarded by `tauri-plugin-single-instance`). Keep `write_legacy_wake_request()` unchanged (already cross-platform).
- **New dependency:** `crates/cli` gains a minimal `windows-sys` (or `windows`) dependency for the token → SID call. The desktop crate already links the `windows` crates.
- **Win32 boundary split:** `current_user_sid_string() -> Result<String>` (thin Win32, untested) is separated from `pipe_name_for(identifier, sid) -> String` (pure, tested), mirroring the existing `socket_path_for_identifier`. Shared between server and CLI (CLI/server must not hardcode the identifier as an unrelated constant, per CONTEXT.md).
- **Test-name override:** honor a `MNEMA_CLI_ACCESS_PIPE_NAME` env var when set, so tests use an isolated pipe name (the Windows analogue of the `MNEMA_APP_CONFIG_DIR` socket redirect).
- **Assumption:** same-user malice stays out of scope (ADR 0014) — a same-user process can connect within the DACL; V1 does not defend beyond user-scoped permissions + user approval.

## Testing Decisions

Match the macOS bar — no `AppHandle` test double, no full `handle_connection` round-trip (macOS has neither).

- **Pure name derivation:** unit-test `pipe_name_for(identifier, sid)` produces `\\.\pipe\{identifier}-{sid}-cli-access`, mirroring `socket_path_uses_configured_identifier`.
- **Cross-platform framing:** convert `request_line_reader_rejects_oversized_requests` from `UnixStream::pair()` to `tokio::io::duplex()`, deleting its `#[cfg(unix)]` gate so it exercises the now-generic `read_request_line` on every OS (also de-risks the genericization). Keep the oversized-request and newline-termination assertions.
- **CLI error mapping:** pure test that `ERROR_FILE_NOT_FOUND` maps to `app_unavailable` and `ERROR_PIPE_BUSY` maps to the retry classification — no real pipe.
- **Optional `#[cfg(windows)]` real-pipe smoke:** create a server instance with the SDDL security attrs + a client via the `MNEMA_CLI_ACCESS_PIPE_NAME` override, push one framed request/response through the generic helpers (not the full handler). Only end-to-end coverage of `create_with_security_attributes_raw` + connect + framing. Include if cheap; it is the single "nice to have" beyond parity.
- **Preserve** the existing platform-neutral tests (grant policy, scope satisfaction, `ActiveRequestGuard`, approval validation, identity resolution) unchanged; they already run in the `windows-check` CI job.
- **CI:** the `duplex()` framing test + all pure tests run in the existing Windows `cargo test` job; the `#[cfg(windows)]` smoke compiles/runs only on the Windows runner.
- **Do not test:** the Win32 SID-lookup call directly (user-dependent), the full onboarding/dialog/pending-window path (AppHandle-coupled, untested on macOS too).
- **On-device HITL (operator):** with the packaged Windows app running, `mnema search --query x` from a fresh state prompts, "Allow" creates a grant, the retried command returns results; `access request --scope all-retained --duration 7d` opens the expanded window; deny/cancel/timeout return exit codes 10/11; a second concurrent command returns `busy`. Deferred to an operator like the other on-device Windows smokes.

## Slices

1. **Shared, transport-generic connection handler**
   - Goal: make `handle_connection`, `read_request_line`, `write_response`/`write_denied`/`write_unavailable` generic over `S: AsyncRead + AsyncWrite + Unpin`; widen the `#[cfg(unix)]`/`#[cfg(any(test, unix))]` gates on shared helpers to `#[cfg(any(unix, windows))]`. Unix `start()` keeps its Unix-socket listener but now calls the generic handler.
   - Areas: `apps/desktop/src-tauri/src/broker_authorization_channel.rs`.
   - Acceptance: macOS still compiles and all existing unix tests pass; the framing test converted to `tokio::io::duplex()` passes on macOS. No behavior change on macOS.
   - Depends on: none.
   - Parallel: no — foundational; slices 3/4 build on it.

2. **Pure endpoint-name + SID helpers**
   - Goal: add `pipe_name_for(identifier, &sid) -> String` (pure) and `current_user_sid_string() -> Result<String>` (Win32), plus the `MNEMA_CLI_ACCESS_PIPE_NAME` override. Shared derivation usable by both server and CLI.
   - Areas: `broker_authorization_channel.rs` (server), a shared/duplicated pure helper reachable by `crates/cli`; `crates/cli/Cargo.toml` gains `windows-sys`.
   - Acceptance: `pipe_name_for` unit test passes on all OSes; the SID function compiles under `#[cfg(windows)]`.
   - Depends on: none.
   - Parallel: yes, with slice 1.

3. **Windows named-pipe server (desktop)**
   - Goal: `#[cfg(windows)]` `start()` builds the SDDL security descriptor once, creates the first instance with `first_pipe_instance(true)` + `reject_remote_clients(true)` + `create_with_security_attributes_raw`, then runs the create-next-instance/connect/spawn accept loop feeding the generic `handle_connection`. `ActiveRequestGuard` unchanged.
   - Areas: `broker_authorization_channel.rs`.
   - Acceptance: Windows desktop crate compiles; optional real-pipe smoke (server instance + client + one framed exchange) passes on the Windows runner. Concurrent second connection receives `busy`.
   - Depends on: slices 1, 2.
   - Parallel: yes, with slice 4 once the pipe-name helper (slice 2) exists.

4. **Windows CLI client transport + launch fix**
   - Goal: `#[cfg(windows)]` `send_authorization_request` opens the pipe via `ClientOptions`, maps `ERROR_FILE_NOT_FOUND` → `app_unavailable` and `ERROR_PIPE_BUSY` → bounded retry, writes/reads the newline-framed request/response with the existing timeouts; replace the `launch_mnema_app()` Windows branch with `mnema://access/request`; widen the unix-gated timeout constants.
   - Areas: `crates/cli/src/main.rs`, `crates/cli/Cargo.toml`.
   - Acceptance: `crates/cli` compiles on Windows; pure error-mapping test passes; `cli_*` parser/exit-code tests unaffected. The removed-alias and identity tests stay green.
   - Depends on: slices 1, 2.
   - Parallel: yes, with slice 3.

5. **Docs + checklist finalization**
   - Goal: after implementation lands, flip `SUPPORTS.md` Storage-access items `:203`/`:205` from `[ ]` to `[x]` and the platform-summary Broker row `:49` to `[x]` (with the on-device HITL noted as operator-deferred); confirm `crates/cli/CONTEXT.md` rules read correctly against the shipped code.
   - Areas: `SUPPORTS.md`, `crates/cli/CONTEXT.md`.
   - Acceptance: checklist reflects reality; no `[ ]` left for the two implemented items.
   - Depends on: slices 3, 4.
   - Parallel: no — closes out.

Parallel groups: `[1, 2]` → `[3, 4]` → `[5]`.

## Out of Scope

- Localhost TCP transport (rejected; fallback only if named pipes prove unworkable).
- Cryptographic client authentication or a shared-secret handshake (ADR 0014: no shared secret; DACL + user approval only).
- Defense against a malicious same-user local process (explicitly out of scope).
- Linux local IPC (would reuse the Unix-socket path; not this change).
- The on-device HITL run itself (operator-owned, like other Windows smokes).
- Deep-link *routing* of `access/request` in the app UI beyond launching/focusing — the native prompt fires on request arrival regardless; UI navigation is the legacy-wake-file's job, unchanged.
- The remaining Storage-access verification items (Open Captured URL, tray, global shortcuts) and Authenticode signing — separate checklist entries.

## Further Notes

- **Dependency check:** confirm `windows-sys` feature flags cover `Win32_Security`, `Win32_Security_Authorization` (SDDL conversion), and `Win32_System_Threading`/`Win32_Foundation` (token) — the smallest feature set that compiles the SID + SDDL calls.
- **Security-descriptor lifetime:** the self-relative SD (and the `SECURITY_ATTRIBUTES` wrapping it) must outlive every `create_with_security_attributes_raw` call in the accept loop — build once above the loop, free with `LocalFree` on shutdown (or intentionally leak for process lifetime, since the server runs until app exit).
- **`first_pipe_instance` failure:** with `tauri-plugin-single-instance` preventing a second same-user app, a first-instance create failure most likely means a squatter — log and let the channel degrade to unavailable (CLI then reports `app_unavailable`), matching how the unix task returns on listener-init failure.
- **Build/verify env (from memory):** Windows `cargo check`/`test` needs the MSVC dev env; run `cargo test` foreground (background risks `LNK1104`), and desktop-crate tests need `ORT_DYLIB_PATH`. The `crates/cli` tests here are lighter (no ORT), but the desktop crate compile pulls the full native chain.
- **Observability:** keep the existing `tauri_plugin_log` error logs on listener/instance init failure; add one on SID-lookup failure so a misconfigured token surfaces rather than silently disabling the channel.
