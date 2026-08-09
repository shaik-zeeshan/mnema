# Semantic Search embedding model bake-off

Offline retrieval harness for [#191](https://github.com/shaik-zeeshan/mnema/issues/191)
(parent #190): **does any candidate embedding model beat `nomic-embed-text-v1.5`
at Mnema's real 256-token window, on real Mnema anchor text?**

> **CAVEAT — read before quoting any number.** This Python/`transformers` path is
> **not** bit-identical to the shipped candle path (F32-vs-F16 on Metal, candle's
> own pooling, tokenizer edge cases). The numbers are a **relative ranking between
> models only** and must not be quoted as absolute quality figures for the shipped
> app. `bench.py` prints this caveat at the top of every report.

## Candidates

| key | HF repo | revision | dims | pooling | query prefix | doc prefix |
|---|---|---|---|---|---|---|
| `nomic` **(baseline)** | `nomic-ai/nomic-embed-text-v1.5` | `e9b6763…` | 768 | mean | `search_query: ` | `search_document: ` |
| `granite` | `ibm-granite/granite-embedding-english-r2` | `47ea694…` | 768 | CLS | — | — |
| `granite-small` | `ibm-granite/granite-embedding-small-english-r2` | `2ab6fa8…` | 384 | CLS | — | — |
| `gte-modernbert` | `Alibaba-NLP/gte-modernbert-base` | `e7f32e3…` | 768 | CLS | — | — |

Full SHAs live in `MODELS` in `bench.py`. Trailing spaces in the nomic prefixes are
significant.

## What it mirrors from production

Read alongside `crates/semantic-search/src/runtime.rs` — the harness reproduces:

- `MAX_EMBED_WINDOW_TOKENS = 256`; effective window `min(model.max_tokens, 256)`
  (every candidate's native window is ≥ 256, so all run at 256).
- Overflowing text is split into token-window chunks (`split_on_overflow`, a port of
  `split_text_on_token_overflow`). Production then caps a **document** at
  `runtime::MAX_DOCUMENT_CHUNKS` windows and drops the rest; the harness is
  **uncapped by default** — pass `--max-doc-chunks 2` to mirror the shipped cap.
- The per-side instruction prompt is prepended to **each chunk string**, and the
  split budget is `256 − prompt_tokens − 2` (`SPECIAL_TOKEN_HEADROOM`), so
  `prompt + chunk + specials` fits the window.
- Chunk vectors are pooled **weighted by chunk byte length** then L2-normalized into
  one vector per text (`runtime::weighted_mean_pool_l2` — a short trailing window must
  not count as much as a full one); a single-chunk text passes its vector through
  unchanged.
- Output vectors are unit-norm (asserted at run time), so brute-force dot product
  ordering ≡ cosine ordering.
- Length-sorted sub-batches of 8, matching `EMBED_SUB_BATCH_SIZE`.
- Retrieval is **exhaustive brute-force over the full corpus** — no ANN, as in prod.

Not mirrored (irrelevant to ranking): per-chunk fault isolation, MRL truncation
(no candidate uses it), the candle backend itself.

Known-and-intended parity detail: a chunk is sliced from the first token's start
offset to the last token's end offset, so a whitespace character *between* two
chunks is dropped. Production does the same; no word is ever lost.

## Inputs

- `--corpus` — JSONL, one anchor per line:
  `{"id":…, "kind":"screenText"|"audioTranscript", "text":…, "startedAt":…, "app":…, "windowTitle":…, "url":…}`.
  The real corpus is **personal capture data**: it lives in the scratchpad and must
  never be copied into the repo.
- `--queries` — JSON: `{"method": "…", "queries": [{"id","query","source_anchor_id","kind"}]}`.
- `queries.example.json` + `corpus.example.jsonl` — a 40-doc **synthetic** smoke set
  (no personal data) so the harness is runnable before the real query set lands.

## Relevance judgements

Binary gain, expanded to near-duplicates (consecutive OCR frames of the same screen
are legitimately the same content):

- the query's `source_anchor_id` is relevant;
- **any** other corpus anchor whose character 5-gram Jaccard similarity with the
  source anchor's text is ≥ `--dup-threshold` (default `0.6`) is also relevant;
- everything else is irrelevant.

The report states how many queries ended up with >1 relevant doc and the max
relevant-set size. Sensitivity-check with `--dup-threshold`.

## Metrics

Per model: nDCG@10 and Recall@10 **overall and split by anchor kind** (a model that
wins on transcripts but loses on OCR is not a win), vector dim, real on-disk
snapshot size in MB (measured, not from a table), corpus embed wall-clock,
anchors/sec, chunks/sec, and the total chunk count (how much the 256-token window
inflates the work). A markdown table goes to stdout; the full per-query detail goes
to the `--out` JSON.

## Run it

Requires [`uv`](https://docs.astral.sh/uv/) only — deps are PEP-723 inline in
`bench.py`, nothing is installed into the system python. Uses torch MPS when
available, else CPU.

```sh
# the real run (from the repo root); SCRATCH = your scratchpad dir
uv run scripts/semantic_bench/bench.py \
    --corpus "$SCRATCH/corpus.jsonl" \
    --queries scripts/semantic_bench/queries.json \
    --out "$SCRATCH/semantic_bench_results.json"
```

```sh
# smoke test on the committed synthetic set (no personal data), one model
uv run scripts/semantic_bench/bench.py \
    --corpus scripts/semantic_bench/corpus.example.jsonl \
    --queries scripts/semantic_bench/queries.example.json \
    --models granite-small --out "$SCRATCH/smoke.json"

uv run scripts/semantic_bench/bench.py --self-test   # maths only, no downloads
```

Useful flags: `--models nomic,granite` (subset), `--dup-threshold`, `--window`,
`--max-doc-chunks` (mirror `runtime::MAX_DOCUMENT_CHUNKS`; `0` = uncapped, the default),
`--device cpu`, `--no-cache`, `--cache-dir`.

**Never point `--out` at a repo path** — results and caches belong in the scratchpad.
`--cache-dir` defaults to `<out dir>/embcache`.

## Reproducibility

Every model is pinned to its revision SHA. Embeddings are cached to
`<cache-dir>/<model>-<hash>.npz`, keyed by (model, revision, corpus content hash,
window), with the throughput measurement stored alongside — so metric tweaks and
threshold sweeps re-score without re-embedding, and a cached run still reports the
original timing.

One caveat on the pinning: `nomic-embed-text-v1.5` uses `trust_remote_code`, and its
modeling file is `auto_map`'d to the **separate** `nomic-ai/nomic-bert-2048` repo,
which `transformers` fetches at its own latest revision. Only the weights repo is
SHA-pinned; the nomic modeling code is not.
