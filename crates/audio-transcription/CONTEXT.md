# Audio Transcription Context

Audio segment transcription jobs, local transcription providers, model choices, timing spans, and transcript search units.

Root entry point: [CONTEXT-MAP.md](../../CONTEXT-MAP.md).

## Language

**Audio Segment**:
A time-bounded persisted audio recording file produced from one recording source during a recording session.
_Avoid_: audio file, raw microphone file, sound clip

**Audio Transcription**:
Recognized speech text, with optional timing relative to its **Audio Segment**, language, confidence, provider, and model metadata, derived from one **Audio Segment**.
_Avoid_: transcript blob, transcription result, speech text

**Audio Transcription Span**:
A searchable time range of recognized speech inside one **Audio Segment**.
_Avoid_: transcript chunk, audio hit, audio clip

**Audio Transcription Job**:
Background work that recognizes speech for one **Audio Segment**.
_Avoid_: transcription task, transcript worker item, speech job

**Audio Transcription Provider**:
A local speech recognition option used by an **Audio Transcription Job** to produce an **Audio Transcription**.
_Avoid_: cloud transcription service, transcription engine, ASR backend

**Audio Transcription Model**:
A local model asset selected for an **Audio Transcription Provider** when that provider requires app-managed model files.
_Avoid_: model file, downloaded artifact, checkpoint

## Relationships

- An **Audio Segment** comes from exactly one recording source, such as microphone or system audio.
- An **Audio Transcription Job** operates on exactly one **Audio Segment**.
- An **Audio Transcription Job** uses exactly one **Audio Transcription Provider**.
- V1 **Audio Transcription Provider** values are local-only: `local_whisper`, `apple_speech_on_device`, and `parakeet`.
- V1 `local_whisper` **Audio Transcription Model** choices are `tiny`, `base`, `small`, and `medium`, with `base` as the default.
- V1 `parakeet` uses `parakeet-tdt-0.6b-v3-onnx` (full) or `parakeet-tdt-0.6b-v3-onnx-int8` as its **Audio Transcription Model** and runs it through the Rust ONNX Runtime adapter.
- **Audio Transcription Model** availability is platform-gated where footprint warrants it: Windows offers only the int8 `parakeet` model, while macOS offers both variants. A selected model that is unavailable on the current platform behaves like a missing **Audio Transcription Model**, not an error.
- `parakeet` runs CPU-only in v1 on every platform; GPU execution providers (DirectML, CUDA) are a future, separately-decided change.
- `apple_speech_on_device` uses OS-managed language models rather than an app-managed **Audio Transcription Model**.
- An **Audio Transcription Provider** may require one selected **Audio Transcription Model**.
- A microphone **Audio Segment** gets one **Audio Transcription Job** for the selected **Audio Transcription Provider** when that provider and its required **Audio Transcription Model** are available.
- An **Audio Transcription Job** freezes the selected **Audio Transcription Provider** and **Audio Transcription Model** at admission time.
- Changing the selected **Audio Transcription Provider** or **Audio Transcription Model** affects future **Audio Segment** values, not existing completed **Audio Transcription** values.
- An app-managed **Audio Transcription Model** may be installed, missing, downloading, or failed.
- If the selected **Audio Transcription Provider** or required **Audio Transcription Model** is unavailable, the microphone **Audio Segment** remains eligible but does not get an **Audio Transcription Job** until backfill can enqueue it.
- An **Audio Transcription** is derived from exactly one **Audio Segment**.
- An **Audio Transcription** contains zero or more **Audio Transcription Span** values.
- An **Audio Transcription Span** belongs to exactly one **Audio Segment** and is derived from transcript timing when available.
- **Audio Transcription Span** derivation prefers provider transcript segments, falls back to word-derived windows only when segments are absent, and falls back to the whole **Audio Segment** only for untimed transcript text.
- **Audio Transcription Span** results may come from microphone or system-audio **Audio Segment** values.
- **Search Context** for an **Audio Transcription Span** should include its recording source.
- Adjacent or overlapping **Audio Transcription Span** hits from the same **Audio Segment** may collapse into one **Search Result Group**.
- **Audio Transcription Span** hits from different **Audio Segment** values or separated moments should remain separate results.
- A search result for audio speech should anchor to an **Audio Transcription Span** when timing is available.
- Audio-to-frame alignment should prefer the latest **Captured Frame** at or before the **Audio Transcription Span** start time.
- An **Audio Transcription Span** remains a valid **Search Result Anchor** even when no nearby **Captured Frame** exists.
- An empty no-speech **Audio Transcription** is a successful **Audio Transcription**, not a failed job.
- An **Audio Transcription** is produced by an **Audio Transcription Job**.

## Flagged Ambiguities

- "audio file" was used to mean the persisted unit for transcription; resolved: use **Audio Segment** for the time-bounded persisted recording file.
- "provider" was considered for both cloud and local transcription services; resolved: **Audio Transcription Provider** means local-only for v1.
