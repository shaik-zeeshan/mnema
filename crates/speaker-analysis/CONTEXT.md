# Speaker Analysis Context

Speaker analysis jobs, diarization result policy, speaker turn alignment, and session-level speaker continuity.

Root entry point: [CONTEXT-MAP.md](../../CONTEXT-MAP.md).

## Decisions

- [ADR 0001: Speaker Model Presets and Per-Preset Voiceprint Scope](docs/adr/0001-speaker-model-presets-and-voiceprint-scope.md) — superseded by 0003
- [ADR 0002: Adopt speakrs as a Second On-Device Diarization Provider, Trending to Replacement](docs/adr/0002-adopt-speakrs-as-second-on-device-diarization-provider.md) — its replacement path is now realized by 0003
- [ADR 0003: Remove sherpa, Make speakrs the Sole On-Device Diarization Provider](docs/adr/0003-remove-sherpa-make-speakrs-sole-diarization-provider.md)

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

**Speaker Model Preset**:
A curated, named choice backed by exactly one combined segmentation+embedding `model_id` in the manifest. There is one shipping preset today — the `speakrs` `pyannote-community-1-wespeaker` artifact, the sole on-device diarization provider. The machinery still supports more than one entry, but only validated presets are added.
_Avoid_: raw segmentation/embedding pickers, model file names, arbitrary user-built model combos

**Voiceprint Space**:
The set of enrolled **Person Profile** embeddings that are comparable to each other, scoped to one **Speaker Model Preset**'s `model_id`; recognition only matches within the active preset's space.
_Avoid_: global voiceprints, cross-model recognition, embedding-only scope

## Relationships

- **Retention Cleanup** preserves **Person Profile** values even when derived speaker rows are deleted.
- A **Speaker Analysis Job** operates on exactly one microphone **Audio Segment**.
- A **Speaker Analysis Job** can complete successfully with no speaker turns.
- Too-short, silent, or valid no-speaker audio produces a successful empty speaker-analysis result, not a failed **Speaker Analysis Job**.
- Missing speaker models, audio decode failures, speaker runtime failures, subprocess failures, malformed helper output, and persistence failures are **Speaker Analysis Job** failures.
- Successful **Speaker Analysis Job** diagnostics live in result provenance.
- Failed **Speaker Analysis Job** diagnostics live in `processing_jobs.last_error`.
- **Speaker Analysis Job** execution has a dedicated single-concurrency processing worker so speaker work does not block OCR/frame-batch or audio-transcription lanes.
- Speaker analysis has a single on-device diarization provider, **speakrs** (pure-Rust pyannote-community-1 segmentation + WeSpeaker embedding + VBx clustering on CoreML; model id `pyannote-community-1-wespeaker`). The helper runs subprocess-per-job; no persistent helper daemon, in-process model reuse, or generic audio-heavy worker abstraction is part of the current design. To bound the transient CoreML memory peak, speakrs runs segments at or below a fixed internal safe-chunk window (180s, `SPEAKRS_SAFE_CHUNK_SECONDS`) whole, and diarizes longer ones in sequential chunks no larger than that window, then stitches the per-chunk speaker clusters back into segment-wide identities by centroid cosine similarity (`SPEAKRS_STITCH_SIMILARITY` = 0.6). Whole-segment diarization spikes a large transient CoreML buffer past ~3min; chunking caps that peak (and is faster) while staying DER-neutral on the VoxConverse bench — ADR 0003's amendment documents this and marks the old "no chunking" rationale disproven by measurement. The window is a fixed internal constant, not a tunable: the dispatch path keeps a generic `provider` axis and the helper accepts a `--safe-chunk-ms` flag, but speakrs accepts-and-ignores it; it existed for a provider with its own per-call ceiling.
- Each **Speaker Analysis Job** freezes its helper timeout in payload option `helperTimeoutSeconds` when admitted, so later settings changes affect only future jobs.
- The speaker-analysis helper timeout defaults to 600 seconds, clamps to 60-3600 seconds, and timeout failures kill/reap the helper before the job follows the normal failed processing path.
- **Speaker Turn Alignment** treats **Audio Transcription** words or segments as the source timeline and assigns them to the best speaker turn annotation.
- **Speaker Continuity** is limited to **Audio Segment** values in the same recording/session, represented by stable cluster rows rather than provider-local cluster ids.
- Speaker provider cluster ids are provenance from the diarization provider and remain provider-local; they are not rewritten to represent stable identity.
- Speaker merge suggestions are preferred over aggressive automatic merges when continuity matching is ambiguous or only moderately similar.
- VAD-based audio cutting or trimming is outside **Speaker Analysis Job** quality policy; audio segment production remains owned by the recording flow.
- Speaker turns may decorate an **Audio Transcription Span** when available, but they do not define the searchable audio unit.
- A **Speaker Model Preset** maps to exactly one manifest `model_id` (one combined segmentation+embedding artifact); presets are curated, not assembled by users.
- Each **Speaker Model Preset** defines its own **Voiceprint Space**; switching presets changes both **Speaker Continuity** keying and which enrolled people are recognizable.
- Switching **Speaker Model Preset** does not delete prior enrollments: **Person Profile** embeddings persist per `model_id`, so an earlier preset's recognition returns if the user switches back.
- Changing **Speaker Model Preset** warns (when enrolled people exist) but does not block, auto-migrate voiceprints, or re-run diarization on past recordings; re-enrollment is organic re-tagging under the new preset.

## Example Dialogue

> **Dev:** "Should speaker turns define the searchable audio result?"
> **Domain expert:** "No — **Audio Transcription Span** comes from transcript timing; speaker turns can label who spoke when that annotation exists."
