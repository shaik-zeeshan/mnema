# MCP connectors: a desktop-side `rmcp` client, trusted per server

## Status

Proposed.

## Context

Ask AI can read capture data through the broker tools but cannot act on the
user's own services — GitHub, Linear, a filesystem, anything the user runs.
MCP (Model Context Protocol) is the ecosystem standard for exactly this: a user
points the agent at a server (stdio or streamable-HTTP) and its tools join the
conversation.

Two shaping constraints. First, the security posture is locked (no bash tool, no
open web search) and `crates/ai-runtime` must stay a provider-agnostic engine
that is **ignorant of what the tools are** ([ADR 0033](0033-ask-ai-migrates-onto-shared-reasoning-engine.md)).
Second, `rig` ships an MCP feature, but its `ToolServer` typestate **disables
`.tools()`** — you cannot enumerate a server's tools, which the tool budget and
the curation modal both require. So the engine's own MCP path is a dead end for us.

## Decision

1. **Desktop drives `rmcp` directly.** `rmcp = "2.1"` (cargo-resolved 2.1.0; the
   `"1.7"` in the original plan was stale) is a direct dependency of the desktop
   crate, with the client-side features
   `client, macros, transport-child-process,
   transport-streamable-http-client-reqwest, transport-io, transport-async-rw`
   (`default-features = false`, so no server-side code and no `native-tls` — the
   HTTP transport rides the workspace's rustls `reqwest`). A persistent
   `McpManager` lives in Tauri managed state.
   `crates/ai-runtime` is **untouched** and rig's `rmcp` feature stays **off** —
   MCP tools are injected into the agent loop as callbacks from the Tauri layer,
   exactly like the broker tools, keeping the engine MCP-ignorant.

2. **Trust-per-server.** Enabling a server in Settings **is** the consent; there
   are no per-call prompts. The single executor MCP-dispatch choke-point is
   marked in code as the seam where a per-call approval hook inserts later.

3. **Instance-id identity, per [ADR 0035](0035-provider-identity-is-a-per-instance-id-not-the-kind.md).**
   Each server has a stable instance `id`, slugified `[a-z0-9-]` from its label
   and suffixed on collision (`github`, `github-2`). That id keys the keychain
   account, the `mcp__<id>__<tool>` model-facing tool prefix, and the settings
   row. **An MCP server is never called a "provider"** — that word means an
   *inference* provider ([ADR 0034](0034-ai-settings-are-provider-centric-with-one-global-default-model.md)/0035);
   an MCP server is a **tool connector**. A per-server `enabled` toggle means
   disabling ≠ deleting: the secret and the tool curation survive a disable.

4. **One optional secret per server**, in the OS keychain keyed by instance id —
   a sibling of `ai_provider_key_store.rs`, the same store pattern. Delivery:
   HTTP → `Authorization: Bearer`; stdio → an env var named by the user-supplied
   `secret_env_name` (e.g. `GITHUB_TOKEN`). Non-secret env rows stay plain
   settings values. Multi-secret servers are a documented deferral.

5. **Lazy connect, warm-on-open discovery.** **Nothing spawns at app launch** —
   the deferred-startup invariant holds, so no `npx` child boots for a server
   never used this session. On chat-surface open (either door) the manager
   background-connects and `list_tools` for enabled servers; a turn build awaits
   *in-flight* discovery ≤ ~15 s (npx cold boot) then proceeds with the ready
   servers only. Handles are cached per app session. Failure policy (amended
   2026-07-04): drop the handle and redial **once**, but ONLY for failures that
   provably happened before the request reached the server — a connect failure
   or a transport-send failure (rmcp `ServiceError::TransportSend`). A failure
   after the request may have arrived (timeout, transport closed while awaiting
   the reply, server-reported error) returns error text to the model with **no**
   auto-retry: retrying there can double-fire a non-idempotent write tool
   (create-issue, send-message), and a silent duplicate side effect is worse
   than a visible error. A second consecutive failure likewise returns error
   text; the remedy is the server's `enabled` toggle. Every tool call is also
   bounded by a flat **60 s execution budget** (discovery keeps its 15 s): on
   timeout the handle is dropped so the next call dials fresh instead of queuing
   behind a hung server, and the timeout surfaces as error text with no
   auto-retry (the request may have landed — see above). Not configurable per
   server until a real connector proves slower. A Settings edit re-dials
   that server by id; app exit kills its children — the whole **process group**
   (amended 2026-07-04), because the documented spawn mechanism is `npx`, a
   launcher whose real server is a grandchild: killing only the direct child
   orphans a running server still holding its secret. Group-kill rides
   `process-wrap`'s `ProcessGroup` (already an rmcp dependency) and is
   Unix-only; Windows needs its `JobObject` sibling when that platform becomes
   real (SUPPORTS.md).

6. **A 32-tool budget with curation.** `enabled_tools: None` → offer the first 32
   tools in server order; the trim is **non-silent** (a tracing log plus one
   preamble line "server X exposes N tools; first 32 available"). `Some(list)` →
   exactly those tools, **no cap**. Tool results are truncated at ~24k chars with
   a visible marker, so one rogue tool cannot flood a turn.

## Considered options

- **Use rig's `rmcp` feature.** Rejected — its `ToolServer` typestate disables
  `.tools()`, so the 32-cap and the curation modal (both of which need the tool
  list) are impossible; and it would pull the `rmcp` dependency *into*
  `crates/ai-runtime`, coupling the provider-agnostic engine to a tool-transport
  concern. Driving `rmcp` from the desktop layer keeps the engine untouched.
- **Per-call approval prompts.** Rejected for v1 — trust-per-server matches how
  users already reason about connecting a service, and per-call friction on every
  tool would make chat unusable. The hook seam is marked so it can be added.
- **Call an MCP server a "provider."** Rejected — "provider" is the
  inference-provider term (0034/0035), whose per-instance identity model this
  reuses; overloading the word would collide with that identity vocabulary. It is
  a *tool connector*.
- **Eager connect at launch.** Rejected — violates the deferred-startup
  invariant and boots child processes for servers a session never touches.
- **Multi-secret per server.** Deferred — one optional secret covers the bearer
  token / single-env-var case that HTTP and stdio servers overwhelmingly use.

## Consequences

- **Identity flows through 0035.** The same "instance id keys everything"
  discipline (keychain account, tool prefix, settings row) extends from inference
  providers to tool connectors. The `[a-z0-9-]` slug rule is **load-bearing**:
  the executor routes by parsing `mcp__<id>__<tool>` on the first `__`, so the
  charset must be enforced at add time or routing breaks.
- **`crates/ai-runtime` is unchanged.** The reasoning engine stays
  provider-agnostic and MCP-ignorant (0033); MCP tools arrive as Tauri-layer
  callbacks alongside the broker tools.
- **Deferred, documented:** ~~OAuth for HTTP servers~~ (now shipped as an auth
  mode on the `Http` transport — [ADR 0051](0051-mcp-oauth-is-an-auth-mode-on-the-http-transport-dcr-only-over-the-deep-link.md));
  health checks / auto-restart / backoff / a live per-server status indicator;
  `tools/list_changed` reactivity; disk-cached tool lists. A dead server surfaces
  as a readable tool error the model relays; the remedy is the `enabled` toggle.
  Add the operational polish when real users are confused by dead-server
  behavior, not before.
- **Token cost is the control surface.** Each MCP tool costs ≈ 200–400 prompt
  tokens per turn; the 32-cap plus the curation modal is how the user bounds it.
- **macOS-first.** stdio spawn/kill is cross-platform via `rmcp`/tokio, but
  teardown of the full process group is Unix-only and only macOS is exercised
  on this branch (SUPPORTS.md).

Reuses the per-instance identity of [ADR 0035](0035-provider-identity-is-a-per-instance-id-not-the-kind.md)
and the shared engine of [ADR 0033](0033-ask-ai-migrates-onto-shared-reasoning-engine.md).
