# Mnema CLI Context

User-facing `mnema` command grammar, brokered capture access UX, authorization requests, structured output, identity handling, and CLI exit contracts.

Root entry point: [CONTEXT-MAP.md](../../CONTEXT-MAP.md).

## Language

**Downstream Capture Access**:
Access to retained capture content after capture, including search, timeline preview, export, and future local AI features.
_Avoid_: raw SQLite access, direct frame-file access, agent bypass

**Brokered Capture Access**:
A policy-aware app or CLI boundary for downstream access to retained capture content that applies retention, deletion, redaction, and access rules before returning results.
_Avoid_: direct database query, raw media crawl, agent file access

**Broker Authorization Channel**:
An app-mediated local authorization channel that lets a CLI or local tool request a user-approved **Brokered Capture Access** grant without routing brokered capture commands through the running app.
_Avoid_: broker command socket, auth file handoff, agent login

**Broker Client Identity**:
A user-facing local tool identity used to label **Brokered Capture Access** prompts, grants, and audit history.
_Avoid_: local agent, process name, anonymous CLI

**CLI Access**:
The user-facing settings area for installing the Mnema CLI and managing **Brokered Capture Access** grants used by local terminal tools.
_Avoid_: Agent Access, broker settings, trust grants

**CLI Access Grant**:
The user-facing name for a **Brokered Capture Access** grant created for Mnema CLI use.
_Avoid_: trust grant, login, auth token

**Broker Output Format**:
The CLI presentation format for brokered data responses, such as JSON, YAML, or TOON, without changing broker policy or returned content.
_Avoid_: broker mode, serialization policy, response type

**All Retained Broker Scope**:
An explicit **Brokered Capture Access** grant scope over all retained broker-visible derived content rather than the default recent-history scope.
_Avoid_: all data, full disk access, raw capture access

## Relationships

- **Brokered Capture Access** is the supported path for AI agents and other downstream tools to inspect retained capture content.
- Agent-facing capture access should be documented only through **Brokered Capture Access**; app-internal APIs and Tauri commands are not agent contracts unless explicitly marked broker-safe.
- The local `mnema-data` agent skill should be updated after the Mnema CLI access redesign lands so agent instructions use the V1 CLI grammar and **CLI Access** flow.
- Direct SQLite or media-file access by agents is outside Mnema's privacy guarantee.
- **Brokered Capture Access** should use the app-owned **Capture Index Key Store** path rather than exposing raw encryption keys to agents.
- **Brokered Capture Access** may run when the Mnema app is not running, but it must use the same policy, redaction, retention, tombstone, and **Capture Index Key Store** paths as the app.
- **Broker Authorization Channel** is only for requesting and approving **Brokered Capture Access** grants; it is not the transport for brokered capture commands.
- A **Broker Authorization Channel** request includes a **Broker Client Identity** when the caller can declare one, falling back to a clear CLI identity when no caller identity is provided.
- **Broker Client Identity** is supplied explicitly by broker clients when available, may be provided through Mnema CLI client or standard non-sensitive environment configuration, may be inferred from non-sensitive local agent markers, and falls back to the executable identity.
- Mnema CLI resolves **Broker Client Identity** in precedence order: explicit `--client`, `MNEMA_CLI_CLIENT`, explicit `AI_AGENT`, allowlisted non-sensitive known-tool markers, then the default `mnema CLI` identity.
- `MNEMA_CLI_CLIENT` is the preferred Mnema environment variable for **Broker Client Identity**; `AI_AGENT` is treated as an optional explicit environment identity when present, not as a universally supported agent signal.
- Mnema CLI treats explicit **Broker Client Identity** as a global invocation option rather than a single-command search/filter option.
- Mnema CLI uses `access` as the command language for **CLI Access Grant** status and requests; `auth` is not documented or retained as a hidden compatibility alias in V1.
- Mnema CLI **CLI Access Grant** creation is user-confirmed in V1; there is no non-interactive auto-grant mode.
- Mnema CLI access requests may declare desired **Brokered Capture Access** scope and duration, constrained to the V1 authorization choices.
- Mnema CLI access commands may revoke a specific **CLI Access Grant** or all grants for a **Broker Client Identity**.
- Mnema CLI revoke-all requires explicit confirmation or a non-interactive confirmation flag, while revoking a specific grant ID does not require an additional prompt.
- Mnema CLI revoke by **Broker Client Identity** uses exact normalized identity matching rather than partial matching.
- Mnema CLI data commands do not expose scope or duration as command options in V1; users pre-authorize through access request or respond to just-in-time authorization.
- Mnema CLI data commands without an active grant request the default recent-history **CLI Access Grant** unless the command necessarily requires **All Retained Broker Scope**.
- Mnema CLI data commands may trigger just-in-time **CLI Access Request** only in interactive terminal contexts; non-interactive contexts should require an explicit access request.
- Mnema CLI supports a no-prompt mode that prevents just-in-time **CLI Access Request** UI and fails when no suitable grant already exists.
- All-history CLI search is expressed through **All Retained Broker Scope** on the active grant rather than a search-specific all-history flag.
- Mnema CLI UX, argument parsing, app-launch behavior, and **Broker Authorization Channel** client transport belong in `crates/cli` rather than app-infra so app-infra remains the shared broker policy and data-access boundary.
- Mnema CLI V1 uses `cli` as the Cargo package/crate boundary, while the installed user-facing command is `mnema`.
- The Tauri sidecar binary may remain named `mnema-cli` for packaging compatibility, with installation exposing it as the `mnema` command.
- `mnema --version` reports the `crates/cli` package version; packaging may align that version with the Tauri app version, but the CLI reads its version from its own Cargo package metadata.
- Mnema CLI broker data commands should support **Broker Output Format** values JSON, YAML, and TOON in V1 while keeping JSON as the default.
- Mnema CLI structured **Broker Output Format** responses use a versioned top-level envelope in V1 rather than untagged command payloads.
- The V1 Mnema CLI structured output envelope includes schema version, command name, resolved **Broker Client Identity** metadata, and command-specific data.
- Mnema CLI search output returns opaque result `id`, kind, redacted snippet, startedAt, endedAt, limit, and nullable nextCursor in V1.
- Mnema CLI search output should not include app names, window titles, browser URLs, media paths, or raw database identifiers in V1.
- Mnema CLI timeline output returns coarse source/time intervals, limit, and nullable nextCursor in V1.
- Mnema CLI timeline interval output may include a nullable summary field, but V1 should not return app names, window titles, browser URLs, media paths, or raw database identifiers.
- Mnema CLI show-text output returns one opaque result `id`, kind, and full redacted text for that result in V1.
- Mnema CLI show-text should return a structured error rather than an empty successful text payload when the result is invalid, unavailable, revoked, expired, or outside the active grant scope.
- Mnema CLI `open` structured output returns the opaque result `id` and opened state in V1, without returning media paths or deep-link URLs.
- Mnema CLI TOON output should use standard encoder behavior in V1 rather than custom compacting options that create a Mnema-specific dialect.
- Mnema CLI YAML output should use `yaml_serde` or another currently maintained Serde-compatible YAML encoder verified during implementation.
- Mnema CLI YAML output should not use deprecated or advisory-flagged YAML encoders such as `serde_yaml` or `serde_yml`.
- Mnema CLI output serialization failures are operational CLI errors rather than broker response errors.
- Mnema CLI treats **Broker Output Format** as a global presentation option, but commands that do not support formatted output should reject it rather than silently ignoring it.
- Mnema CLI non-data commands such as access requests and installation/status helpers may use human-readable output instead of structured broker response output.
- Mnema CLI data commands include search, timeline, show-text, and `open` for the app-mediated open action; non-data commands include access request, help, version, and CLI helper/status commands.
- Mnema CLI access status is human-readable by default but may use **Broker Output Format** for scripts when explicitly requested.
- Mnema CLI access status focuses on active **CLI Access Grant** state by default, while **Access Settings** owns fuller inactive grant and access history review.
- Mnema CLI access status defaults to the resolved **Broker Client Identity** and may show all active identities only when explicitly requested.
- Mnema CLI access status structured output includes active grant ID, client identity/provenance, scope, createdAt, expiresAt, and revoked state for the resolved client by default.
- Mnema CLI access status may include active grants for all clients only when explicitly requested, such as with `--all-clients`.
- Mnema CLI writes command results to stdout, progress and waiting messages to stderr, and operational failures to stderr with nonzero exit; structured broker data output should stay parseable without progress noise.
- Mnema CLI distinguishes success, usage errors, operational or broker errors, authorization denial, authorization timeout or app unavailability, and outside-grant-scope failures with stable exit codes.
- Mnema CLI structured errors use the same versioned output envelope as successful structured output, with an error object containing a stable code, message, and retryable flag.
- Mnema CLI V1 uses stable exit codes: 0 success, 2 usage or validation error, 10 authorization required or denied, 11 authorization timeout, 12 Mnema app unavailable, 13 outside grant scope, 20 broker/data operation failed, and 21 output serialization failed.
- Mnema CLI authorization-required and authorization-denied failures may share exit code 10, but they remain distinct structured error codes.
- Mnema CLI behavior should be verified with both parser/unit tests and binary-level integration tests for command shape, output, and exit-code contracts.
- Mnema CLI and access redesign verification should include parser/unit tests in `crates/cli`, binary-level stdout/stderr/exit-code tests, app-infra identity/grant/migration tests, local socket protocol tests, and Tauri reopen fallback tests.
- **Brokered Capture Access** should expose a dedicated CLI contract backed by shared Rust policy/query code rather than relying on app-internal Tauri commands as the agent interface.
- **Brokered Capture Access** requires user authorization before an agent or downstream tool can query capture data, and that authorization grants redacted/searchable derived access rather than original media export.
- A **Broker Authorization Channel** approval creates the **Brokered Capture Access** grant directly; settings are for installation, inspection, and revocation rather than the primary approval path.
- A CLI command waiting on the **Broker Authorization Channel** may continue the original **Brokered Capture Access** request after approval; denial, timeout, or app-unavailable states remain explicit authorization failures.
- Mnema CLI verifies that active grants satisfy the original request after **Broker Authorization Channel** approval before retrying the pending broker command.
- **Broker Authorization Channel** requests are synchronous from the CLI user's perspective: the original command waits for approval, denial, timeout, or app-unavailable failure instead of fire-and-forget authorization.
- Mnema CLI interactive **Broker Authorization Channel** requests wait up to 120 seconds for approval, denial, or unavailable response before returning an authorization-timeout failure.
- **Broker Authorization Channel** V1 treats CLI disconnect as cancellation of the waiting request rather than requiring a separate protocol-level cancel command.
- **Broker Authorization Channel** approval creates a grant only while the originating request remains active; late approval after CLI disconnect should not create a grant.
- **Broker Authorization Channel** V1 has at most one active authorization decision at a time; concurrent requests should not create overlapping prompts or grants.
- **Broker Authorization Channel** V1 rejects concurrent authorization requests rather than queuing them.
- **Broker Authorization Channel** runtime endpoints are ephemeral local IPC endpoints, not durable app configuration or broker grant state.
- **Broker Authorization Channel** endpoint discovery should be deterministic within the local user session rather than depending on durable socket path files.
- **Broker Authorization Channel** V1 uses a per-user runtime or temporary-directory Unix domain socket path derived from the configured desktop app identifier, and CLI/server code should not duplicate that identifier as an unrelated hardcoded constant.
- Mnema CLI first attempts the deterministic **Broker Authorization Channel** endpoint, launches Mnema and retries briefly when the endpoint is missing or unavailable, and treats failure after retry as app unavailable unless compatibility fallback succeeds.
- Mnema CLI treats an existing but unconnectable **Broker Authorization Channel** socket path as a stale endpoint, but stale socket unlinking is owned by the desktop app startup/bind path rather than the CLI.
- The legacy broker authorization request file is used only as a V1 compatibility fallback after socket startup or connection retry fails, not as a normal successful authorization path.
- **Broker Authorization Channel** V1 relies on local user socket permissions and user approval rather than a separate shared secret or cryptographic client authentication.
- **Broker Authorization Channel** V1 handles stale or invalid local endpoints as unavailable but does not attempt strong defense against a malicious same-user local process.
- **Broker Authorization Channel** requests and responses use a versioned structured local protocol rather than ad hoc strings.
- **Broker Authorization Channel** requests include schema version, request ID, **Broker Client Identity** label and provenance, command type, minimum and preferred scope, minimum and preferred duration, interactivity, and creation timestamp.
- **Broker Authorization Channel** requests must not include raw query text, snippets, opaque result IDs, app/window titles, browser URLs, media paths, raw database identifiers, or command arguments that reveal retained content.
- **Broker Authorization Channel** grant responses may include safe grant metadata such as grant ID, client identity, scope, and expiry, but no captured content.
- **Broker Authorization Channel** responses include schema version, request ID, and a decision value of approved, denied, or unavailable.
- Approved **Broker Authorization Channel** responses include safe **CLI Access Grant** metadata such as grant ID, client identity, approved scope, and expiry.
- **Broker Authorization Channel** responses must not include captured content or original command arguments.
- **Broker Authorization Channel** decision outcomes are separate from **Brokered Capture Access** data response and error types, with Mnema CLI mapping both layers to user messages and exit codes.
- **Broker Authorization Channel** V1 is macOS-first while keeping the domain boundary abstract enough for future Windows or Linux local IPC implementations.
- The existing broker authorization request file is a compatibility fallback for **Broker Authorization Channel** startup or unavailable-app cases, not the primary authorization flow.
- First **Brokered Capture Access** authorization requires Mnema UI; standalone CLI access may use existing valid grants but should return `authorization_required` when no valid grant exists.
- **Brokered Capture Access** V1 grants are read-only, redacted, time-bounded, revocable, and limited to searchable-content commands such as search, show-text, timeline, and the CLI `open` action.
- **Brokered Capture Access** V1 grants apply to the full V1 broker data command set rather than per-command permissions.
- **Brokered Capture Access** grants may be time-scoped, and **All Retained Broker Scope** requires an explicit user choice in an expanded authorization surface rather than the default one-click prompt.
- **Brokered Capture Access** authorization V1 offers `Last day` and **All Retained Broker Scope** as scope choices, and `1 hour`, `24 hours`, and `7 days` as duration choices; V1 does not offer month-long or permanent grants.
- **Brokered Capture Access** authorization creates reusable grants for the approved **Broker Client Identity**, scope, and duration rather than one-shot command approvals.
- **Brokered Capture Access** grant matching is scoped by **Broker Client Identity** in V1, while making clear that the client identity is locally declared or inferred from non-sensitive environment markers rather than cryptographically attested.
- **Broker Client Identity** inference uses an explicit allowlist of non-sensitive agent markers and must not read, store, or classify token, API-key, session-id, trace-id, or model-name environment values as identity.
- **Brokered Capture Access** grant matching uses the normalized **Broker Client Identity** label; identity source is provenance metadata rather than a matching key.
- **Broker Client Identity** labels are trimmed, whitespace-normalized, control-character-free display strings matched case-insensitively and never used as filesystem paths.
- Explicit invalid **Broker Client Identity** input fails validation, while invalid inferred identity markers are ignored in favor of later fallback identity sources.
- **Brokered Capture Access** grants have opaque stable grant identifiers; identity, scope, and duration are grant attributes used for matching and revocation rather than natural IDs.
- Multiple active **Brokered Capture Access** grants for the same **Broker Client Identity** combine by union of scope until each grant expires or is revoked.
- Broader **Brokered Capture Access** grants coexist with narrower grants for the same **Broker Client Identity** rather than automatically replacing them.
- An active **Brokered Capture Access** grant for the same **Broker Client Identity** should be reused without prompting when the requested command is within the grant scope.
- Mnema CLI should not opportunistically extend an active **Brokered Capture Access** grant during ordinary in-scope commands; duration extension requires an explicit access request.
- Explicit **Brokered Capture Access** time ranges outside the active grant scope should fail with an outside-scope authorization response rather than silently clamping returned results.
- **Brokered Capture Access** scope limits the candidate content considered by broker commands, not only the returned result set.
- Brokered search and timeline date filters are request refinements that must fit inside active **Brokered Capture Access** grant scope; they do not themselves grant broader access.
- Opaque broker result follow-up commands such as show-text and `open` are authorized against current active identity-matched grant scope rather than the original grant that produced the opaque result.
- **Broker Authorization Channel** requests distinguish preferred scope from minimum required scope so the app can offer safe downgrades only when the pending command can still succeed.
- **Brokered Capture Access** returns redacted derived content and opaque identifiers by default, not raw SQLite rows or media file paths.
- The **Broker Authorization Channel** and Mnema CLI access redesign should not expand broker-visible search result content; search response shape remains owned by **Brokered Capture Access** policy.
- **Brokered Capture Access** should use bounded result limits and opaque pagination, and must not provide an unrestricted dump-all searchable text command.
- **Brokered Capture Access** may return full redacted OCR or transcript text only for a specific opaque result identifier within grant scope, not through bulk all-content commands.
- **Brokered Capture Access** must not expose original-media paths by default because agents could use those paths to recover secrets from frame, video, or audio media outside the redacted searchable-text path.
- **Brokered Capture Access** may provide an app-mediated open action for opaque result identifiers so original media inspection stays mediated by app UI warnings and confirmations.
- **Brokered Capture Access** V1 does not include privileged original-media export, media-path return, raw DB dump, or raw OCR/transcript dump commands.
- **Brokered Capture Access** may support app, source, and time refinements, but should minimize returned app/window/browser metadata and avoid returning full browser URLs by default.
- **CLI Access** lives under **Access Settings** for Mnema CLI installation, grant inspection, revocation, and non-content access history; it manages existing grants rather than manually creating new standing grants in V1.
- **Access Settings** owns Mnema CLI installation, reinstall, and status detection controls.
- **Access Settings** should distinguish managed Mnema CLI installation, bundled sidecar availability, shell PATH discovery, and unmanaged `mnema` commands rather than exposing a single installed/not-installed state.
- **Access Settings** may verify discovered Mnema CLI commands with non-prompting version/status checks and short timeouts.
- **Access Settings** should distinguish install, update, repair/reinstall, unmanaged-command warning, and PATH guidance states for Mnema CLI setup.
- Mnema CLI installation should not overwrite an unmanaged existing `mnema` command in V1.
- **Access Settings** may provide PATH guidance for Mnema CLI setup but should not automatically edit shell startup files in V1.
- User-facing Mnema CLI commands and help should avoid `broker` terminology; broker remains internal domain and code language where needed.
- The CLI `open` command is an app-mediated broker command that may launch or focus the desktop app and should re-authorize the opaque result before navigation.
- `mnema-broker` is not a user-facing V1 binary.
- Denied or unavailable **Broker Authorization Channel** responses use stable reason codes such as userCancelled, closed, onboardingRequired, requestSuperseded, busy, timeout, appUnavailable, unsupportedVersion, and invalidRequest.
- The in-app AI chat surface (PI Agent SDK) is a **Brokered Capture Access** consumer, not a new data-access path: it reads retained capture content only through the redacted, retention-aware broker policy/query code that backs the CLI data commands, reusing them rather than reaching app-infra rows or media directly. Redaction applies even though the agent runs inside Mnema because the context it is given leaves the machine to PI's cloud model, which is the same cloud boundary that justifies broker redaction for external agents.
- The in-app **Ask AI** agent's tool surface is exactly the broker data command set (`search`, `timeline`, `show-text`), so the in-app PI tool contract is identical to the external-agent contract documented in the `mnema-data` skill rather than a separate in-app tool API.
