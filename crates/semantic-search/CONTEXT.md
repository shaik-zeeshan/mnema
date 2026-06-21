# Semantic Search

Local, on-device embedding of captured text into vectors for hybrid (vector ⊕ FTS5) search. Runs the same model weights on the Apple GPU (Metal) or CPU via candle, behind a pluggable **Semantic Search Backend** ([ADR 0037](../../docs/adr/0037-semantic-search-embeddings-on-candle-with-pluggable-backend.md)).

## Language

**Semantic Search Model**:
A hand-coded catalog entry (`SemanticSearchModelDescriptor`) stating every fact candle needs to run a model: architecture, dimension, pooling, max tokens, prompt, hf repo, on-disk layout. There is no upstream registry — every fact is hand-stated and guarded against the model's own `config.json`.

**Semantic Search Model Tier**:
The user-facing role a model fills. **English** (the default, `nomic-embed-text-v1.5`), **Multilingual** (`multilingual-e5-small`), or **Custom** (opt-in stronger models offered in the picker; `bge-m3` today).
_Avoid_: "guided model", "preset".

**Architecture**:
The candle model family a descriptor dispatches to (`SemanticSearchArchitecture`). Hand-coded per model, never inferred from an id. A model is only addable if candle-transformers ships its architecture.

**Pooling**:
How a model collapses token hidden states into one vector — `Mean` over the attention mask, `Cls` (the `[CLS]` token), or last-token (EOS). A declared descriptor field; mean-pooling a CLS/last-token model silently yields a wrong, lower-quality vector.

**Prompt**:
The per-model input text a model was trained to prepend, distinguished by **Query** vs **Document** because some models are asymmetric (e.g. e5 uses `query:` / `passage:`; nomic uses `search_query:` / `search_document:`). A symmetric or instruction-free model (e.g. bge-m3 dense) carries no prompt. Declared per descriptor and filled in for every model.
_Avoid_: "prefix" (a prompt may be a full instruction, not just a token prefix).

**Anchor**:
The unit a single stored vector represents — "one stored vector per anchor" is the kept pooling/dedup invariant. Text overflowing the window is split into token-window chunks, each embedded, then mean-pooled back into the one anchor vector (never silently truncated).

## Relationships

- A **Semantic Search Model** has exactly one **Architecture**, one **Pooling**, and (now) one optional **Prompt** pair (Query, Document).
- Each **Tier** points at one **Semantic Search Model**; **Custom** may offer several.
- Every model in the catalog must have a **pairwise-distinct dimension** — the vector store uses dimension as the *only* discriminator between embedding spaces during a model switch, so two same-dimension models could silently cross-contaminate. Adding a same-dimension model requires a stronger model-identity/epoch guard in the store first.

## Example dialogue

> **Dev:** "Can we add `multilingual-e5-large` as a Custom tier?"
> **Domain expert:** "Its **Architecture** (XLM-Roberta) is already wired, but its **dimension** is 1024 — identical to `bge-m3`. Same-dimension models collide in the store, so either truncate it to a free dimension or add a model-identity guard first. And fill in its e5 **Prompt** (`query:` / `passage:`) or it runs degraded."

## Flagged ambiguities

- "prefix" vs "instruction" — unified under **Prompt** (Query/Document), since instruction-tuned models prepend a full instruction, not just a tag.
- ADR 0037 deferred per-model prompts to keep the candle cutover "behavior-identical". On this pre-release branch (no users, no stored vectors) that deferral is being **lifted**: prompts are filled in for every model. ADR 0037's "prefixes deferred" rationale is amended accordingly.
