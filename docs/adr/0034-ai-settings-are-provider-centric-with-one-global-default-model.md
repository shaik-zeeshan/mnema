---
status: accepted
---

# AI settings are provider-centric: a flat provider list, one global default model, one resolution chain

[ADR 0033](0033-ask-ai-migrates-onto-shared-reasoning-engine.md) put every AI feature on the shared
Rust-side **Reasoning Engine**, and in doing so grew `AiRuntimeSettings` "from a single engine
selection into a small *set* of configured engines": one privileged **default engine** (the
Cloud-vs-Local selection with its provider, key, and model) plus bolted-on extra engines for the
per-thread picker. That shape forces the user to answer "which engine is THE engine?" before they
have decided what any feature should use, and it answers the *model* question three separate times —
the default engine's model (Intelligence tab), the `access.askAiModel` override (Access tab, labeled
"Quick Recall model" even though it governs Chat too), and the per-thread pin — with no stated
precedence. The settings for one feature ended up split across two tabs, and "Configured Engines"
duplicated provider credentials conceptually owned elsewhere.

This decision flattens the shape: **providers, not engines, are the unit of configuration**, exactly
one **global default model** is chosen from the combined pool, and every model decision resolves
through one precedence chain.

## Decision

1. **Provider-centric configuration.** `AiRuntimeSettings` holds a flat list of connected
   **providers** (Anthropic, OpenAI, OpenAI-compatible, Ollama, Llamafile), each carrying only its
   non-secret connection details (base URL / endpoint); the secret stays in the OS keychain per
   provider id (the **Capture Index Key Store** boundary, unchanged — it was already keyed by
   provider). There is no privileged default engine and no separate "additional engines" list; an
   *engine identity* is just `{provider, model}`, the same shape conversation pins already use.

2. **One global default model.** The user picks a single **default model** from the merged pool of
   models discovered across connected providers (free-form model-id entry stays allowed, per ADR
   0033's discovery rule). Ask AI, User Context derivation, digests, and unpinned Chat threads all
   inherit it.

3. **One resolution chain.** Every model decision resolves as **thread pin → feature override →
   global default model**, implemented as a single resolver in the Tauri layer. ADR 0033's
   per-workload model *split* survives as the optional feature-override layer — `access.askAiModel`
   remains the Ask AI override (relabeled "Model override"; it applies to Quick Recall *and* Chat) —
   but overrides are optional refinements, not required choices. A fresh user makes exactly one
   model decision.

4. **The master switch stays.** A single "AI features" switch sits above everything. Mnema's product
   is trust in what leaves the device, and one glanceable answer to "is anything being sent to an AI
   right now?" is worth one extra toggle. **Wipe User Context** keeps flipping this switch off.
   ADR 0033's two-layer gating is preserved with its bottom layer re-grounded: the shared
   prerequisite becomes *master switch on + at least one usable provider (key present or reachable
   endpoint) + a default model chosen*, and the two independent feature opt-ins (the **Ask AI
   Setting**, the continuous-derivation opt-in) sit beneath it unchanged.

5. **One settings surface.** All AI configuration lives in a single settings tab ("AI") with three
   cards: **Providers** (master switch, provider list with key save/clear and per-provider test
   connection, the global default model picker), **Ask AI** (opt-in, privacy disclosure, tool-call
   cap, optional model override), and **User Context** (opt-in, budget tier, backfill, derived-data
   surfaces, wipe). Ask AI leaves the Access tab, which returns to brokered/agent capture access
   only; User Context leaves the Reasoning Engine card and stands alone.

6. **Migration is deserialization-level.** A legacy settings file (default engine + additional
   engines) deserializes into the provider list, with the old default engine's `{provider, model}`
   becoming the global default model. Saves write only the new shape; no startup data migration.

## Considered Options

- **Keep the default-engine + extras shape** and only fix labels/tab placement. Rejected — the
  asymmetry *is* the confusion: it has no answer to "which model does this feature actually use?"
  that fits in one sentence.
- **Per-feature required model pickers** (no global default; Ask AI and User Context each demand a
  model, since their workloads differ). Rejected — it keeps asking the model question twice; the
  workload split is real but is served by *optional* overrides on top of one default.
- **Drop the master switch** (feature toggles + "no providers connected" as the natural off state).
  Rejected — for a screen-recording product the single kill switch is a trust affordance, and "Wipe
  User Context turns the engine off" needs a switch to land on.

## Consequences

- **Amends [ADR 0033](0033-ask-ai-migrates-onto-shared-reasoning-engine.md)** — specifically its
  point 5 (the "small set of configured engines" becomes the provider list; the per-thread picker
  now chooses from the provider-tagged model pool) and point 6 (the engine-configured prerequisite
  is re-grounded on providers + default model, beneath an explicit retained master switch). Point
  3's model split survives as the optional override layer. Everything else in 0033 — one engine,
  the agent loop, brokered access, stateless-per-turn, discovery, free-form entry — stands.
- **One pool, three consumers.** The settings default-model picker, the Ask AI override picker, and
  the Chat thread picker all consume the same provider-tagged model pool from
  `ai_runtime_list_models`; there is no longer a picker whose option set is defined differently.
- **The privacy boundary is untouched.** This reshapes *which model gets called*, not what any model
  is allowed to see; Brokered Capture Access, redaction, and audit are unchanged.
- **Same-provider key sharing becomes explicit.** Multiple configured engines on one provider always
  shared a keychain entry; the provider list makes that the visible mental model instead of a quirk.
- The `AGENTS.md` bullets describing `AiRuntimeSettings`' cloud/local fields, the
  `read_ask_ai_model`-style resolution helpers, and the Access-tab Ask AI card become stale when
  this lands and must be updated with the implementation.
