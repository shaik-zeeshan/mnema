# Frontend Integration Context

This context captures the domain language for the desktop capture app so architecture discussions use stable project terms.

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

**Captured Frame Reprocessing**:
A request to re-run OCR for an existing **Captured Frame** that is already persisted.
_Avoid_: force processing, rerun pipeline, requeue screenshot

**Scrub Preview**:
A disposable, display-sized preview image for a screen segment time position, used while navigating the dashboard timeline.
_Avoid_: exact frame, OCR source, screenshot, thumbnail

**Scrub Preview Generation**:
Background work that materializes generated **Scrub Preview** cache artifacts for finalized screen segment intervals.
_Avoid_: scrub-time extraction, exact frame preview generation, thumbnail pipeline

**Recording Lifecycle**:
The in-memory control flow for one coordinated recording runtime that starts capture, owns pause/resume decisions, rotates segments, recovers after wake, and stops capture across the requested sources. Screen and system audio share the screen capture backend, while microphone runs as a separate native session.
_Avoid_: capture runtime, recorder service, session manager

**Audio Segment**:
A time-bounded persisted audio recording file produced from one recording source during a recording session.
_Avoid_: audio file, raw microphone file, sound clip

**Audio Transcription**:
Recognized speech text, with optional timing relative to its **Audio Segment**, language, confidence, provider, and model metadata, derived from one **Audio Segment**.
_Avoid_: transcript blob, transcription result, speech text

**Audio Transcription Job**:
Background work that recognizes speech for one **Audio Segment**.
_Avoid_: transcription task, transcript worker item, speech job

**Speaker Analysis Job**:
Local diarization and optional recognition work for one microphone **Audio Segment**.
_Avoid_: speaker task, diarization worker item, speaker recognition task

**Speaker Turn Alignment**:
The policy that assigns **Audio Transcription** words or segments to speaker turns. Transcription timing is primary; speaker turns annotate that text and do not stretch, split, or duplicate transcript text.
_Avoid_: transcript rewriting, diarization-owned text timing, speaker text retiming

**Speaker Continuity**:
The session-level policy that keeps a real speaker associated with a stable speaker cluster across **Audio Segment** values with the same `session_id`, provider, and model.
_Avoid_: segment-local speaker identity, provider cluster identity, cross-session speaker identity

**Audio Transcription Provider**:
A local speech recognition option used by an **Audio Transcription Job** to produce an **Audio Transcription**.
_Avoid_: cloud transcription service, transcription engine, ASR backend

**Audio Transcription Model**:
A local model asset selected for an **Audio Transcription Provider** when that provider requires app-managed model files.
_Avoid_: model file, downloaded artifact, checkpoint

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

**Audio Activity Sample**:
A raw audio probe reading such as latest normalized level or last-sample timestamp, exposed for debug visibility but not itself used as the inactivity decision.
_Avoid_: audio activity event, microphone activity, system audio activity

**Audio Activity Decision**:
The threshold-qualified inactivity-policy view of audio activity, including enabled state, threshold, and derived idle used for pause/resume decisions.
_Avoid_: raw audio sample, activity reading, latest level

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

## Relationships

- A **Screen Frame Artifact** becomes a **Captured Frame** only after app-infra persists it.
- A **Captured Frame Pipeline** persists one **Captured Frame**.
- A **Captured Frame Pipeline** attaches each **Captured Frame** to exactly one **Frame Batch**.
- A **Captured Frame Pipeline** may enqueue one **OCR Job** for a **Captured Frame**.
- A **Scrub Preview** represents a screen segment time position, not **Captured Frame** identity.
- Multiple nearby **Captured Frame** values may share one **Scrub Preview** when they fall within the same preview interval.
- A generated **Scrub Preview** interval is one second and is represented by the first indexed screen position inside that one-second video-offset bucket.
- The v1 generated **Scrub Preview** rendition is JPEG quality 72 with a 360 px maximum dimension at one preview per second.
- A generated **Scrub Preview** is an app-owned cache artifact under the app cache directory, not a durable artifact under the **Managed Storage Layout** recordings tree.
- A generated **Scrub Preview** cache identity is tied to the source screen segment, preview interval, rendition settings, and source video/frame-index freshness.
- Generated **Scrub Preview** source freshness uses canonical source path identity plus source video/frame-index size and modified time, not full media content hashing.
- Generated **Scrub Preview** files live under a dedicated app-cache scrub preview root, grouped by rendition and source segment cache directory.
- The dedicated generated **Scrub Preview** cache root is app-owned and may be allowed recursively through Tauri asset scope.
- A generated **Scrub Preview** segment cache directory requires valid metadata matching source freshness before its preview files can be returned.
- Generated **Scrub Preview** cache access is tracked at segment-directory granularity with throttled last-access updates for pruning.
- **Scrub Preview** availability returns only source-fresh cache files; missing or stale indexed intervals may be enqueued for background regeneration.
- A generated **Scrub Preview** cache interval is keyed by source segment video offset, while dashboard availability is requested and displayed by timeline time.
- Dashboard timeline mapping for generated **Scrub Preview** intervals uses **Capture Segment** timing plus segment video offset rather than per-frame captured timestamp jitter.
- Generated **Scrub Preview** coverage is derived from indexed screen positions, not raw segment recording duration.
- A timeline interval with a usable frame index but no indexed screen position is unavailable for **Scrub Preview** without treating the whole frame index as missing.
- The generated **Scrub Preview** cache defaults to a 512 MB budget and 7-day last-access window, pruned by segment cache directory rather than individual preview file.
- Generated **Scrub Preview** cache policy is separate from exact frame preview cache policy.
- Existing exact preview cache TTL settings do not control generated **Scrub Preview** disk cache lifetime.
- V1 generated **Scrub Preview** interval, rendition, and cache budget are fixed product policy rather than user-facing recording settings.
- V1 may expose a developer/debug action to clear only generated **Scrub Preview** cache without clearing exact preview cache or adding regular user-facing cache controls.
- V1 developer/debug surfaces may expose generated **Scrub Preview** cache status and queue status for verification.
- **Scrub Preview Generation** runs outside the active scrub interaction path; timeline navigation may request availability, but missing generated **Scrub Preview** values are materialized in background work.
- **Scrub Preview Generation** uses a single coalescing worker where the newest visible timeline window takes priority over stale queued preview intervals.
- **Scrub Preview Generation** queue state is non-durable and rebuilds from finalized-segment events or dashboard availability demand.
- **Scrub Preview Generation** stays outside app-infra processing job lanes and frame/OCR persistence transactions.
- Startup validates/prunes generated **Scrub Preview** cache but does not warm missing previews for existing segments.
- **Scrub Preview Generation** processes interval work in bounded chunks so visible-window demand can preempt full-segment warming.
- A finalized screen **Capture Segment** enqueues full one-second-interval **Scrub Preview Generation**, bounded by the 5-minute **Capture Segment Duration** cap.
- Historical **Capture Segment** values encountered through dashboard demand enqueue only visible-window intervals, not full-segment warming.
- Automatic **Scrub Preview Generation** is triggered after the screen **Capture Segment** is committed, outside capture primitive code.
- App-infra owns **Capture Segment** discovery for **Scrub Preview** availability, while the desktop Tauri layer owns generated **Scrub Preview** cache files, native extraction, asset scope, queueing, and cache-change events.
- Completed **Scrub Preview Generation** chunks notify the dashboard with coalesced cache-change events so visible windows can refresh availability without polling continuously; those events invalidate ranges rather than carrying preview file paths.
- **Scrub Preview** generation failures are non-durable availability states with short-lived retry backoff, not persisted app-infra records.
- Dashboard navigation requests **Scrub Preview** availability by timeline window rather than **Captured Frame** identity.
- Dashboard **Scrub Preview** availability requests may enqueue missing visible-window intervals for background generation but must not synchronously extract preview images.
- Dashboard **Scrub Preview** availability responses include ready and unavailable/queued interval statuses, while display uses only ready intervals.
- **Scrub Preview** availability may report queued status from non-durable in-memory generation queue or in-flight work.
- Dashboard initial load requests **Scrub Preview** availability for the initial visible timeline window once timeline data and viewport dimensions are available.
- Dashboard **Scrub Preview** availability requests cover the visible timeline window plus small overscan, not the entire loaded timeline history.
- Dashboard scroll debounce applies only to backend **Scrub Preview** availability/enqueue requests; already-known ready **Scrub Preview** cache entries should display immediately during timeline movement.
- Dashboard timeline movement does not start exact preview requests as a fallback for missing **Scrub Preview** values; exact preview requests are settle/inspection behavior.
- Dashboard **Scrub Preview** state is interval-based, with active **Captured Frame** display derived from the matching interval instead of a frame-id preview cache.
- **Scrub Preview** availability is derived from screen **Capture Segment** rows and their frame indexes; disposable preview cache entries are not modeled as durable app-infra rows.
- A **Scrub Preview** may be lower resolution or timing-tolerant and is never the source for OCR, copy, download, or **Captured Frame** truth.
- A **Scrub Preview** can stand in only while timeline navigation is in motion; a parked active **Captured Frame** resolves through the exact preview path.
- A **Scrub Preview** may remain visible as a placeholder while the exact preview for the parked active **Captured Frame** is loading.
- A **Scrub Preview** must not populate exact preview cache state or enable exact **Captured Frame** actions.
- When a requested **Scrub Preview** is absent during timeline movement, the dashboard may keep showing the previous available preview rather than blocking movement or showing a loading state.
- Dashboard previous-preview placeholders should be cleared when timeline movement jumps far enough that the displayed preview is no longer near the active interval.
- An existing **Screen Frame Artifact** may satisfy a **Scrub Preview** for its segment time position without generating a separate preview.
- A generated **Scrub Preview** depends on a screen segment frame index; frames without indexed segment timing fall back to the exact preview path instead of guessed scrub output.
- Generated **Scrub Preview** values apply only to finalized screen **Capture Segment** values; live or incomplete segments rely on existing **Screen Frame Artifact** paths or return no **Scrub Preview**.
- **Scrub Preview Generation** eligibility requires a finalized screen **Capture Segment** with an openable screen recording and usable frame index.
- Historical finalized screen **Capture Segment** values are eligible for demand-driven **Scrub Preview Generation** when they have a usable binary or legacy frame index.
- Automatic full-segment **Scrub Preview Generation** runs only after a screen **Capture Segment** finalizes, not while that segment is actively being captured.
- Automatic **Scrub Preview Generation** is opportunistic: it may defer under shutdown, source invalidity, cache pressure, or higher-priority visible-window demand, and it must not block segment finalization.
- **Scrub Preview Generation** prefers an existing matching **Screen Frame Artifact** when available, then falls back to the finalized screen segment recording plus frame index.
- **Hidden Segment Workspace** cleanup does not wait on **Scrub Preview Generation**; existing frame artifacts are used opportunistically but the finalized segment recording remains the regeneration source.
- A **Recording Lifecycle** coordinates screen, microphone, and system-audio capture within one recording runtime.
- A **Recording Lifecycle** may pause or resume requested sources based on inactivity policy.
- A **Recording Lifecycle** commits requested audio sources as **Audio Segment** values.
- A **Recording Lifecycle** creates one **Capture Session** for a user recording and **Capture Segment** rows only for produced artifacts.
- **Capture Segment Duration** applies to each rotated **Capture Segment**, not to total **Capture Session** length.
- **Capture Segment Duration** is capped at 5 minutes in persisted settings, runtime validation, and user-facing settings surfaces.
- An **Audio Segment** comes from exactly one recording source, such as microphone or system audio.
- A **Retention Policy** applies only to the active **Managed Storage Layout** and active app-infra database.
- **Retention Cleanup** skips active capture segments and subjects with running processing/finalize jobs.
- **Retention Cleanup** preserves **Person Profile** values even when derived speaker rows are deleted.
- **Retention Cleanup** best-effort removes generated **Scrub Preview** cache directories for deleted screen **Capture Segment** values, while cache validation and pruning remain responsible for stale orphan safety.
- **Retention Cleanup** reaches generated **Scrub Preview** cache through the desktop Tauri cache service rather than app-infra owning Tauri app-cache paths.
- A dashboard `timeline_data_changed` retention event should prune loaded rows older than the cutoff and preserve the active retained item when possible.
- A microphone **Audio Segment** becomes eligible for an **Audio Transcription Job** when the **Recording Lifecycle** commits it, even if the eventual transcript is empty.
- An **Audio Transcription Job** operates on exactly one **Audio Segment**.
- A **Speaker Analysis Job** operates on exactly one microphone **Audio Segment**.
- A **Speaker Analysis Job** can complete successfully with no speaker turns.
- Too-short, silent, or valid no-speaker audio produces a successful empty speaker-analysis result, not a failed **Speaker Analysis Job**.
- Missing speaker models, audio decode failures, speaker runtime failures, subprocess failures, malformed helper output, and persistence failures are **Speaker Analysis Job** failures.
- Successful **Speaker Analysis Job** diagnostics live in result provenance.
- Failed **Speaker Analysis Job** diagnostics live in `processing_jobs.last_error`.
- **Speaker Analysis Job** execution has a dedicated single-concurrency processing worker so speaker work does not block OCR/frame-batch or audio-transcription lanes.
- The sherpa speaker-analysis helper remains subprocess-per-job in this stage; no persistent helper daemon, in-process model reuse, or generic audio-heavy worker abstraction is part of the current design.
- Each **Speaker Analysis Job** freezes its helper timeout in payload option `helperTimeoutSeconds` when admitted, so later settings changes affect only future jobs.
- The speaker-analysis helper timeout defaults to 600 seconds, clamps to 60-3600 seconds, and timeout failures kill/reap the helper before the job follows the normal failed processing path.
- **Speaker Turn Alignment** treats **Audio Transcription** words or segments as the source timeline and assigns them to the best speaker turn annotation.
- **Speaker Continuity** is limited to **Audio Segment** values in the same recording/session, represented by stable cluster rows rather than provider-local cluster ids.
- Speaker provider cluster ids are provenance from the diarization provider and remain provider-local; they are not rewritten to represent stable identity.
- Speaker merge suggestions are preferred over aggressive automatic merges when continuity matching is ambiguous or only moderately similar.
- VAD-based audio cutting or trimming is outside **Speaker Analysis Job** quality policy; audio segment production remains owned by the recording flow.
- An **Audio Transcription Job** uses exactly one **Audio Transcription Provider**.
- V1 **Audio Transcription Provider** values are local-only: `local_whisper`, `apple_speech_on_device`, and `parakeet`.
- V1 `local_whisper` **Audio Transcription Model** choices are `tiny`, `base`, `small`, and `medium`, with `base` as the default.
- V1 `parakeet` uses `parakeet-tdt-0.6b-v3-onnx` as its **Audio Transcription Model** and runs it through the Rust ONNX Runtime adapter.
- `apple_speech_on_device` uses OS-managed language models rather than an app-managed **Audio Transcription Model**.
- An **Audio Transcription Provider** may require one selected **Audio Transcription Model**.
- A microphone **Audio Segment** gets one **Audio Transcription Job** for the selected **Audio Transcription Provider** when that provider and its required **Audio Transcription Model** are available.
- An **Audio Transcription Job** freezes the selected **Audio Transcription Provider** and **Audio Transcription Model** at admission time.
- Changing the selected **Audio Transcription Provider** or **Audio Transcription Model** affects future **Audio Segment** values, not existing completed **Audio Transcription** values.
- An app-managed **Audio Transcription Model** may be installed, missing, downloading, or failed.
- If the selected **Audio Transcription Provider** or required **Audio Transcription Model** is unavailable, the microphone **Audio Segment** remains eligible but does not get an **Audio Transcription Job** until backfill can enqueue it.
- Missing selected **Audio Transcription Model** status is surfaced when recording starts and in a dedicated Transcription settings surface, not once per committed microphone **Audio Segment**.
- An **Audio Transcription** is derived from exactly one **Audio Segment**.
- An empty no-speech **Audio Transcription** is a successful **Audio Transcription**, not a failed job.
- An **Audio Transcription** is produced by an **Audio Transcription Job**.
- A **Managed Storage Layout** is derived from one `saveDirectory` value.
- A **Managed Storage Layout** contains the recordings tree under `<saveDirectory>/recordings`.
- **Captured Frame Equivalence** determines whether a new **Captured Frame** needs a new **OCR Job**.
- **Captured Frame Equivalence Scope** determines which earlier **Captured Frame** values are eligible comparison candidates.
- A **Captured Frame Pipeline** skips a new **OCR Job** when an earlier equivalent **Captured Frame** in the same session is already eligible as the OCR fallback.
- A **Frame Batch** can be finalized only after its **OCR Job** entries are terminal.
- **Captured Frame Reprocessing** operates on an existing **Captured Frame**, not on a new **Screen Frame Artifact**.
- A **Hidden Segment Workspace** may be preserved when an incomplete **Frame Batch** or nonterminal **OCR Job** still references it.
- **Hidden Segment Workspace Repair** removes only **Hidden Segment Workspace** values that are safe to remove.
- An **Audio Activity Sample** can inform an **Audio Activity Decision**, but the two are not interchangeable.
- An **Audio Activity Decision** is what the inactivity policy uses to pause or resume capture.

## Example dialogue

> **Dev:** "When a **Captured Frame** is equivalent to an earlier frame in the same session, does the **Captured Frame Pipeline** still create an **OCR Job**?"
> **Domain expert:** "No — the frame is persisted and attached to its **Frame Batch**, but if an earlier equivalent frame is already eligible, the pipeline reuses that OCR fallback instead of creating another **OCR Job**."

> **Dev:** "Is `microphoneActivityLastUnixMs` the same thing as the audio signal the inactivity policy uses?"
> **Domain expert:** "No — that timestamp is an **Audio Activity Sample**; the inactivity pause logic uses an **Audio Activity Decision** derived from threshold-qualified activity."

## Flagged ambiguities

- "pipeline" previously meant both frame intake and OCR execution; resolved: **Captured Frame Pipeline** means frame intake through batch-finalization readiness, while **OCR Job** means the recognition work for one frame.
- "**Scrub Preview**" was previously described as a visual representation of a **Captured Frame**; resolved: it is a disposable segment-time preview used during timeline navigation, while exact **Captured Frame** inspection goes through the exact preview path.
- "audio activity" previously referred to both raw probe output and inactivity-policy state; resolved: raw probe output is an **Audio Activity Sample**, while policy-facing threshold-qualified state is an **Audio Activity Decision**.
- "audio file" was used to mean the persisted unit for transcription; resolved: use **Audio Segment** for the time-bounded persisted recording file.
- "provider" was considered for both cloud and local transcription services; resolved: **Audio Transcription Provider** means local-only for v1.
