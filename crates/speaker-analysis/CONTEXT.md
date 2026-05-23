# Speaker Analysis Context

Speaker analysis jobs, diarization result policy, speaker turn alignment, and session-level speaker continuity.

Root entry point: [CONTEXT-MAP.md](../../CONTEXT-MAP.md).

## Language

**Speaker Analysis Job**:
Local diarization and optional recognition work for one microphone **Audio Segment**.
_Avoid_: speaker task, diarization worker item, speaker recognition task

**Speaker Turn Alignment**:
The policy that assigns **Audio Transcription** words or segments to speaker turns. Transcription timing is primary; speaker turns annotate that text and do not stretch, split, or duplicate transcript text.
_Avoid_: transcript rewriting, diarization-owned text timing, speaker text retiming

**Speaker Continuity**:
The session-level policy that keeps a real speaker associated with a stable speaker cluster across **Audio Segment** values with the same `session_id`, provider, and model.
_Avoid_: segment-local speaker identity, provider cluster identity, cross-session speaker identity

## Relationships

- **Retention Cleanup** preserves **Person Profile** values even when derived speaker rows are deleted.
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
- Speaker turns may decorate an **Audio Transcription Span** when available, but they do not define the searchable audio unit.

## Example Dialogue

> **Dev:** "Should speaker turns define the searchable audio result?"
> **Domain expert:** "No — **Audio Transcription Span** comes from transcript timing; speaker turns can label who spoke when that annotation exists."
