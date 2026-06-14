# Ask AI Streaming — Agent Reference

Deep-dive on the backend-owned chat streaming contract shared by both Ask AI doors
(Quick Recall + Insights Chat). Referenced from `CLAUDE.md`. See also
[ADR 0033](../adr/0033-ask-ai-migrates-onto-shared-reasoning-engine.md) and
[ADR 0031](../adr/0031-quick-recall-and-chat-share-one-persistent-conversation-store.md).

---

## Ownership boundary (the core invariant)

**The backend owns ALL chat turn state and logic; the frontend only renders.**

`ask_ai.rs` runs the agent loop, executes tools, parses the answer into typed
blocks, formats tool labels, resolves app-icon paths, and holds the one
authoritative, render-ready **`TurnView`** per in-flight turn in memory. The
frontend (`Chat.svelte`, `routes/quick-recall/+page.svelte`) holds a copy of that
`TurnView`, applies versioned updates, and renders each block by `kind`. It does
**no** fence parsing, tool-label formatting, icon-cache batching, or phase
machine.

The single exception that stays frontend-side: **Markdown → HTML rendering** (it
is genuinely the render step, and the XSS hardening — link allowlist, html-escape,
image strip — lives in `AnswerProse.svelte` / `markdown.ts`). So `AnswerBlock::Prose`
carries RAW `markdown`, never HTML.

If you find yourself adding parsing/label/format logic to the frontend, it is in
the wrong layer — put it in the backend view model.

---

## Wire shapes (hand-mirrored — there is NO TS codegen)

Defined in `crates/capture-types/src/conversation.rs`, mirrored BY HAND in
`apps/desktop/src/lib/insights/conversation.ts`. Drift is silent; the only guards
are the serde round-trip / exact-shape tests in `conversation.rs` and
`bun run check`. Keep both green on every type change.

- **`AnswerBlock`** — `#[serde(tag = "kind")]`: `Prose { markdown }`, `Bars { title, items }`,
  `Dossier { items }`, `Timeline { title, items }`. Item structs: `BarsItem`,
  `DossierItem`, `TimelineItem` (ported from the old `Chat.svelte` parser).
- **`ToolActivityEntry`** — `{ kind, label, app, appIconPath }`. Backend supplies the
  humane `label` AND the resolved icon **filesystem path**; the frontend's only
  transform is `convertFileSrc(appIconPath)` (a pure path→URL helper).
- **`TurnView`** — `{ turnIndex, question, phase, blocks, reasoning, toolActivities,
  liveActivity, sources, errorMessage, seededResultCount }`. `phase` is one of
  `seeding | thinking | streaming | done | error`. `sources` is opaque JSON the
  frontend round-trips into source cards.
- **`TurnSnapshot`** — `{ conversationId, version, view }`.
- **`TurnUpdate`** — `#[serde(tag = "op")]`: `Phase`, `AppendProse { text }`,
  `OpenBlock { block }`, `Reasoning { text }`, `ToolActivity { entry }`,
  `LiveActivity { entry }` (None clears the live line), `Sources { sources }`,
  `Error { message }`, `Done`.

---

## Transport — one versioned event + one snapshot command

- **`ask_ai_update`** event = `{ conversationId, version, turnIndex, update }`, emitted
  once per op. `version` is monotonic per turn (assigned under the `LiveTurn` lock).
- **`ask_ai_snapshot(conversationId) -> Option<TurnSnapshot>`** command returns the
  current in-memory `LiveTurn` view + version, or `None` when no turn is live (the
  frontend then falls back to the persisted DB turn via `get_conversation`).
- There is NO legacy `ask_ai_status/delta/reasoning/done/error/source` surface — it
  was deleted once both doors migrated. Streaming is low-latency via the structured
  deltas (`AppendProse` / `OpenBlock`); the version + snapshot is the safety net,
  not the hot path.

### Self-healing reattach (the reliability fix)

The frontend applies an update only when `version === last + 1`. On a gap (it was
detached) it re-fetches a snapshot and adopts `snapshot.view` + `snapshot.version`;
on `version <= last` it ignores (stale/duplicate). On attach to a conversation it
calls `ask_ai_snapshot` to bootstrap a live turn race-free. This closes the
lost/duplicated-token races that plagued the old stateless stream — missing an
event can no longer lose data.

The frontend's `applyUpdate(turn, update)` reducer MUST stay a 1:1 mirror of the
Rust `apply_update_to_view` (in `ask_ai.rs`), especially the `AppendProse`
coalescing (append to the trailing prose block, else start a new one), or streaming
and reload will diverge.

---

## LiveTurn store, ownership, and always-terminal

- In-memory `Mutex<HashMap<conversationId, LiveTurn>>` (module-level static in
  `ask_ai.rs`, mirroring the `ASK_AI_INFLIGHT` registry). `LiveTurn { view, version,
  turn_token }`.
- **`turn_token`** (a process-global `AtomicU64`) is the per-turn ownership identity,
  the `LiveTurn` analogue of the inflight `cancel` Arc. A newer turn for the same
  conversation OVERWRITES the map entry; the displaced older turn must not mutate or
  evict the newer turn's `LiveTurn` (`apply_live_update` / `remove_live_turn_if_owner`
  are token-guarded, exactly like `remove_inflight_if_owner`).
- **A terminal update (`Done` / `Error`) is emitted on EVERY exit path** — clean
  finish, error, cooperative cancel, displacement, and seeding-phase cancel — so the
  UI's "Writing…" indicator always settles. A displaced turn (whose `LiveTurn` was
  overwritten) emits its terminal DIRECTLY at `last_version + 1` for its own
  `turnIndex`, continuing that turn's sequence with no gap. Early-return failures
  (storage missing, engine unresolved) that happen before `LiveTurn` registration
  emit a direct `ask_ai_update` error terminal so the failure is still visible.

The per-conversation single in-flight turn rule (the `register_inflight`
displacement rule) is unchanged; displacement now additionally emits the terminal.

---

## Streaming answer parser (`ask_ai/answer_view.rs`)

`AnswerView` accumulates raw answer text and maintains an **append-only**
`Vec<AnswerBlock>`, returning the minimal `Vec<TurnUpdate>` ops per delta. The
mutable tail is either a growing `Prose` block or a PENDING open `mnema-*` fence
held back (not emitted) until its closing ``` ``` ``` arrives.

- Valid closed fence → `OpenBlock`. **Malformed JSON → degrade to prose** (emit the
  raw fenced text). Unterminated fence at `finalize()` → degrade to prose. This
  matches the old silent fallback (renders as a code block, no visible regression).
- The parser is delta-split safe: a fence opener/closer or JSON body arriving in
  pieces buffers until it can decide. Only `mnema-bars` / `mnema-dossier` /
  `mnema-timeline` info strings are intercepted; any other fence is ordinary prose.
- **Op-replay equivalence** is the load-bearing test: applying the emitted ops ==
  parsing the full text == feeding it char-by-char. This is what guarantees a live
  stream and a cold reload can never diverge. Preserve it.

Tool labels + icons are ported server-side in `ask_ai/tool_activity.rs`
(`format_tool_activity` mirrors the old `formatToolActivity`; icon paths resolve via
`native_capture::resolve_app_icons`).

---

## Persistence + cold reattach

- Migration `0036_conversation_turn_blocks.sql` adds a nullable `blocks` (JSON)
  column to `conversation_turns`. `ConversationTurn` DTO gains
  `blocks: Option<Vec<AnswerBlock>>`.
- Every persist (initial, throttled partial, finalize) writes the parsed
  `Vec<AnswerBlock>` alongside the raw `answer`. Raw `answer` is retained — the agent
  loop's history contract still feeds plain text.
- `blocks = NULL` means a LEGACY turn predating the column. `blocks = []` (Some,
  empty) means a parsed turn with no blocks. The store NEVER parses (`app-infra`
  stays a pure store).
- **Cold reattach / legacy parse-on-read happens in the DESKTOP `get_conversation`
  Tauri command** (`conversation/commands.rs`), which can reach the parser: for any
  turn with `blocks == NULL` and a non-empty answer, it populates
  `blocks = parse_answer_to_blocks(answer)` once on read. So the frontend receives
  populated `blocks` for EVERY turn (live, reattached, or reloaded after restart)
  and never parses — charts/sources/tool steps render identically across all three.

---

## When changing this surface

- Touch the wire shapes? Update `conversation.rs` AND `conversation.ts` together and
  keep the serde round-trip test + `bun run check` green.
- Add a new op? Add it to the `TurnUpdate` enum, the Rust `apply_update_to_view`
  reducer, AND the frontend `applyUpdate` reducer (both doors) — they must agree.
- Both doors (`Chat.svelte` and `quick-recall/+page.svelte`) consume the identical
  surface; change them together.
