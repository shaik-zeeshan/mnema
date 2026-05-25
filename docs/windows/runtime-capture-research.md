# Windows Runtime Capture Research

_Last researched: 2026-05-25_

This note records the current recommendation for a future Windows native capture backend in Mnema. It is based on Microsoft Learn docs plus a quick Rust crate survey.

## Short recommendation

Use native Windows APIs, not browser capture:

- **Screen:** primary backend = **Windows Graphics Capture (WGC)** through `Windows.Graphics.Capture` / Direct3D 11. Keep **DXGI Desktop Duplication** as a possible fallback or diagnostic backend.
- **Microphone:** **WASAPI capture** from `eCapture` endpoints.
- **System audio:** **WASAPI loopback** from `eRender` endpoints. Treat it as an independent source on Windows unless product semantics intentionally tie it to screen.
- **Encoding/finalization:** prefer **Media Foundation Sink Writer** + **MPEG-4 file sink** for native `.mp4`/`.m4a` output. Use FFmpeg only if Media Foundation extraction/encoding becomes too slow to implement or too limiting.
- **Rust bindings:** use the `windows` crate for exact control. `wasapi` is a strong helper crate for audio. `windows-capture` is useful for prototyping/samples, but production needs to verify timing, segment rotation, frame sidecar, and privacy hooks before depending on it.

Likely minimum target if we want direct HWND/HMONITOR capture without only using a picker: **Windows 10 1903 / build 18362**. Some quality/privacy helpers need newer versions: `WDA_EXCLUDEFROMCAPTURE` needs Windows 10 2004, and Application Loopback Audio needs Windows 10 build 20348.

## Windows 11 note

Windows 11 is the easiest first target for the Windows runtime capture backend. All recommended baseline APIs are present:

- WGC display/window capture and HWND/HMONITOR interop are available.
- WGC cursor capture, border controls, and newer capture-session helpers are available on normal Windows 11 builds.
- `WDA_EXCLUDEFROMCAPTURE` is available for hiding Mnema's own windows from capture.
- WASAPI microphone capture and system loopback are available.
- Application Loopback Audio's process-tree include/exclude API is available on Windows 11 builds because it requires build 20348+ and Windows 11 starts at build 22000.
- Media Foundation H.264/AAC encoding and MP4/M4A file writing are available.

Recommended rollout: **build and verify Windows 11 first**, then decide whether to keep Windows 10 support at 1903/2004/20348 with feature gates.

## Screen capture

### Primary: Windows Graphics Capture

Use WGC for `crates/capture-screen` on Windows.

Why:

- Microsoft positions `Windows.Graphics.Capture` for acquiring frames from a **display or application window**.
- It produces Direct3D 11 frames through `Direct3D11CaptureFramePool`.
- Frames include `SystemRelativeTime`, a QPC timestamp that can be used for media synchronization and frame-index sidecars.
- It has an explicit `GraphicsCaptureItem.Closed` event for liveness/stop handling.
- The system picker gives user consent and Windows draws a capture border/indicator around captured items.
- Win32 interop APIs can create capture items for an `HWND` or `HMONITOR` on Windows 10 1903+.

Implementation shape:

1. Create a D3D11 device.
2. Choose capture item:
   - user picker for first prototype / consent flow, or
   - `IGraphicsCaptureItemInterop::CreateForMonitor(HMONITOR, ...)` for monitor capture, or
   - `CreateForWindow(HWND, ...)` for selected-window capture.
3. Create `Direct3D11CaptureFramePool` with `DXGI_FORMAT_B8G8R8A8_UNORM` for SDR. Consider `R16G16B16A16_FLOAT` + tone mapping later for HDR displays.
4. On `FrameArrived`, pull frames off the frame pool on a non-UI thread.
5. Copy/convert frames for:
   - video encoder input,
   - JPEG frame artifacts,
   - captured-frame equivalence data,
   - screen activity samples.
6. Use frame `SystemRelativeTime` plus encoder sample timing to write the binary frame-index sidecar.

Open design points:

- **All screens vs one screen:** WGC captures a selected display/window item. For multiple monitors, decide whether Mnema records only primary, one selected monitor, one segment per monitor, or composites monitors into one video.
- **Window picker vs automatic monitor capture:** picker is clearer consent UX; direct HMONITOR capture better matches background recording.
- **HDR:** SDR capture can look washed out on HDR systems unless the whole pipeline supports HDR or tone maps.
- **Protected content:** expect protected/DRM content to be blank/blocked by the OS.

### Fallback: DXGI Desktop Duplication

DXGI Desktop Duplication is viable for monitor capture fallback, but not the first choice.

Pros:

- Windows 8+ desktop API.
- Gives a DXGI surface for frame-by-frame desktop updates.
- Provides dirty rects, move rects, pointer shape/position, and explicit access-lost errors.

Cons:

- Monitor/desktop oriented, not a modern picker/consent flow.
- More fragile around desktop switches, secure desktop, mode changes, full-screen transitions, RDP/session changes.
- Concurrent duplication is limited.
- Rotated displays and pointer composition require explicit handling.

Use it only if WGC is unavailable or if we need a no-picker desktop fallback for specific environments.

## Audio capture

### Microphone: WASAPI capture

Use WASAPI directly or through the `wasapi` crate.

Required behavior for Mnema:

- list devices and persist selected device identity,
- default-device tracking,
- reconnect/device-invalidated handling,
- event-driven shared-mode capture,
- PCM feed for VAD/inactivity,
- sample timestamps or stable capture-clock mapping for audio segment metadata.

A simpler CPAL prototype is possible, but WASAPI gives better access to Windows-specific loopback, device notification, and application-loopback behavior.

### System audio: WASAPI loopback

Use WASAPI loopback on the default or selected render endpoint:

- get an `IMMDevice` for `eRender`,
- initialize an `IAudioClient` capture stream in shared mode with `AUDCLNT_STREAMFLAGS_LOOPBACK`,
- read with `IAudioCaptureClient` just like microphone capture.

Important behavior:

- Loopback captures the system mix from the render endpoint.
- It does **not** require screen capture, unlike Mnema's current macOS ScreenCaptureKit coupling.
- Event-driven loopback is supported on Windows 10 1703+; older Windows needed a workaround.
- Protected audio can be blocked by DRM/trusted-driver rules.

### Optional later: Application Loopback Audio

Windows has an Application Loopback API using `ActivateAudioInterfaceAsync` with process-tree include/exclude semantics. It can capture only a target process tree or capture all audio except that process tree. This requires Windows 10 build 20348+.

Use cases:

- exclude Mnema's own app audio from system-audio capture,
- capture/exclude a specific app family later,
- improve privacy semantics for audio independently from screen capture.

Do not make this the baseline system-audio dependency unless we are willing to require build 20348+.

## Media writers, frame export, and previews

Focused follow-up: `docs/windows/media-writers-preview-research.md` maps the current macOS writer/preview code paths to Windows 11 replacements and open implementation decisions.

### Recommended native path: Media Foundation

Use Media Foundation in `crates/capture-writers` for Windows:

- `IMFSinkWriter` encodes uncompressed audio/video samples.
- It can host encoders and write encoded streams to a media sink/file.
- MPEG-4 file sink creates MP4 files and supports H.264/AVC video and AAC audio sample descriptions.
- Microsoft provides H.264 and AAC encoders through Media Foundation.

Suggested output formats:

- screen segments: `.mp4` with H.264 video; decide later whether to mux system audio or keep audio separate like macOS,
- microphone/system audio: `.m4a` AAC, or `.wav` only for early prototype/debug.

Mnema-specific requirements:

- segment finalization must validate the video is openable, not just that a file exists,
- frame-index sidecars should be derived from actual finalized video sample timing where possible,
- exact frame preview and scrub preview extraction need a Windows decoder path, likely Media Foundation Source Reader or FFmpeg if MF seeking is too much work,
- audio decode to mono PCM for Whisper/Parakeet/speaker analysis/system-audio speech activity needs a Windows decoder path.

### Alternative: FFmpeg

FFmpeg can reduce implementation time for encoding, trimming, decoding, and preview extraction, but adds packaging/licensing/update complexity. If chosen, keep it behind the same writer/decoder seams so macOS and Windows can share higher-level behavior.

## Privacy and permissions

Windows does not map 1:1 to macOS ScreenCaptureKit privacy.

- **Screen capture consent:** WGC picker gives explicit user choice and Windows draws capture borders. Direct interop capture needs careful UX because it bypasses the picker flow.
- **Arbitrary app exclusion:** no WGC/DXGI equivalent to ScreenCaptureKit app-exclusion filters for full-monitor capture. Do not promise live per-app screen exclusion on Windows until proven otherwise.
- **Own-window exclusion:** `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)` can hide Mnema's own top-level windows from capture on Windows 10 2004+, but the window must belong to the current process. This helps “exclude Mnema itself”, not “exclude any current app”.
- **Microphone:** use Windows microphone privacy UX and deep link such as `ms-settings:privacy-microphone`; desktop app behavior must be tested when privacy is disabled.
- **System audio:** ordinary WASAPI loopback has no macOS-style permission prompt.

Product implication: on Windows, `Exclude Current App` should be hidden/degraded unless it targets Mnema's own windows or a selected-window capture mode. Sensitive app protection should rely on clear disclosure, pause, and Delete Recent Capture unless a later ADR defines a Windows-specific alternative.

## Inactivity, metadata, and liveness

Windows equivalents to wire into the Recording Lifecycle:

- input idle: `GetLastInputInfo`,
- active app/window metadata: `GetForegroundWindow`, `GetWindowThreadProcessId`, process path/name, window title, optional AppUserModelID/package identity,
- sleep/display/session recovery: `WM_POWERBROADCAST`, `RegisterPowerSettingNotification`, `WTSRegisterSessionNotification`, display-change notifications,
- WGC liveness: `GraphicsCaptureItem.Closed`, device lost/frame-pool recreate,
- DXGI fallback liveness: `DXGI_ERROR_ACCESS_LOST`, `DXGI_ERROR_SESSION_DISCONNECTED`, `DXGI_ERROR_WAIT_TIMEOUT`,
- WASAPI liveness: device invalidation, default-device change notifications, silence/peak-since-last-poll preservation for inactivity.

## Rust crate survey

- `windows`: best production binding for WGC, Direct3D/DXGI, WASAPI, Media Foundation, User32, WTS, and power APIs.
- `windows-capture = 2.0.0`: high-level WGC/DXGI Rust crate with encoder examples. Good prototype/reference; verify low-level timing, segment rotation, sidecar, and error semantics before production dependency.
- `wasapi = 0.23.0`: safe Rust wrapper around WASAPI. Supports playback/capture, shared/exclusive, event/polled buffering, loopback capture, device notifications, and has `record_application` example using application loopback.
- `cpal = 0.17.3`: good cross-platform PCM stream abstraction. On Windows, WASAPI output devices can be used as input devices to enable loopback. Useful for simple mic prototypes, but lower control than `wasapi` for Mnema's Windows-specific needs.
- `scap = 0.1.0-beta.1`: cross-platform screen capture library using WGC on Windows. Interesting but beta; likely too high-level until we verify privacy/timing/segment requirements.
- `ffmpeg-next`: Rust FFmpeg wrapper. Useful if we accept FFmpeg packaging/licensing complexity.

## Suggested first tracer-bullet plan

1. Add Windows-only compile gates and CI before runtime work.
2. Prototype WGC monitor capture to JPEG frames with QPC timestamps; validate frame activity, resize, display sleep, lock/unlock, HDR, multi-monitor.
3. Prototype Media Foundation MP4 screen writer from captured frames; write a frame-index sidecar and validate exact preview extraction.
4. Prototype WASAPI microphone capture to PCM + VAD feed + `.m4a`/`.wav` writer.
5. Prototype WASAPI loopback capture independent of screen.
6. Integrate the three sources behind Recording Lifecycle adapters: start/stop, segment rotation, user pause/resume, inactivity pause/resume, liveness errors, sleep/wake/session recovery.
7. Add Windows UX gates for unsupported live app privacy and Windows-specific permission/settings links.

## Sources checked

- Microsoft Learn: Screen capture / `Windows.Graphics.Capture` — https://learn.microsoft.com/en-us/windows/uwp/audio-video-camera/screen-capture
- Microsoft Learn: `IGraphicsCaptureItemInterop::CreateForWindow` — https://learn.microsoft.com/en-us/windows/win32/api/windows.graphics.capture.interop/nf-windows-graphics-capture-interop-igraphicscaptureiteminterop-createforwindow
- Microsoft Learn: `IGraphicsCaptureItemInterop::CreateForMonitor` — https://learn.microsoft.com/en-us/windows/win32/api/windows.graphics.capture.interop/nf-windows-graphics-capture-interop-igraphicscaptureiteminterop-createformonitor
- Microsoft Learn: Desktop Duplication API — https://learn.microsoft.com/en-us/windows/win32/direct3ddxgi/desktop-dup-api
- Microsoft Learn: WASAPI Capturing a Stream — https://learn.microsoft.com/en-us/windows/win32/coreaudio/capturing-a-stream
- Microsoft Learn: WASAPI Loopback Recording — https://learn.microsoft.com/en-us/windows/win32/coreaudio/loopback-recording
- Microsoft Learn sample: Application Loopback Audio Capture — https://learn.microsoft.com/en-us/samples/microsoft/windows-classic-samples/applicationloopbackaudio-sample/
- Microsoft Learn: Media Foundation Sink Writer — https://learn.microsoft.com/en-us/windows/win32/medfound/sink-writer
- Microsoft Learn: MPEG-4 File Sink — https://learn.microsoft.com/en-us/windows/win32/medfound/mpeg-4-file-sink
- Microsoft Learn: H.264 Video Encoder — https://learn.microsoft.com/en-us/windows/win32/medfound/h-264-video-encoder
- Microsoft Learn: AAC Encoder — https://learn.microsoft.com/en-us/windows/win32/medfound/aac-encoder
- Microsoft Learn: `SetWindowDisplayAffinity` — https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowdisplayaffinity
- crates.io/docs.rs: `windows-capture`, `wasapi`, `cpal`, `scap`, `ffmpeg-next`
