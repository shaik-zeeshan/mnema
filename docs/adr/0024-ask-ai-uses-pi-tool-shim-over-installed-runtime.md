# Ask AI exposes broker tools through a PI tool shim

**Quick Recall**'s **Ask AI** uses the user's installed PI runtime and PI's stored auth, but it does not rely on `pi --mode rpc` to forward model tool calls back to Mnema. PI RPC is a JSONL session-control protocol for prompts, state, model selection, bash/session operations, messages, and slash-command discovery; its current `RpcCommand` union has no host-tool registration or tool-call-result channel. PI's supported custom-tool surface is in-process TypeScript: extension `pi.registerTool(...)` or SDK `defineTool(...)`/`customTools`, whose `execute(...)` callback runs inside the PI runtime.

Therefore the Ask AI wiring is a thin TypeScript PI tool shim for the data tools `search`, `timeline`, and `show-text` (plus the presentation-only `reference_captures` tool added in the amendment below). The shim runs in the installed PI process, loaded by PI's extension/SDK mechanism, and calls back into Mnema's Tauri host commands. Rust remains the enforcement point: the Ask AI broker commands call `BrokeredCaptureAccess::execute_for_ask_ai` with PI's inferred broker identity, so authorization, all-retained-history scope, redaction, opaque IDs, and audit stay in the existing **Brokered Capture Access** path. (`execute_for_ask_ai` is the Ask-AI-specific entry point: unlike the CLI's `execute_for_identity`, it injects a synthetic All-Retained grant — Ask AI is gated by the Ask AI Setting at the Tauri layer rather than a persisted disk grant — and rejects `open_in_mnema`, which is an app-mediated handoff rather than a data tool.) `open` remains an app-mediated dashboard handoff and is not exposed as an Ask AI data tool.

The tool-enabled slice should use PI's SDK/session tooling with custom tools and no built-in coding-agent file/bash tools (`noTools: "builtin"` or an allowlist containing only Mnema custom tools). The seeded-single-answer slice may use the same SDK path with no broker follow-up tools, so both slices share auth/model/session behavior and avoid a second agent runtime. If a future PI release adds first-class RPC host-tool callbacks, this decision can be revisited; the current source does not expose that mechanism.

**Confirmed PI surfaces**

- `packages/coding-agent/src/modes/rpc/rpc-types.ts` defines JSONL RPC commands/responses and extension UI events, but no `register_tool`, `tool_call`, or `tool_call_result` host callback: <https://github.com/earendil-works/pi/blob/main/packages/coding-agent/src/modes/rpc/rpc-types.ts>.
- `packages/agent/src/types.ts` defines `AgentContext.tools?: AgentTool[]` and `AgentTool.execute(toolCallId, params, signal?, onUpdate?)`, confirming tool execution is part of the in-process agent loop: <https://github.com/earendil-works/pi/blob/main/packages/agent/src/types.ts>.
- `packages/coding-agent/src/core/extensions/types.ts` exposes `registerTool<TParams, TDetails, TState>(tool: ToolDefinition<...>): void`: <https://github.com/earendil-works/pi/blob/main/packages/coding-agent/src/core/extensions/types.ts>.
- PI SDK docs show `AuthStorage.create()` / `ModelRegistry.create(authStorage)`, `createAgentSession(...)`, `customTools: [myTool]`, and built-in tool disabling/allowlisting: <https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/sdk.md>.

**Installed PI detection and auth**

Mnema detects PI separately from the bundled `mnema` CLI: unmanaged configured path, future managed runtime path, then terminal `PATH`. The initial implementation reports `PATH` or `missing` because Mnema does not yet install PI. Readiness requires a detected `pi`, `pi --version` at or above `0.65.0` (the PI release line that includes the `defineTool` helper used by the shim), and an auth file at PI's agent auth path. Mnema only checks whether the auth file exists; it does not read, parse, copy, migrate, or store provider credentials.

PI's auth path is PI-owned. Source shows `getAgentDir()` defaults to `~/.pi/agent`, can be overridden with `PI_CODING_AGENT_DIR`, and `AuthStorage.create()` defaults to `join(getAgentDir(), "auth.json")`: <https://github.com/earendil-works/pi/blob/main/packages/coding-agent/src/config.ts> and <https://github.com/earendil-works/pi/blob/main/packages/coding-agent/src/core/auth-storage.ts>. This keeps ADR 0023 intact: Ask AI consumes the user's existing PI stored auth as-is, and Mnema handles no provider credentials.

**Ask AI broker command contract**

The Tauri host exposes three Rust-backed commands for the TypeScript shim:

- `ask_ai_broker_search(request: BrokerSearchRequest)` → `BrokeredCaptureRequest::Search`.
- `ask_ai_broker_timeline(request: BrokerTimelineRequest)` → `BrokeredCaptureRequest::Timeline`.
- `ask_ai_broker_show_text({ opaqueId })` → `BrokeredCaptureRequest::ShowText`.

Each command executes as `BrokerClientIdentity::new("PI", BrokerClientIdentitySource::Inferred)`, matching existing `mnema` CLI identity normalization for PI clients. The response shape is the existing broker response/error envelope, so the shim only translates PI tool parameters/results; it does not implement capture policy.

## Amendment: `reference_captures` is a fourth tool, and it is a presentation signal, not a data tool

The original decision said the agent is bounded to `search`, `timeline`, and `show-text` "with no new tools." That constraint was about **data access** — keeping the redaction/retention boundary tight so the agent can reach no capture data beyond those three broker queries. To let **Ask AI** surface **Answer Sources** (the captures behind an answer, rendered as the dashboard-handoff card strip described in `apps/desktop/CONTEXT.md`), the shim registers a **fourth** custom tool, `reference_captures`, and we clarify that "no new tools" means **no new data-access tools**.

`reference_captures` does not widen that boundary:

- Its input is a list of opaque ids the agent **already received** from `search`/`show-text`; it introduces no way to reach a new capture, frame file, transcript, or app-infra row.
- It **returns no capture data** to the model — only a small acknowledgement (counts of accepted/dropped ids). The model learns nothing it did not already hold.
- The host validates each opaque id's HMAC, drops any that fail, decodes the survivors to their **Captured Frame** / **Audio Transcription Span** identity, and emits them to the panel, which hydrates cards from **local full-fidelity** data — the same non-redacted path **Quick Search** already uses, since the cards render in the user's own app.

So it is the batch, non-navigating sibling of `open`: where `open` is an app-mediated dashboard handoff for one capture, `reference_captures` is an app-mediated *presentation* declaration for a set of captures. Both are app-mediated UI signals, not **Brokered Capture Access** data tools.

Consequences:

- `reference_captures` is **exempt from `access.askAiMaxToolCalls`**, because that cap bounds data-access tool calls and this tool reaches no data; counting it would let a long search session starve the model's ability to declare its evidence.
- It adds **no new access-audit entry**: the nominated captures were already accessed (and audited) through the `search`/`show-text` calls that produced their opaque ids.
- The rendered strip is capped at **6 frame + 4 audio** **Answer Sources** in the model's nomination order (relevance = order, no score); excess nominations are dropped.
- A future PI release with first-class RPC host-tool callbacks does not change this classification: `reference_captures` stays a presentation signal regardless of transport.
