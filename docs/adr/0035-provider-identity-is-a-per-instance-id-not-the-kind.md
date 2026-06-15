---
status: accepted
---

# Provider identity is a per-instance id, not the kind

[ADR 0034](0034-ai-settings-are-provider-centric-with-one-global-default-model.md) made **providers,
not engines, the unit of configuration**: a flat list of connected providers, each an
`AiProviderConfig { kind, baseUrl }`, with the secret kept in the OS keychain "keyed by provider id
(the **Capture Index Key Store** boundary, unchanged — it was already keyed by provider)". In that
shape the *kind* **was** the id: the snake_case `AiProviderKind` string (`anthropic`, `openai`,
`openai_compatible`, `ollama`, `llamafile`) doubled as the keychain account, the `provider` tag on
every discovered model, and the conversation engine-pin `provider`. That collapses provider identity
onto provider *type*, so a kind can appear at most once — there is no field to tell two of them
apart. ADR 0034 even named this a feature ("Same-provider key sharing becomes explicit").

In practice the OpenAI-compatible kind is not a single provider; it is a *protocol* spoken by many
distinct endpoints (a local llama-swap box, OpenRouter, Together, Fireworks, a second self-hosted
server). The local kinds are likewise per-endpoint: two Ollama machines are two providers. Capping
each kind at one instance means a user must choose *which* OpenAI-compatible endpoint they have,
forever, and cannot connect two at once. The single `openai_compatible` slot is the binding
constraint that surfaced this.

## Decision

1. **Identity is a stable per-instance id, separate from the kind.** `AiProviderConfig` carries an
   `id: String` that is the identity used everywhere a provider is referenced — the OS-keychain
   account, the `provider` tag on discovered models, the conversation engine-pin `provider`, and the
   global default model's `provider`. The `kind` stays, demoted to a pure *type* tag: it selects the
   rig-core client (cloud vs local, which provider enum) and supplies the default localhost
   endpoint. Multiple instances of one kind coexist by carrying distinct ids.

2. **`AiEngineRef.provider` is the instance id (a string), not the kind enum.** An engine identity is
   still `{provider, model}`; `provider` is now `AiProviderConfig.id`. Conversation pins and the
   keychain were already string-keyed, so no schema migration is needed there — only the typed
   `AiEngineRef.provider` field loosens from the `AiProviderKind` enum to a string.

3. **First instance of a kind keeps `id == kind.id()`; extras are suffixed.** A new provider whose
   kind is not yet present takes the kind id verbatim (`openai_compatible`); a second takes
   `openai_compatible-2`, then `-3`, and so on. This makes the migration a no-op for existing data:
   keys, default-model refs, and pins recorded before instance ids existed all named the kind id,
   which is exactly the first instance's id.

4. **An optional user-facing `label` distinguishes same-kind instances.** `AiProviderConfig.label`
   is an editable display name (e.g. "llama-swap box"); empty falls back to a `kind · host` label
   derived from the base URL. The label is cosmetic — identity and resolution use the id alone.

5. **Migration is deserialization-level, same as ADR 0034.** A persisted provider with an empty `id`
   is backfilled to `kind.id()` at load (the legacy single-per-kind case). The settings
   normalization step dedupes the provider list by **instance id** (was: by kind) — same-kind
   instances survive as long as their ids differ; a genuine duplicate id is dropped (first wins).

## Considered Options

- **Keep kind-as-identity; let users swap the one OpenAI-compatible slot.** Rejected — it forbids
  two endpoints of the same protocol at once, which is the actual use case (a local box *and* a
  hosted router), and offers no path to a second local runtime.
- **Add a `name`/`label` field but still key the keychain and tags by kind.** Rejected — the label
  would distinguish rows visually while the keychain account, model `provider` tag, and pin still
  collide on the kind, so the two instances would share a key and be indistinguishable to the
  resolver. Identity, not just display, has to move off the kind.
- **A random UUID instance id.** Rejected for the *first* instance — it would orphan every existing
  keychain key and pin (recorded under the kind id) and force a real data migration. Suffixing only
  the additional instances keeps the common case migration-free; UUIDs were unnecessary since a
  monotonic `kind-N` suffix is already unique within one settings file.

## Consequences

- **Amends [ADR 0034](0034-ai-settings-are-provider-centric-with-one-global-default-model.md) point
  1** (provider identity) and its "Same-provider key sharing becomes explicit" consequence: a kind
  is no longer capped at one instance, and same-kind instances each get their own keychain entry.
  Points 2–6 (one global default model, the single resolution chain, the master switch, the single
  settings surface, deserialization-level migration) are untouched; the resolution chain still reads
  **thread pin → feature override → global default**, only now keyed by instance id.
- **The keychain boundary is unchanged in shape, only in key.** `ai_provider_key_store` was already
  string-keyed; it now receives the instance id instead of the kind id. Existing keys keep resolving
  because the first instance's id equals the old kind id.
- **The merged model pool stays attributable.** `ai_runtime_list_models` tags each discovered model
  with the provider *instance* id, so two same-kind endpoints contribute distinct, attributable
  entries to the one pool feeding the default-model picker, the Ask AI override picker, and the Chat
  thread picker.
- **Pacing resolves id → kind.** User-Context background pacing depends on cloud-vs-local, so it
  resolves the default model's instance id back to its kind via the provider list before pacing.
- **Pin labels degrade gracefully.** The Chat thread picker labels a pin by its provider string; a
  first instance shows the friendly kind name, an additional instance shows its raw id
  (e.g. `ollama-2`) until/unless surfaced with its user label. Resolution is unaffected — pins match
  by id.
- The privacy boundary is untouched: this reshapes *which provider instance gets called*, not what
  any model may see. Brokered Capture Access, redaction, and audit are unchanged.
