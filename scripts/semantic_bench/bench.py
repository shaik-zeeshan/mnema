#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = [
#   "torch>=2.5",
#   "transformers>=4.48,<5",   # >=4.48 is the first release with ModernBERT
#   "huggingface_hub>=0.26",
#   "numpy>=1.26",
#   "einops>=0.8",             # nomic's trust_remote_code modeling file needs it
# ]
# ///
"""Offline retrieval bake-off: does any candidate embedding model beat
nomic-embed-text-v1.5 at Mnema's real 256-token window, on real Mnema anchors?

Mirrors `crates/semantic-search/src/runtime.rs`: 256-token window, per-side
instruction prompt prepended to EVERY chunk, split budget reduced by the prompt
tokens + 2 special-token headroom, chunk vectors pooled weighted by chunk byte
length then L2-normalized into one vector per text, exhaustive brute-force cosine
over the whole corpus.

See README.md. Smoke test (no personal data):

    uv run scripts/semantic_bench/bench.py \
        --corpus scripts/semantic_bench/corpus.example.jsonl \
        --queries scripts/semantic_bench/queries.example.json \
        --models granite-small --out /tmp/smoke.json
"""

from __future__ import annotations

import argparse
import glob
import hashlib
import json
import math
import os
import re
import sys
import time
from pathlib import Path

CAVEAT = """\
> CAVEAT: this Python/transformers path is NOT bit-identical to the shipped candle
> path (F32-vs-F16 on Metal, candle's own pooling, tokenizer edge cases). These
> numbers are a RELATIVE ranking between models only. Do not quote them as absolute
> quality figures for the shipped app."""

# Mirrors runtime.rs: MAX_EMBED_WINDOW_TOKENS and SPECIAL_TOKEN_HEADROOM.
MAX_EMBED_WINDOW_TOKENS = 256
SPECIAL_TOKEN_HEADROOM = 2
# Mirrors runtime.rs EMBED_SUB_BATCH_SIZE (length-sorted sub-batches).
SUB_BATCH = 8

# revision SHAs are pinned; pooling/prefixes are per-model and wrong values
# silently invalidate the run.
MODELS = {
    "nomic": {  # BASELINE
        "repo": "nomic-ai/nomic-embed-text-v1.5",
        "revision": "e9b6763023c676ca8431644204f50c2b100d9aab",
        "dim": 768,
        "pooling": "mean",
        "query_prompt": "search_query: ",
        "document_prompt": "search_document: ",
        "trust_remote_code": True,
        # The weights SHA does NOT pin the code: this repo's `auto_map` points at the
        # SEPARATE `nomic-ai/nomic-bert-2048`, which transformers otherwise fetches at
        # that repo's moving `main` and execs on this machine.
        "code_revision": "7710840340a098cfb869c4f65e87cf2b1b70caca",
    },
    "granite": {
        "repo": "ibm-granite/granite-embedding-english-r2",
        "revision": "47ea694b257b703fee9253d75c2b1f2985180498",
        "dim": 768,
        "pooling": "cls",
        "query_prompt": None,
        "document_prompt": None,
        "trust_remote_code": False,
    },
    "granite-small": {
        "repo": "ibm-granite/granite-embedding-small-english-r2",
        "revision": "2ab6fa8ea2d674564defd37171ae19079b864b33",
        "dim": 384,
        "pooling": "cls",
        "query_prompt": None,
        "document_prompt": None,
        "trust_remote_code": False,
    },
    "gte-modernbert": {
        "repo": "Alibaba-NLP/gte-modernbert-base",
        "revision": "e7f32e3c00f91d699e8c43b53106206bcc72bb22",
        "dim": 768,
        "pooling": "cls",
        "query_prompt": None,
        "document_prompt": None,
        "trust_remote_code": False,
    },
    # --- multilingual field. Not English-default candidates (the multilingual
    # tier is out of scope in #190) — measured here to answer "would one model
    # serve both tiers?" and to size the resource cost of each.
    "e5-small-ml": {  # the SHIPPED multilingual tier — the multilingual baseline
        "repo": "intfloat/multilingual-e5-small",
        "revision": "614241f622f53c4eeff9890bdc4f31cfecc418b3",
        "dim": 384,
        "pooling": "mean",
        "query_prompt": "query: ",
        "document_prompt": "passage: ",
        "trust_remote_code": False,
    },
    "granite-ml-97m": {
        "repo": "ibm-granite/granite-embedding-97m-multilingual-r2",
        "revision": "835ad14087e140460703cf0fae09f97d469d65c2",
        "dim": 384,
        "pooling": "cls",
        "query_prompt": None,
        "document_prompt": None,
        "trust_remote_code": False,
    },
    "granite-ml-311m": {
        "repo": "ibm-granite/granite-embedding-311m-multilingual-r2",
        "revision": "44399559930365213510b1ee2eb15ded83374f0e",
        "dim": 768,
        "pooling": "cls",
        "query_prompt": None,
        "document_prompt": None,
        "trust_remote_code": False,
    },
    # --- the three models the catalog already ships as Custom options and that
    # nobody has ever measured, plus the one Apache-2.0 model with a credible
    # claim to beating nomic outright. All four need no new candle arm:
    # XlmRoberta / StellaEnV5 are in the catalog, qwen3 is in candle 0.10.2.
    "bge-m3": {
        "repo": "BAAI/bge-m3",
        "revision": "5617a9f61b028005a4858fdac845db406aefb181",
        "dim": 1024,
        "pooling": "cls",
        "query_prompt": None,
        "document_prompt": None,
        "trust_remote_code": False,
    },
    "arctic-l-v2": {
        "repo": "Snowflake/snowflake-arctic-embed-l-v2.0",
        "revision": "ac6544c8a46e00af67e330e85a9028c66b8cfd9a",
        "dim": 1024,
        "pooling": "cls",
        "query_prompt": "query: ",
        "document_prompt": None,
        "trust_remote_code": False,
        # Matryoshka: prod truncates each CHUNK vector to 256 and renormalizes
        # BEFORE the cross-chunk fan-in (runtime.rs:320).
        "mrl": 256,
    },
    "stella": {
        "repo": "NovaSearch/stella_en_400M_v5",
        "revision": "ffeb2b7ee715c226d4ffe5e4619f7dbb48624c20",
        "dim": 2048,
        "pooling": "mean",
        "query_prompt": (
            "Instruct: Given a web search query, retrieve relevant passages that "
            "answer the query.\nQuery: "
        ),
        "document_prompt": None,
        "trust_remote_code": True,
        # In-repo `auto_map`, so this is the same SHA as the weights — stated, not
        # inferred, so "is the exec'd code pinned?" is answerable locally.
        "code_revision": "ffeb2b7ee715c226d4ffe5e4619f7dbb48624c20",
        # Backbone hidden is 1024; the dense head projects mean-pooled → 2048.
        "dense_head": "2_Dense_2048/model.safetensors",
        # Stella's remote code defaults to xformers memory-efficient attention +
        # input unpadding, which are CUDA-only; force the plain path for MPS.
        "model_kwargs": {"use_memory_efficient_attention": False, "unpad_inputs": False},
    },
    "qwen3-0.6b": {
        "repo": "Qwen/Qwen3-Embedding-0.6B",
        "revision": "97b0c614be4d77ee51c0cef4e5f07c00f9eb65b3",
        "dim": 1024,
        "pooling": "last",  # EOS-token pooling, per the model card
        "query_prompt": (
            "Instruct: Given a web search query, retrieve relevant passages that "
            "answer the query\nQuery: "
        ),
        "document_prompt": None,
        "trust_remote_code": False,
    },
}


# ---------------------------------------------------------------- chunking ---
# Port of runtime.rs `split_text_on_token_overflow`. `offsets_of` returns the
# per-token (start, end) spans of `text` with NO special tokens — injectable so
# the self-test can drive it without downloading a tokenizer.


def split_on_overflow(offsets_of, text: str, max_tokens: int) -> list[str]:
    offsets = offsets_of(text)
    budget = max(max_tokens - SPECIAL_TOKEN_HEADROOM, 1)
    if len(offsets) <= budget:
        return [text]
    chunks = []
    start = 0
    while start < len(offsets):
        end = min(start + budget, len(offsets))
        piece = text[offsets[start][0] : offsets[end - 1][1]]
        if piece.strip():
            chunks.append(piece)
        start = end
    return chunks or [text]


def l2(v):
    import numpy as np

    return v / (np.linalg.norm(v) + 1e-12)


# ------------------------------------------------------------------ model ---


def snapshot(spec: dict) -> str:
    from huggingface_hub import snapshot_download

    pats = ["*.json", "*.txt", "*.py", "*.model", "*.safetensors"]
    path = snapshot_download(spec["repo"], revision=spec["revision"], allow_patterns=pats)
    if not glob.glob(os.path.join(path, "*.safetensors")):
        path = snapshot_download(
            spec["repo"], revision=spec["revision"], allow_patterns=pats + ["*.bin"]
        )
    return path


def dir_bytes(path: str) -> int:
    total = 0
    for root, _, files in os.walk(path):
        for name in files:
            try:
                total += os.stat(os.path.realpath(os.path.join(root, name))).st_size
            except OSError:
                pass
    return total


class Embedder:
    """One loaded model + the runtime.rs semantics around it."""

    def __init__(self, spec: dict, device: str, window: int, max_doc_chunks: int = 0):
        import torch
        from transformers import AutoModel, AutoTokenizer

        self.spec = spec
        self.device = device
        self.window = window
        # 0 = uncapped (the pre-Fix-2 semantics). Any positive value mirrors
        # runtime.rs `MAX_DOCUMENT_CHUNKS`.
        self.max_doc_chunks = max_doc_chunks
        self.path = snapshot(spec)
        self.size_mb = dir_bytes(self.path) / 1e6
        trust = spec["trust_remote_code"]
        # `code_revision` pins the .py transformers downloads and EXECUTES. Without
        # it a cross-repo `auto_map` (nomic's points at `nomic-ai/nomic-bert-2048`)
        # resolves that repo's moving `main` and execs it on the machine holding the
        # capture DB key and the harvested corpus. See MODELS.
        code_rev = spec.get("code_revision")
        self.tok = AutoTokenizer.from_pretrained(
            self.path, trust_remote_code=trust, code_revision=code_rev
        )
        self.model = AutoModel.from_pretrained(
            self.path, trust_remote_code=trust, code_revision=code_rev,
            torch_dtype=torch.float32, **spec.get("model_kwargs", {}),
        )
        self.model.to(device).eval()
        # Stored vector width: the MRL-truncated width when the model is
        # Matryoshka-truncated, else the model's native width.
        self.out_dim = spec.get("mrl") or spec["dim"]
        self.dense = None
        if spec.get("dense_head"):
            from safetensors.torch import load_file

            head = load_file(os.path.join(self.path, spec["dense_head"]))
            w = next(v for k, v in head.items() if k.endswith("weight")).to(device)
            b = next((v for k, v in head.items() if k.endswith("bias")), None)
            self.dense = (w, b.to(device) if b is not None else None)

    def _offsets(self, text: str):
        enc = self.tok(text, add_special_tokens=False, return_offsets_mapping=True)
        return enc["offset_mapping"]

    def _prompt_tokens(self, prompt: str | None) -> int:
        if not prompt:
            return 0
        return len(self.tok(prompt, add_special_tokens=False)["input_ids"])

    def chunk(self, texts: list[str], kind: str) -> tuple[list[str], list[int], list[float]]:
        """Prompt-prefixed chunk strings + per-text chunk counts + per-chunk
        fan-in weights (byte length, measured BEFORE the prompt is prepended —
        runtime.rs weights the same way, and the constant prompt would flatten the
        ratio a short trailing window depends on)."""
        prompt = self.spec[f"{kind}_prompt"]
        budget = self.window - self._prompt_tokens(prompt)
        chunks, counts, weights = [], [], []
        for text in texts:
            parts = split_on_overflow(self._offsets, text, budget)
            # Mirrors runtime.rs MAX_DOCUMENT_CHUNKS: a DOCUMENT contributes at most
            # this many windows to its vector; the tail is dropped and never indexed.
            # QUERIES are never capped there, so they are never capped here either.
            if kind == "document" and self.max_doc_chunks:
                parts = parts[: self.max_doc_chunks]
            counts.append(len(parts))
            weights.extend(float(len(p.encode())) for p in parts)
            chunks.extend((prompt + p) if prompt else p for p in parts)
        return chunks, counts, weights

    def forward(self, chunks: list[str]):
        """One unit-norm vector per chunk, in input order. Length-sorted
        sub-batches so padding tracks each sub-batch's own width (runtime.rs)."""
        import numpy as np
        import torch

        out = np.zeros((len(chunks), self.out_dim), dtype=np.float32)
        order = sorted(range(len(chunks)), key=lambda i: len(chunks[i]))
        for i in range(0, len(order), SUB_BATCH):
            idx = order[i : i + SUB_BATCH]
            enc = self.tok(
                [chunks[j] for j in idx],
                padding=True,
                truncation=True,
                max_length=self.window,
                return_tensors="pt",
            ).to(self.device)
            with torch.no_grad():
                hidden = self.model(**enc).last_hidden_state
            if self.spec["pooling"] == "cls":
                pooled = hidden[:, 0]
            elif self.spec["pooling"] == "last":
                # EOS-token pooling with right padding: the last REAL token.
                last = enc["attention_mask"].sum(1) - 1
                pooled = hidden[torch.arange(hidden.size(0), device=hidden.device), last]
            else:
                mask = enc["attention_mask"].unsqueeze(-1).to(hidden.dtype)
                pooled = (hidden * mask).sum(1) / mask.sum(1).clamp(min=1e-9)
            if self.dense is not None:
                w, b = self.dense
                pooled = torch.nn.functional.linear(pooled, w, b)
            pooled = torch.nn.functional.normalize(pooled, p=2, dim=1)
            if self.spec.get("mrl"):
                # Truncate then RENORMALIZE, per chunk, before the fan-in (runtime.rs).
                pooled = torch.nn.functional.normalize(
                    pooled[:, : self.spec["mrl"]], p=2, dim=1
                )
            out[idx] = pooled.float().cpu().numpy()
        return out

    def embed(self, texts: list[str], kind: str):
        """One vector per text: single chunk passes through, multi-chunk is pooled
        WEIGHTED BY CHUNK BYTE LENGTH then L2-normalized — runtime.rs
        `weighted_mean_pool_l2`, so a short trailing window does not count as much
        as a full one."""
        import numpy as np

        chunks, counts, weights = self.chunk(texts, kind)
        vecs = self.forward(chunks)
        out = np.zeros((len(texts), self.out_dim), dtype=np.float32)
        cursor = 0
        for i, count in enumerate(counts):
            group = vecs[cursor : cursor + count]
            # Mirrors runtime.rs: a non-positive weight falls back to 1.0.
            w = np.asarray(weights[cursor : cursor + count], dtype=np.float32)
            cursor += count
            out[i] = group[0] if count == 1 else l2((group * np.where(w > 0, w, 1.0)[:, None]).sum(axis=0))
        return out, len(chunks)


# --------------------------------------------------------------- judgement ---


def grams(text: str, n: int = 5) -> set:
    return {text[i : i + n] for i in range(max(len(text) - n + 1, 1))}


def relevant_sets(corpus, queries, threshold: float) -> dict[str, list[str]]:
    """source anchor + every corpus doc whose char 5-gram Jaccard with it >= t."""
    by_id = {d["id"]: d for d in corpus}
    corpus_grams = [(d["id"], grams(d["text"])) for d in corpus]
    out = {}
    cache: dict[str, list[str]] = {}
    for q in queries:
        src = q["source_anchor_id"]
        if src not in cache:
            if src not in by_id:
                raise SystemExit(f"query {q['id']}: source_anchor_id {src!r} not in corpus")
            sg = grams(by_id[src]["text"])
            rel = [src]
            for cid, cg in corpus_grams:
                if cid == src:
                    continue
                inter = len(sg & cg)
                if inter and inter / (len(sg) + len(cg) - inter) >= threshold:
                    rel.append(cid)
            cache[src] = rel
        out[q["id"]] = cache[src]
    return out


# ----------------------------------------------------------------- metrics ---


def ndcg_at_k(ranked: list[str], rel: set, k: int = 10) -> float:
    dcg = sum(1 / math.log2(i + 2) for i, d in enumerate(ranked[:k]) if d in rel)
    idcg = sum(1 / math.log2(i + 2) for i in range(min(k, len(rel))))
    return dcg / idcg if idcg else 0.0


def recall_at_k(ranked: list[str], rel: set, k: int = 10) -> float:
    return len(set(ranked[:k]) & rel) / len(rel) if rel else 0.0


# -------------------------------------------------------------------- main ---


def sha(*parts: str) -> str:
    h = hashlib.sha256()
    for p in parts:
        h.update(p.encode())
        h.update(b"\0")
    return h.hexdigest()[:16]


def run_model(name, spec, corpus, queries, device, window, cache_dir, args):
    import numpy as np

    corpus_hash = sha(*(d["id"] + "\0" + d["text"] for d in corpus))
    # The chunk cap changes the DOC vectors, so it must key the doc cache (the
    # query cache derives from `key`, and queries are never capped — harmless).
    # "wmean" = the fan-in rule; bump it whenever the pooling changes, or a cache
    # written by the old rule is silently re-scored as if it were the new one.
    key = sha(name, spec["revision"], corpus_hash, str(window), f"cap{args.max_doc_chunks}", "wmean")
    cache = cache_dir / f"{name}-{key}.npz"
    qkey = sha(key, *(q["query"] for q in queries))
    qcache = cache_dir / f"{name}-q-{qkey}.npz"

    emb = None
    if cache.exists() and qcache.exists() and not args.no_cache:
        blob = np.load(cache, allow_pickle=True)
        doc_vecs, meta = blob["vecs"], json.loads(str(blob["meta"]))
        query_vecs = np.load(qcache)["vecs"]
        print(f"  [{name}] cache hit ({cache.name})", file=sys.stderr)
    else:
        emb = Embedder(spec, device, window, args.max_doc_chunks)
        t0 = time.perf_counter()
        doc_vecs, chunk_count = emb.embed([d["text"] for d in corpus], "document")
        secs = time.perf_counter() - t0
        meta = {
            "embed_seconds": secs,
            "chunks": chunk_count,
            "size_mb": emb.size_mb,
            "anchors_per_sec": len(corpus) / secs,
            "chunks_per_sec": chunk_count / secs,
        }
        np.savez(cache, vecs=doc_vecs, meta=json.dumps(meta))
        query_vecs, _ = emb.embed([q["query"] for q in queries], "query")
        np.savez(qcache, vecs=query_vecs)

    norms = np.linalg.norm(doc_vecs, axis=1)
    assert np.allclose(norms, 1.0, atol=1e-4), f"{name}: vectors are not unit-norm"

    ids = [d["id"] for d in corpus]
    scores = query_vecs @ doc_vecs.T  # exhaustive brute-force cosine, no ANN
    ranks = np.argsort(-scores, axis=1)

    per_query = []
    for i, q in enumerate(queries):
        ranked = [ids[j] for j in ranks[i]]
        rel = set(args.rel[q["id"]])
        per_query.append(
            {
                "id": q["id"],
                "kind": q["kind"],
                "ndcg@10": ndcg_at_k(ranked, rel),
                "recall@10": recall_at_k(ranked, rel),
                "rank_of_source": ranked.index(q["source_anchor_id"]) + 1,
            }
        )

    def agg(rows):
        if not rows:
            return None
        return {
            "n": len(rows),
            "ndcg@10": sum(r["ndcg@10"] for r in rows) / len(rows),
            "recall@10": sum(r["recall@10"] for r in rows) / len(rows),
        }

    return {
        "model": name,
        "repo": spec["repo"],
        "revision": spec["revision"],
        "dim": spec.get("mrl") or spec["dim"],  # the STORED width
        "pooling": spec["pooling"],
        **meta,
        "overall": agg(per_query),
        "screenText": agg([r for r in per_query if r["kind"] == "screenText"]),
        "audioTranscript": agg([r for r in per_query if r["kind"] == "audioTranscript"]),
        "per_query": per_query,
    }


def table(results):
    head = (
        "| model | dim | size MB | nDCG@10 | R@10 | nDCG screen | nDCG audio "
        "| chunks | embed s | anchors/s |"
    )
    rows = [head, "|" + "---|" * 10]
    for r in results:
        def cell(agg, k):
            return f"{agg[k]:.3f}" if agg else "—"

        rows.append(
            f"| {r['model']} | {r['dim']} | {r['size_mb']:.0f} | "
            f"{cell(r['overall'], 'ndcg@10')} | {cell(r['overall'], 'recall@10')} | "
            f"{cell(r['screenText'], 'ndcg@10')} | {cell(r['audioTranscript'], 'ndcg@10')} | "
            f"{r['chunks']} | {r['embed_seconds']:.1f} | {r['anchors_per_sec']:.1f} |"
        )
    return "\n".join(rows)


def self_test():
    """One runnable check for the non-trivial maths (no model download)."""
    import numpy as np

    def ws_offsets(text):  # whitespace tokenizer: one token per word
        out, i = [], 0
        for word in text.split(" "):
            if word:
                out.append((i, i + len(word)))
            i += len(word) + 1
        return out

    # budget = 4 - 2 special = 2 words/chunk; nothing truncated or dropped.
    text = "alpha bravo charlie delta echo foxtrot"
    assert split_on_overflow(ws_offsets, text, 4) == [
        "alpha bravo",
        "charlie delta",
        "echo foxtrot",
    ]
    assert split_on_overflow(ws_offsets, text, 99) == [text]
    # remainder survives
    assert split_on_overflow(ws_offsets, "a b c d e", 4)[-1] == "e"

    pooled = l2(np.array([[3.0, 0.0], [0.0, 3.0]]).mean(axis=0))
    assert abs(np.linalg.norm(pooled) - 1) < 1e-6 and abs(pooled[0] - pooled[1]) < 1e-6

    # Cross-chunk fan-in mirrors runtime.rs `weighted_mean_pool_l2`: a long window
    # and a 1-byte tail must NOT count the same. Stub Embedder — no model download.
    stub = object.__new__(Embedder)
    stub.spec = {"document_prompt": None, "query_prompt": None}
    stub.window, stub.max_doc_chunks, stub.out_dim = 4, 0, 2
    stub._offsets = ws_offsets
    stub._prompt_tokens = lambda prompt: 0
    stub.forward = lambda chunks: np.array([[1.0, 0.0], [0.0, 1.0]], dtype=np.float32)
    # "aaa… bbb… c" -> chunks "aaa… bbb…" (39 bytes) and "c" (1 byte).
    vec, _ = stub.embed(["a" * 19 + " " + "b" * 19 + " c"], "document")
    assert abs(np.linalg.norm(vec[0]) - 1) < 1e-5
    assert abs(vec[0][0] - 39 / math.hypot(39, 1)) < 1e-4, (
        f"fan-in is not byte-length weighted: {vec[0]} (uniform pooling gives 0.707)"
    )

    assert ndcg_at_k(["a", "b"], {"a"}) == 1.0
    assert abs(ndcg_at_k(["b", "a"], {"a"}) - 0.6309) < 1e-3
    assert ndcg_at_k(["a", "b"], {"a", "b"}) == 1.0
    assert recall_at_k(["a", "x"], {"a", "b"}) == 0.5

    assert grams("abcde") == {"abcde"}
    j = lambda a, b: len(grams(a) & grams(b)) / len(grams(a) | grams(b))
    assert j("hello world", "hello world") == 1.0 and j("aaaaaaa", "zzzzzzz") == 0.0
    # `trust_remote_code` downloads a .py from the Hub and EXECS it as this user — on
    # the machine that holds the capture DB key and the harvested corpus. A spec whose
    # `auto_map` points at ANOTHER repo is NOT pinned by its weights SHA (transformers
    # falls back to that repo's `main`), so every such spec must pin `code_revision`.
    for _name, _spec in MODELS.items():
        if not _spec["trust_remote_code"]:
            continue
        _pin = _spec.get("code_revision")
        assert _pin and re.fullmatch(r"[0-9a-f]{40}", _pin), (
            f"{_name}: trust_remote_code without a pinned code_revision ({_pin!r}) — "
            f"modeling code would be fetched from HEAD of {_spec['repo']}'s auto_map "
            f"repo and executed"
        )

    print("self-test ok")


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--corpus", help="JSONL: {id, kind, text, ...}")
    p.add_argument("--queries", help="JSON: {method, queries:[{id, query, source_anchor_id, kind}]}")
    p.add_argument("--out", help="results JSON path (NEVER a repo path — use the scratchpad)")
    p.add_argument("--models", default=",".join(MODELS), help=f"subset of {','.join(MODELS)}")
    p.add_argument("--window", type=int, default=MAX_EMBED_WINDOW_TOKENS)
    p.add_argument("--dup-threshold", type=float, default=0.6, help="5-gram Jaccard near-dup cutoff")
    p.add_argument("--max-doc-chunks", type=int, default=0,
                   help="cap each DOCUMENT at N token windows (runtime.rs MAX_DOCUMENT_CHUNKS); "
                        "0 = uncapped. Queries are never capped.")
    p.add_argument("--cache-dir", help="embedding cache (default: <out dir>/embcache)")
    p.add_argument("--no-cache", action="store_true")
    p.add_argument("--device", default=None, help="mps|cpu (default: mps if available)")
    p.add_argument("--self-test", action="store_true")
    args = p.parse_args()

    if args.self_test:
        return self_test()
    for required in ("corpus", "queries", "out"):
        if not getattr(args, required):
            p.error(f"--{required} is required")

    import torch

    device = args.device or ("mps" if torch.backends.mps.is_available() else "cpu")

    corpus = [json.loads(line) for line in Path(args.corpus).read_text().splitlines() if line.strip()]
    qdoc = json.loads(Path(args.queries).read_text())
    queries = qdoc["queries"]
    # A query set outlives the corpus harvest that produced it; an anchor the
    # broker no longer serves would otherwise abort the whole run. Drop those
    # queries, loudly — every model is then scored on the identical set.
    have = {d["id"] for d in corpus}
    dropped = [q["id"] for q in queries if q["source_anchor_id"] not in have]
    if dropped:
        print(
            f"WARNING: {len(dropped)} of {len(queries)} queries dropped — source anchor "
            f"not in corpus: {', '.join(dropped)}",
            file=sys.stderr,
        )
        queries = [q for q in queries if q["source_anchor_id"] in have]
    out_path = Path(args.out)
    cache_dir = Path(args.cache_dir) if args.cache_dir else out_path.parent / "embcache"
    cache_dir.mkdir(parents=True, exist_ok=True)

    args.rel = relevant_sets(corpus, queries, args.dup_threshold)
    sizes = [len(v) for v in args.rel.values()]
    dup = {
        "threshold": args.dup_threshold,
        "queries_with_multiple_relevant": sum(1 for s in sizes if s > 1),
        "max_relevant_set_size": max(sizes),
        "mean_relevant_set_size": sum(sizes) / len(sizes),
    }

    results = []
    for name in args.models.split(","):
        name = name.strip()
        if name not in MODELS:
            raise SystemExit(f"unknown model {name!r}; known: {', '.join(MODELS)}")
        print(f"[{name}] embedding {len(corpus)} anchors on {device}…", file=sys.stderr)
        results.append(run_model(name, MODELS[name], corpus, queries, device, args.window, cache_dir, args))

    report = {
        "caveat": CAVEAT,
        "device": device,
        "window": args.window,
        "corpus_size": len(corpus),
        "query_count": len(queries),
        "queries_dropped_missing_source": dropped,
        "query_method": qdoc.get("method"),
        "near_duplicate_expansion": dup,
        "results": results,
    }
    out_path.write_text(json.dumps(report, indent=2))

    print(f"\n{CAVEAT}\n")
    print(f"corpus={len(corpus)} anchors, queries={len(queries)}, window={args.window}, device={device}")
    print(
        f"near-dup expansion @ {dup['threshold']}: "
        f"{dup['queries_with_multiple_relevant']}/{len(queries)} queries have >1 relevant doc, "
        f"max relevant set = {dup['max_relevant_set_size']}, "
        f"mean = {dup['mean_relevant_set_size']:.2f}\n"
    )
    print(table(results))
    print(f"\nwrote {out_path}")


if __name__ == "__main__":
    main()
