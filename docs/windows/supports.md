# Windows Runtime Capture — Support & Behavior Contract

_Scope of this document: the **runtime screen-capture** bring-up for Windows only._
_It records what the Windows capture backend does and, deliberately, does **not** do_
_in the MVP, plus the platform floor and known limitations we accept for now._

Companion research: [`runtime-capture-research.md`](./runtime-capture-research.md),
[`media-processing-research.md`](./media-processing-research.md).

## Platform floor

- **Windows 11 (build 22000+) only.** Windows 10 is intentionally unsupported.
- Support is gated at runtime on
  `GraphicsCaptureSession::IsSupported() && ApiInformation::IsPropertyPresent(GraphicsCaptureSession, "IsBorderRequired")`.
  Tying "supported" to the `IsBorderRequired` property (Win11 22000+) rather than
  an OS build string is what excludes Win10 — we depend on disabling the capture
  border, which Win10 cannot do.
- On an unsupported environment `support_for_current_platform()` reports
  `native_capture_supported = false`, and the existing "native capture
  unsupported" settings UI renders. No new failure UI.

## What the Windows runtime captures

- **Screen video** of the **primary monitor**, via Windows Graphics Capture
  (`CreateForMonitor`), encoded to **H.264 `.mp4`** through the Media Foundation
  `IMFSinkWriter`.
- **Microphone audio** from the selected/default WASAPI capture endpoint,
  encoded to **AAC `.m4a`** through Media Foundation.
- **Independent system audio** via WASAPI loopback when a default render endpoint
  is available, encoded to **AAC `.m4a`**. Windows does not require screen
  capture to request system audio.
- **Cursor** is included in the recording.
- **Resolution presets and custom output sizes are honored.** WGC still captures
  the primary monitor at native size, then the CPU BGRA -> NV12 conversion scales
  to the resolved output size. "Original" preserves the native size.
- **Bitrate presets and custom bitrate are honored** through `MF_MT_AVG_BITRATE`
  on the Media Foundation H.264 output type.
- **JPEG frame export** writes low-cadence frame artifacts at the configured
  output resolution, matching the encoded video size.

## Explicitly out of scope for current Windows support (deferred follow-ups)

These are **not bugs** — they are deliberately deferred:

- **No finalized-video frame-index sidecars yet.** Windows can export JPEG frame
  artifacts, but indexed finalized-video preview timing remains a later follow-up.
- **No Windows OCR provider default yet.** The cross-platform Tesseract provider
  is the intended later path.
- **No privacy / per-app exclusion filters.** WGC has no ScreenCaptureKit-style
  live app-exclusion for full-monitor capture; do not promise it.
- **No hardware-accelerated encode.** Software H.264 only for now; hardware
  encode is a later optimization behind the same backend trait.
- **No multi-monitor or per-window capture.** Primary monitor only.

## Behavioral notes & known limitations

### Inactivity pause is source-family aware on Windows

Windows uses system-input snapshots plus capture-sourced screen, microphone, and
system-audio activity samples for inactivity decisions. When inactivity pause is
enabled, Windows pauses the requested source families independently and resumes
only the paused families when relevant activity returns. Whole-runtime
inactivity pause/resume and screen transient-liveness resume use the same
segment-start path, preserving shared `CaptureClock` / `SegmentSchedule`
boundaries and segment naming.

### Primary monitor only

A multi-monitor Windows user records only the **primary** display; secondary
monitors are silently not captured. This matches macOS (single main display) but
is more likely to surprise Windows users.

### Disruption handling

- **Resolution / DPI / display-mode change:** when a frame reports a
  `ContentSize` different from the current WGC frame-pool size, the capture
  thread calls `Direct3D11CaptureFramePool::Recreate(...)` in-session and
  continues recording without restarting the runtime. The encoded output size is
  still the configured segment size; frames from the new native source size are
  scaled into that output size.
- **Monitor sleep / DPMS off:** frames pause and resume; usually no action needed.
- **Display unavailable / primary monitor loss (#62):** when the runtime observes
  the display-unavailable transient-liveness path, it suspends **screen only** as
  `TransientLiveness { DisplayUnavailable }`, keeps microphone and independent
  system audio running, and auto-resumes screen through the shared
  transient-liveness/start-segment path when a display returns.
- **Windows session lock (#63):** `Win+L` / workstation lock suspends **screen
  only** as `TransientLiveness { SessionLock }`. Microphone remains active/not
  paused, independent system audio remains active/not paused when requested, and
  audio source-session ids stay stable. Unlock automatically resumes screen
  through the shared transient-liveness/start-segment path; audio must not stop
  or rotate solely because of the lock.

### Frame pacing

Capture is variable-frame-rate and change-driven (mirrors macOS): frames are
emitted on screen change, capped to `screen_frame_rate`. A static screen produces
few frames; per-segment timestamps still span the full 60 s wall-clock.

### CPU cost / high frame rates

Common profiles (720p or 1080p @ 1 fps) cost well under ~1–2% of one CPU core.
Because the MVP uses **software** H.264 encode, **1080p @ 30 fps is expensive
(~30–80% of one core while it is on)**. 30 fps is an opt-in option for occasional
inspection, not the expected mode; a Settings hint notes that higher frame rates
use more CPU on Windows. Hardware encode (a later optimization) removes this cost.

## Settings copy that changes on Windows

- Screen capture reads as **supported** (was "macOS-only").
- Resolution preset/custom controls are enabled on Windows and describe output
  scaling.
- Bitrate preset/custom controls are enabled on Windows and describe Media
  Foundation H.264 bitrate control.
- System audio is supported through independent WASAPI loopback when the support
  probe finds a default render endpoint; it does not require screen capture on
  Windows.
- Microphone capture is supported through WASAPI endpoints with best-effort
  Windows privacy-denial surfacing.
- Inactivity and transient-liveness controls are Windows-aware: display
  unavailable and session lock pause screen only, while requested microphone and
  independent system audio continue.
