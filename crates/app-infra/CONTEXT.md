# App Infra Context

SQLite-backed capture state, migrations, processing jobs, frame/OCR persistence, retention cleanup, search projections, broker policy, and storage layout.

Root entry point: [CONTEXT-MAP.md](../../CONTEXT-MAP.md).

## Language

**Screen Frame Artifact**:
A just-produced native capture output that has not yet been persisted into app-infra.
Screen frame artifacts are currently written as JPEG files.
_Avoid_: captured frame, transient screenshot, raw frame record

**Captured Frame Pipeline**:
A persisted flow for a captured screen frame from intake through batch attachment, OCR planning, and batch-finalization readiness.
_Avoid_: frame processing service, frame ingestion component, OCR pipeline

**Captured Frame**:
A screen image captured during a recording session and persisted as frame metadata plus a file path. A **Captured Frame** is the stored database-backed record, not the just-produced native artifact.
_Avoid_: screenshot, image record

**Frame Batch**:
A time-windowed group of captured frames for one recording session.
_Avoid_: bucket, chunk, frame group

**OCR Job**:
A processing job that recognizes text for one captured frame.
_Avoid_: processor task, recognition work item

**OCR Throughput Budget**:
The user-facing policy for limiting how much **OCR Job** work Mnema admits and schedules over time while preserving useful recognition quality.
_Avoid_: CPU cap, OCR governor, hard CPU limit

**OCR Quality Floor**:
The minimum acceptable recognition quality for automatic **OCR Job** work.
_Avoid_: fast fallback, cheap OCR mode, best-effort recognition

**OCR Settings Selection**:
The user-selected OCR provider, model, language, recognition mode, and provider-specific options used for automatic **OCR Job** admission.
_Avoid_: automatic provider choice, adaptive OCR mode, hidden fallback

**OCR Admission Budget**:
The part of the **OCR Throughput Budget** that decides which automatic **Captured Frame** values should receive an **OCR Job**.
_Avoid_: frame dropper, OCR enqueue throttle, skip rule

**OCR Candidate**:
A **Captured Frame** that is eligible for automatic **OCR Job** admission after budget and value checks.
_Avoid_: sampled frame, selected screenshot, OCR frame

**OCR-Relevant Change**:
A visual or context change in a **Captured Frame** that is likely to change useful recognized text.
_Avoid_: pixel change, screen difference, visual delta

**OCR-Relevance Probe**:
An extra pre-admission analysis pass that tries to detect whether a **Captured Frame** likely contains useful text before admitting an **OCR Job** and costs meaningful CPU beyond already available capture or persistence data.
_Avoid_: lightweight OCR, pre-OCR, text detector

**OCR Admission Reason**:
A runtime/debug explanation for why automatic OCR was or was not admitted for an **OCR Candidate**. It may be exposed through live debug state or logs, but it is not durable user data.
_Avoid_: debug log, skip note, enqueue reason

**OCR Budget Telemetry**:
Live-only cost and usefulness summary data used to inspect **OCR Throughput Budget** behavior during the current app run without duplicating recognized text. It may be kept in bounded memory and logs, but it is not durable user data.
_Avoid_: OCR text copy, performance log, debug-only metric

**OCR Execution Budget**:
The part of the **OCR Throughput Budget** that paces when already queued **OCR Job** values run.
_Avoid_: worker sleep, queue delay, CPU limiter

**OCR Catch-Up**:
The execution mode that processes deferred **OCR Job** backlog after recording stops or the app determines the machine is idle enough for more work.
_Avoid_: batch OCR, drain mode, background sweep

**Captured Frame Reprocessing**:
A request to re-run OCR for an existing **Captured Frame** that is already persisted.
_Avoid_: force processing, rerun pipeline, requeue screenshot

**Encrypted Capture Index**:
A future ADR-backed storage protection for Mnema's SQLite-backed searchable and contextual capture data, excluding original frame, video, and audio media.
_Avoid_: encrypted capture store, media encryption, secure erase

**Capture Index Key Store**:
The platform-owned secret storage boundary that holds **Encrypted Capture Index** keys outside the recording save directory.
_Avoid_: key file, save-directory secret, hard-coded key

**User Context Store**:
The app-infra storage owner for the User Context dossier (`crates/app-infra/src/user_context/store.rs`, `UserContextStore`) over the `user_context_*` tables added in migrations `0022`–`0025`: Activities + evidence + derivation runs, Conclusions + evidence, Confidence history, and pinned/dismissal state. It also owns the deterministic policy that needs no model — the fixed Confidence Policy math (`confidence.rs`) and the Sensitive Category Guardrail (`guardrail.rs`, soft instruction text plus the hard `is_sensitive` post-filter) — plus the capture-window reader (`capture_source.rs`) that assembles already-redacted OCR/transcript text. The LLM call itself lives in the desktop Tauri layer, so app-infra takes no `ai-runtime`/`rig-core` dependency.
_Avoid_: profile table, inference engine, dossier service, ai-runtime dependency

**Captured Frame Equivalence**:
The rule for when two **Captured Frame** values should be treated as the same OCR-relevant visual content for downstream decisions such as **OCR Job** admission. **Captured Frame Equivalence** is defined over normalized visual content rather than persisted artifact bytes, and intentionally ignores cursor-sized changes plus limited localized visual noise that does not materially change OCR-relevant content.
_Avoid_: dedupe hash, screenshot sameness, OCR skip heuristic

**Captured Frame Equivalence Scope**:
The rule for where an earlier equivalent **Captured Frame** may be searched when applying **Captured Frame Equivalence**. **Captured Frame Equivalence Scope** is session-wide by default, but narrows to the same hidden segment workspace when the candidate **Captured Frame** originated from a hidden segment workspace artifact path.
_Avoid_: workspace filter, lookup scope, same-segment rule

**OCR Fallback Eligibility**:
The rule that an earlier equivalent **Captured Frame** can stand in for a later frame's OCR only when that earlier frame already has a non-failed **OCR Job** — that is, a job whose status is queued, running, or completed (including completed with no recognized text). Eligibility does not require that the job already produced text: because equivalent frames share the same OCR-relevant content by definition, a later frame cannot contribute text that the representative's identical content didn't, so an equivalent group with a completed-but-textless job is still correctly suppressed. A **Captured Frame** that was itself skipped by the **OCR Admission Budget** and has no **OCR Job** is not an eligible fallback (there is no job whose result could ever project back); a frame whose only **OCR Job** has failed is likewise not eligible, since a failed job produced and persisted no result to project.
_Avoid_: dedupe reuse, equivalent skip, text borrow

**Visual Novelty Admission**:
The bounded **OCR Admission Budget** rule that admits a non-equivalent **Captured Frame** whose **Captured Frame Equivalence** fingerprint is new within its admission scope this run, so a one-off readable screen inside an unchanging window is still read rather than lost. It reuses the existing fingerprint as an **OCR-Relevant Change** signal and adds no **OCR-Relevance Probe**. It is bounded by the high-pressure gate, a per-scope rate cap, and a continuous-novelty suppressor that falls back to fixed time cadence for video/animation; it complements **OCR Fallback Eligibility**, which covers repeated frames while **Visual Novelty Admission** covers one-off frames.
_Avoid_: new-frame OCR, fingerprint trigger, scroll detector, video OCR

**Orphaned Processing Job**:
A processing job left in `running` because its execution was abandoned — the owning worker was aborted at app quit or on a crash — with no live executor still working it.
_Avoid_: stuck job, zombie job, hung job, running-forever job

**Processing Job Reclamation**:
The startup-and-shutdown policy that returns an **Orphaned Processing Job** to `queued` so it re-runs and still produces its result, instead of marking it permanently failed.
_Avoid_: orphan cleanup, fail-all-running sweep, requeue hack

**Hidden Segment Workspace**:
A hidden per-segment directory (`.<session>-segment-####/`) that stores temporary capture artifacts and exported JPEG frames for one screen segment. A **Hidden Segment Workspace** lives beside its visible sibling segment recording file.
_Avoid_: temp folder, segment scratch dir, hidden segment temp

**Hidden Segment Workspace Repair**:
The scan-and-cleanup flow that classifies a **Hidden Segment Workspace** using **Frame Batch** references, **OCR Job** references, whether the visible sibling segment is present and openable, pending frame artifacts, and whether the workspace is still the active screen session before deciding whether it is safe to remove.
_Avoid_: temp cleanup, workspace GC, segment dir sweep

**Managed Storage Layout**:
The derived on-disk layout rooted at `<saveDirectory>` that owns app-infra state such as SQLite and the recordings tree.
_Avoid_: save dir helper, path utility, storage paths

**Capture Session**:
The logical user recording that groups produced screen, microphone, and system-audio source sessions for retention and deletion policy.
_Avoid_: recording row, source session, runtime session

**Capture Segment**:
The unified retention deletion unit for one produced screen, microphone, or system-audio artifact. A **Capture Segment** points at DB-derived subjects such as **Captured Frame** and **Audio Segment** rows and at DB-owned filesystem paths when present.
_Avoid_: file row, media chunk, retention item

**Capture Segment Duration**:
The configured maximum wall-clock duration for one produced **Capture Segment** before the **Recording Lifecycle** rotates to the next segment.
_Avoid_: recording duration, session length

**Retention Policy**:
The user-selected local retention window for capture data: never, 7 days, 14 days, or 30 days. Calendar policies keep today plus the previous local calendar days, not rolling hours.
_Avoid_: cleanup interval, TTL

**Retention Cleanup**:
The app-infra deletion flow that removes eligible **Capture Segment** values and their derived frames, audio segments, processing jobs/results, speaker rows, segment-derived voice embeddings, and rejections while preserving user-authored **Person Profile** rows.
_Avoid_: purge, vacuum, file cleanup

**Search Result Anchor**:
The domain object and time position a search result opens, either a **Captured Frame** or an **Audio Transcription Span**.
_Avoid_: result row, search document, record match

**Text Search**:
The search tier that matches query terms against recognized screen text, recognized speech text, and capture context.
_Avoid_: keyword-only search, exact search, string filter

**Semantic Search**:
The search tier that matches meaning rather than only literal query terms across searchable captured content.
_Avoid_: embedding support, vector lookup, AI search

**Semantic Search Model**:
The model used to derive meaning vectors for **Semantic Search** — a local model asset by default, and only a remote model when a cloud **Semantic Search Backend** is explicitly opted into.
_Avoid_: cloud embedding service, embedding API, vector model

**Semantic Search Backend**:
The runtime that derives a **Semantic Search Vector** from text, pluggable behind one seam. The default and only shipped backend is **candle**, running on-device on the Apple GPU (Metal) or CPU — it replaces the earlier in-process `fastembed`/ONNX runtime. Local backends (candle now; a local Ollama/llamafile runtime later) embed raw body text on-device; a remote/cloud backend (e.g. an OpenAI-compatible embeddings API) is **opt-in only, never the default**, and may embed only text that has crossed the egress-redaction boundary. Switching the backend re-derives every **Semantic Search Vector** (vectors from different backends are not comparable), exactly like switching the **Semantic Search Model Tier**.
_Avoid_: embedding engine, inference provider, ONNX runtime

**Semantic Search Vector**:
A local meaning vector derived for one **Search Result Anchor** and stored inside the **Encrypted Capture Index**. Computed only for direct (non-reused) anchors; equivalent anchors reuse their group's vector.
_Avoid_: embedding blob, vector row, anchor embedding

**Semantic Index Backfill**:
The background sweep that derives a **Semantic Search Vector** for anchors that have searchable text but no vector yet, covering live and historical capture in one pass. It runs only on idle/background capacity, prioritizes freshly captured anchors over backlog, and is pausable and resumable.
_Avoid_: reindex job, embedding queue, vector worker

**Semantic Search Model Tier**:
The user-facing model choice for **Semantic Search**: guided tiers (an English default and a Multilingual option) plus a curated selection of other supported models. Model choice is a deliberate setup decision because changing it re-derives every **Semantic Search Vector**. Because the **Semantic Search Backend** (candle) has no model catalog of its own, each tier is a **hand-maintained descriptor** declaring its architecture, dimension, pooling, token window, weight layout (a `model.safetensors`, or a PyTorch `.bin`/`.pth` checkpoint for a repo that ships no safetensors, e.g. `bge-m3`), and source repo. A CI guard cross-checks each descriptor against the model's own `config.json` (dimension, layer count), so a hand-coded fact that drifts from the real weights fails loudly. Only models whose architecture the backend implements (e.g. nomic-bert, XLM-RoBERTa, BERT) can be offered — there is no open "any model" picker.
_Avoid_: model dropdown, preset list, embedding provider, parallel model catalog

**Hybrid Search**:
The product search policy that combines **Text Search** and **Semantic Search** so literal and meaning-based matches can rank together.
_Avoid_: search mode toggle, vector-only search, fuzzy search

**Semantic Candidate Set**:
The in-scope **Search Result Anchor**s the **Semantic Search** tier returns at query time, ordered nearest-first, before **Hybrid Search** RRF fusion. It is produced by a filter-then-rank `vec0` KNN constrained to the active **Search Refinement** scope and is the read-time counterpart to **Semantic Index Backfill**'s write. The set carries only anchor *order* (rank-only) — never a vector distance score — because **Hybrid Search** fuses by rank, and it is empty whenever the live `vec0` column dimension disagrees with the query vector so a dimension mismatch degrades to keyword-only.
_Avoid_: knn results, vector matches, nearest neighbors, candidate list

**Search Index Projection**:
A durable derived view of searchable capture content used to answer search queries.
_Avoid_: search cache, search result table, indexing job output

**Search Context**:
Captured contextual labels that help find or filter a **Search Result Anchor**, such as app name, window title, browser URL, or speaker label.
_Avoid_: metadata blob, result decoration, search tags

**Search Refinement**:
An explicit user control that narrows an active search by retained capture context such as date range, app, source, or result type.
_Avoid_: advanced search, search mode, query syntax

**Search Query Syntax**:
An opt-in typed mini-language in the search input that the parser interprets beyond plain-text term matching; plain text stays the default and syntax is never required.
_Avoid_: required query language, search mode toggle, expert-only search

**Field Operator**:
A typed `key:value` token (`app:`, `after:`, `before:`, `source:`) that desugars into a visible, removable **Search Refinement** instead of matching body text.
_Avoid_: hidden filter syntax, inline scope flag, raw column filter

**Body Match Operator**:
A typed token (quoted phrase, `-term` exclusion, `OR`, `term*` prefix) that shapes how recognized text is matched, while space between terms stays an implicit AND.
_Avoid_: column filter, scope operator, refinement syntax

**Search Operator Suggestion**:
An opt-in two-tier autocomplete in the search input that offers **Search Query Syntax** operator names for discovery and then the values for the chosen operator.
_Avoid_: query builder, advanced search panel, filter wizard

**Search Entry Point**:
A contextual user action that starts search with an initial scope derived from where the user is in Mnema.
_Avoid_: search shortcut, preset search, smart search

**Visible Timeline Search**:
A **Search Entry Point** that scopes search to the captured time range currently visible in the dashboard timeline.
_Avoid_: loaded timeline search, current page search, visible rows search

**Current App Search**:
A **Search Entry Point** that scopes search to the retained app context of the active dashboard **Captured Frame**.
_Avoid_: frontmost app search, Mnema app search, system focus search

**App Search Refinement**:
A **Search Refinement** that narrows frame results by retained app identity, preferring bundle identifier and falling back to app name only when no bundle identifier was captured.
_Avoid_: app-name filter, window filter, bundle filter

**Audio Source Search Refinement**:
A **Search Refinement** that narrows audio results by retained audio recording source, such as microphone or system audio.
_Avoid_: result type filter, media filter, audio mode

**Screen Source Search Refinement**:
A **Search Refinement** that narrows results to captured frames (the screen capture source), the frame-side counterpart of an **Audio Source Search Refinement**. Produced by `source:screen`.
_Avoid_: frames tab, screenshot filter, video filter

**Date Range Search Refinement**:
A **Search Refinement** that narrows results to **Search Result Anchor** values whose captured time overlaps a selected time range.
_Avoid_: loaded range, page window, result date label

**Search Context Alignment**:
A derived relationship that decorates a **Search Result Anchor** with nearby retained context from another capture source without treating that context as native to the anchor.
_Avoid_: inferred source, guessed app, audio app context

**Search Snippet**:
A query-specific preview of why a **Search Result Anchor** matched.
_Avoid_: saved preview text, stored excerpt, result summary

**Search Result Group**:
A collapsed search result that represents multiple equivalent **Search Result Anchor** values.
_Avoid_: duplicate result, grouped row, result cluster

## Relationships

- A **Screen Frame Artifact** becomes a **Captured Frame** only after app-infra persists it.
- A **Captured Frame Pipeline** persists one **Captured Frame**.
- A **Captured Frame Pipeline** attaches each **Captured Frame** to exactly one **Frame Batch**.
- A **Captured Frame Pipeline** may enqueue one **OCR Job** for a **Captured Frame**.
- Mnema sanitizes browser URL metadata before persistence; full URL metadata remains an explicit user choice because query strings and fragments may contain secrets.
- Search ranking may use redaction categories conservatively, but should not strongly boost results merely because a secret was redacted.
- A **Semantic Search Vector** is derived from the same body text that **Text Search** indexes (not a separately-redacted copy) and is stored inside the **Encrypted Capture Index**; because both tiers live inside that boundary, the vector's at-rest exposure equals **Text Search**'s, and redaction for **Semantic Search** is enforced at any boundary that takes a vector or its text out of the index (export, external index, cloud reranker), not by redacting the at-rest vector.
- **Encrypted Capture Index** protects SQLite-backed searchable and contextual capture data, not original frame, video, or audio media.
- Original media files remain sensitive even when **Encrypted Capture Index** is enabled, and raw media encryption is out of scope for the first storage-security phase.
- **Encrypted Capture Index** should use maintained page-level SQLite encryption rather than hand-rolled field encryption.
- **Encrypted Capture Index** keys belong in a **Capture Index Key Store** outside `saveDirectory`; macOS should use Keychain through that abstraction.
- Each **Encrypted Capture Index** should have its own key tied to a stable index identity rather than sharing one global app key.
- An **Encrypted Capture Index** may expose non-secret index identity metadata through a readable header or sidecar so the app and broker can locate the corresponding **Capture Index Key Store** entry.
- If an **Encrypted Capture Index** key is missing or inaccessible, Mnema treats the index as undecryptable unless an explicit future backup/export key flow exists; fallback keys must not live in `saveDirectory`.
- V1 does not change default browser URL metadata settings.
- **OCR Admission Budget** is not a privacy layer and does not skip **Captured Frame** values based on inferred sensitive content in V1.
- **App Privacy Exclusion** is app-based rather than website-, title-, private-browser-, or private-window-based.
- A **Capture Session** can filter or group search results, but is not itself a content-bearing **Search Result Anchor**.
- **Capture Segment Duration** applies to each rotated **Capture Segment**, not to total **Capture Session** length.
- A **Capture Segment** supports storage, recovery, and retention boundaries rather than acting as a **Search Result Anchor**.
- A **Retention Policy** applies only to the active **Managed Storage Layout** and active app-infra database.
- **Retention Cleanup** skips active capture segments and subjects with running processing/finalize jobs.
- A search result for screen text should anchor to a **Captured Frame**.
- A **Captured Frame** should be indexed as one searchable screen document even when OCR returns multiple text observations.
- OCR observations may provide highlight context for a **Captured Frame** search result, but they are not separate **Search Result Anchor** values.
- Equivalent **Captured Frame** values that reuse the same OCR text should remain searchable at their own capture times.
- Equivalent **Captured Frame** search hits should collapse into a **Search Result Group** so unchanged screens do not flood the result list.
- A **Search Result Group** for equivalent **Captured Frame** hits should open the newest matching frame by default.
- **Text Search** should search recognized screen or speech body text plus typed **Search Context** fields.
- Recognized screen or speech body text should rank higher than **Search Context** matches by default.
- **Search Context** should include only context Mnema actually captured and retained.
- **Search Context** should enrich content-bearing search results rather than create standalone search results.
- **Semantic Search** augments **Text Search** rather than replacing it.
- **Semantic Search** is **local-first**: the default **Semantic Search Backend** runs entirely on-device, and any remote/cloud backend is opt-in only and never the default.
- The **Semantic Search Backend** is pluggable behind one seam; **candle** (Apple GPU via Metal, or CPU) is the default and only shipped backend, replacing the earlier in-process `fastembed`/ONNX runtime. `ort` remains in the workspace only for audio transcription.
- **Semantic Search** indexing may run as separate background work because embedding generation is model work, not simple projection maintenance.
- By default **Semantic Search** derives vectors on-device and sends no captured content off-device; a remote/cloud **Semantic Search Backend** is opt-in only and may embed only egress-redacted text — never raw capture — so a local backend and a cloud backend produce non-comparable vectors (the cloud tier trades recall for the egress guarantee).
- **Hybrid Search** is the product direction once **Semantic Search** exists.
- A **Semantic Search Vector** is derived per **Search Result Anchor** (one vector per searchable document), computed only for direct anchors while equivalent anchors reuse their group's vector.
- A **Semantic Search Vector** is stored inside the **Encrypted Capture Index** (a `vec0` table in the encrypted database), never in a plaintext sidecar file.
- **Hybrid Search** fuses **Text Search** and **Semantic Search** by rank (reciprocal rank fusion) rather than by combining their raw scores, at the **Search Result Anchor** level before **Search Result Group** creation and pagination.
- **Semantic Search** ranks only within the **Search Refinement**-scoped candidate set (filter-then-rank), which is required for correct top-k results, not merely faster.
- The **Semantic Search** read path returns a **Semantic Candidate Set** (the in-scope anchors, nearest-first) from one seam that owns the `vec0` substrate — the query-vector serialization, the `MATCH … k … rowid IN (…)` KNN, and the live-dimension gate — so the meaning tier's vector format and KNN live in one place beside the **Semantic Index Backfill** write serializer, and a future int8/binary/ANN change is a single-module edit rather than a fusion-SQL edit.
- A **Semantic Candidate Set** carries only anchor order (rank-only), never a vector distance, because **Hybrid Search** fuses by rank; surfacing a distance would invite the weighted-score fusion ADR 0036 rejected. Adding a distance later is a deliberate, ADR-touching change, not a default.
- A **Semantic Candidate Set** is empty when the live `vec0` column dimension disagrees with the query vector, so the single dimension authority gates the read at its source and a mismatch degrades to keyword-only without ever reaching the `vec0` error path.
- A **Search Snippet** for a meaning-only **Semantic Search** hit shows a leading body-text excerpt marked as a meaning match and reuses the same redaction masking as a **Text Search** snippet.
- **Hybrid Search** keeps recency as a tie-break only and adds no time-decay ranking boost in its first version.
- **Semantic Search** is default-on but inert without an installed **Semantic Search Model**: the same "no model means the feature is unavailable" shape as local transcription, and Mnema never auto-downloads a model nor blocks capture on its absence.
- **Semantic Index Backfill** prioritizes newly projected anchors over historical backlog, drains backlog newest-first, and never runs on the capture hot path.
- A **Search Index Projection** delete removes the matching **Semantic Search Vector** in the same cleanup, and reprocessing a **Captured Frame** or **Audio Segment** replaces its **Semantic Search Vector** when the new result completes.
- Changing the selected **Semantic Search Model Tier** — or the **Semantic Search Backend** — re-derives every **Semantic Search Vector**, because vectors from different models or backends are not comparable; the choice is therefore a confirmed action rather than a casual toggle.
- Changing the model is **atomic**: one backend command resolves the target model's dimension, rebuilds the `vec0` table at that dimension first (the step that can fail), and persists the new selection only after the rebuild commits — a failed rebuild leaves the old model selected and the old-dimension table intact, never a half-switched state. There is no separate "persist then re-index" two-step.
- The single authority for the active vector width is the **live `vec0` column dimension** (`float[N]` in the `search_document_vectors` DDL), not the separately-persisted model selection. Both the **Semantic Index Backfill** store path and the **Semantic Search** query path read it and skip/idle on a mismatch rather than storing or querying a wrong-length vector.
- On startup (deferred-startup seam, before the **Semantic Index Backfill** worker spawns), the live `vec0` dimension is reconciled against the selected **Semantic Search Model**'s expected dimension and the table is rebuilt if they disagree; this is idempotent (a matching table is untouched) and self-heals an index left stuck by a failed model switch.
- A **Semantic Search** fetch error (including a dimension mismatch) **degrades to keyword-only** — the error is logged and the meaning hits fuse as an empty list, so the whole search never hard-fails — preserving the "no usable runtime means the feature is unavailable, not broken" shape **Text Search** already has.
- Storing a **Semantic Search Vector** is **conditioned on the owning Search Result Anchor still existing** in the same statement (an insert that selects from `search_documents`), so a retention or Delete Recent delete racing the backfill worker inserts zero rows instead of leaving an orphan vector of deleted content at rest. Non-finite (NaN/Inf) embeddings are rejected before insert.
- The **Semantic Index Backfill** quarantines a poison-pill anchor after a few consecutive **deterministic** embed failures (distinct from transient store/DB retries), so one pathological anchor cannot error-loop the sweep forever; quarantine is in-memory only (no migration), and reprocessing the source produces a new anchor id that is retried fresh. Repeated **Semantic Search Model** load failures surface a "model appears corrupt — reinstall" signal rather than retrying a broken model indefinitely.
- The default **Semantic Search Model Tier** is English; multilingual capture is served by an explicit Multilingual tier or a Custom model so a non-English user is guided to a fitting model rather than silently degraded. The shipped Multilingual tier is `multilingual-e5-small` (MIT, ungated) rather than the originally-named `embeddinggemma-300m`, whose Hugging Face repo is access-gated and so unreachable by the desktop-owned downloader.
- Because the **Semantic Search Backend** (candle) has no model catalog of its own, every **Semantic Search Model Tier** is a **hand-maintained descriptor** declaring its architecture, dimension, pooling, token window, weight layout, and source repo. This reverses the earlier fastembed-synthesized "never hand-restate" overlay — there is no catalog left to overlay once fastembed is removed. A CI guard cross-checks each descriptor against the model's own `config.json` (dimension, layer count) so a hand-coded fact that drifts from the real weights fails loudly; it reads committed fixtures (no model download) and is the `descriptor_dimension_matches_config_json` test, run on every PR. The end-to-end bge-m3 PyTorch-`.bin` loadability test (`tests/bge_m3_pth_load.rs`) and the candle lossless-parity test (`tests/candle_parity.rs`) need the multi-hundred-MB weights, so they are `#[ignore]`/env-gated and are NOT run automatically; run them by hand with the documented env vars. Only models whose architecture the backend implements can be offered; a user wanting another model reaches for a different **Semantic Search Backend** (e.g. a local Ollama runtime), not an open "any model" picker.
- A **Semantic Search Model**'s on-disk file list has one authority — the descriptor's declared layout (the weights file, `config.json`, and `tokenizer.json`) — consumed by both the install-completeness check and the download plan, so what the downloader fetches and what the completeness check requires can never diverge. The weights file is usually `model.safetensors`, but for a repo that ships no safetensors it may instead be a PyTorch `.bin`/`.pth` checkpoint loaded via the pickle path (e.g. `bge-m3`, which ships only `pytorch_model.bin`); the descriptor's declared layout is the single authority either way. Bumping the model-manifest version invalidates older ONNX-shaped installs so they re-download in the descriptor's declared layout.
- A downloaded **Semantic Search Model** is **integrity-verified fail-closed for every pinned tier**: the guided tiers (`nomic-embed-text-v1.5` English, `multilingual-e5-small` Multilingual) AND the curated `bge-m3` Custom model carry a real per-file SHA256 (the weights file plus `config.json`/`tokenizer.json` for the guided tiers, the `pytorch_model.bin` for bge-m3), verified against the downloaded bytes before the install marker is written — a tampered/truncated file fails verification and never installs. The guided tiers go further: a fail-closed gate refuses to mark them Installed unless their weights matched a pinned digest, so a guided tier can never slip down the unverified path. Beyond the pinned digests, every download is also pinned to an immutable Hugging Face commit revision (not the mutable `main` ref) and guarded by a Content-Length truncation check. Only genuinely-unpinned **Custom** picks (e.g. Stella, Arctic) remain unverified-by-design — no digest is known ahead of time, so they install down the logged "integrity unverified" path trusting the revision pin + TLS.
- A **Search Index Projection** should produce one mixed result stream over typed **Search Result Anchor** values.
- **Search Result Group** creation should happen before result pagination is presented to the user.
- **Search Refinement** should apply to **Search Result Anchor** values before choosing the representative anchor for a **Search Result Group**.
- **Search Refinement** values should compose when their result-type constraints are compatible.
- A **Search Result Group** should be included in a **Date Range Search Refinement** when at least one grouped **Search Result Anchor** overlaps the selected time range.
- A **Search Result Group** shown under a **Date Range Search Refinement** should open a representative **Search Result Anchor** inside the selected time range.
- Search results should rank by relevance with a recency bias by default.
- **Search Refinement** values should be applied by search query semantics before pagination rather than by frontend-only result filtering.
- Search responses should expose the normalized **Search Refinement** values that were applied.
- **Search Query Syntax** is opt-in: a query containing no operators matches today's plain-text behavior exactly.
- A **Field Operator** desugars into a visible, removable **Search Refinement** and never remains hidden body-text scope, so typed scope stays as explicit and reversible as a UI-added refinement.
- A **Field Operator** targets only an existing refinement: `app:` to **App Search Refinement**, `after:`/`before:` to **Date Range Search Refinement**, and `source:` to either an **Audio Source Search Refinement** (`source:mic`, `source:system`) or a **Screen Source Search Refinement** (`source:screen`).
- A typed `app:` value matches either a captured app name or bundle identifier (the existing `Any` match kind), case-insensitive, and a multi-word value is quoted as `app:"Google Chrome"`.
- `after:` and `before:` each take a point value (a calendar date such as `2026-01-01`, or a relative point such as `today`, `yesterday`, `7d`, or `1h`), write one bound of the single **Date Range Search Refinement**, and compose into a range.
- `date:` takes a whole day or named period (`today`, `yesterday`, `last-week`, `this-week`, `last-month`, `this-month`, or a single calendar date) and writes both bounds of the **Date Range Search Refinement**.
- All date operators resolve to frozen concrete timestamps at parse time; named-period week boundaries follow the macOS locale first weekday, defaulting to Monday.
- Date operators write one start/end slot: repeated bounds and `date:` overwrite per slot with the last write winning, which is distinct from strict validation and applies only to well-formed operators.
- **App Search Refinement** and **Audio Source Search Refinement** are multi-valued: repeated `app:` or `source:` operators accumulate as a set with OR semantics, so `app:Safari app:Chrome` matches either app and `source:mic source:system` matches both sources; each value is an independently removable chip and duplicates collapse.
- A typed **Field Operator** merges into the existing **Search Refinement** chips by each field's multiplicity rule: `app:` and `source:` add to their set while a date operator overwrites the single **Date Range Search Refinement** slot.
- Conflicts among `app:`/`source:` are about frame-side versus audio-side scope: an **Audio Source Search Refinement** cannot be combined with an **App Search Refinement** (`app_source_conflict`) or a **Screen Source Search Refinement** (`screen_audio_source_conflict`), surfaced as strict conflict errors. A **Screen Source Search Refinement** and an **App Search Refinement** combine freely since both narrow the frame side.
- `app:` value suggestions for a **Search Operator Suggestion** come from distinct retained captured apps (bundle identifier and name), ordered by recency then frequency and globally scoped in V1 rather than narrowed by other active refinements, so every suggestion can produce results.
- Result type can be selected by the existing UI tabs or implicitly by a `source:` value: an **Audio Source Search Refinement** narrows to audio and a **Screen Source Search Refinement** narrows to frames. There is no dedicated result-type **Field Operator**; result-type selection rides on `source:` rather than a separate `type:`/`in:` operator.
- A **Body Match Operator** is translated into a safe FTS5 match expression rather than passing raw user input to FTS5 MATCH.
- Only the known keys `app:`, `after:`, `before:`, and `source:` form a **Field Operator**; any other `key:value` such as a URL or `error:404` stays literal body text so URL, code, and error searches keep working.
- A quoted phrase forces literal matching, so captured text containing operator-like characters can still be searched verbatim.
- **Search Query Syntax** validation is strict for clear operator mistakes: a malformed known **Field Operator** value such as an unparseable date, or a malformed **Body Match Operator** such as an unbalanced quote, surfaces a parse error rather than silently running a misleading or empty search.
- **Search Query Syntax** parsing is backend-canonical: app-infra owns operator recognition, validation, **Field Operator** to **Search Refinement** extraction, and **Body Match Operator** to FTS5 translation, and the search response returns the residual body query, the extracted refinements, and any parse errors with spans.
- Strict validation problems, including the `app:`/`source:` conflict and an unparseable date, are returned in-band with the search response as parse errors rather than failing the search call; a failed search call is reserved for system errors such as database failures.
- **App Search Refinement** should use retained bundle identifier as canonical identity when available and app name only as a fallback.
- **App Search Refinement** should apply to **Captured Frame** results only until audio results have an explicit **Search Context Alignment** policy.
- Combining **App Search Refinement** with **Date Range Search Refinement** should produce frame results inside the selected time range.
- A **Search Index Projection** is user data because it duplicates searchable recognized text.
- A **Search Snippet** should be generated when the user searches rather than stored ahead of time.
- A **Managed Storage Layout** is derived from one `saveDirectory` value.
- A **Managed Storage Layout** contains the recordings tree under `<saveDirectory>/recordings`.
- **Captured Frame Equivalence** determines whether a new **Captured Frame** needs a new **OCR Job**.
- **Captured Frame Equivalence Scope** determines which earlier **Captured Frame** values are eligible comparison candidates.
- A **Captured Frame Pipeline** skips a new **OCR Job** when an earlier equivalent **Captured Frame** in the same session satisfies **OCR Fallback Eligibility**; an equivalent frame that has no **OCR Job** does not suppress admission, so the **OCR Admission Budget** decision stands and the frame may be admitted.
- When an **OCR Job** completes with recognized text, that text is projected to every equivalent **Captured Frame** in scope that lacks its own text, earlier and later, so one admitted representative makes the whole equivalent group searchable.
- An **OCR Throughput Budget** limits **OCR Job** admission and scheduling rather than guaranteeing a hard process-wide CPU ceiling.
- An **OCR Throughput Budget** must preserve the **OCR Settings Selection** instead of silently switching provider, model, recognition mode, or provider-specific options.
- An **OCR Throughput Budget** has an **OCR Admission Budget** and an **OCR Execution Budget**.
- **OCR Admission Budget** memory is scoped to active capture session/workspace state and should be cleared when that recording session stops.
- **OCR Admission Reason** values may remain app-infra pipeline decision types, but live **OCR Budget Telemetry** DTOs belong to the desktop runtime.
- An **OCR Admission Budget** limits new automatic **OCR Job** creation.
- **OCR Admission Budget** comparisons should remain scoped to the relevant capture session or workspace where appropriate.
- **Captured Frame Equivalence** is the first automatic OCR duplicate filter; an **OCR Admission Budget** may add a conservative second filter for non-equivalent but low OCR-value frames.
- An **OCR Admission Budget** should avoid creating **OCR Job** debt for automatic low OCR-value **OCR Candidate** values.
- An **OCR Admission Budget** should prefer materially changed **OCR Candidate** values over fixed time cadence.
- A fixed time cadence may dampen stable or low-change capture periods but must not hide materially new searchable text.
- **Visual Novelty Admission** may admit a non-equivalent **OCR Candidate** whose **Captured Frame Equivalence** fingerprint is new in scope, reusing that fingerprint rather than adding an **OCR-Relevance Probe**.
- **Visual Novelty Admission** must stay bounded: it does not fire under high **OCR Job** queue pressure, it is rate-capped to at most one novelty **OCR Job** per scope per short interval, and it suppresses back to fixed time cadence after a sustained run of continuously-novel frames so video/animation is not read frame-by-frame.
- **Visual Novelty Admission** never overrides **Captured Frame Equivalence** or **OCR Fallback Eligibility**: equivalence reuse is evaluated first, so a novelty-admitted frame that has an eligible earlier equivalent still reuses that text instead of creating a new **OCR Job**.
- **OCR Admission Budget** may use queued plus running **OCR Job** count as a pressure signal during active recording, admitting only stronger automatic **OCR Candidate** values when backlog is high.
- **OCR-Relevant Change** is narrower than any visible pixel change and excludes cursor movement, small animations, spinners, video playback, and tiny layout shifts.
- Foreground context changes such as app bundle, window title, browser URL, or display changes are strong positive **OCR-Relevant Change** signals when the frame is not equivalent to an earlier frame.
- Unchanged foreground context does not prove there is no **OCR-Relevant Change**.
- An **OCR Throughput Budget** may use cheap admission signals that reuse already available capture or persistence data.
- **OCR Admission Budget** memory should not persist budget decisions, but it may read existing **Captured Frame** metadata to detect **OCR-Relevant Change**.
- An **OCR Throughput Budget** should not add an **OCR-Relevance Probe**, because extra pre-admission analysis can become another CPU cost on the capture hot path.
- **OCR Admission Reason** values explain current runtime decisions and should not be persisted as durable per-frame data.
- **OCR Admission Reason** values should not be invented historically for older frames.
- **OCR Budget Telemetry** should include timing and usefulness summaries such as recognized text length or observation count, but should not duplicate recognized text.
- **OCR Budget Telemetry** usefulness summaries should be computed after **OCR Job** completion, not estimated before admission.
- **OCR Budget Telemetry** should be live-only by default and should not be stored in the main app database.
- **OCR Budget Telemetry** debug events should not include full frame or workspace paths.
- Equivalent-frame reuse should produce a live **OCR Admission Budget** debug event with the related **Captured Frame** id even though it does not create a new **OCR Job**.
- An **OCR Execution Budget** paces existing queued **OCR Job** execution.
- An **OCR Execution Budget** should start with deterministic pacing based on recording state and observed OCR job timing rather than live process-wide CPU measurement.
- An **OCR Execution Budget** should use bounded cost-adaptive pacing based on recent **OCR Job** runtime rather than a single fixed cooldown.
- **OCR Execution Budget** pacing should use observed **OCR Job** runtime regardless of whether the job completed or failed.
- **OCR Execution Budget** pacing should be global across sessions because OCR work consumes shared machine resources.
- An **OCR Execution Budget** should be more conservative during active recording than during **OCR Catch-Up**.
- Automatic **OCR Job** work still occurs during active recording; active recording is neither OCR-off nor governed by a fixed time cadence for materially changed frames.
- **OCR Catch-Up** may process deferred **OCR Job** backlog more aggressively after recording stops or while the machine is idle.
- **OCR Catch-Up** processes already admitted **OCR Job** backlog and should not automatically create new **OCR Job** values for low OCR-value candidates skipped by admission.
- **OCR Throughput Budget** semantics apply across OCR providers and should use provider/job timing telemetry for future tuning rather than changing the **OCR Settings Selection**.
- Existing queued **OCR Job** values should be preserved and processed under the **OCR Execution Budget** rather than deleted or reclassified by the **OCR Admission Budget**.
- An **OCR Throughput Budget** should start as a default product policy rather than a visible user-configurable resource setting.
- A **Frame Batch** can be finalized only after its **OCR Job** entries are terminal.
- **OCR Job** values that are not admitted by the **OCR Admission Budget** do not block **Frame Batch** finalization.
- An admitted **OCR Job** still blocks its **Frame Batch** from finalizing until the job is terminal.
- **Captured Frame Reprocessing** operates on an existing **Captured Frame**, not on a new **Screen Frame Artifact**.
- **Captured Frame Reprocessing** bypasses the **OCR Admission Budget** but still respects the **OCR Execution Budget**.
- A **Hidden Segment Workspace** may be preserved when an incomplete **Frame Batch** or nonterminal **OCR Job** still references it.
- **Hidden Segment Workspace Repair** removes only **Hidden Segment Workspace** values that are safe to remove.
- New **Encrypted Capture Index** databases are encrypted by default and should not expose a user-facing plaintext mode.
- **User Context Store** tables (migrations `0022`–`0025`) live inside the **Encrypted Capture Index** alongside OCR/transcript text, store timestamps as INTEGER unix-millis, and deliberately carry no foreign key to frame or audio_segment rows so the derived dossier survives **Retention Policy** aging as a durable evidence floor.
- **Retention Cleanup** does NOT cascade into the **User Context Store**: a structural test (`retention_cleanup_source_never_touches_user_context_tables`) asserts that the `capture_retention.rs` delete path never references any `user_context_*` table, so derived Activities/Conclusions outlive the raw captures they came from.
- **Delete Recent Capture** DOES cascade into the **User Context Store** through `delete_derived_for_capture_subjects`: it purges Activities derived from the deleted frame/audio window, drops Conclusions that lose all their evidence (no ungrounded Conclusions), keeps still-grounded ones, and leaves dismissal state alone.
- **Wipe User Context** (`UserContextStore::wipe_all`) clears every `user_context_*` table — all derived data plus dismissal state — without touching raw captures or other settings; the desktop layer also turns the engine off as part of the wipe.
- The fixed Confidence Policy (`confidence.rs`) and the Sensitive Category Guardrail (`guardrail.rs`) are pure, unit-tested deterministic logic in app-infra; the Guardrail's hard `is_sensitive` post-filter runs at derivation time so a sensitive Conclusion never enters the **User Context Store**, and app-infra keeps no `ai-runtime`/`rig-core` dependency because the model call lives in the desktop Tauri layer.
- **Broker Client Identity** normalization is shared through app-infra so CLI execution, desktop grant creation, Access Settings, and revocation use the same matching rules.
- app-infra enforces **Broker Client Identity** grant matching for **Brokered Capture Access** rather than relying on Mnema CLI filtering.
- **Brokered Capture Access** grant state is app access-control state outside the capture index; V1 non-secret grant metadata belongs in app config, while future grant secrets or tokens belong in platform secret storage if introduced.
- **Brokered Capture Access** audit history stores non-content events such as client identity, authorization outcomes, grant revocation, command type, timestamp, and result count, not raw query text, returned snippets, OCR text, transcripts, browser URLs, or media paths.
- **Brokered Capture Access** audit history stays in app config storage for V1 and must remain non-content because it is not part of the encrypted capture index.
- **Brokered Capture Access** grant and audit files should be schema-versioned and preserve compatibility with existing unversioned grant and audit files.
- Legacy `Local agent` grants should migrate to the default `mnema CLI` **Broker Client Identity** while preserving grant ID, scope, expiry, and revocation state.
- Unexpired and unrevoked legacy **Brokered Capture Access** grants remain active after schema migration.
- Legacy unversioned **Brokered Capture Access** grant files should migrate lazily on load to schema-versioned grant storage, preserving known fields and unknown fields where practical.
- Legacy unversioned **Brokered Capture Access** audit records should migrate lazily to schema-versioned non-content access history, preserving timestamp, command type, result count, scope class, and normalized client identity.
- **Text Search** is the first search tier for searchable **Captured Frame** and **Audio Transcription Span** content.
- **Text Search** entries should be updated transactionally with completed OCR or transcription results.
- **Text Search** should include completed OCR and transcription results only.
- Empty completed OCR or transcription results should not create **Search Result Anchor** values.
- Search should not create OCR or transcription work by itself.
- Reprocessing a **Captured Frame** or **Audio Segment** should replace its existing **Search Index Projection** when the new result completes.
- A **Search Index Projection** may carry type-specific anchor data for **Captured Frame** and **Audio Transcription Span** results.
- A **Search Index Projection** may be rebuilt from retained OCR and transcription results.
- **Retention Cleanup** must remove **Search Index Projection** rows for deleted **Captured Frame** and **Audio Segment** sources in the same cleanup transaction.
- **Processing Job Reclamation** requeues an **Orphaned Processing Job** rather than failing it, so abandoned capture work (audio transcription, speaker analysis, system-audio speech activity, and OCR) still completes after a quit or crash.
- **Processing Job Reclamation** distinguishes abandonment from failure: an abandoned job re-runs without spending a failure attempt and is bounded only by a generous safety ceiling, while a genuinely failed job is bounded by a small retry cap with backoff.
- Bounded automatic retry with backoff applies to audio processors as well as OCR; it is no longer OCR-only.
- Graceful shutdown requeues in-flight jobs before aborting workers, so a normal quit does not strand work as an **Orphaned Processing Job**; the short shutdown timeout is kept rather than blocking quit on a multi-minute job.
- A reclaimed transcription job that completes re-chains its speaker analysis, so recovering a transcription also recovers downstream speaker work.
- **Processing Job Reclamation** runs at startup and at shutdown only; there is no live-session reclamation watchdog, because a single sequential worker cannot orphan its own lane mid-session.

## Example Dialogue

> **Dev:** "When a **Captured Frame** is equivalent to an earlier frame in the same session, does the **Captured Frame Pipeline** still create an **OCR Job**?"
> **Domain expert:** "No — the frame is persisted and attached to its **Frame Batch**, but if an earlier equivalent frame is already eligible, the pipeline reuses that OCR fallback instead of creating another **OCR Job**."

> **Dev:** "Does an **OCR Admission Budget** replace **Captured Frame Equivalence**?"
> **Domain expert:** "No — **Captured Frame Equivalence** remains the duplicate filter, while the budget may conservatively filter non-equivalent low OCR-value frames."

> **Dev:** "If the nearest earlier equivalent **Captured Frame** was itself skipped and has no **OCR Job**, should the pipeline still reuse it and skip the new frame?"
> **Domain expert:** "No — a skipped frame with no **OCR Job** fails **OCR Fallback Eligibility**, so it must not suppress admission. Honor the admission decision so the dwelled-on screen finally gets read; the completed text then projects back across the whole equivalent group."

> **Dev:** "Should a low OCR-value automatic **OCR Candidate** still create a deferred **OCR Job**?"
> **Domain expert:** "No — avoid creating OCR debt for that candidate, keep the **Captured Frame**, and allow manual **Captured Frame Reprocessing** later."

> **Dev:** "Should the timeline label every frame skipped by the **OCR Admission Budget**?"
> **Domain expert:** "No — keep normal timeline quiet, but expose admission-skip reasons in debug surfaces."

> **Dev:** "Can an **OCR Admission Reason** live only in logs?"
> **Domain expert:** "No — it should be persisted so the frame remains explainable after restart."

> **Dev:** "Should **OCR Budget Telemetry** store another copy of recognized text?"
> **Domain expert:** "No — store cost and usefulness summaries; recognized text belongs in the normal OCR result."

> **Dev:** "Should equivalent-frame reuse have an **OCR Admission Reason** even though **Captured Frame Equivalence** already works?"
> **Domain expert:** "Yes — equivalence remains the duplicate filter, and the reason explains why no separate **OCR Job** exists."

> **Dev:** "Should Mnema backfill **OCR Admission Reason** for old **Captured Frame** values?"
> **Domain expert:** "No — persist reasons for newly captured frames after the policy exists instead of inventing historical explanations."

> **Dev:** "Should an **OCR Throughput Budget** force Mnema to stay below 30% CPU at every moment?"
> **Domain expert:** "No — it should limit how much **OCR Job** work is admitted and scheduled over time, while short engine-level CPU spikes may still happen."

> **Dev:** "Can the **OCR Throughput Budget** switch automatic Apple Vision OCR from accurate to fast when the machine is busy?"
> **Domain expert:** "No — the budget should run fewer or later **OCR Job** values instead of changing the **OCR Settings Selection**."

> **Dev:** "Should the **OCR Throughput Budget** only slow the worker after every frame already has a queued **OCR Job**?"
> **Domain expert:** "No — the **OCR Admission Budget** should control how much OCR debt is created, and the **OCR Execution Budget** should control how quickly that debt is processed."

> **Dev:** "Should the **OCR Execution Budget** constantly read live CPU usage to decide whether OCR can run?"
> **Domain expert:** "No — start with deterministic pacing from recording state and observed OCR job timing, then add CPU feedback later only if needed."

> **Dev:** "Should **OCR Execution Budget** pacing use one fixed delay after every **OCR Job**?"
> **Domain expert:** "No — use bounded cost-adaptive pacing so expensive recent OCR work slows the next job more than cheap recent OCR work."

> **Dev:** "Should users configure the **OCR Throughput Budget** directly?"
> **Domain expert:** "Not yet — users configure the **OCR Settings Selection**, while the throughput budget starts as a default product policy."

> **Dev:** "Can the **OCR Admission Budget** admit only one **OCR Candidate** every few seconds?"
> **Domain expert:** "No — materially changed **OCR Candidate** values should bypass cadence, while cadence only limits stable or low-change periods."

> **Dev:** "Does every visible pixel difference create an **OCR-Relevant Change**?"
> **Domain expert:** "No — OCR admission should ignore visual noise that is unlikely to change useful recognized text."

> **Dev:** "If the active app, title, browser URL, or display changes, should that count toward **OCR-Relevant Change**?"
> **Domain expert:** "Yes — foreground context changes are strong positive signals, but unchanged context does not prove the text is unchanged."

> **Dev:** "Can the **OCR Throughput Budget** use cheap visual or context signals before admitting automatic OCR?"
> **Domain expert:** "Yes — if the signal reuses already available capture or persistence data; do not add an **OCR-Relevance Probe** that costs meaningful extra CPU."

> **Dev:** "Can the **OCR Admission Budget** reject **Captured Frame Reprocessing**?"
> **Domain expert:** "No — reprocessing is explicit user intent, but the resulting **OCR Job** still runs under the **OCR Execution Budget**."

> **Dev:** "Should deferred **OCR Job** backlog run at the same pace during recording and after recording stops?"
> **Domain expert:** "No — active recording should use a conservative **OCR Execution Budget**, while **OCR Catch-Up** can process backlog more aggressively."

> **Dev:** "Can **OCR Catch-Up** later create **OCR Job** values for low OCR-value frames skipped during admission?"
> **Domain expert:** "No — catch-up drains already admitted OCR backlog; manual **Captured Frame Reprocessing** is the escape hatch for skipped frames."

> **Dev:** "Should the **OCR Throughput Budget** pick different providers when one provider is slower?"
> **Domain expert:** "No — provider choice belongs to **OCR Settings Selection**; the budget can use provider/job timing telemetry for future tuning."

> **Dev:** "Does the conservative active-recording budget mean automatic OCR is off while recording?"
> **Domain expert:** "No — automatic **OCR Job** work still happens for **OCR-Relevant Change**, but stable or low-change periods can be paced or deferred."

> **Dev:** "Should users have to choose between exact text matching and meaning-based search?"
> **Domain expert:** "No — start with **Text Search**, then move toward **Hybrid Search** so literal and semantic matches work together."

> **Dev:** "A transcription job is stuck `running` while other transcription jobs complete — is it hung?"
> **Domain expert:** "No — a single worker can't run two at once, so it's an **Orphaned Processing Job**: its worker was aborted at quit. **Processing Job Reclamation** should requeue it so it re-runs, not fail it."

> **Dev:** "If the user quits while reprocessing a segment a few times, should that segment eventually be marked permanently failed?"
> **Domain expert:** "No — that's abandonment, not failure. It re-runs without spending a failure attempt. Only genuine engine failures count toward the small retry cap."

## Flagged Ambiguities

- "pipeline" previously meant both frame intake and OCR execution; resolved: **Captured Frame Pipeline** means frame intake through batch-finalization readiness, while **OCR Job** means the recognition work for one frame.
- "CPU cap" suggested a hard process-wide ceiling; resolved: use **OCR Throughput Budget** for limiting OCR work over time.
- "fast OCR" and provider fallback were considered as optimizations; resolved: the **OCR Throughput Budget** must not change the **OCR Settings Selection**.
- "eligible as the OCR fallback" was ambiguous between any equivalent **Captured Frame** and one that actually has an **OCR Job**; resolved as **OCR Fallback Eligibility**: only a frame with an **OCR Job** is eligible, so an admission-skipped textless frame no longer cancels a later frame's admission.
- "30% CPU" suggested live CPU feedback; resolved: the first **OCR Execution Budget** should use deterministic pacing rather than live process-wide CPU measurement.
- "search support" was used to mean both literal matching and embedding-based retrieval; resolved: **Text Search** is the first tier, while **Hybrid Search** is the direction once **Semantic Search** exists.
- "query syntax" was listed only as something a **Search Refinement** avoids; resolved: typed **Search Query Syntax** is supported as an opt-in input path whose **Field Operator** tokens desugar into **Search Refinement** values, while a **Search Refinement** itself remains a UI control rather than a syntax mode.
- "running forever" meant a job stuck in `running` that never completes; resolved: this is an **Orphaned Processing Job** (execution abandoned at quit/crash), addressed by **Processing Job Reclamation** (requeue, not fail) per [ADR 0020](../../docs/adr/0020-reclaim-orphaned-processing-jobs-by-requeue.md), not by a job-execution timeout or an audio throughput budget.
- "Semantic Search embeddings from redacted text" was the early privacy stance; resolved per [ADR 0036](../../docs/adr/0036-semantic-search-v1-hybrid-fastembed-vectors-with-fts5.md): a **Semantic Search Vector** is derived from the same raw body text **Text Search** indexes and protected at rest by the **Encrypted Capture Index**, while redaction is enforced at any boundary that takes a vector or its text out of that index — embeddings are not separately redacted at rest.
- "Semantic Search is local-only" overstated the posture; resolved per [ADR 0037](../../docs/adr/0037-semantic-search-embeddings-on-candle-with-pluggable-backend.md): **Semantic Search** is **local-first** — the default **Semantic Search Backend** (candle, on-device) sends nothing off-device, while a remote/cloud backend is opt-in only and embeds egress-redacted text. The raw-text-at-rest rule above still governs local backends.
- "embeddings are produced in-process by fastembed/ONNX" was the v1 runtime; resolved per [ADR 0037](../../docs/adr/0037-semantic-search-embeddings-on-candle-with-pluggable-backend.md): the runtime is **candle** (Apple GPU via Metal, or CPU) behind a pluggable **Semantic Search Backend**, and the model catalog is hand-maintained (candle has none) guarded by a `config.json` cross-check. `fastembed`/`ort` leave **Semantic Search** (`ort` stays only for transcription).
- "one vector per anchor" was used for two different operations; resolved: **one (stored) vector per anchor** is the pooling/dedup invariant (one stored vector per anchor; text overflowing the window is split and mean-pooled into that one vector), whereas **first-window embed** is a distinct, deferred quality lever (embed only the first token window instead of pooling all chunks). They are not the same change.
