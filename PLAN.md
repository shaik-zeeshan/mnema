# Plan: Provider-centric AI settings and a chat surface that feels like AI

## Problem

AI features are configured in three scattered places: the "Ask AI" switch and a misleadingly named "Quick Recall model" picker live in the Access tab, while the Reasoning Engine and User Context share one oversized card in the Intelligence tab. The engine model is asymmetric — one privileged "default engine" plus bolted-on "Configured Engines" — which forces the model question to be answered three times (engine default model, `askAiModel` override, per-thread pin) with unclear precedence. Meanwhile the Chat surface doesn't feel like interacting with an AI: the composer is a bare textarea with a detached model dropdown that only sometimes renders, there is no visible stop control even though `ask_ai_cancel` exists, and the history rail is a flat undifferentiated list whose "titles" are raw first-question text with always-visible delete buttons and turn-count noise.

## Solution

Flip settings to a provider-centric shape: a flat list of connected providers (Anthropic, OpenAI, OpenAI-compatible, Ollama, Llamafile), each holding only credentials/endpoint, feeding one combined model pool. One global default model is chosen from that pool; model resolution becomes a single precedence chain: **thread pin → feature override → global default**. A master "AI features" switch stays at the top (Mnema's trust story warrants one glanceable kill switch; Wipe User Context keeps flipping it off). Everything consolidates into one settings tab — "AI" — with three cards: Providers, Ask AI, User Context. In Chat, build a rich composer (textarea + integrated bottom bar with an always-visible model picker and a send button that morphs into stop while streaming, with a live activity line above) and upgrade the rail (date grouping, model-generated short titles, hover-revealed rename/delete).

## User Stories

1. As a user, I want to connect AI providers once and pick one default model, so that every AI feature works without re-answering the model question per feature.
2. As a user, I want all AI settings in one tab with clear cards, so that I can reason about what's enabled and what data leaves my device.
3. As a user, I want one master AI switch, so that I can answer "is anything being sent to an AI?" at a glance.
4. As a user, I want the composer to show which model I'm talking to and what the AI is doing, so that chatting feels like interacting with an AI rather than a search box.
5. As a user, I want to stop a streaming answer from the composer, so that I'm not stuck waiting on a bad question.
6. As a user, I want the chat history grouped by date with short readable titles, so that I can find a past thread by scanning.
7. As a user, I want to rename or delete a thread from a quiet hover menu, so that the rail isn't cluttered with destructive buttons.

## Implementation Decisions

### Settings model (Rust + wire types)
- Reshape `AiRuntimeSettings` in `crates/capture-types`: replace the `engineKind`/`cloudProvider`/`cloudModel`/`localKind`/`localEndpoint`/`localModel` + `additionalEngines` shape with `{ enabled: bool, providers: Vec<ProviderConfig>, defaultModel: Option<EngineRef> }`, where `ProviderConfig` carries provider kind + non-secret connection details (base URL/endpoint) and `EngineRef` is `{ provider, model }` (the same shape conversation pins already use).
- Legacy deserialization is back-compat: an old settings file with a default engine + `additionalEngines` deserializes into the providers list, with the old default engine's `{provider, model}` becoming `defaultModel`. Saves write only the new shape.
- The OS-keychain key store (`crates/app-infra/src/ai_provider_key_store.rs`) is already keyed by provider id — unchanged. Multiple configured engines on the same provider already shared a key; the provider list makes that explicit.
- Model resolution precedence is one function in the Tauri layer: thread pin → feature override (`access.askAiModel` for Ask AI; a User Context override if/when set) → global `defaultModel`. `resolve_engine_config_for_pin` and `read_ask_ai_model` collapse into this single resolver.
- The master switch keeps ADR 0033's two-layer gating, relocating the bottom layer: `engine_configured_prerequisite` becomes "master switch on + at least one usable provider + a default model chosen." Availability reason codes update accordingly (e.g. `no_default_model`, `no_provider_key` carrying which provider).
- Wipe User Context still flips the master switch off, exactly as today.
- `ai_runtime_list_models` enumerates models per connected provider and merges into the pool, tagging each model with its provider; the settings default-model picker, the Ask AI override picker, and the Chat thread picker all consume the same pool. Free-form model-id entry stays allowed everywhere.

### Settings UI (`apps/desktop/src/routes/settings/+page.svelte`)
- One tab named "AI" replaces the Intelligence tab's AI content. Three cards, in order:
  1. **Providers** — master "AI features" switch at top; the provider list (add/remove provider, key save/clear with keychain badge, endpoint/base-URL fields, per-provider test connection); the **global default model** picker at the bottom of the card (combobox over the merged pool, free-form allowed).
  2. **Ask AI** — opt-in switch, privacy disclosure, tool-call limit, and an optional **"Model override"** picker (renamed from "Quick Recall model"; it governs Quick Recall *and* Chat). Availability status pill stays here.
  3. **User Context** — opt-in switch, status, Derivation Budget tier (cloud default model only), History Backfill, recent Activities/Conclusions, Run-now/Refresh, Wipe.
- Ask AI leaves the Access tab entirely; Access returns to brokered/agent capture access only.
- API key save/clear remains an explicit invoke, never the keystroke-debounced autosave (existing rule).
- Settings cards stay inline in `settings/+page.svelte` per existing convention (`SettingsCard.svelte` is a known trap); the mount `$effect` must keep its loaders untracked.

### Rich composer (`apps/desktop/src/lib/insights/Chat.svelte`)
- One bordered composer block: textarea on top, slim bottom row inside the block with the **model picker on the left** (always rendered, label shows the resolved model, e.g. "Default · claude-haiku-4-5") and the **send button on the right, morphing into a stop button** while a turn is streaming. Stop invokes `ask_ai_cancel` (first UI exposure of an existing command).
- The existing floating "Model" dropdown above the composer is removed; its pin logic (`set_conversation_engine`, pinnable list, custom model entry) moves into the composer picker unchanged in behavior.
- A live **activity line** renders just above the composer fed by `ask_ai_status` phases: seeding → thinking → tool (with tool name), clearing on `done`/`error`.
- No scope/context chips in the composer: the agent decides its own retrieval; show what it *did* (existing tool chips in the transcript), don't pre-declare what it'll search.

### Chat rail
- **Date grouping**: section headers Today / Yesterday / This week / earlier months, replacing the flat list. Grouping is computed client-side from existing timestamps.
- **Generated titles**: after a thread's *first* turn completes, the backend makes one cheap `extract` call against the resolved default model for a 3–6 word title and persists it to the conversation store, emitting `conversation_changed`. Skips silently when the engine is unavailable or the call fails; fallback remains first-question truncation. The title call is fire-and-forget after the turn persists — it must not delay or fail the turn.
- **Quieter rows**: drop the turn count; delete moves behind hover, joined by a **rename** action (inline edit) backed by a `set_conversation_title` Tauri command (added if absent). A user-set title is never overwritten by the generator.

## Testing Decisions

- Rust unit tests for `AiRuntimeSettings` legacy deserialization: old default-engine + `additionalEngines` files resolve into providers + `defaultModel`; new-shape round-trips.
- Rust unit tests for the single model resolver: pin beats override beats default; missing default yields the right availability reason.
- Verify Wipe User Context still flips the master switch off (existing test updated, not removed).
- Title generation: test that a failed/unavailable title call leaves the fallback title and never fails the turn.
- `bun run check` for all Svelte changes; `cargo check -p capture-types`, `cargo test -p app-infra ai_provider_key_store`, and full `cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml` for the cross-stack settings change (sidecar must be prepared first per AGENTS.md).
- Manual checks: settings migration from an existing `recording-settings.json`; stop button mid-stream; rail grouping around midnight boundaries.
- Do not test Svelte internal state shapes; test through saved settings payloads and rendered behavior.

## Slices

1. **Settings model reshape + resolver (Rust)**
   - Goal: provider-list `AiRuntimeSettings`, legacy deserialization, single model resolver, updated availability reasons.
   - Areas: `crates/capture-types`, `apps/desktop/src-tauri/src/ai_runtime.rs`, `ask_ai.rs`, `user_context/`.
   - Acceptance: back-compat + resolver tests pass; `ai_runtime_list_models` returns provider-tagged pool.
   - Depends on: none. Parallel: yes, with 4 and 5.
2. **Settings UI: AI tab with three cards**
   - Goal: consolidate Providers / Ask AI / User Context into one tab; remove Ask AI from Access; rename override label.
   - Areas: `apps/desktop/src/routes/settings/+page.svelte`.
   - Acceptance: `bun run check`; saves land in the right domains; key save stays explicit.
   - Depends on: slice 1 wire types. Parallel: no (after 1).
3. **Title generation + rename command (Rust)**
   - Goal: post-first-turn title `extract`, `set_conversation_title` command, user-title precedence.
   - Areas: `apps/desktop/src-tauri/src/ask_ai.rs`, conversation store in `crates/app-infra`, `lib.rs` registration.
   - Acceptance: title persisted + `conversation_changed` emitted; failure leaves fallback; turn never blocked.
   - Depends on: slice 1 (resolver). Parallel: yes, with 2.
4. **Rich composer**
   - Goal: bordered composer with integrated model picker, morphing stop button wired to `ask_ai_cancel`, activity line.
   - Areas: `apps/desktop/src/lib/insights/Chat.svelte`.
   - Acceptance: `bun run check`; stop cancels mid-stream; picker shows resolved model when unpinned.
   - Depends on: slice 1 only for the resolved-model label (can stub with current pin logic). Parallel: yes.
5. **Chat rail upgrade**
   - Goal: date grouping, generated/renamable titles in rows, hover actions, drop turn count.
   - Areas: `apps/desktop/src/lib/insights/Chat.svelte`.
   - Acceptance: `bun run check`; groups correct; rename persists; delete behind hover.
   - Depends on: slice 3 for rename/title backend. Parallel: rail layout can start immediately.

Parallel groups: [1, 4, 5-layout], then [2, 3], then [5-wiring].

## Out of Scope

- Thread pinning/favorites in the rail.
- Segregating Quick Recall threads from chats — a promoted thread is just a chat.
- Scope/context chips in the composer (pre-declared retrieval scope).
- Per-feature *required* model pickers (rejected in favor of the global default + optional overrides).
- The rich Insights surfaces (issue #89) and any change to the Brokered Capture Access tool set or redaction boundary.
- Quick Recall window's own input UX (only its model-override label/semantics are touched via settings).

## Further Notes

- The decision is recorded in [ADR 0034](docs/adr/0034-ai-settings-are-provider-centric-with-one-global-default-model.md), which amends ADR 0033's points 5–6 (engine set → provider list; engine-configured prerequisite re-grounded under the retained master switch). The AGENTS.md bullets describing `AiRuntimeSettings`, `read_ask_ai_model`, and the Configured Engines picker will be stale after slice 1–2 and must be updated in the same PR as the implementation.
- Settings migration is deserialization-level, not a data migration: nothing to run at startup, but the first save after upgrade rewrites the new shape — worth a log line.
- Title generation adds one small engine call per new thread; it reuses the guardrail-free Ask AI path (titles see only the user's question text, no capture data), so no new redaction surface.
