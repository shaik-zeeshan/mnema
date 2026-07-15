---
status: accepted
---

# Windows Broker Authorization Channel is a named pipe with a per-user SDDL DACL

The **Broker Authorization Channel** ([ADR 0014](0014-use-app-mediated-cli-access-authorization-channel.md)) is macOS-first: a per-user Unix domain socket under the app config dir carries a newline-framed JSON authorization protocol between `crates/cli` (client) and the desktop app (server). This ADR discharges the "abstract enough for future Windows/Linux local IPC" clause of that decision by porting the transport to Windows as a **named pipe**, keeping the protocol, grant policy, prompt UX, and one-active-request semantics byte-for-byte identical.

## Context

Today the entire server (`apps/desktop/src-tauri/src/broker_authorization_channel.rs`) and the client transport (`crates/cli/src/main.rs::send_authorization_request`, `authorization_socket_path`) are `#[cfg(unix)]`; on non-unix the server `start()` is a no-op returning `Ok` and the client stub returns `app_unavailable`. Everything else — the `AuthorizationChannelRequest`/`Response` structs, the grant policy, the `ActiveRequestGuard`, the onboarding gate, the pending-request window, identity resolution — is already platform-neutral. The stream is only ever touched through `AsyncRead`/`AsyncWrite`.

Windows has no Unix-socket idiom that is portable through Tokio without friction. The realistic transports are a **named pipe** or a **localhost TCP loopback**. Loopback TCP needs a port-discovery mechanism (no fixed port for a per-user, side-by-side-installable app), invites a Windows Firewall prompt on first bind, and requires loopback-binding hardening — all to reach parity a named pipe gives natively. Named pipes provide a deterministic per-user endpoint name, a kernel-enforced security descriptor (the direct analogue of the Unix socket's `0o700` owner-only parent dir), and no firewall surface.

Two Windows-specific facts shape the security design:

- Named pipes live in a **machine-global** namespace (`\\.\pipe\…`). A bare name like `\\.\pipe\mnema-cli-access` would let the first user's app own the endpoint on a multi-session box (terminal server, fast-user-switching) and starve or cross-wire a second user's app.
- The **default** named-pipe security descriptor grants read/connect to **Everyone** and to the **anonymous** account. Created with defaults, any local user — and anonymous — could connect to the authorization endpoint.

## Decision

**Windows implements the Broker Authorization Channel as a named pipe.** Tokio's `tokio::net::windows::named_pipe` (already available — `tokio`'s `net` feature is enabled on both crates) carries the unchanged newline-framed JSON protocol.

- **Shared handler, forked listener.** The connection handler and protocol I/O (`handle_connection`, `read_request_line`, the `write_*` helpers, the 64 KiB cap, the 5 s read timeout, the schema-version check) are made generic over `S: AsyncRead + AsyncWrite + Unpin` and shared single-source across platforms — this is the security-sensitive parsing code we least want to maintain in two copies. Only endpoint setup and the accept loop are `#[cfg]`-forked. No transport trait: two `cfg` functions express the divergence more plainly than a runtime abstraction, and the *conceptual* boundary already lives in `crates/cli/CONTEXT.md`.

- **Endpoint name: `\\.\pipe\{app_identifier}-{current_user_sid}-cli-access`.** The server reads `app.config().identifier`, the CLI reads `env!("MNEMA_APP_IDENTIFIER")` (same value, so dev/prod/side-by-side installs get distinct pipes, mirroring how the Unix path keys off the identifier). Both processes derive the SID from **their own** process token (`OpenProcessToken` → `GetTokenInformation(TokenUser)` → `ConvertSidToStringSidW`); running as the same user, they compute the same string with no handoff and **no durable discovery file at all** — strictly better than the Unix path artifact for the "deterministic within the local user session" rule. The SID sits in the *name* (not only the DACL) because the name determines *which user's server owns the endpoint*; the DACL determines *who may connect*.

- **Protected, user-SID-only DACL via SDDL.** The pipe is created with `ServerOptions::create_with_security_attributes_raw` passing a self-relative security descriptor built from the SDDL string `D:P(A;;GA;;;{current_user_sid})` (via `ConvertStringSecurityDescriptorToSecurityDescriptorW`): a protected DACL (`P`, no inherited ACE can widen it) with a single ACE granting `GENERIC_ALL` to the current user's SID — no Everyone, no anonymous, no SYSTEM/Administrators. A same-user UAC-elevated CLI still carries the user's SID and still matches. This is the faithful Windows mapping of ADR 0014's "relies on local user socket permissions and user approval rather than a separate shared secret": the DACL *is* the permission, and the no-shared-secret / no-crypto-attestation posture is preserved. `reject_remote_clients(true)` is set explicitly (named pipes are reachable via `\\host\pipe\…`), and `first_pipe_instance(true)` on the first instance makes us fail loudly rather than adopt a pre-existing squatted endpoint. The security descriptor is built once and its raw pointer reused for every instance — each instance must carry the DACL, not just the first.

- **Accept model mirrors macOS, guard stays in the handler.** Named pipes have no `accept()` loop: the server creates an instance, `connect().await`s a client on it, and to serve the next client must create the *next* instance itself. It does so immediately after each `connect()` returns (with the same security attributes, without `first_pipe_instance`), then spawns the handler for the just-connected instance. `ActiveRequestGuard` is **unchanged and stays inside `handle_connection`**, so the single-active-request rule is enforced at exactly the same layer as macOS and a concurrent second client still *connects* and receives the same fast `busy` response rather than blocking on connect.

- **Client error mapping; no stale-file machinery.** The CLI opens with `ClientOptions`. `ERROR_FILE_NOT_FOUND` (pipe absent — app not running / channel not up) maps to `app_unavailable`, driving the existing launch-and-retry. `ERROR_PIPE_BUSY` (all instances momentarily taken — a transient race, since the server pre-creates the next instance on every connect) maps to a short bounded retry before falling back to `app_unavailable`. A named pipe is a kernel object that vanishes when the server exits, so there is **no stale-endpoint artifact**: the ADR-0014 rule that the CLI treats an unconnectable socket file as stale and the desktop unlinks it on bind is **macOS-specific**, and `first_pipe_instance` covers ownership on Windows.

- **App-launch uses the registered scheme.** The CLI's Windows launch path replaces the risky bare `cmd /C start "" mnema` (which can resolve to a `PATH` executable) with `cmd /C start "" "mnema://access/request"` — a real invocation of the already-registered `mnema` deep-link scheme that starts the app (so the pipe server binds) or is forwarded by `tauri-plugin-single-instance` if already running. The legacy wake-request file is kept unchanged (it is already cross-platform and gives the app a UI navigation hint at zero cost).

- **Testing matches the macOS bar.** The Win32 boundary (`current_user_sid_string`) is split from pure logic (`pipe_name_for(identifier, sid)`, unit-tested like `socket_path_for_identifier`). The oversized-request/framing test moves from `UnixStream::pair()` to platform-neutral `tokio::io::duplex()`, so it exercises the now-generic reader on every OS. CLI pipe-open error mapping is a pure, tested function. There is deliberately no `AppHandle` test double and no full `handle_connection` round-trip — macOS has neither. One optional `#[cfg(windows)]` real-pipe smoke (server-instance-with-DACL + client + one framed exchange through the generic helpers, using a `MNEMA_CLI_ACCESS_PIPE_NAME` test-name override) is the only coverage of `create_with_security_attributes_raw` end-to-end.

## Considered Options

- **Localhost TCP loopback.** Rejected as the V1 transport: no fixed per-user port, so it needs a discovery mechanism the named pipe's deterministic name makes unnecessary; risks a Windows Firewall prompt on first bind; and requires loopback-binding hardening to match what the pipe DACL gives for free. Held as a fallback only if named pipes prove unworkable.
- **AF_UNIX on Windows.** The OS supports it, but Rust/Tokio portability and NSIS packaging are less straightforward than named pipes for no gain.
- **Default named-pipe security descriptor.** Rejected: Microsoft documents the default as granting read/connect to Everyone and anonymous — unacceptable for an authorization endpoint even though a connection leaks no captured content and cannot mint a grant.
- **SID in the DACL only, generic pipe name.** Rejected: the DACL controls who may *connect*, not who *owns* the name. Without a per-user discriminator in the name itself, two interactive sessions collide on endpoint ownership.
- **A runtime `AuthorizationTransport` trait.** Rejected: ceremony for two implementations that a pair of `cfg` functions expresses more plainly; the abstract boundary is already captured in CONTEXT.md.
- **An `AppHandle` test double for a full Windows round-trip.** Rejected: it would build test scaffolding macOS never had, exceeding the parity bar for a transport port.

## Consequences

- `crates/cli` gains a minimal `windows-sys` (or `windows`) dependency for the token → SID call; the desktop crate already links the `windows` crates.
- Several helpers currently gated `#[cfg(unix)]` / `#[cfg(any(test, unix))]` (e.g. `REQUEST_MAX_BYTES`, `read_request_line`, `ActiveRequestGuard`) widen to `#[cfg(any(unix, windows))]`.
- This single change closes both Storage-access checklist items together — `SUPPORTS.md` Broker Authorization Channel and the `crates/cli` authorization request path — and flips the platform-summary row.
- Same-user malice remains explicitly out of scope (ADR 0014): a same-user process can already connect within the DACL, and V1 does not defend against it beyond user-scoped permissions and user approval.
