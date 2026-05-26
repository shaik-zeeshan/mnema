# Linux Runtime Capture Research

_Last researched: 2026-05-26_

This note records the current recommendation for a future Linux native capture backend in Mnema. It is based on freedesktop.org / PipeWire / Wayland-protocols / systemd docs plus a Rust crate survey. It is scoped to **runtime capture** (screen, microphone, system audio, the capture lifecycle, idle/liveness, and the writers needed to finalize segments). Processing providers, storage key-stores, and release packaging are tracked separately in `SUPPORTS.md`.

## Short recommendation

Build the Linux backend on **PipeWire as the unifying substrate**, behind the existing capture seams:

- **Screen:** primary backend = **xdg-desktop-portal `org.freedesktop.portal.ScreenCast` + PipeWire**, with **X11 (XShm/XComposite)** kept only as a legacy-Xorg fallback. There is no silent "capture everything" path on Wayland — user consent through the portal picker is mandatory by design; the only way to avoid re-prompting is `persist_mode` + `restore_token`.
- **Microphone:** capture through **PipeWire** (directly, or via `cpal` with the PipeWire host) for raw PCM + VAD feed.
- **System audio:** capture a **PipeWire/PulseAudio monitor source**. Unlike macOS (where ScreenCaptureKit ties system audio to screen capture), on Linux system audio can be an **independent, un-prompted source** — closer to the Windows WASAPI-loopback model. The portal does **not** expose audio capture as of ScreenCast spec v5.
- **Encoding/finalization/preview:** prefer **GStreamer** (it ingests PipeWire screencast nodes natively via `pipewiresrc` and does VA-API hardware H.264). Keep **FFmpeg (`ffmpeg-next`)** as the alternative for frame-accurate seek/decode and one-shot trims. Either sits behind the `crates/capture-writers` seam.
- **Idle:** prefer the Wayland **`ext-idle-notify-v1`** protocol; fall back to **GNOME `org.gnome.Mutter.IdleMonitor`** / **`org.freedesktop.ScreenSaver` GetSessionIdleTime** D-Bus, then **X11 XScreenSaver** (`XScreenSaverQueryInfo`).
- **Sleep/wake/lock:** systemd-**logind** D-Bus (`PrepareForSleep`, session `Lock`/`Unlock`, delay inhibitor locks). On resume/unlock, treat the PipeWire screencast node as stale and renegotiate/reconnect using the `restore_token` rather than assuming the old node is live.

**Defining constraint:** unlike macOS/Windows, Linux capability is **uneven per compositor**. Screen capture and audio work broadly through portals/PipeWire, but **live per-app screen exclusion, reliable idle queries, and active-window metadata are X11-only or wlroots-only and are blocked/absent on GNOME and KDE Wayland**. The `SUPPORTS.md` Linux matrix should track capability per compositor, and the app should drive UI from `get_capture_support` / permission state rather than assuming a uniform Linux.

## What Mnema uses today, and the Linux replacement

This is the runtime-capture slice of the macOS-only source map in `SUPPORTS.md`. The shared serde/domain types in `crates/capture-types` are already platform-neutral (e.g. `CapturePermissionState::Unsupported`, `CaptureSupportResponse`, `CaptureSources`), and the capture crates already carry `#[cfg(not(target_os = "macos"))]` stubs, so Linux needs gated implementations behind the same public APIs rather than new seams.

| Capability | macOS today | Linux replacement |
| --- | --- | --- |
| Screen capture (`crates/capture-screen`) | ScreenCaptureKit (`cidre` `sc`), AVFoundation fallback | Portal ScreenCast + PipeWire; X11 XShm/XComposite fallback |
| System audio (`crates/capture-screen`) | ScreenCaptureKit, coupled to screen | PipeWire/PulseAudio monitor source, **decoupled** from screen |
| Microphone (`crates/capture-microphone`) | AVFoundation device list/change notifier/permission/VAD PCM | PipeWire (or `cpal`); graph events for hot-plug/default-change |
| Writers / encode (`crates/capture-writers`) | AVAssetWriter, VideoToolbox (`vt`), CoreMedia (`cm`), `afconvert`/`ffmpeg` trim | GStreamer (`pipewiresrc` + VA-API H.264) or FFmpeg behind the same seam |
| Frame export + exact/scrub preview (`capture-screen`, `app_infra/frame_preview.rs`) | AVAssetImageGenerator + ImageIO (`iio`) | GStreamer `appsink` seek or FFmpeg frame decode |
| Input idle (`native_capture_system_idle.rs`) | CoreGraphics `CGEventSourceSecondsSinceLastEventType` | `ext-idle-notify-v1`; Mutter/ScreenSaver D-Bus; X11 XScreenSaver |
| App/window metadata + browser URL (`native_capture_metadata.rs`) | NSWorkspace / CoreGraphics window list / AppleScript | wlr-foreign-toplevel (wlroots) or X11 EWMH; degraded/absent on GNOME/KDE Wayland; no portable browser-URL probe |
| Live app-exclusion privacy (`native_capture/privacy.rs`) | ScreenCaptureKit app-exclusion filters | **No portal/Wayland equivalent** for full-screen capture; treat as unsupported/degraded |
| Liveness + sleep/wake (`native_capture/lifecycle.rs`) | SCStreamDelegate stop errors + NSWorkspace sleep notifications | PipeWire stream state + logind `PrepareForSleep`/`Lock`/`Unlock` |
| Capture support / permission state | `support_for_current_platform()` → `ScreenCaptureSupport` | Same struct; report screen/system-audio support per detected portal/compositor |

## Screen capture

### Primary: xdg-desktop-portal ScreenCast + PipeWire (Wayland)

Use the portal for `crates/capture-screen` on Linux. Current spec is **ScreenCast version 5**.

D-Bus flow on `org.freedesktop.portal.ScreenCast`:

1. `CreateSession`.
2. `SelectSources` — set `types` bitmask (MONITOR=1, WINDOW=2, VIRTUAL=4), `multiple: true` for multi-output, `cursor_mode` (must be one of `AvailableCursorModes`), and `persist_mode` + any stored `restore_token`.
3. `Start` — shows the consent picker, then the Response carries the list of PipeWire streams as `(node_id: u32, props)`, plus a new `restore_token` if persistence was granted.
4. `OpenPipeWireRemote` — returns a **file descriptor**; pass it to `pw_context_connect_fd` to build a `pw_core`, then connect a `pw_stream` to each `node_id`. Only the consented nodes are visible on that remote.

Map this onto the existing `start_capture_session_with_options` / `rotate_screen_capture_session` / `stop_screen_capture_session` lifecycle. Each captured frame carries SPA buffer metadata with a timestamp usable for Mnema's binary frame-index sidecar (the analogue of CoreMedia sample timing).

Consent and persistence:

- `persist_mode`: `0` = none, `1` = transient (lives while the app runs), `2` = persistent. When granted, store the **new** `restore_token` returned by every `Start` — the token is **single-use and rotates each session**. If the stored output/window is gone or permission was withdrawn, the token is ignored and the user is reprompted (and there is a known issue where persistence does not always preserve the exact monitor selection — flatpak/xdg-desktop-portal#1371).
- `cursor_mode`: Hidden (default), Embedded (drawn into frames), or Metadata (delivered out-of-band).
- Multi-monitor: each selected output is a separate PipeWire node; decide whether Mnema records one selected output, one segment per output, or composites — same open design point flagged for Windows WGC.
- **Hard constraint:** there is no silent background "capture everything" path; the picker is unavoidable. Architect the start flow and UX around `restore_token`-based re-acquisition, not against the consent model.

### Fallback: X11 (XShm / XComposite)

Worth supporting only as a legacy-Xorg fallback (many setups still run Xorg or XWayland, and X11 capture needs **no permission prompt**). Use **XShm (MIT-SHM)** + `XShmGetImage` for performant full-screen grabs and **XComposite** for per-window capture. Plain `XGetImage` is slow; the XCB protocol path is dramatically faster than Xlib round-trips. X11 has **no app-exclusion** for full-screen capture (you can only capture specific windows via Composite).

## Audio capture

### Microphone: PipeWire (or cpal)

Go through PipeWire (the default substrate on modern distros; it proxies ALSA/Pulse). For portability use `cpal` with the PipeWire/PulseAudio host; for graph-level control use `pipewire-rs` directly.

Required behavior to match the macOS microphone seam:

- device enumeration and persisted selected-device identity,
- default-device tracking,
- hot-plug / device-invalidated reconnect — `cpal` does **not** emit hot-plug or default-change events, so watch PipeWire registry/node events (or libpulse `subscribe`) for these,
- raw interleaved PCM in the stream callback for the VAD / inactivity feed (`take_microphone_vad_pcm_frames`),
- peak-since-last-poll preservation for the coarse (1s) inactivity poll, as the lifecycle already requires.

Build gotchas: on Linux `cpal` always needs ALSA dev headers (`libasound2-dev`) even with the PipeWire/Pulse features, and a raw ALSA-host stream can fail with `DeviceBusy` if PipeWire/Pulse holds the device — prefer the PipeWire/Pulse path.

### System audio: monitor source (decoupled from screen)

Every PipeWire/PulseAudio output device exposes a **monitor source** (e.g. `*.monitor`) that yields the full system mix with **no permission dialog**. Capture that node directly (or via a `module-loopback` virtual sink).

Key differences from macOS:

- System audio is **independent of screen capture** on Linux, so Mnema can drop the macOS ScreenCaptureKit coupling here (matches the Windows WASAPI-loopback decision).
- The portal does **not** expose audio capture — the ScreenCast spec (through v5) negotiates **video only**; audio-in-ScreenCast remains an open, unstandardized discussion (flatpak/xdg-desktop-portal#957, discussion #1142). So system audio is effectively captured **un-prompted** via monitor sources today.

Tradeoff to record as a decision point: silent monitor capture is simple but has no consent UX and no per-app isolation. Abstract the system-audio source so a future portal-audio (consent-tied) path can slot in without reworking the lifecycle.

## Media writers, frame export, and previews

Pick one stack behind `crates/capture-writers` and `app_infra/frame_preview.rs`, mirroring the AVAssetWriter/VideoToolbox + AVAssetImageGenerator roles.

### Recommended: GStreamer

Best fit for a PipeWire pipeline: the `pipewiresrc` element ingests portal screencast nodes directly into an encode pipeline (`pipewiresrc ! videoconvert ! vah264enc/x264enc ! mp4mux`), with VA-API hardware H.264. Exact-frame preview via seek + `appsink`; audio decode to mono PCM via `decodebin ! audioconvert ! audioresample ! capsfilter(channels=1) ! appsink`. This is the idiomatic Linux/PipeWire path and keeps screen frames in the GPU pipeline.

### Alternative: FFmpeg

`ffmpeg-next` (libav*) is lower-level and maps closely to how the macOS writer/seeking code is structured — good for frame-accurate seek/decode and one-shot trims (the `afconvert`/`ffmpeg` trim analogue). It has no PipeWire source, so you feed it frames already pulled from PipeWire.

Mnema-specific requirements carry over from macOS: segment finalization must validate the output is openable (not just that a file exists — the `.mov` `moov` check analogue), the frame-index sidecar should derive from real finalized sample timing, and exact-vs-scrub preview tiers stay distinct.

Packaging/licensing: GStreamer core is LGPL, but `x264enc` lives in GPL `gst-plugins-ugly`; FFmpeg with `libx264` is GPL. For a proprietary app, prefer **VA-API hardware H.264** (avoids x264's GPL) or ship LGPL builds with dynamic linking, and document codec licensing. Decide whether Linux output stays `.mp4`/`.m4a` or keeps the macOS `.mov`/`.m4a` convention.

## Idle, metadata, privacy, and liveness

### Input idle / inactivity

There is no universal cross-compositor "seconds since last input" query on Wayland; most options are event-based.

- **Primary (Wayland):** `ext-idle-notify-v1` (stabilized in wayland-protocols 1.27). Create a notification with a timeout and track `idled`/`resumed` edges (the input-only variant ignores idle-inhibitors). Supported by wlroots compositors and KDE; **GNOME/Mutter lags**.
- **GNOME fallback:** `org.gnome.Mutter.IdleMonitor` `GetIdletime` (ms) — but restricted to extensions/unsafe-mode on many GNOME 41+ builds.
- **Generic D-Bus:** `org.freedesktop.ScreenSaver` `GetSessionIdleTime` (KDE implements; coverage varies).
- **X11/XWayland:** `XScreenSaverQueryInfo` returns idle ms — the reliable Xorg path and the closest analogue to today's CoreGraphics idle. `native_capture_system_idle.rs::current_system_idle_ms()` already returns `None` off macOS; wire the chain there: ext-idle-notify → Mutter/ScreenSaver → XScreenSaver.

### Active app / window metadata

Wayland **deliberately withholds** foreground-window / title / app_id from other apps; there is no portal for it.

- **wlroots compositors:** `wlr-foreign-toplevel-management-unstable-v1` exposes `title`, `app_id`, and `activated` state to clients (sway/Hyprland/etc.). **KWin does not implement it** (KDE bug 502647) and **GNOME does not** either.
- **GNOME:** `org.gnome.Shell.Eval` is disabled by default; you'd ship a Shell extension.
- **KDE/KWin:** scripting is restricted to registered scripts.
- **X11/XWayland:** EWMH works — read `_NET_ACTIVE_WINDOW`, then `_NET_WM_NAME` / `WM_CLASS` / `_NET_WM_PID`.
- **App identity:** map to the `.desktop` file id / Wayland `app_id` (e.g. `org.mozilla.firefox`) as the bundle-id analogue. Browser URL (the macOS AppleScript path) has **no portable Wayland equivalent**; do not add per-browser plumbing without an ADR.

Conclusion: rich active-window metadata is X11-only or wlroots-only; on GNOME/KDE Wayland plan for degraded/absent foreground metadata.

### Live app-exclusion privacy

There is **no portal/Wayland equivalent** of ScreenCaptureKit app-exclusion filters for full-screen capture, and X11 cannot exclude apps from a full grab either. Per ADR 0006 / ADR 0008, do not silently downgrade privacy: expose Linux live per-app screen exclusion as an explicit **unsupported/degraded** capability and rely on App Privacy disclosure, User Capture Pause, and confirmed Delete Recent Capture — the same posture the Windows research reached. Window-mode capture (capturing only a chosen window) is the nearest in-consent mitigation.

### Liveness, sleep/wake, and session lock

- **Liveness:** track PipeWire stream state/errors and the portal session close as the analogue of SCStreamDelegate stop errors; on a dead node, drop only the live screen/system-audio state while preserving microphone continuation, exactly as the lifecycle already reconciles macOS signals.
- **Sleep/wake/lock:** subscribe on the system bus to `org.freedesktop.login1.Manager` `PrepareForSleep(bool)` (act on the `false` resume edge) and per-session `Lock`/`Unlock` signals. Use a **delay inhibitor lock** (`Inhibit("sleep", ...)`) to finalize the active segment before suspend, then release the fd — the NSWorkspaceWillSleep analogue.
- **PipeWire across suspend/lock:** a screencast node does **not** reliably survive suspend/resume or session-lock (nodes get paused/suspended and the compositor stops feeding frames while locked/blanked). On resume/unlock, finalize the interrupted segment and **renegotiate/reconnect** via `restore_token` rather than assuming the old node is live. Preserve the stale screen/system-audio segment paths in `current_segment_output_files` across the gap, as wake recovery already requires.

## Rust crate survey (current, 2026)

- **`ashpd` 0.13.x** (0.13.11) — safe high-level XDG portal wrapper over `zbus`; covers `desktop::screencast` (CreateSession/SelectSources/Start, `PersistMode`, `CursorMode`, `SourceType`, `restore_token()`, `open_pipe_wire_remote()`) and `desktop::remote_desktop`. Runtime-agnostic (tokio/async-std).
- **`pipewire` 0.9.2** (`pipewire-sys` 0.9.2) — official freedesktop pipewire-rs bindings; connect the portal fd, attach `pw_stream`s to nodes, read graph/registry events for hot-plug/default-change.
- **`libspa` / `libspa-sys` 0.8.0** — SPA buffer/format/POD handling for stream data and timestamps.
- **`cpal` 0.17.3** — cross-platform host abstraction (ALSA/PipeWire/Pulse/JACK) for a simpler mic prototype; needs ALSA dev headers; no hot-plug events.
- **`libpulse-binding` 2.30.1** (+ `libpulse-simple-binding` / `psimple`) — PulseAudio path for mic and monitor-source system audio.
- **`gstreamer` 0.25.2** (+ `gstreamer-app` 0.23.5, `gstreamer-video`, `gstreamer-audio`) — encode/decode/seek pipelines with native `pipewiresrc` ingest and VA-API encoders.
- **`ffmpeg-next`** (latest, FFmpeg 4+ compatible; actively maintained) — alternative frame-accurate decode/seek/trim; `video-rs` is a higher-level option on top.
- **`zbus` 5.12.x** — pure-Rust D-Bus for logind sleep/lock signals and Mutter/ScreenSaver idle; `zbus_systemd` (`login1`) offers ready-made proxies.
- **`x11rb`** (pure-Rust XCB) — X11 fallback: `shm`/`composite`/`damage` for capture, `screensaver` feature for `XScreenSaverQueryInfo`, EWMH for active-window metadata. `xcap` wraps both X11 and Wayland capture if a single abstraction is preferred.
- **`wayland-client` + `wayland-protocols`** — bindings for `ext-idle-notify-v1` and `wlr-foreign-toplevel-management` where no higher-level crate exists.

## Suggested first tracer-bullet plan

1. Add a Linux compile gate + CI (`cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml`, `cargo check --workspace`, `bun run check`) and audit the `#[cfg(not(target_os = "macos"))]` stubs so Linux builds cleanly.
2. Prototype portal ScreenCast → PipeWire monitor capture to JPEG frames with SPA timestamps; validate the picker, `restore_token` re-acquisition, multi-monitor, lock/unlock, and suspend/resume node behavior.
3. Prototype a GStreamer `pipewiresrc → vah264enc → mp4mux` screen writer; write a frame-index sidecar and validate exact preview extraction via `appsink` seek.
4. Prototype PipeWire microphone capture to PCM + VAD feed + audio writer.
5. Prototype monitor-source system-audio capture, independent of screen.
6. Integrate the three sources behind the Recording Lifecycle adapters: start/stop, segment rotation, user pause/resume, inactivity pause/resume, PipeWire liveness errors, and logind sleep/wake/lock recovery.
7. Add Linux capability/UX gates: report screen/system-audio support and permission state through `get_capture_support`, mark live per-app screen exclusion unsupported/degraded, and choose the idle/metadata chain per detected compositor.

## Sources checked

- freedesktop: `org.freedesktop.portal.ScreenCast` — https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.ScreenCast.html
- ScreenCast interface XML — https://github.com/flatpak/xdg-desktop-portal/blob/main/data/org.freedesktop.portal.ScreenCast.xml
- ScreenCast persist/monitor-selection quirk — https://github.com/flatpak/xdg-desktop-portal/issues/1371
- Audio-in-ScreenCast (not standardized) — https://github.com/flatpak/xdg-desktop-portal/issues/957 and discussion https://github.com/flatpak/xdg-desktop-portal/discussions/1142
- ashpd ScreenCast — https://docs.rs/ashpd/latest/ashpd/desktop/screencast/index.html
- pipewire-rs — https://pipewire.pages.freedesktop.org/pipewire-rs/
- PipeWire `module-loopback` — https://docs.pipewire.org/page_module_loopback.html
- PipeWire suspend/resume behavior — https://wiki.archlinux.org/title/PipeWire
- `ext-idle-notify-v1` — https://wayland.app/protocols/ext-idle-notify-v1 ; wayland-protocols 1.27 — https://www.phoronix.com/news/Wayland-Protocols-1.27
- `wlr-foreign-toplevel-management-unstable-v1` — https://wayland.app/protocols/wlr-foreign-toplevel-management-unstable-v1 ; KWin support bug — https://bugs.kde.org/show_bug.cgi?id=502647
- systemd-logind D-Bus — https://www.freedesktop.org/software/systemd/man/252/org.freedesktop.login1.html ; inhibitor locks — https://systemd.io/INHIBITOR_LOCKS/
- x11rb — https://github.com/psychon/x11rb ; xcap — https://github.com/nashaofu/xcap
- gstreamer-rs — https://gitlab.freedesktop.org/gstreamer/gstreamer-rs ; ffmpeg-next — https://docs.rs/ffmpeg-next
- zbus — https://docs.rs/zbus/latest/zbus/ ; zbus_systemd login1 — https://docs.rs/zbus_systemd/latest/zbus_systemd/login1/index.html
- crates.io/docs.rs: `ashpd`, `pipewire`, `libspa`, `cpal`, `libpulse-binding`, `gstreamer`, `gstreamer-app`, `ffmpeg-next`, `zbus`, `x11rb`
