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
A persisted explanation for why automatic OCR was or was not admitted for a **Captured Frame**.
_Avoid_: debug log, skip note, enqueue reason

**OCR Budget Telemetry**:
Durable cost and usefulness summary data used to tune **OCR Throughput Budget** behavior without duplicating recognized text.
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
- A **Recording Lifecycle** coordinates screen, microphone, and system-audio capture within one recording runtime.
- A **Recording Lifecycle** may pause or resume requested sources based on inactivity policy.
- A **Recording Lifecycle** commits requested audio sources as **Audio Segment** values.
- A **Recording Lifecycle** creates one **Capture Session** for a user recording and **Capture Segment** rows only for produced artifacts.
- An **Audio Segment** comes from exactly one recording source, such as microphone or system audio.
- A **Retention Policy** applies only to the active **Managed Storage Layout** and active app-infra database.
- **Retention Cleanup** skips active capture segments and subjects with running processing/finalize jobs.
- **Retention Cleanup** preserves **Person Profile** values even when derived speaker rows are deleted.
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
- An **OCR Throughput Budget** limits **OCR Job** admission and scheduling rather than guaranteeing a hard process-wide CPU ceiling.
- An **OCR Throughput Budget** must preserve the **OCR Settings Selection** instead of silently switching provider, model, recognition mode, or provider-specific options.
- An **OCR Throughput Budget** has an **OCR Admission Budget** and an **OCR Execution Budget**.
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
- An **OCR Throughput Budget** should not add an **OCR-Relevance Probe**, because extra pre-admission analysis can become another CPU cost on the capture hot path.
- An **OCR Admission Reason** should be persisted for **Captured Frame** values that do not receive an automatic **OCR Job**, including equivalent-frame reuse.
- **OCR Admission Reason** values apply to newly captured frames after the policy exists and should not be invented historically for older frames.
- Low OCR-value admission skips should be visible in debug surfaces but not noisy in the normal timeline.
- **OCR Budget Telemetry** should include timing and usefulness summaries such as recognized text length or observation count, but should not duplicate recognized text.
- **OCR Budget Telemetry** usefulness summaries should be computed after **OCR Job** completion, not estimated before admission.
- An **OCR Execution Budget** paces existing queued **OCR Job** execution.
- An **OCR Execution Budget** should start with deterministic pacing based on recording state and observed OCR job timing rather than live process-wide CPU measurement.
- An **OCR Execution Budget** should use bounded cost-adaptive pacing based on recent **OCR Job** runtime rather than a single fixed cooldown.
- **OCR Execution Budget** pacing should use observed **OCR Job** runtime regardless of whether the job completed or failed.
- **OCR Execution Budget** pacing memory may reset on app startup, while durable job timing remains available for debug and telemetry.
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
- An **Audio Activity Sample** can inform an **Audio Activity Decision**, but the two are not interchangeable.
- An **Audio Activity Decision** is what the inactivity policy uses to pause or resume capture.

## Example dialogue

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

> **Dev:** "Is `microphoneActivityLastUnixMs` the same thing as the audio signal the inactivity policy uses?"
> **Domain expert:** "No — that timestamp is an **Audio Activity Sample**; the inactivity pause logic uses an **Audio Activity Decision** derived from threshold-qualified activity."

## Flagged ambiguities

- "pipeline" previously meant both frame intake and OCR execution; resolved: **Captured Frame Pipeline** means frame intake through batch-finalization readiness, while **OCR Job** means the recognition work for one frame.
- "CPU cap" suggested a hard process-wide ceiling; resolved: use **OCR Throughput Budget** for limiting OCR work over time.
- "fast OCR" and provider fallback were considered as optimizations; resolved: the **OCR Throughput Budget** must not change the **OCR Settings Selection**.
- "30% CPU" suggested live CPU feedback; resolved: the first **OCR Execution Budget** should use deterministic pacing rather than live process-wide CPU measurement.
- "audio activity" previously referred to both raw probe output and inactivity-policy state; resolved: raw probe output is an **Audio Activity Sample**, while policy-facing threshold-qualified state is an **Audio Activity Decision**.
- "audio file" was used to mean the persisted unit for transcription; resolved: use **Audio Segment** for the time-bounded persisted recording file.
- "provider" was considered for both cloud and local transcription services; resolved: **Audio Transcription Provider** means local-only for v1.
