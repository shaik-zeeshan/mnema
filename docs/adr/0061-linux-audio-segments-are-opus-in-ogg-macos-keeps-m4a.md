# Linux audio segments are Opus in Ogg; macOS keeps m4a/AAC

## Status

Accepted (2026-11-01) for the Linux port; not yet implemented.
Adds a Linux note to [ADR 0052](0052-system-audio-is-an-independent-capture-family-on-core-audio-process-taps.md),
whose "Output is unchanged m4a/AAC" now reads "m4a/AAC on macOS, Opus/Ogg on Linux". The same
sentence in `CLAUDE.md` (Capture Pipeline) changes with it. Source: `docs/linux-port-report.md`
§4.5, §10 decision 5.

## Context

Every microphone and system-audio segment on macOS is AAC in an `.m4a` file, written by
AVAssetWriter and decoded by AVFoundation (`crates/capture-writers/src/lib.rs`, with
`trim_audio_file_to_m4a` at `:26` and `decode_audio_file_to_mono_pcm` at `:165`). None of that
exists on Linux.

The `.m4a` name is baked in widely:

- The segment planner names every audio file `*.m4a` (`crates/capture-runtime/src/lib.rs:209-238`).
- 38 Rust, TypeScript and Svelte files under `crates/` and `apps/desktop/src/` mention `.m4a` or
  `"m4a"`.
- The audio MIME map knows only m4a/mp4/aac/wav (`apps/desktop/src-tauri/src/app_infra.rs:1150-1161`).
- Deepgram uploads hard-code `Content-Type: audio/mp4`
  (`crates/audio-transcription/src/providers/deepgram.rs:102`).

Keeping AAC on Linux is costly:

- Encoding needs `fdk-aac` (licence has no patent grant, GPL-incompatible, no longer "allowed" in
  Fedora) or FFmpeg's encoder through a large native dependency that varies by distro.
- WebKitGTK plays AAC only with `gst-libav` or the fdkaac plugin, which are often missing. Voice
  enrollment plays segments in an `<audio>` element.

Opus is royalty-free, libopus builds statically, and Ogg/Opus plays with `gst-plugins-base`, which
WebKitGTK already depends on. Deepgram accepts Ogg/Opus.

## Decision

**Linux writes Opus in Ogg. macOS keeps m4a/AAC. Each platform reads only its own format.**

1. **One per-platform extension constant** in the segment planner, used for microphone, reconnect
   and system-audio files. Every other site that matches the literal `m4a` switches to the constant
   or to a "known audio extension" check. File names keep their current shape; only the extension
   differs.
2. **Encode** with libopus (the `opus`/`audiopus` crates, built statically) and the `ogg` crate for
   pages, on a writer thread fed from a ring buffer (the PipeWire callback only copies).
3. **Trim** (idle-pause tail trim) decodes, slices and re-encodes. Segments are at most 5 minutes, so
   this is cheap. The function is renamed from `trim_audio_file_to_m4a` to a format-neutral name and
   keeps its signature.
4. **Decode** through one shared non-macOS `decode_audio_file_to_mono_pcm` in `capture-writers`:
   `symphonia` 0.6 for Ogg demux, `symphonia-adapter-libopus` for Opus (Symphonia has no native
   Opus decoder), and `rubato` to resample to 16 kHz. Transcription, diarization, system-audio
   speech detection and waveform peaks all call it.
5. **MIME and upload types come from the extension.** The MIME map gains the Ogg/Opus entry, and
   Deepgram's `Content-Type` is derived from the file extension instead of the fixed `audio/mp4`.

## Considered options

- **AAC on Linux through fdk-aac or FFmpeg.** Keeps one format everywhere, but brings a licence
  question, a heavy or distro-dependent native dependency, and unreliable webview playback.
- **GStreamer for both encode and decode** (`appsrc ! opusenc ! oggmux`, `decodebin`). One framework,
  much like AVFoundation on macOS, and already present under WebKitGTK. Heavier, and plugin
  availability varies. Kept as the fallback if the libopus/symphonia stack hits a wall.
- **Write AAC, serve decoded PCM/WAV to the webview.** Avoids the GStreamer codec dependency for
  playback but keeps the encoder licence problem.

## Consequences

- The recordings directory is no longer one format across platforms. Nothing moves recordings
  between a Mac and a Linux machine today, so no converter is planned.
- A cross-cutting change touches about 38 files. Any new code that assumes `.m4a` is a Linux bug;
  reviewers should treat the literal as a smell outside macOS-only modules.
- Transcription, VAD, speaker analysis, retention and search are unchanged beyond the decoder swap,
  because they work on decoded PCM and file paths.
- Tests that assert `.m4a` paths need to go through the constant or be macOS-only.
- The Linux CPU cost of encoding two 48 kHz streams is estimated at under 1% of a core; it is not
  yet measured.
