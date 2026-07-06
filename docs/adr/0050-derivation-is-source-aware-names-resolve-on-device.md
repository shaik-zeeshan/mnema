# Derivation is source-aware; speaker names resolve on-device

**Status:** accepted, implementation deferred (a follow-up to
[ADR 0049](0049-receipt-plays-cited-audio-as-bounded-synced-clips.md)).

## Context

User Context **Activity** derivation feeds the LLM only the audio transcript `result_text`
(`capture_source.rs` selects `id, started_at, result_text` — nothing else). The engine is therefore
speaker- and source-blind and **misattributes**: words spoken *to* the user (system audio — the
other side of a call) come out as words the user said ("you decided X"). Mnema already has the
signal — every `audio_segment` records `source_kind` (`microphone`/`system_audio`) and diarization
writes speaker turns — it just never reaches the prompt. The **Receipt** (ADR 0049) lets the user
*catch* the misattribution; this ADR is about stopping it at the source.

## Decision

Make derivation **source-aware**: tag each `a<id>` window item with its source — *you* (microphone)
vs *the other side* (system audio) — so the engine stops the you/other misattribution at the source.
Anonymous "Speaker A/B" turn structure is an optional next layer for the in-person, one-mic-many-people
case. **Recognized names never enter the prompt of a cloud engine** — a person's name is identifiable
third-party data and sending it past the "only redacted text crosses the wire" line is a new egress we
refuse. Speaker identity is carried **by id and resolved to a display name on-device at read time**,
never frozen into derived data: an activity references the `audio_segment` (and its speaker cluster),
and the Receipt resolves the *current* recognized name live — so a voice the user names *after* the
activity was derived shows its real name next open, and an unnamed voice shows "Speaker N", never a
stale "unknown Speaker 2" baked in at derivation time.

## Consequences

- The you/other fix is nearly free: one column added to the audio window query + one tag in
  `build_prompt`. Not PII, safe for a cloud engine.
- Correcting a speaker (Timeline / recognized-people) now **propagates**: re-derivation reads the
  corrected source/turn structure, and display resolves the corrected name by id — which is why the
  Receipt needs no activity-correction affordance of its own.
- Names-to-cloud stays forbidden; a local engine could in principle receive more, but the default
  and safe path is source + anonymous turns only.
- **Deferred and separable:** this is a derivation/prompt change. The Receipt's read-only attribution
  display (ADR 0049) reads local speaker data directly and can ship first; this lands as its own
  work-item.
