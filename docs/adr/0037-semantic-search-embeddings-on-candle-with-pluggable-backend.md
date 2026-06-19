---
status: accepted
---

# Semantic Search embeddings move to candle (on-device GPU/CPU) behind a pluggable Semantic Search Backend

**Semantic Search** embeddings now run on **candle** (`candle-core`/`candle-nn`/`candle-transformers`)
instead of `fastembed`/ONNX, behind a pluggable **Semantic Search Backend** seam. candle is the
default and only shipped backend; it runs the *identical* model weights on the Apple GPU (Metal)
or CPU. This **supersedes the "Runtime" and "Model selection" sections of
[ADR 0036](0036-semantic-search-v1-hybrid-fastembed-vectors-with-fts5.md)**; everything else in
0036 (the hybrid FTS5 ⊕ `vec0` RRF substrate, filter-then-rank, the atomic model-switch and
dimension authority, the encrypted-at-rest + egress-redaction privacy model, model-gated
default-on rollout) stands unchanged.

**Why replace fastembed.** 0036 chose fastembed *specifically* to avoid "a second native runtime,
no new signed artifact," reusing the `ort` ONNX runtime Parakeet ships. But on real `~/.mnema` the
fastembed/ort path was too heavy to recommend: ~140% CPU pinned on the **P-cores**, ~1.7 GB resident
(an `ort` CPU memory arena that never returns to the OS), needing a stack of mitigations (a per-thread
QoS downclock, an intra-op thread cap, a 256-token window clamp, an arena-off execution provider).
A measured bake-off showed candle running the same `nomic-embed-text-v1.5` weights on the Metal GPU is
**lossless** (cosine 1.0 at F32; 0.986 overlap at F16), drops CPU to ~35%, and drops RAM to ~720 MB at
F16 — with no ONNX arena to leak. Because **no model ships by default** (Semantic Search is model-gated
and inert until a user installs one), the heavy cost was opt-in and rare, but it was exactly what made
the feature un-recommendable. candle is the lever that makes it recommendable. We consciously accept a
second native ML runtime in the bundle (candle links Metal); `ort` does **not** leave the app — it
remains for audio transcription — so the "one runtime" property of 0036 is retired regardless.

**Pluggable Semantic Search Backend.** Embedding is now one seam (`text → normalized vector`) with
candle as the first implementation. The seam exists so future runtimes can be added: a **local**
Ollama/llamafile backend is a natural next step. A **remote/cloud** backend (e.g. an
OpenAI-compatible embeddings API) is **permitted but opt-in only, never the default** — this is a
*scoped extension* of 0036's rejection of cloud embeddings, not a flat reversal, and it mirrors how
the Reasoning Engine already lets users opt into cloud providers for Ask AI. A cloud backend may embed
only text that has crossed the **egress-redaction boundary** (the rule the broker already applies):
local backends embed raw `body_text`, a cloud backend embeds *redacted* text, so the cloud tier has
lower recall by construction and is non-comparable with local vectors (switching backends re-derives
the index via the same machinery as switching models). Building a cloud backend is its own follow-up.

**Model catalog is hand-maintained.** candle has no model registry, so the fastembed-`ModelInfo`
synthesis that produced every descriptor (dimension, pooling, output key, on-disk layout, the open
Custom-picker list) is removed. Each model is now a **hand-coded descriptor**
(`{ architecture, dimension, pooling, max_tokens, safetensors_layout, hf_repo }`); guided tiers are
explicit entries. This **reverses commit `524975e`'s "synthesize from the catalog, never hand-restate"
overlay** — there is no catalog left to overlay. A CI guard cross-checks each descriptor against the
model's own `config.json` (dimension, layer count) as the new drift guard, replacing the old
"resolves-in-fastembed" guard. Feasibility verified against candle-transformers 0.10.2: it ships
`nomic_bert` (English default, proven) and `xlm_roberta` (covers the multilingual-e5 family **and**
`bge-m3`), plus `bert`/`jina_bert`/`distilbert`/`modernbert` and the `nvembed_v2`/`stella_en_v5`
embedders — so all three current guided tiers have a candle architecture. The open "any ONNX model"
Custom picker becomes a **curated** candle-backed list; "I want a different model" is served by adding
a **Semantic Search Backend** (e.g. local Ollama), not by an arbitrary-architecture loader.

**Device & precision.** candle backends are compile-time features, so device is per-build: **Metal on
macOS** (verified; F16 default for the RAM win, accepting ~11% slower than F32 — F16 has no fast Metal
kernel) and **candle-CPU elsewhere** (F32 default; F16 is emulated/slow on CPU). The runtime tries
Metal then falls back to CPU. **CUDA is deferred** — a later build option, not a v1 shipped artifact;
macOS is the only verified platform regardless. `SUPPORTS.md` keeps Windows/Linux at `[~]` until
candle-CPU is measured (CPU%, throughput, RSS) — that measurement is the gate to claim non-Mac support.

**Lossless migration; quality levers gated.** candle ships **behavior-identical** to the fastembed
path — no task prefixes, multi-chunk mean-pool, 256-token window — so the cutover is a provable
equivalence needing no quality gate. Two quality levers are explicitly **deferred behind a larger
judged set** (the current harness is 26 queries / single judge): (1) **per-model input prefixes** —
production today embeds bare text, so `nomic`/`e5` run *without* the `search_query:`/`search_document:`
(and e5 `query:`/`passage:`) prefixes they were trained with, a latent recall leak; (2) **first-window
embed** — embedding only the first 254-token window instead of pooling all chunks (measured −0.118 on
concept queries on a single judge). Terminology is sharpened to keep these distinct: **"one stored
vector per anchor"** is the kept pooling/dedup invariant; **"first-window embed"** is the deferred lever.

**Considered Options.** We rejected **keeping fastembed as a universal fallback** under candle
(per-model architecture routing): it preserves the open Custom picker but keeps the second ONNX runtime,
the `ort` pin-lockstep, and the arena machinery we are trying to delete — and the only real loss from
dropping it, arbitrary-model support, is better served by the backend abstraction (Ollama). We rejected
**shipping candle-everywhere including a CUDA artifact in v1**: candle-CPU is unmeasured and a CUDA build
is scope we have no verified platform for. We rejected **fixing the prefix leak in v1**: it forces a
re-index and deserves its own judged measurement on this unusual (short OCR/transcript) corpus first.

**Consequences.** A second native ML runtime (candle-metal) ships in the bundle; `ort` stays only for
transcription. Removing `ort` from `crates/semantic-search` deletes a pile of now-dead machinery — the
`BackfillEmbedQosGuard` per-thread QoS downclock (Metal frees the P-cores by construction), the
intra-op thread cap, the arena-off CPU execution provider, the `ort` pin-lockstep guard test, and the
crate's `native-tls`↔`rustls` divergence (the downloader aligns to `rustls`). The 256-token window cap
is **kept but re-justified**: not the retired ORT arena leak (gone), but to bound the padded GPU tensor.
Backend-agnostic reliability hardening is **kept**: the poison-pill quarantine, the `is_finite` guard,
and the row-existence-conditioned insert. Rollout needs **no `vec0` schema migration** (nomic stays
768-dim); the model-manifest version is bumped so older ONNX-shaped installs re-download in the
safetensors layout; the backend cutover re-indexes through 0036's existing atomic-switch +
startup-reconcile + dimension-authority machinery (a no-op for the ~all users who have no model/vectors
today); model-gated default-on and "never auto-download a model" are unchanged. The candle-CPU
measurement and the cloud-backend egress design are tracked follow-ups.
