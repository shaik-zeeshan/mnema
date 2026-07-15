# Media Foundation is the only Windows media backend

## Status

Accepted.

## Context

Completing the Windows recording pipeline requires media primitives beyond capture itself: decoding captured `.m4a`/`.mp4` audio to mono PCM (Local Whisper, Parakeet, Sherpa speaker analysis, system-audio speech activity), trimming the inactivity tail at audio finalization, extracting video frames (exact and scrub previews, finalized-video validation), and validating finalized artifacts. `docs/windows/media-processing-research.md` surveyed three candidates: Media Foundation Source Reader/Sink Writer, Symphonia (pure-Rust audio decode only), and FFmpeg (broadest coverage, but LGPL/GPL compliance, updater surface, and binary size make shipping it a product/legal decision).

The Windows capture side already builds on Media Foundation: screen segments are written with an H.264 IMFSinkWriter (`crates/capture-screen/src/windows_capture.rs`) and audio segments with the AAC `WindowsAacM4aSinkWriter` (`crates/capture-microphone/src/windows_microphone.rs`).

## Decision

Media Foundation is the single Windows backend for all media file processing: audio decode to mono PCM, audio trim/convert/finalization, video decode and frame extraction, and artifact validation.

- Decode uses MF Source Reader; encode/trim uses MF Sink Writer — the same APIs the capture writers already use.
- FFmpeg is explicitly not shipped in the Windows product. It may remain a developer-only comparison/debug tool.
- Symphonia is not added; a second decode stack for the formats Mnema itself produces is complexity without payoff.

## Alternatives Rejected

- **Symphonia for audio decode.** Pure Rust and testable off-Windows, but audio-only (video frame extraction still needs MF), feature/licensing-gated format coverage, and a second decode stack to maintain alongside MF.
- **FFmpeg as the backend.** Broadest format coverage and easy trim/extract, but packaging, LGPL/GPL legal surface, update/security burden, and binary size are disproportionate when Mnema only needs to process its own captured formats.

## Consequences

All Windows processing seams share one backend and one set of COM/threading idioms already proven in capture code. MF seeking lands on keyframes, so exact frame extraction must decode forward from the seek point to the target timestamp — the same reconciliation the macOS `AVAssetImageGenerator` path and the binary frame-index sidecar already handle. Format coverage is bounded by MF codecs; if Mnema later imports externally produced media, that decision must be revisited rather than quietly reaching for FFmpeg.
