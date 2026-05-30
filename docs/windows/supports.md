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
- **Cursor** is included in the recording (hardcoded on for the MVP).
- **Native resolution only** ("Original"). Capture runs at the primary monitor's
  native size; resolution/bitrate preset honoring is a deferred follow-up.

## Explicitly out of scope for the MVP (deferred follow-ups)

These are **not bugs** — they are deliberately deferred:

- **No system-audio capture** (WASAPI loopback). Screen video only.
- **No microphone capture.**
- **No JPEG frame export.** The macOS backend emits `ScreenFrameArtifact`s at
  ~1 fps for OCR; the Windows MVP produces the `.mp4` only. Frame export (and the
  OCR/search that consumes it) is a deferred follow-up — `supports_frame_export()`
  stays `false` on Windows, and `default_ocr_provider()` is still `AppleVision`
  (macOS-only). The cross-platform Tesseract provider is the intended later path.
- **No resolution / bitrate preset honoring.** Capture is fixed at the primary
  monitor's native resolution ("Original"). Downscaling and bitrate presets are a
  later follow-up.
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

- **Resolution / DPI / display-mode change:** frames whose content size no longer
  matches the encoder's fixed size are **skipped** for the MVP (logged once);
  in-session `frame_pool.Recreate(...)` re-acquisition is a follow-up.
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
- Resolution stays locked to **"Original"** on Windows (we capture at native
  resolution); resolution/bitrate preset honoring is a deferred follow-up, so the
  existing "Original-locked" copy is unchanged.
- System audio, microphone, privacy filters, and inactivity remain **macOS-only**
  / hidden on Windows.
