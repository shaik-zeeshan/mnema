---
status: accepted
---

# Semantic Search v1 is local Hybrid Search: fastembed vectors fused with FTS5 inside the Encrypted Capture Index

> **Partially superseded by [ADR 0037](0037-semantic-search-embeddings-on-candle-with-pluggable-backend.md).** The **"Runtime"** and **"Model selection"** sections below are superseded: embeddings now run on **candle** (on-device GPU/CPU) behind a pluggable **Semantic Search Backend** rather than `fastembed`/ONNX, and the model catalog is hand-maintained rather than synthesized from fastembed's `ModelInfo`. Everything else in this ADR — the hybrid FTS5 ⊕ `vec0` RRF substrate, filter-then-rank, the atomic model-switch + dimension authority, the encrypted-at-rest + egress-redaction privacy model, and model-gated default-on rollout — still holds.

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
via Custom). *(Superseded in implementation: the Multilingual tier ships
`multilingual-e5-small` — `embeddinggemma-300m`'s HF repo is access-gated. See the
"implementation outcomes" amendment below.)* Because different models produce incomparable vectors and `vec0` is a fixed-dim
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

**Amendment — the model switch is atomic, the live `vec0` dimension is the one authority, and a stuck index self-heals.**

The original "persist `model_id`, then re-index" was a non-atomic two-step. A `vec0` table
is fixed-dimension, but the persisted model selection and the live table dimension were two
separate pieces of state with no shared authority and no revert-on-failure. If the re-index
ever failed after the persist (a `DROP TABLE` needs an exclusive lock, and the always-on
backfill worker writes concurrently — DB-busy is realistic), the table stayed at the old
dimension while the selection named a new-dimension model. From there: every search hard-failed
on a dimension mismatch (not the promised keyword-only degrade), and the worker stored
wrong-length blobs that `vec0` rejected, error-looping the same doomed batch every retry
forever. Re-selecting the same model could not recover it (the Settings picker early-returns on
an unchanged pick). Four changes close this as one design, not four patches:

1. **Atomic switch.** Model selection is one transactional backend command
   (`select_semantic_search_model`): it resolves the target model's dimension, **rebuilds the
   `vec0` table at that dimension first** (the step that can fail), and **persists the selection
   only after the rebuild commits**. A failed rebuild leaves the old model selected and the
   old-dimension table intact, so the frontend surfaces the error against a consistent backend
   state. The frontend makes one invoke, not two.

2. **One dimension authority = the live `vec0` column.** The active vector width is read
   straight from the `search_document_vectors` table DDL (`float[N]`), never inferred from the
   separately-persisted model. Both the worker store path and the query path consult it.

3. **Degrade, don't fail.** The query path checks the live dimension against the query
   embedding and skips the KNN on a mismatch; any semantic-fetch error is logged and fused as an
   empty meaning list, so search degrades to keyword-only (the ADR's "no usable runtime → feature
   unavailable" promise) instead of failing the whole search. The worker skips a wrong-dimension
   store (not an error) and idles that pass, so it never error-loops a vector the live column
   cannot accept.

4. **Startup reconciliation (self-heal).** On the deferred-startup seam, before the backfill
   worker spawns, the live `vec0` dimension is reconciled against the selected model's expected
   dimension: a table that disagrees is rebuilt so the index can backfill again. Idempotent — a
   matching table (the common case) is untouched. This is what recovers a permanently-stuck index
   on the next launch when re-selecting the same model cannot.

**Amendment — implementation outcomes (v1 as shipped).**

The v1 implementation landed on the design above with the following concrete choices and
deviations from the body of this ADR. They are recorded here so the ADR matches what ships, and
so the remaining gates to un-drafting the feature are explicit.

1. **Multilingual tier ships `multilingual-e5-small`, not `embeddinggemma-300m`.** The body names
   `embeddinggemma-300m` as the Multilingual tier. That model's Hugging Face repo is access-gated,
   and Mnema's manual (desktop-owned) downloader cannot fetch a gated repo, so the Multilingual
   tier ships **`multilingual-e5-small`** instead (MIT license, 384-dim). This is intentional and
   preserves the tier's intent — a permissively-licensed, ungated, downloadable multilingual model.
   `bge-m3` remains reachable via the Custom picker. (A stray `embeddinggemma-300m` literal still
   lingers in a `capture-types` serde round-trip test as an arbitrary value — harmless, flagged but
   not changed.)

2. **Pooling and `output_key` are read from fastembed, not guessed from the model id.** Pooling is
   now sourced from fastembed's `get_default_pooling_method` and carried through the model
   descriptor; `output_key` is carried the same way. The prior `starts_with("bge")` heuristic was
   correct only for the guided tiers and silently mean-pooled CLS-trained Custom models
   (`mxbai-embed-large-v1`, the GTE family, `snowflake-arctic-embed-*`), degrading their recall.
   Reading pooling from fastembed makes every supported model — guided or Custom — pool correctly.

3. **Custom models can now be activated.** The status command surfaces Custom models that are
   installed on disk (not just the guided tiers plus the persisted selection), so a downloaded
   Custom model can reach the "Use this model" action. Previously a Custom model could be downloaded
   but never selected — the entire Custom path was a dead end.

4. **Download integrity — fail-closed plumbing in place, guided-tier hashes not yet pinned
   (tracked gap).** SHA256 verification plumbing was added (mirroring the OCR/transcription
   downloaders' `validate_artifact_sha256`), fail-closed by construction, plus a Content-Length
   truncation guard on the streaming downloader. **However, no guided-tier hashes are pinned yet:**
   the hash table returns `None` for every tier, so every guided download currently resolves to
   "integrity unverified" rather than verified. Pinning the real per-file SHA256 constants is an
   explicit follow-up — until then, **guided downloads are not checksum-verified.** Custom
   (user-entered) models are unverified by design, since no digest is known ahead of time. This is
   accepted with a tracked gap; do not describe guided downloads as integrity-verified until the
   constants are pinned.

5. **`ort` TLS-stack divergence (known, needs a deliberate call) + a lockstep guard.** The
   `semantic-search` crate builds `ort` with the **`native-tls`** feature, while the rest of the
   workspace uses **`rustls`**. This divergence is known and left as-is pending a deliberate
   decision on which TLS stack the ONNX path should use. Separately, a new test now mechanically
   asserts the `ort` version pin stays in lockstep between `crates/semantic-search` and
   `crates/audio-transcription`, so a future fastembed/Parakeet bump that drifts the two pins fails
   loudly instead of silently compiling two native ONNX runtimes (or failing the static link with no
   early signal).

6. **CPU pacing during backfill is a light governor, not OCR's Execution Budget.** The body says
   backfill "runs only on idle/background capacity." In v1 that is realized as a clamped inter-batch
   cooldown: the prior 0 ms yield is replaced by a work-time-scaled sleep bounded to a **150 ms –
   2000 ms** band (longer batches earn a longer cooldown). This is deliberately **lighter** than the
   OCR Execution Budget — there is **no** recording-active mode switching and **no** persisted budget
   state. It is one lower-priority tokio task with a bounded cooldown, not a full throughput budget.

7. **Reliability hardening that landed with v1.** The store path is now an atomic
   row-existence-conditioned insert (a delete racing the worker inserts zero rows instead of leaving
   an orphan vector at rest); an `is_finite` guard rejects non-finite embeddings before insert; an
   in-memory poison-pill quarantine sidelines an anchor after 3 consecutive **deterministic** embed
   failures (distinct from transient DB-store retries, no migration — a reprocess gets a new anchor
   id and re-tries); and 3 consecutive ONNX **load** failures surface a "model appears corrupt —
   reinstall" signal over the download-progress event.

**Gates still outstanding before un-drafting.** These were deferred or are follow-ups, not fixed in
this branch:

- **p95 unfiltered-query latency measurement (ANN gate).** v1 ships **no ANN**: the default
  unrefined Quick Recall search brute-force scans the full vector corpus per query. The ADR defers
  ANN behind a *measured* p95 trigger, but ships **no** measurement. Measuring p95 unfiltered-query
  latency at a realistic corpus is the explicit gate before relying on the brute-force path at scale
  — it is a gate, not something implemented here.
- **Guided-tier SHA256 pinning** (see point 4 above) — required to honor the fail-closed integrity
  posture for guided downloads.
- **Packaged-build acceptance sign-off** — AC-1 (CASCADE-delete trigger) and AC-2 (per-chunk
  download progress) are proven at the unit level against the real code paths; the ADR asks for
  confirmation in a packaged SQLCipher build, which remains a sign-off formality.
- **Cross-platform verification** — built and tested on macOS (darwin) only; Windows/Linux are
  unverified (see `SUPPORTS.md`).
