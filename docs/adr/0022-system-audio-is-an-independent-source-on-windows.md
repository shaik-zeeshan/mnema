# System audio is an independent source on Windows

## Status

Accepted.

## Context

On macOS, Mnema captures system audio through ScreenCaptureKit, which delivers it alongside the screen stream — so system audio physically cannot exist without screen capture. That platform reality was encoded as a cross-platform domain invariant: "system audio requires screen capture." It is enforced in several places that are **not** macOS-gated:

- domain validation in `apps/desktop/src-tauri/src/native_capture_settings.rs` (rejects `capture_system_audio && !capture_screen`),
- the settings page and onboarding UI (`apps/desktop/src/routes/settings/+page.svelte`, `onboarding/+page.svelte`),
- the native status-bar tray (`apps/desktop/src-tauri/src/status_bar.rs` disables the system-audio toggle and clears it when screen is unchecked),
- and tests asserting all of the above.

On Windows, system audio is captured with WASAPI loopback from the default render endpoint. Loopback reads the render mix directly and has no dependency on screen capture. Forcing the macOS coupling onto Windows would mean a user who wants only "record what I hear" must also run full WGC screen capture (CPU, GPU, disk, and a capture border), purely to satisfy an invariant that does not apply to the platform.

## Decision

Whether system audio requires screen capture is modeled as a **platform capability**, not a hardcoded rule.

- `CaptureSupportResponse` gains a `system_audio_requires_screen` flag (macOS = `true`, Windows = `false`), surfaced through `get_capture_support`.
- Validation, settings UI, onboarding, and the status-bar tray read that capability instead of hardcoding "system audio requires screen." On macOS the behavior is unchanged; on Windows system audio can be selected and recorded on its own.
- In the Recording Lifecycle, Windows system audio is an independent native audio session (`active_system_audio_session: Box<dyn AudioCaptureSession>`), parallel to the microphone session and decoupled from `active_screen_session`. macOS keeps routing system audio through the screen/ScreenCaptureKit backend.

This follows the cross-platform principle in `CONTEXT-MAP.md`: prefer capability-driven UI over direct platform checks in shared validation and Svelte.

## Alternatives Rejected

- **Keep system audio coupled to screen on Windows too.** Zero invariant changes and a single mental model, but it forces redundant screen capture for audio-only recording and contradicts the WASAPI loopback reality. The wasted capture cost and worse UX outweigh the simplicity.
- **Fork the invariant with `#[cfg(windows)]` branches.** Avoids touching the shared validation type, but scatters platform `cfg`/checks through settings, onboarding, status-bar, and validation — exactly what the capability-driven-UI principle exists to prevent. A single capability flag keeps the platform knowledge in `get_capture_support`.

## Consequences

Windows users can record system audio without screen capture; macOS behavior is unchanged. The "system audio requires screen" relationship is no longer a fixed rule anywhere in the stack — it is read from `system_audio_requires_screen`, so any new surface that gates the two sources must consult the capability rather than assume coupling. The Recording Lifecycle carries a distinct `active_system_audio_session` on Windows, which segment rotation, liveness, pause/resume, and inactivity handling must treat as its own source family rather than a rider on the screen session.
