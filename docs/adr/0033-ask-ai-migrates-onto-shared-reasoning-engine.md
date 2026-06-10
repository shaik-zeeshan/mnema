---
status: accepted
---

# Ask AI (Quick Recall + Chat) migrates onto the shared Rust-side Reasoning Engine, retiring the PI shim

Mnema had **two** ways to talk to a language model: **Quick Recall** and **Insights Chat** drove the
user's installed **PI** runtime through a bundled Node shim ([ADR 0023](0023-ask-ai-delegates-auth-to-installed-pi.md),
[0024](0024-ask-ai-uses-pi-tool-shim-over-installed-runtime.md), [0026](0026-ask-ai-multi-turn-reuses-live-pi-session.md)),
while the **Reasoning Engine** for **User Context** derivation called models in-process via `rig-core`
([ADR 0028](0028-ai-features-call-models-rust-side-via-rig-core.md)). Two runtimes, two credential
stories, two model-selection settings. ADR 0028 already named the destination — *"User Context is the
first feature on this path; Ask AI migrates onto it later"* — and [ADR 0031](0031-quick-recall-and-chat-share-one-persistent-conversation-store.md)
already said both Quick Recall and Chat *"run the same Rust-side rig-core engine."* This decision is
that migration: **Ask AI moves onto the shared `rig-core` Reasoning Engine and the PI/Node shim is
deleted**, so there is one way to interface with AI, in one process, behind one redaction/audit
boundary.

## Decision

1. **One engine.** Quick Recall and Chat call models in-process through `rig-core`, the same engine
   User Context derivation uses. The PI/Node shim is removed entirely: `pi_agent_session.rs`, the
   bundled `pi-ask-ai-shim.mjs` resource, the `MNEMA_PI_*` env contract, PI detection / version
   gating, and `ask_ai_list_models`' PI list-mode. The "first install PI/Node" prerequisite — the
   exact dependency 0028 moved to escape — is gone for every AI feature.

2. **The engine crate grows a tool-agnostic streaming agent loop.** `crates/ai-runtime` gains an
   interactive agent/tool loop (streamed tokens, model-driven tool calls, cancellation, a tool-call
   cap) alongside its existing batch `extract::<T>()`. Both share `EngineConfig`, the keychain
   credential, provider wiring, and connection probing. The loop stays **ignorant of what the tools
   are**: the **Brokered Capture Access** tools (`search` / `timeline` / `show_text` /
   `recall_context`), redaction, audit, and `reference_captures` presentation remain in the Tauri
   layer and are injected as callbacks — preserving ADR 0028's line that the engine depends only on
   `rig-core` and the broker is supplied by the Tauri layer.

3. **Shared key, model split per workload.** One bring-your-own-key in the OS keychain (the **Capture
   Index Key Store** boundary) powers both background derivation and interactive chat. Model
   *selection* stays split: the background-derivation model lives in `AiRuntimeSettings`, and
   `access.askAiModel` survives but is **reinterpreted** from a PI `provider:id` into a `rig-core`
   model id (empty → the engine default); `access.askAiMaxToolCalls` survives unchanged. This lets a
   user run a cheap/local model for continuous background work and a stronger model for the rare
   interactive question on the same provider account.

4. **Model discovery from the authoritative per-provider source.** A `rig-core`/HTTP capability lists
   models from the provider that owns them — OpenAI and OpenAI-compatible (and Anthropic) via
   `GET /v1/models`, native Ollama via `GET /api/tags`, single-model local servers via what they
   report — replacing the PI registry. Discovery only *populates* the picker; **free-form model-id
   entry stays allowed**, so a failed or absent list endpoint never blocks selection.

5. **Per-conversation engine override.** A Chat thread may pin an engine identity (`{provider,
   model}`, not a bare model id) so a local-Ollama thread and a cloud-Anthropic thread can coexist,
   each resolving the right endpoint/key. The picker chooses among **already-configured** engines;
   adding a brand-new provider or key stays a Settings action — the picker is not a key-entry surface.
   `AiRuntimeSettings` therefore grows from a single engine selection into a small *set* of configured
   engines. Unpinned threads and all Quick Recall use the global default engine.

6. **Two-layer gating.** A shared **engine-configured** prerequisite (a key present or a reachable
   local endpoint, and a model selected — the old Reasoning Engine "master toggle" *becomes* this)
   sits beneath **two independent feature opt-ins**: the **Ask AI Setting** gates interactive
   Quick Recall/Chat (low consent bar — runs only when asked), and the **continuous-derivation
   opt-in** gates the 24/7 background derivation (high bar — ongoing redacted egress + cost). User
   Context's on/off is the continuous-derivation opt-in, no longer the shared prerequisite.

7. **Stateless-per-turn; resident-session machinery deleted.** Because the model now lives in-process
   and conversations already persist (ADR 0031), a "session" is just the saved thread. Every turn —
   first question or follow-up — loads the thread's history from the conversation store, runs the
   agent loop, streams, and persists the new turn. The resident-session registry, the follow-up
   routing channel, the 30-minute unseen expiry, and resurrect-from-transcript are all deleted. One
   per-*in-flight-question* task is kept, for streaming and cancellation. **Background completion** is
   retained but reframed onto persistence: a dismissed-but-streaming question (in either door) finishes
   its task and writes the turn to the store; re-opening reads it back. Only an explicit cancel stops a
   task; app exit aborts in-flight tasks (a partial turn at most).

8. **Live reattach mid-answer.** Returning to a thread that is still generating shows it live: the
   in-flight task persists incremental partial progress, and the reopened surface loads that partial
   then subscribes to ongoing `delta` events for the thread (events already carry a `conversationId`).

## Considered Options

- **Keep both engines.** Rejected — it is the two-runtimes problem this decision exists to remove, and
  contradicts the destination 0028/0031 already set.
- **Migrate but keep PI for interactive only** (rig-core for derivation, PI for chat). Rejected — it
  keeps the Node prerequisite and a second credential/auth path forever, for the half of the product
  that is most user-facing.
- **One unified AI setting for everything** (a single model and a single on/off for both background
  derivation and chat). Rejected — ADR 0028 deliberately made continuous cloud derivation its own
  opt-in with its own disclosure; fusing them either over-exposes chat-only users to continuous egress
  or forces one model across two workloads with very different cost profiles.
- **Keep the warm resident session for follow-ups** (re-attach to an in-memory session instead of
  re-sending history). Rejected — it is the PI-process scaffolding we are removing; with an in-process
  engine and a persistent store the per-turn rehydration is simple and robust, and the extra tokens on
  a follow-up are acceptable for chat-length threads.

## Consequences

- **Supersedes [ADR 0023](0023-ask-ai-delegates-auth-to-installed-pi.md),
  [0024](0024-ask-ai-uses-pi-tool-shim-over-installed-runtime.md), and
  [0026](0026-ask-ai-multi-turn-reuses-live-pi-session.md)** — PI delegation, the PI tool shim, and
  live-PI-session reuse are all retired. It **supersedes the mechanism of
  [ADR 0027](0027-ask-ai-threads-complete-in-background-and-resurrect-from-transcript.md)** (resident
  session + resurrect-from-transcript) while keeping its *intent* — background completion — reframed
  onto the ADR 0031 persistent store. It **completes [ADR 0028](0028-ai-features-call-models-rust-side-via-rig-core.md)**
  and builds on **[ADR 0031](0031-quick-recall-and-chat-share-one-persistent-conversation-store.md)**.
- **Credential migration cost.** A user whose Ask AI worked purely on PI's own `auth.json`, who never
  set a Reasoning Engine key, will find Quick Recall/Chat unavailable until they paste a key into the
  keychain — PI was the only thing holding their credential. This is disclosed at the same place the
  "engine configured" prerequisite is surfaced.
- **The privacy boundary is unchanged.** Both doors still reach capture data only through **Brokered
  Capture Access** (redaction, retention, opaque ids, audit, All-Retained scope); moving the agent
  loop in-process changes *where the model runs*, not *what it is allowed to see*. A cloud engine still
  sees only redacted text, never frames or audio.
- **Follow-ups re-send thread history** to the model each turn (no in-memory warmth), a modest token
  increase traded for deleting the resident-session lifecycle.
- **Local engines now serve chat too.** Tool-calling reliability on local models is weaker than cloud;
  a local-engine user gets interactive Q&A subject to that limitation rather than a separate path.
