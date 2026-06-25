# Reclaim orphaned processing jobs by requeue, not permanent failure

## Status

Accepted.

## Context

A processing job is marked `running` by the single per-kind worker that claims it. If that worker is aborted while mid-job — at app quit (the background-worker shutdown timeout is 15s, far shorter than a Whisper pass or a speaker-diarization pass) or on a crash/force-kill — the job's execution future is dropped and the row is left `running`. The only cleanup was a blunt startup sweep (`reconcile_orphaned_running_jobs`) that marks **every** `running` row `failed`, and bounded retry existed for OCR only ("audio jobs are intentionally excluded"). So an abandoned audio job became permanently `failed` with no re-run, the segment's transcript and speaker labels were lost, and — because a **Speaker Analysis Job** is chained off transcription *completion* — a failed transcription silently killed its downstream speaker work too. Users saw this as a job "running forever" that, after restart, turned into permanent data loss.

We confirmed the scope before designing: workers spawn once and tear down once (no mid-session re-init), a single sequential worker cannot orphan its own lane mid-session, and the audio reprocess paths are enqueue-only (they refuse to touch a `running` job). So abandonment of an audio job happens only at quit/crash, not during a live recording.

## Decision

Mnema treats an abandoned `running` job as **recoverable work**, not a failure:

- **Reclamation requeues, it does not fail.** Startup reclamation, and graceful shutdown, return an **Orphaned Processing Job** to `queued` so it re-runs and still produces its result. This covers `audio_transcription`, `speaker_analysis`, `system_audio_speech_activity`, and OCR.
- **Graceful shutdown requeues in-flight jobs before aborting** the workers, so a normal quit does not strand work. The 15s shutdown timeout is kept — we requeue and exit fast rather than blocking quit on a multi-minute job.
- **Abandonment and failure are bounded separately.** A genuinely *failed* job (engine error, malformed output, the existing speaker-helper subprocess timeout) is bounded by a small retry cap with backoff, mirroring OCR. An *abandoned* job re-runs **without spending a failure attempt**, bounded only by a generous absolute ceiling as a backstop against a pathological crash-loop. A job is never permanently lost merely because the user closed the app.

## Alternatives Rejected

- **Mid-session reclamation watchdog:** Considered (and initially planned) but cut. Since audio jobs cannot orphan during a live session, a runtime watchdog would only ever catch the OCR/frame-batch inline-reprocess paths, which were not the reported problem. Startup + shutdown reclamation fully covers the audio data-loss hole.
- **Audio throughput budget ("OCR budget for audio"):** The originally requested fix. Rejected for this problem: a budget gates *admission*, but the audio lanes are already single-file and sequential, so simultaneous admission of microphone + system-audio jobs is paced by execution, not a real source of pressure. A budget would not touch a stuck `running` row.
- **A job-execution timeout for transcription:** A `process()` timeout only fires while work is actively running; an orphan has nothing running, so a timeout never trips. (Speaker analysis already has a 600s subprocess timeout; in-process Whisper/Parakeet do not, but a live transcription hang was not the confirmed symptom, so adding one is deferred.)
- **A single shared retry cap:** Counting abandonment toward the same cap as failure would re-introduce data loss — reprocessing a segment and quitting before it finished a few times would permanently drop it.
- **Longer shutdown timeout:** Blocking quit on a multi-minute Whisper pass is worse UX than requeue-and-exit.

## Consequences

Abandoned capture work survives a quit or crash and completes on the next run, and a recovered transcription re-chains its speaker analysis. Poison segments still give up after a small number of genuine failures. Separating the two retry reasons requires tracking failure attempts distinctly from total attempts (an additive schema change). Reclamation remains a startup-and-shutdown policy; there is no live-session watchdog.
