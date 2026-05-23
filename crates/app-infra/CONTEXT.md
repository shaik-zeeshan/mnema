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

**Captured Frame Equivalence**:
The rule for when two **Captured Frame** values should be treated as the same OCR-relevant visual content for downstream decisions such as **OCR Job** admission. **Captured Frame Equivalence** is defined over normalized visual content rather than persisted artifact bytes, and intentionally ignores cursor-sized changes plus limited localized visual noise that does not materially change OCR-relevant content.
_Avoid_: dedupe hash, screenshot sameness, OCR skip heuristic

**Captured Frame Equivalence Scope**:
The rule for where an earlier equivalent **Captured Frame** may be searched when applying **Captured Frame Equivalence**. **Captured Frame Equivalence Scope** is session-wide by default, but narrows to the same hidden segment workspace when the candidate **Captured Frame** originated from a hidden segment workspace artifact path.
_Avoid_: workspace filter, lookup scope, same-segment rule

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
A local model asset used to derive meaning vectors for **Semantic Search**.
_Avoid_: cloud embedding service, embedding API, vector model

**Hybrid Search**:
The product search policy that combines **Text Search** and **Semantic Search** so literal and meaning-based matches can rank together.
_Avoid_: search mode toggle, vector-only search, fuzzy search

**Search Index Projection**:
A durable derived view of searchable capture content used to answer search queries.
_Avoid_: search cache, search result table, indexing job output

**Search Context**:
Captured contextual labels that help find or filter a **Search Result Anchor**, such as app name, window title, browser URL, or speaker label.
_Avoid_: metadata blob, result decoration, search tags

**Search Refinement**:
An explicit user control that narrows an active search by retained capture context such as date range, app, source, or result type.
_Avoid_: advanced search, search mode, query syntax

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
- Future **Semantic Search** embeddings should be derived from redacted text and stored under the **Encrypted Capture Index** boundary.
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
- **Semantic Search** is local-only.
- **Semantic Search** indexing may run as separate background work because embedding generation is model work, not simple projection maintenance.
- **Semantic Search** uses a **Semantic Search Model** rather than sending searchable capture content to a cloud embedding service.
- **Hybrid Search** is the product direction once **Semantic Search** exists.
- A **Search Index Projection** should produce one mixed result stream over typed **Search Result Anchor** values.
- **Search Result Group** creation should happen before result pagination is presented to the user.
- **Search Refinement** should apply to **Search Result Anchor** values before choosing the representative anchor for a **Search Result Group**.
- **Search Refinement** values should compose when their result-type constraints are compatible.
- A **Search Result Group** should be included in a **Date Range Search Refinement** when at least one grouped **Search Result Anchor** overlaps the selected time range.
- A **Search Result Group** shown under a **Date Range Search Refinement** should open a representative **Search Result Anchor** inside the selected time range.
- Search results should rank by relevance with a recency bias by default.
- **Search Refinement** values should be applied by search query semantics before pagination rather than by frontend-only result filtering.
- Search responses should expose the normalized **Search Refinement** values that were applied.
- **App Search Refinement** should use retained bundle identifier as canonical identity when available and app name only as a fallback.
- **App Search Refinement** should apply to **Captured Frame** results only until audio results have an explicit **Search Context Alignment** policy.
- Combining **App Search Refinement** with **Date Range Search Refinement** should produce frame results inside the selected time range.
- A **Search Index Projection** is user data because it duplicates searchable recognized text.
- A **Search Snippet** should be generated when the user searches rather than stored ahead of time.
- A **Managed Storage Layout** is derived from one `saveDirectory` value.
- A **Managed Storage Layout** contains the recordings tree under `<saveDirectory>/recordings`.
- **Captured Frame Equivalence** determines whether a new **Captured Frame** needs a new **OCR Job**.
- **Captured Frame Equivalence Scope** determines which earlier **Captured Frame** values are eligible comparison candidates.
- A **Captured Frame Pipeline** skips a new **OCR Job** when an earlier equivalent **Captured Frame** in the same session is already eligible as the OCR fallback.
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

## Example Dialogue

> **Dev:** "When a **Captured Frame** is equivalent to an earlier frame in the same session, does the **Captured Frame Pipeline** still create an **OCR Job**?"
> **Domain expert:** "No — the frame is persisted and attached to its **Frame Batch**, but if an earlier equivalent frame is already eligible, the pipeline reuses that OCR fallback instead of creating another **OCR Job**."

> **Dev:** "Does an **OCR Admission Budget** replace **Captured Frame Equivalence**?"
> **Domain expert:** "No — **Captured Frame Equivalence** remains the duplicate filter, while the budget may conservatively filter non-equivalent low OCR-value frames."

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

## Flagged Ambiguities

- "pipeline" previously meant both frame intake and OCR execution; resolved: **Captured Frame Pipeline** means frame intake through batch-finalization readiness, while **OCR Job** means the recognition work for one frame.
- "CPU cap" suggested a hard process-wide ceiling; resolved: use **OCR Throughput Budget** for limiting OCR work over time.
- "fast OCR" and provider fallback were considered as optimizations; resolved: the **OCR Throughput Budget** must not change the **OCR Settings Selection**.
- "30% CPU" suggested live CPU feedback; resolved: the first **OCR Execution Budget** should use deterministic pacing rather than live process-wide CPU measurement.
- "search support" was used to mean both literal matching and embedding-based retrieval; resolved: **Text Search** is the first tier, while **Hybrid Search** is the direction once **Semantic Search** exists.
