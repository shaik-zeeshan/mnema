# Windows 11 Media Writers and Preview Research

_Last researched: 2026-05-26_

This note is a focused follow-up to `docs/windows/runtime-capture-research.md`. It maps Mnema's current macOS media/preview stack to Windows 11 options so the future Windows backend can preserve Capture Segment, Captured Frame, frame-index, exact preview, and Scrub Preview semantics.

## Short recommendation

Use a native Windows stack first:

- **Writers/finalization:** Media Foundation `IMFSinkWriter` with the MPEG-4 file sink.
- **Screen segment format:** `.mp4` with H.264/AVC video. If we switch from `.mov`, first make hidden-workspace and preview path inference extension-aware.
- **Audio segment format:** AAC in an MPEG-4 container after a proof of `.m4a`/audio-only writer behavior; use `.wav` only as an early prototype/debug fallback.
- **Frame artifacts and cached previews:** WIC JPEG. Do not require WebP on Windows because WIC WebP support is an extension codec, not a baseline built-in codec.
- **Exact/video-backed previews:** Media Foundation `IMFSourceReader` seek/decode plus WIC JPEG encode.
- **Scrub previews:** generate from finalized indexed segment offsets; prefer sequential per-segment decode or batched source-reader work over one source-reader instance per frame.
- **Audio decode/trim:** Media Foundation Source Reader + Sink Writer, with Audio Resampler DSP or our own conversion to the AAC encoder's accepted PCM format.
- **Fallback if native cost is too high:** FFmpeg behind the same writer/decoder/preview seams. Avoid wiring FFmpeg directly into lifecycle or app-infra logic.

## Current Mnema behaviors that must be replaced

| Current code | macOS mechanism | Windows 11 replacement target | Notes |
| --- | --- | --- | --- |
| `crates/capture-writers/src/lib.rs` video writer | `AVAssetWriter` QuickTime `.mov`, H.264 output settings, lazy-created from first screen sample, optional bitrate | Media Foundation Sink Writer, MPEG-4 sink, H.264 encoder | H.264 encoder input is not BGRA; WGC BGRA frames need conversion to NV12/I420/YUY2/etc. before writing unless Sink Writer finds a compatible transform. |
| `crates/capture-writers/src/lib.rs` audio writer | `AVAssetWriter` `.m4a` AAC, 48 kHz stereo default, sample-format fallback candidates, tail buffering | WASAPI PCM -> normalize/resample -> Sink Writer AAC | Microsoft AAC encoder expects 16-bit PCM at 44.1/48 kHz and mono/stereo/5.1. Keep zero-sample cleanup and tail-trim behavior. |
| `crates/capture-writers::trim_audio_file_to_m4a` | shells out to `ffmpeg` for VAD boundary trim | Source Reader seek/decode -> Sink Writer AAC, or use FFmpeg fallback | Native path avoids bundling FFmpeg just for trim. |
| `convert_recording_audio_to_m4a` | `/usr/bin/afconvert` | Media Foundation or remove if Windows never muxes screen+audio | macOS-only command has no Windows equivalent. |
| `decode_audio_file_to_mono_pcm` plus transcription/speaker decode modules | AVFoundation/AVAudioFile decode to mono f32 | Media Foundation Source Reader to PCM + downmix/resample | Needed for Local Whisper, Parakeet, speaker analysis, and system-audio speech activity. |
| `crates/capture-screen` frame export | ScreenCaptureKit sample -> VideoToolbox CGImage -> ImageIO JPEG | WGC D3D11 texture -> CPU/D2D conversion -> WIC JPEG | Preserve `frame-<captured_at>-<index>.jpg` naming and captured-frame equivalence input. |
| `crates/capture-screen` frame-index finalization | `AVAssetReader` over finalized `.mov` samples to derive actual video-relative offsets | Source Reader over finalized `.mp4` samples to derive actual offsets | Do not derive sidecar offsets only from wall-clock guesses. |
| `apps/desktop/src-tauri/src/app_infra/frame_preview.rs` exact preview | `AVAssetImageGenerator`, zero tolerance when indexed; WebP then JPEG | Source Reader seek/decode; compare returned timestamp to indexed target; WIC JPEG | Exact inspection must fall back to persisted segment frames when decode/seek misses. |
| Same file Scrub Preview generation | `AVAssetImageGenerator` batch, tolerant time window, max size, JPEG | Source Reader decode from indexed offsets, WIC JPEG | Keep cache keying by finalized video, offset interval, rendition, and source freshness. |
| `mov_file_appears_openable_for_preview` and hidden workspace cleanup | shallow `.mov`/`moov` check | extension-aware finalized media validation, preferably Source Reader open + video stream check | `crates/app-infra/src/hidden_segment_workspace.rs` currently derives visible segment siblings as `.mov`; changing to `.mp4` requires a storage/path migration or per-platform extension support. |

## Media Foundation writer details

Media Foundation is the best fit because it is present on Windows 11 and covers the same jobs we currently delegate to AVFoundation:

- The **Sink Writer** hosts a media sink and optional encoders; the app feeds audio/video samples and the sink writer writes encoded bitstreams to a file.
- It supports uncompressed input with compressed output, which matches WGC/WASAPI input -> H.264/AAC output.
- It does **not** generally do resize, frame-rate conversion, or audio resampling for us. Plan explicit conversion before `WriteSample`.
- The **MPEG-4 file sink** creates MP4 files and can generate sample descriptions for H.264/AVC video, AAC audio, and MP3 audio. It does not itself encode.
- The Microsoft **H.264 encoder** accepts I420/IYUV/NV12/YUY2/YV12 input and outputs `MFVideoFormat_H264`.
- The Microsoft **AAC encoder** encodes AAC-LC only. Its documented input is 16-bit PCM, 44.1 or 48 kHz, with 1, 2, or 6 channels.

Implementation implications:

1. Capture WGC frames with stable QPC timestamps.
2. Convert `DXGI_FORMAT_B8G8R8A8_UNORM` frames to an H.264 encoder input format, preferably NV12.
3. Build `IMFSample`s with 100-ns timestamps/durations derived from capture timing.
4. `AddStream` with H.264/AAC output media type, `SetInputMediaType` with converted input type, `BeginWriting`, `WriteSample`, then `Finalize` on segment rotation/stop.
5. Treat `Finalize` success plus Source Reader reopen/video-stream validation as the Windows equivalent of current `.mov` openability checks.

## Preview and image encoding details

Use Media Foundation Source Reader for decoded video frames and WIC for image output:

- Source Reader is intended for getting raw data from a media source without writing a full Media Foundation playback pipeline.
- `IMFSourceReader::SetCurrentPosition` seeks in 100-ns units for normal media sources; `ReadSample` returns samples and timestamps.
- For exact previews, seek to the indexed offset, decode forward until the requested timestamp or nearest acceptable sample, then log/handle timestamp misses like the current `video_exact_miss` path.
- For Scrub Previews, avoid many isolated seeks. Use per-segment planning from the binary frame-index sidecar and decode requested buckets in batches/sequential order where possible.
- Use WIC JPEG for `image/jpeg` preview files. PNG is useful for diagnostics but too large for scrub caches. WebP can be optional only if the WebP Image Extension is installed.

## Alternatives checked

| Option | Use for Mnema? | Why / risk |
| --- | --- | --- |
| Media Foundation + WIC | **Recommended primary** | Native, no bundled binary, Windows 11 baseline, covers H.264/AAC, MP4, decode, and JPEG. More COM/D3D conversion work. |
| FFmpeg binary or `ffmpeg-next` | Keep as fallback | Fastest to implement exact extraction, trim, decode, and encode. Adds packaging, size, LGPL/GPL configuration, codec patent, security-update, and notarized/updater artifact complexity. |
| GStreamer | Not first choice | Capable cross-platform pipeline, but bundling/runtime plugin management is heavy for a Tauri desktop app. |
| `windows-capture` crate | Prototype/reference only | Useful WGC abstraction and examples, but production must verify timing, segment rotation, writer finalization, frame-index sidecars, liveness, and privacy behavior. |
| `scap` crate | Prototype/reference only | Cross-platform and WGC-backed on Windows, but beta/high-level; verify before depending on it. |
| CPAL for audio | Prototype only | Easier PCM capture, but WASAPI directly or `wasapi` crate gives better loopback/device/liveness control. |
| Pure Rust codec/mux stack | Not first choice | Real-time H.264/AAC/MP4 plus exact decode/preview would be a lot of risk; still may need platform codecs or FFmpeg. |

## Open decisions before implementation

- Do Windows screen outputs become `.mp4`, or do we preserve visible segment naming with `.mov` and accept a non-native writer path? Native Media Foundation points to `.mp4`.
- Are microphone/system-audio outputs `.m4a`, `.mp4` audio-only, or `.wav` during early bring-up?
- Is system audio muxed into screen `.mp4`, or kept separate like current macOS segment outputs? Keeping it separate preserves current source-family pause/resume and processing semantics.
- Do we build a D3D11/GPU conversion path for BGRA -> NV12 immediately, or start with CPU conversion and optimize later?
- What exact tolerance is acceptable for Windows Source Reader seeks before falling back to persisted frame artifacts?
- Should the media seam become trait-based now (`ScreenVideoWriter`, `AudioSegmentWriter`, `VideoFrameExtractor`, `ImageEncoder`, `AudioDecoder`) before adding Windows, or only during the Windows slice?

## Sources checked

- Microsoft Learn: Sink Writer — https://learn.microsoft.com/en-us/windows/win32/medfound/sink-writer
- Microsoft Learn: Using the Sink Writer — https://learn.microsoft.com/en-us/windows/win32/medfound/using-the-sink-writer
- Microsoft Learn: Tutorial: Using the Sink Writer to Encode Video — https://learn.microsoft.com/en-us/windows/win32/medfound/tutorial--using-the-sink-writer-to-encode-video
- Microsoft Learn: MPEG-4 File Sink — https://learn.microsoft.com/en-us/windows/win32/medfound/mpeg-4-file-sink
- Microsoft Learn: MPEG-4 Support in Media Foundation — https://learn.microsoft.com/en-us/windows/win32/medfound/mpeg-4-support-in-media-foundation
- Microsoft Learn: H.264 Video Encoder — https://learn.microsoft.com/en-us/windows/win32/medfound/h-264-video-encoder
- Microsoft Learn: AAC Encoder — https://learn.microsoft.com/en-us/windows/win32/medfound/aac-encoder
- Microsoft Learn: Source Reader — https://learn.microsoft.com/en-us/windows/win32/medfound/source-reader
- Microsoft Learn: Using the Source Reader to Process Media Data — https://learn.microsoft.com/en-us/windows/win32/medfound/processing-media-data-with-the-source-reader
- Microsoft Learn: `IMFSourceReader::SetCurrentPosition` — https://learn.microsoft.com/en-us/windows/win32/api/mfreadwrite/nf-mfreadwrite-imfsourcereader-setcurrentposition
- Microsoft Learn: `IMFSourceReader::ReadSample` — https://learn.microsoft.com/en-us/windows/win32/api/mfreadwrite/nf-mfreadwrite-imfsourcereader-readsample
- Microsoft Learn: WIC codecs from Microsoft — https://learn.microsoft.com/en-us/windows/win32/wic/native-wic-codecs
- Microsoft Learn: JPEG Format Overview — https://learn.microsoft.com/en-us/windows/win32/wic/jpeg-format-overview
- Microsoft Learn: Screen capture / Windows Graphics Capture — https://learn.microsoft.com/en-us/windows/apps/develop/media-authoring-processing/screen-capture
- Microsoft Learn: `Direct3D11CaptureFrame.SystemRelativeTime` — https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.direct3d11captureframe.systemrelativetime
- crates.io/docs.rs: `windows`, `windows-capture`, `wasapi`, `cpal`, `scap`, `ffmpeg-next`
- FFmpeg legal notes — https://www.ffmpeg.org/legal.html
