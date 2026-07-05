# Receipt plays cited audio as bounded synced clips

**Status:** accepted (reverses the "Receipt never grows audio playback" rule of the Jul 3 2026 grill / [ADR 0029](0029-user-context-outlives-raw-retention-privacy-delete-cascades.md)-era Receipt doctrine in `docs/user-context/CONTEXT.md`).

## Context

The **Receipt** (Timelapse) is the proof surface behind every Journal **Activity**: it scrubs the
**Captured Frame** pixels over the Activity's span and marks the engine-cited evidence frames.
But an Activity's evidence is frames **and** **Audio Transcription Span** segments (the LLM cites
both, `a<id>`/`f<id>`), and the Receipt discarded every `audio_segment` ref — filtering evidence to
`subjectType === "frame"`. Two failures followed: (1) an Activity grounded only in audio (a call
with the screen off, a mic-only stretch) rendered a **false "footage expired"** panel, and (2) an
Activity with frames but audio-cited evidence undercounted its proof and lost an audio-headlined
poster. The original doctrine sent all audio to **Timeline** ("Receipt is proof, not inspection;
never grows audio playback"). We decided that doctrine is wrong for audio: hearing the cited moment
*is* the proof, the same way seeing the cited pixel is.

## Decision

The Receipt is **both proof and inspection of one Activity**. It keeps the fast frame timelapse and
adds **bounded, synchronized audio-plus-screen clips**: an engine-cited audio segment becomes a
"relive this moment" control that plays *that segment's* real audio at 1× (`get_audio_segment_media`,
already used by Timeline) while the frame viewer runs over the same window, clocked by the audio
element (`started_at + audio.currentTime` → frame index). The boundary moves from **silent → audible**
to **bounded → unbounded**: everything in the Receipt stays inside the one Activity's span, and
**Timeline** keeps *unbounded* inspection — continuous cross-Activity audio scrubbing, OCR text
copy/download, frame/audio export, navigation past the span. "Open in Timeline" remains the handoff
for anything wider than this Activity.

## Considered options

- **Timeline handoff only (status quo doctrine).** Rejected: leaves audio-grounded Activities with a
  lying empty state and makes the proof surface silent about half its own evidence.
- **Audio over the 8× timelapse.** Rejected as incoherent: continuous audio cannot ride a sparse
  frame-swap sped up 8×/16×. Sync is only meaningful at 1× over a bounded window.
- **Silent proof only (ticks/count/snippet, no playback).** Rejected by the product call: a frame
  tick self-reveals on scrub, but a silent audio tick reveals nothing, so audio proof would stay
  second-class. Playback is what gives audio parity with frames.

## Consequences

- The timelapse (2×/8×/16× frame scan) and the cited audio clip (1× real-time) are **two speeds of
  one bounded surface**, not two features. The clip is clocked by `<audio>`, not the rAF timer.
- **Audio-only Activities** (no frames, ever or after retention) turn the Receipt into a plain
  bounded audio player and must render as audio evidence, never "footage expired".
- Segments are already bounded (≤ the 5-min capture-segment cap, each with its own media file), so
  "bounded clip" needs no new limit — the segment *is* the bound.
- Frontend-only: no new tables, no new Tauri commands. Reuses `get_audio_segment_media` and the
  `audio_segment` evidence refs already hydrated on the Journal range read.
- The Receipt **displays** speaker attribution on cited segments, **read-only**: source
  (`microphone` ≈ you / `system_audio` ≈ the other side) always, plus a recognized name or anonymous
  "Speaker N" when diarization has it. This exists because the engine is speaker-blind and
  misattributes (words spoken *to* the user become "you said X") — the Receipt is where the user
  catches it. Attribution is **late-bound by id**: the activity references the `audio_segment` (and
  its speaker cluster), the name is resolved live at display time and **never frozen** into derived
  data, so a voice named *after* the activity was derived shows its real name next open (freezing
  "unknown Speaker 2" as a fact would be wrong). It is **surface-and-handoff** — the Receipt shows
  who spoke but never corrects it: naming/merging voices stays **Timeline**/recognized-people, a
  mis-derived card routes to the Activity's own Dismiss/correct, and no correction UI lives in the
  modal. Making *derivation itself* source-aware so it stops misattributing at the source is a
  separate follow-up — [ADR 0050](0050-derivation-is-source-aware-names-resolve-on-device.md).
