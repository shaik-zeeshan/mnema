---
status: proposed
---

# Semantic Search v1 is local Hybrid Search: fastembed vectors fused with FTS5 inside the Encrypted Capture Index

Mnema adds **Semantic Search** as a local meaning tier that fuses with the existing FTS5
**Text Search** into **Hybrid Search**, completing the direction deferred in
[ADR 0007](0007-search-v1-text-search-in-app-db.md). Everything runs on-device; no captured
content is sent to a cloud embedding service (Anthropic, the frictionless default provider,
ships no embeddings API, so a cloud default is impossible anyway — and local keeps the
privacy posture intact).

**Runtime.** Embeddings are produced in-process by `fastembed`, which reuses the ONNX
runtime (`ort`) already shipped for Parakeet transcription — no second native runtime, no new
signed artifact. This requires pinning the direct `ort` dependency to fastembed's exact
version (`=2.0.0-rc.12`) and bumping the two in lockstep; that pin coupling is the one
hard-to-reverse cost, accepted because fastembed gives a model catalog plus rerankers that
hand-rolling tokenizer/pooling per model family would not.

**Embedding unit.** One **Semantic Search Vector** per **Search Result Anchor** (one
`search_documents` row). Vectors are computed only for `direct` rows; `equivalent_reuse`
anchors reuse their group's vector, so structural frame dedup already collapses the count
with no separate dedup pass. Text that overflows the model's token window is **auto-split
only on overflow** (never silently dropped); chunk-level multi-vector is deferred to a later
quality pass.

**Substrate.** Vectors are stored in `sqlite-vec`'s `vec0` virtual table **inside** the
SQLCipher-encrypted **Encrypted Capture Index** — `sqlite-vec` statically linked into the
same SQLCipher amalgamation (verified in a throwaway spike: one `libsqlite3-sys`, `vec0`
KNN + encryption-at-rest + hybrid fusion all green). Vectors are stored as **f32**. There is
**no ANN in v1**: `vec0` stable is itself a brute-force scan, which is correct and fast enough
because queries scan a filtered slice, not the whole corpus. Footprint quantization
(int8 → binary) and an eventual ANN/DiskANN index are later *config flips on the same
substrate*, each gated on a **measured** trigger (disk pressure; p95 unfiltered-query
latency) — not storage-engine migrations, and not in this ADR.

**Scope (filter-then-rank).** Semantic ranking is **filter-then-rank**, a correctness
requirement under top-k: `vec0` stays a pure `{rowid, embedding}` store, all scoping reuses
the existing **Search Refinement** `WHERE` on `search_documents` (date range, app,
window-title substring, audio/screen source), and the KNN is constrained with
`MATCH … AND rowid IN (…)` (spike-proven to pre-filter, not post-filter, so a scoped query
never returns empty when in-scope answers exist). An unrefined query falls back to plain
`vec0` KNN with `LIMIT k`.

**Fusion.** **Text Search** and **Semantic Search** are fused by **reciprocal rank fusion**
(rank-only, no score calibration — BM25 and vector distance are incomparable scales) at the
**Search Result Anchor** level, then fed into the existing group→paginate path where the BM25
rank sits today. A meaning-only hit (no FTS term to highlight) renders a leading `body_text`
excerpt tagged "found by meaning". Recency stays a **tie-break only** — v1 adds no time-decay
boost (none exists today).

**Production & rollout.** A single **sweep-loop** worker on the deferred-startup seam embeds
`direct` anchors lacking a vector — live and backfill in one self-healing query, fresh capture
always preempting backlog (drained newest-first), resumable across restarts; no OCR-style
admission policy is needed because dedup is structural. The feature is **default-on but
model-gated**, exactly like local transcription/OCR: with no **Semantic Search Model**
installed it is silently inert (a no-op admission, never a capture-blocking error), and Mnema
**never auto-downloads** a model. The user installs a model from Settings; the sweep starts the
moment it is `Installed`.

**Lifecycle & privacy.** Keying to `search_documents.id` makes lifecycle nearly free: deletion
(retention, Delete Recent, reprocess) flows through one `AFTER DELETE ON search_documents`
trigger that drops the `vec0` row — a near-copy of the shipping FTS sync trigger, inheriting
its correctness — and reprocessing (delete + reinsert with a new id) is replaced automatically
by the sweep. Vectors are derived from the **same raw `body_text` that Text Search already
indexes**, not a separately-redacted copy: because both live inside the Encrypted Capture
Index, the vector's at-rest exposure equals Text Search's, and a redaction-stricter meaning
tier would be inconsistent and hurt recall. Redaction is instead enforced at any **boundary**
that takes a vector or its text *out* of the encrypted index (export, a future external index,
a cloud reranker) — the rule the broker already applies — and the meaning-only snippet reuses
the same `secret_redactions` masking as a Text Search snippet. **This overturns the prior
ADR 0007 / CONTEXT.md stance that embeddings be "derived from redacted text."**

**Model selection.** Selection is **guided tiers + a Custom picker** over the models fastembed
supports (which it auto-downloads + caches, so exposing the full list is ~free). The default
tier is **`nomic-embed-text-v1.5`** (English, 8192-token, Apache-2.0, ungated) — long context
makes truncation a non-issue and the permissive license keeps the default path obligation-free.
**Multilingual** is its own tier (`embeddinggemma-300m`, ~300 MB quantized; `bge-m3` available
via Custom). Because different models produce incomparable vectors and `vec0` is a fixed-dim
table, **changing the model rebuilds the entire index** — so model choice is a deliberate,
confirmed setup decision, and a non-English user is guided to a fitting tier (optionally
recommended by OS locale) rather than silently degraded by the English default.

**Considered Options**

We rejected the **hand-rolled BLOB + cosine** pattern (reused from voice diarization): the only
strong reason to prefer it was the unverified SQLCipher-static-link risk, which the spike
retired; building on `vec0` instead makes quantization and ANN later config flips rather than a
storage migration. We rejected **ANN in v1**: its encrypted-at-rest story is unproven and v1
does not need it, since filter-then-rank keeps the *scanned* set small even as the *stored* set
grows. We rejected **embedding redacted-text-only**: FTS5 already indexes the raw text inside
the same encrypted index, so a stricter meaning tier is inconsistent and punches recall holes —
redaction belongs at the egress boundary. We rejected **weighted-score fusion** (fragile
cross-scale calibration) in favor of RRF, and **chunk-level multi-vector** is deferred to a
quality pass. We rejected a **multilingual default** in favor of an English default plus an
explicit Multilingual tier and a Custom picker, so no user is restricted while the common case
stays small and license-clean. We rejected a **separate cloud "embedding provider" setting**:
it reintroduces egress and leaves the local-first posture.

**Consequences**

A new migration (next free at implementation time; `0038` is now taken on `main`, so `0039`)
adds the embeddings/`vec0` table and the `AFTER DELETE`
trigger, and the build statically links `sqlite-vec` into the SQLCipher amalgamation (a
one-time build change). The `ort` exact-pin couples Parakeet and fastembed — they must bump
together. Semantic Search is unavailable until the user installs a **Semantic Search Model**:
default/Anthropic-only users get keyword-only search until then (the same "no usable runtime →
feature unavailable" shape as local transcription). Switching models triggers a full background
re-index behind a clear confirm. Two items must be verified in the real build during
implementation: the `vec0` `AFTER DELETE` trigger firing on a CASCADE-driven frame delete, and
programmatic download-progress wiring from fastembed for the Settings UI. Footprint
quantization and ANN remain future work (their own ADRs/config changes), triggered by measured
pressure rather than a guessed vector count.
