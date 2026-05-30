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

## What the Windows MVP captures

- **Screen video** of the **primary monitor**, via Windows Graphics Capture
  (`CreateForMonitor`), encoded to **H.264 `.mp4`** through the Media Foundation
  `IMFSinkWriter`.
- **JPEG frame export** at ~1 fps (`ScreenFrameArtifact`), the same artifact the
  macOS backend emits.
- **Cursor** is included in the recording (hardcoded on for the MVP).
- **Resolution presets** (Original / 1080p / 720p / 540p) and **bitrate presets**
  (Low / Medium / High) are honored, reusing the same preset math as macOS.
  Output is downscaled from native during capture; exported JPEG frames use the
  downscaled output resolution.

## Explicitly out of scope for the MVP (deferred follow-ups)

These are **not bugs** — they are deliberately deferred:

- **No system-audio capture** (WASAPI loopback). Screen video only.
- **No microphone capture.**
- **No OCR / search on Windows yet.** Capture produces `.mp4` + JPEG frames and
  emits `ScreenFrameArtifact`s, but `default_ocr_provider()` is still
  `AppleVision` (macOS-only). **On Windows the captured frames are written but not
  yet consumed** — searchability is a separate follow-up (the cross-platform
  Tesseract provider already exists and is the intended path).
- **No privacy / per-app exclusion filters.** WGC has no ScreenCaptureKit-style
  live app-exclusion for full-monitor capture; do not promise it.
- **No inactivity auto-pause.** See below.
- **No hardware-accelerated encode.** Software H.264 only for the MVP; hardware
  encode is a later optimization behind the same backend trait.
- **No multi-monitor or per-window capture.** Primary monitor only.

## Behavioral notes & known limitations (accepted for the MVP)

### Inactivity pause is disabled on Windows

macOS defaults to pausing capture when the user is idle
(`pause_capture_on_inactivity: true`). On Windows there is **no usable activity
signal** (`current_system_idle_ms()` is `None`, screen-activity polling is
macOS-only, audio is deferred) and the pause mechanism itself is macOS-only.

**Therefore Windows records continuously whenever recording is on**, regardless of
the `pause_capture_on_inactivity` setting, which is treated as off and hidden in
Settings on Windows. Consequence: an idle Windows machine keeps recording and
rotating 60 s segments — **more disk and battery use than macOS**. Idle-based
pausing (`GetLastInputInfo` + hard stop/start) is a self-contained follow-up.

### Primary monitor only

A multi-monitor Windows user records only the **primary** display; secondary
monitors are silently not captured. This matches macOS (single main display) but
is more likely to surprise Windows users.

### Disruption handling

- **Resolution / DPI / display-mode change:** survived in-session via
  `frame_pool.Recreate(...)` — gapless, no restart.
- **Monitor sleep / DPMS off:** frames pause and resume; usually no action needed.
- **Primary monitor disconnected (`GraphicsCaptureItem.Closed`):** the session is
  marked failed and surfaced; **recording stops until the user restarts it.** No
  automatic re-acquire in the MVP.

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
- Resolution and bitrate read as **live** on Windows (the recent gating commit's
  "macOS-only / Original-locked" copy is updated).
- System audio, microphone, privacy filters, and inactivity remain **macOS-only**
  / hidden on Windows.
