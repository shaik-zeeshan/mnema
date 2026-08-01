# Semantic Search

Local, on-device embedding of captured text into vectors for hybrid (vector ⊕ FTS5) search. Runs the same model weights on the Apple GPU (Metal) or CPU via candle, behind a pluggable **Semantic Search Backend** ([ADR 0037](../../docs/adr/0037-semantic-search-embeddings-on-candle-with-pluggable-backend.md)).

## Language

**Semantic Search Model**:
A hand-coded catalog entry (`SemanticSearchModelDescriptor`) stating every fact candle needs to run a model: architecture, dimension, pooling, max tokens, prompt, hf repo, on-disk layout. There is no upstream registry — every fact is hand-stated and guarded against the model's own `config.json`.

**Semantic Search Model Tier**:
The user-facing role a model fills. **English** (the default, `nomic-embed-text-v1.5`), **Multilingual** (`multilingual-e5-small`), or **Custom** (opt-in alternatives offered in the picker: `bge-m3`, `stella_en_400M_v5`, `snowflake-arctic-embed-l-v2.0`, and the ModernBERT English trio `gte-modernbert-base` / `granite-embedding-english-r2` / `granite-embedding-small-english-r2`). **Custom does not mean "stronger"** — the ModernBERT trio was measured against the default on real capture data at the shipped 256-token window (issue #191) and none of it wins; they trade disk and throughput against quality, which is the choice the picker offers.
_Avoid_: "guided model", "preset".

**Architecture**:
The candle model family a descriptor dispatches to (`SemanticSearchArchitecture`). Hand-coded per model, never inferred from an id. A model is only addable if candle-transformers ships its architecture.

**Pooling**:
How a model collapses token hidden states into one vector — `Mean` over the attention mask, `Cls` (the `[CLS]` token), or last-token (EOS). A declared descriptor field; mean-pooling a CLS/last-token model silently yields a wrong, lower-quality vector.

**Prompt**:
The per-model input text a model was trained to prepend, distinguished by **Query** vs **Document** because some models are asymmetric (e.g. e5 uses `query:` / `passage:`; nomic uses `search_query:` / `search_document:`). A symmetric or instruction-free model (e.g. bge-m3 dense) carries no prompt. Declared per descriptor and filled in for every model.
_Avoid_: "prefix" (a prompt may be a full instruction, not just a token prefix).

**Model Epoch**:
Which model's embedding space the live vector table holds, recorded as a `model_id` stamp written in the same transaction that builds the table (`app-infra/src/semantic_search.rs`). It is what makes a **Semantic Search Model** switch detectable: every write is gated on it, and startup reconciliation rebuilds the table when it disagrees with the selection. It replaced dimension-as-identity, which stopped working the moment the catalog gained same-width models.
_Avoid_: "dimension check", "column width" — those describe the mechanism it replaced.

**Anchor**:
The unit a single stored vector represents — "one stored vector per anchor" is the kept pooling/dedup invariant. Text overflowing the window is split into token-window chunks, each embedded, then mean-pooled **weighted by chunk length** back into the one anchor vector (a short trailing window must not count as much as a full one).

A **document**'s chunks are capped at `runtime::MAX_DOCUMENT_CHUNKS` (currently **2**), so an overflowing document is indexed from its **first two windows and the rest is not searchable**. This replaces the crate's former "never silently truncated" invariant on the document side, deliberately — it is the largest cost lever in the embed path and the only one that does not also slow the backfill down.

The cap is paid for in unindexed content, measured over 2,409 real anchors (1.47 M tokens): a cap of **1** leaves **60.0%** of corpus tokens unindexed, **2** leaves **25.6%**, **3** leaves 12.0%. The loss falls almost entirely on screen frames (94.2% span more than one window; audio anchors lose ~1%). Cost runs the other way: chunks/anchor is 1.00 / 1.93 / 2.56 against 2.97 uncapped. **2** is the chosen balance — a cap of 1 discarded three fifths of everything read on screen, which is too much to trade for heat the proportional cooldown already governs. Earlier bake-off evidence at a cap of 1: judged nDCG@10 0.760 → 0.777 with a **concept-query soft spot (−0.118)**, on 26 queries and a single grader. **Queries are never capped.**

## Relationships

- A **Semantic Search Model** has exactly one **Architecture**, one **Pooling**, and (now) one optional **Prompt** pair (Query, Document).
- Each **Tier** points at one **Semantic Search Model**; **Custom** may offer several.
- Every model in the catalog must have a **distinct `model_id`** — the vector store stamps the id of the embedding space its table holds and rejects a write from any other model. Dimensions need NOT be distinct and deliberately are not: `gte-modernbert-base` and `granite-embedding-english-r2` are 768 like `nomic-embed-text-v1.5`, and `granite-embedding-small-english-r2` is 384 like `multilingual-e5-small`. (Before the stamp, distinct dimensions *were* the guard; do not reinstate that rule.)

## Example dialogue

> **Dev:** "Can we add `multilingual-e5-large` as a Custom tier?"
> **Domain expert:** "Its **Architecture** (XLM-Roberta) is already wired, and its **dimension** being 1024 — identical to `bge-m3` — is fine now: the store stamps the model id, not the width. What it still needs is its own **model_id**, its e5 **Prompt** (`query:` / `passage:`) filled in or it runs degraded, and a **Pooling** read off `1_Pooling/config.json` rather than guessed. And measure it before calling it stronger."

## Flagged ambiguities

- "prefix" vs "instruction" — unified under **Prompt** (Query/Document), since instruction-tuned models prepend a full instruction, not just a tag.
- ADR 0037 deferred per-model prompts to keep the candle cutover "behavior-identical". On this pre-release branch (no users, no stored vectors) that deferral is being **lifted**: prompts are filled in for every model. ADR 0037's "prefixes deferred" rationale is amended accordingly.
