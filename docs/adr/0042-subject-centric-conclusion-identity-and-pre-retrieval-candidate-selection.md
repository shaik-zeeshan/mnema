---
status: accepted (write-rule superseded by ADR 0043; candidate-recall + Subject-vector machinery retained)
date: 2026-06-30
---

# Conclusion identity is subject-centric, and the distillation LLM reuses handles via layered candidate recall

A **Conclusion**'s identity becomes its **Subject** alone, not the `(subject, statement)` pair, so
recurring evidence **reinforces** an existing belief instead of minting a reworded duplicate. The
distillation **Reasoning Engine** is the matcher — it is shown a **KNOWN SUBJECTS** block and told to
reuse a handle verbatim — and that candidate list is the **union of three recall legs**: a **recency
floor** of the newest handles, a model-free **lexical-overlap** leg, and **semantic KNN** against a new
**Subject Vector Store** when an embedding model is installed. This is forward-only; the existing
duplicate sprawl self-fades.

**Context — 0 warming and ~2,000 duplicate subjects.** Two root causes, both confirmed against the
live `~/.mnema` DB. (1) **Confidence History was decay-only**: the only writer of a history point was
the decay beat, which records non-increasing values, so a positive slope — the thing the **Subject**
view's "warming" tier detects — was structurally impossible, and a user had never once seen a subject
warm up. (2) **Beliefs duplicated instead of reinforcing**: a **Conclusion**'s identity was an exact
case-insensitive match on **both** `subject` AND `statement`, but the distillation LLM writes fresh
free-form text every window and was never shown the beliefs it already held, so the same real-world
belief came out slightly reworded each pass, missed the match key, and inserted a new row at formation
confidence (~0.54). One subject ("Marvel Rivals") had 133 near-identical rows; the dossier had
ballooned to ~2,000 subjects. The two causes share an upstream fix: make the same belief reinforce —
reinforcement is what climbs confidence, and climbing is what "warming" shows. (`fading = 0` is correct
data, not a bug — nothing has decayed near the 0.15 display floor — and is left alone.)

**Subject-centric reinforcement (the write path).** Two changes in
`crates/app-infra/src/user_context/store.rs` (`upsert_conclusion_with_evidence`), forward-only and not
touching the **Confidence Policy** math: (1) the upsert now snapshots a `user_context_confidence_history`
point when confidence ratchets **up** during reinforcement (and seeds one on formation), making a
positive trajectory slope reachable — the decay beat still owns the down direction. (2) the lookup is
now **subject-only** (`subject COLLATE NOCASE`, non-dismissed): if any visible row exists for the
subject it reinforces the **canonical row — highest confidence, ties broken by lowest id** — and does
not insert; a new row is inserted only for a genuinely new subject. On reinforce it bumps confidence,
replaces evidence, and snapshots the up-step but **freezes the statement text** — this dodges the
migration-0037 `UNIQUE(subject, statement)` index (kept as a harmless safety net), keeps one clean
per-row trajectory, and accepts that the displayed phrasing is frozen at first formation.

**LLM-as-matcher with a three-leg candidate recall union.** Code does not decide identity; the
**Reasoning Engine** does. `derivation.rs` carries a new **"KNOWN SUBJECTS — reuse these handles"**
prompt block (mirroring the existing user-authored / dismissed blocks) and one preamble sentence
instructing the model to reuse a handle verbatim when a belief is about an existing subject and to coin
a new handle only for a genuinely new one. Lexical *matching* — as the identity decider — was rejected
at the source: measured token overlap between duplicate rephrasings was ~31%, far too low to decide
identity. But lexical overlap is a fine *recall* signal: even one shared distinctive token surfaces a
candidate the LLM then judges, so it earns a place as a recall leg while the LLM stays the matcher.

The candidate list is the **union** of three legs, deduped case-insensitively, the **recency floor
first** so the freshest (most duplication-prone) handles always survive the `KNOWN_SUBJECTS_CHAR_CAP =
4000`-char truncation, then the related (lexical + semantic) legs, then the older recency tail:

- **Recency floor** — the newest `KNOWN_SUBJECTS_RECENCY_FLOOR = 30` distinct non-dismissed handles
  (`list_subject_handles_by_recency`, newest-supported-first). Always present, model or no model.
- **Lexical leg** — `list_subject_handles_by_lexical_overlap`: rank ALL non-dismissed subjects by
  whole-word, IDF-weighted overlap (name-boosted) of their name/statements against the recent Activity
  text, keep the best `KNOWN_SUBJECTS_LEXICAL_LIMIT = 20`. Reuses the same `recall_*` tokenizer/stemmer
  the `recall_context` broker tool uses, lifted into a shared `crate::lexical` module (Rust twin of the
  frontend `subjectSearch.ts`). **Model-free and lag-free** — it needs no embedding and no backfill, so
  it works in the default/prod config and catches the common case (a reworded duplicate shares words).
- **Semantic leg — Mode 1 (model installed)** (`subject_candidates.rs`): per-activity embed the
  distillation window (`EmbedKind::Query`), KNN top-`K_PER_ACTIVITY = 5` against the **Subject Vector
  Store**, union and dedup case-insensitively (keep max similarity), drop below cosine floor
  `SUBJECT_CANDIDATE_COSINE_FLOOR = 0.3`, cap at `SUBJECT_CANDIDATE_CAP = 40`. Catches *non-lexical*
  relatedness ("Apple" ↔ "iPhone") that the lexical leg cannot. Empty (no-op) when no model is
  installed.

The union replaces an earlier `semantic OR recency` **either/or** that was a live duplication bug: a
non-empty semantic set suppressed the recency fallback, and because the embedding backfill embeds a new
subject's vector only *after* the distillation that creates it, the freshest subjects — exactly the
ones the next overlapping distillation re-derives — were structurally invisible to semantic KNN and got
reworded into duplicates ("Marvel Rivals / gaming" → "Marvel Rivals gaming videos", confirmed in the
running app). The recency floor and the lag-free lexical leg both close that gap; the semantic leg adds
non-lexical recall on top.

Graceful degradation is **load-bearing**, not incidental: the semantic leg no-ops cleanly when no model
is installed — the common production configuration today (prod ships zero embedding model) — leaving the
recency + lexical legs, which need no model. Gated on `default_semantic_search_enabled()`, default model
`nomic-embed-text-v1.5`, **not bundled** (opt-in download); the day a model is installed the backfill
worker self-runs and the semantic leg turns on retroactively.

**Subject Vector Store (app-infra stays embedding-free).** Migration `0043` adds a **plain** table
`user_context_subject_vectors(subject TEXT PRIMARY KEY COLLATE NOCASE, embedding BLOB, embedded_at_ms
INTEGER)` — deliberately **not** `vec0`. At ~2k subjects, loading the BLOBs and brute-force f32 cosine
in Rust is microseconds, and `vec0` is rowid-keyed (awkward for string subjects); the **Semantic
Search** ANN gate ([ADR 0036](0036-semantic-search-v1-hybrid-fastembed-vectors-with-fts5.md)) does not
apply at this corpus size. `SubjectVectorStore` (`subject_vectors.rs`) does upsert/get/mark-stale/
needs-embedding/brute-force-cosine-KNN over `db.write()`. The crate **only stores BLOBs and does
cosine** — it holds no embedder, the same boundary that keeps `ai-runtime` out of app-infra. The
desktop **subject embedding backfill worker** (`subject_vector_worker.rs`, spawned on the
deferred-startup seam) produces the vectors with the same `semantic_search_worker` embedder helpers,
embedding text `"{subject}: {canonical_statement}"` (statement enrichment so a terse handle like
"Apple" carries disambiguating context). It is an **idle no-op when no model is installed**;
`user_context_dismiss_conclusion` marks the affected subject's vector stale for lazy re-embed.

**Vectors are model-identity aware (migration `0044` adds `embedded_model`).** Each row records the
`provider/model_id` it was embedded under. The worker's `list_subjects_needing_embedding(active_model,
…)` treats a vector embedded under a *different* model as stale, so after a model switch the existing
backfill worker continuously re-embeds the whole dossier under the new model — **no separate
startup reconciliation pass**. `subject_vector_knn(query, active_model, k)` ranks only vectors under
the active model, so a stale cross-model vector never contributes a meaningless cosine while it waits
to be re-embedded. This is deliberately **stricter than the Semantic Search index**, whose
dimension-only `vec0` rebuild
([ADR 0036](0036-semantic-search-v1-hybrid-fastembed-vectors-with-fts5.md)) is *forced* by `vec0`'s
fixed-dimension table — it never solved a same-dimension model swap, and re-embeds only as a
side-effect of the structural rebuild. The plain subject table has no such constraint, so keying off
model identity directly (covering both a dimension change *and* a same-dimension swap) is both simpler
and more correct here than copying the dimension-only behavior.

**Considered Options.** We rejected **code-side automatic merge by similarity threshold**: a wrong
merge silently collapses two genuinely distinct beliefs with no correction path, whereas the LLM
matcher errs visibly and reversibly. We rejected **`vec0`/ANN for subject vectors**: brute-force is
faster than the index machinery at ~2k subjects and avoids the rowid-keying mismatch. We rejected a
**backfill/collapse migration** of the ~2,000 existing duplicates: forward-only is chosen instead — the
canonical row reinforces going forward and stale siblings fade under the existing decay, so no risky
one-time merge of legacy data is run. We rejected **embeddings as the matcher** (rather than recall):
measured lexical overlap ~31% rules out lexical matching, but the LLM remains the semantic decider
because it judges the actual belief, not just vector proximity.

**Consequences.** The fix is forward-only, so **transitional duplicate rows remain** (a legacy subject
still shows all its near-identical rows in the expanded view until the stale siblings fade below the
display floor — self-heals, no action). With **no embedding model** (the common prod config) the recency
floor + lexical leg still catch any duplicate that shares words with recent activity; only **non-lexical**
relatedness (an abstract-handle comeback like "Apple" ↔ "iPhone") is missed until a model is installed —
the accepted floor, and the thing that fixes it is embeddings, absent by definition there.
The **Confidence Policy** and reinforce math are untouched — only routing changed. Per
[ADR 0029](0029-user-context-outlives-raw-retention-privacy-delete-cascades.md), the subject vectors
are derived **User Context** and are cleared by Wipe User Context, never cascaded by **Retention
Policy**. The embedder seam is the same **Semantic Search** machinery
([ADR 0036](0036-semantic-search-v1-hybrid-fastembed-vectors-with-fts5.md) /
[ADR 0037](0037-semantic-search-embeddings-on-candle-with-pluggable-backend.md)), so the candle backend
and model-gated default-on rollout carry over for free.

**Empirical validation still outstanding (runtime, not code).** The
`SUBJECT_CANDIDATE_COSINE_FLOOR` / `K_PER_ACTIVITY` / `SUBJECT_CANDIDATE_CAP` values are starting
points to be **calibrated against real subject clusters** (intra-duplicate vs cross-subject similarity)
once a model runs on real data; a too-high floor starves recall, a too-low one floods the prompt with
noise. Two behaviors must be confirmed in the running app, not in unit tests: that the distillation LLM
**actually reuses a supplied handle** when one is offered (the "attention over the candidate list" risk
the whole dedup benefit rests on), and that **non-zero "warming" appears** in the Subjects view after
recurring activity reinforces a subject.
