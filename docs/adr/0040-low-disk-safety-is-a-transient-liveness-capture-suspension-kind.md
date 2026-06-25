# Low-disk safety is a transient-liveness Capture Suspension kind

## Status

Accepted.

## Context

The capture pipeline had no disk-space awareness anywhere (issue #130). Recording could start on a nearly-full volume; there was no mid-recording low-disk handling; and when the disk filled mid-segment the AVFoundation writer failed its writes and left a corrupt/partial `.mov`/`.m4a` at its **final** path (there is no temp-file-then-atomic-rename). Retention is time-based only (`Never`/7/14/30 days), so nothing bounds total disk usage.

This is platform-neutral work implemented macOS-first; the Windows branch picks it up after merging `main` (the same design covers the Windows Media-Foundation file-lock amplification, because we discard partials rather than leave a locked corrupt file behind).

Two existing "stop writing for a while" mechanisms were candidates to build on: the per-family **inactivity pause** (`InactivityState`, idle-driven) and the **transient-liveness suspension** introduced by [ADR 0021](0021-recover-from-display-unavailable-as-transient-liveness.md) (`PrivacyCaptureSuspension` + `CaptureSuspensionKind`, an environmental "can't capture right now, keep the session alive and auto-recover" condition that the segment loop already owns a throttled recovery driver for).

## Decision

Low disk is modeled as a transient-liveness **Capture Suspension**, not an inactivity pause:

- **Rename + new kind.** `PrivacyCaptureSuspension` → `CaptureSuspension` and the runtime field `privacy_capture_suspension` → `capture_suspension` (internal `pub(crate)`, macOS-only, ~21 refs, no wire/serde/frontend impact). Add a third `CaptureSuspensionKind::LowDisk`.
- **All-source scope.** `LowDisk` suspends **screen, system audio, and the microphone** — unlike the existing kinds, which keep the mic alive — because every source writes to the same recordings volume. Recovery therefore restarts the mic session too.
- **Check at new-file boundaries, not a 1s poll.** Free space is checked exactly when a new segment file is about to be opened: the **preflight** is this check applied to the first segment (refuse to start), and each **rotation** is the same check applied to the next segment (suspend instead of opening). No continuous healthy-path polling.
- **Settings-derived thresholds (fixed coefficients, no user setting).** `next_segment_estimate = (effective_screen_bitrate_bps / 8 + audio_bytes_per_sec) × segment_duration_seconds`. Pause/preflight when `free < RESERVE_FLOOR + next_segment_estimate`; resume when `free ≥ RESERVE_FLOOR + 2 × next_segment_estimate` (hysteresis). `RESERVE_FLOOR = 1 GiB`. This is correct across the 5 MB (audio-only) → ~4.5 GB (120 Mbps × 5 min) per-segment range; a flat byte constant is not.
- **Recovery while suspended.** The existing segment-loop recovery driver gains a `LowDisk` branch: it re-probes `fs2::available_space` every ~10 s (disk recovers far slower than a display waking) and auto-resumes across all suspended sources once free is back above the resume threshold. `LowDisk`, like `DisplayUnavailable`, never escalates to a manual restart.
- **Mid-segment disk-full → no corrupt segment.** If a boundary check passed but the disk fills mid-segment (another process), the writer fails (`ENOSPC`); we detect it, **best-effort delete the partial file at its final path and commit no Capture Segment row**, then drop into the same `LowDisk` suspension. This is how "no corrupt segment" is achieved without atomic-rename.
- **Graceful stop is spatial, not timed.** Below the pause threshold but at or above the reserve floor, the session waits indefinitely and auto-resumes. If free space drops **below the reserve floor itself (< 1 GiB)** — the app's own SQLite DB / OCR / OS are now at risk — the session **stops gracefully**: clean-finalize what is safely closable, end the session, and surface a "recording stopped — disk full" error. "Cannot recover" is expressed as "can no longer protect our own storage," not "waited N minutes."
- **Surfacing.** Add `is_low_disk_suspended: bool` to `NativeCaptureSession` so the tray/dashboard read **"Paused — low disk"** instead of lying about recording. Notify on suspend (`AppNotification`, severity `warning`, cleared on resume), unlike the silent display-unavailable case, because low disk only heals if the user frees space. Preflight refusal returns `CaptureErrorResponse { code: "insufficient_disk_space", … }`. Graceful stop notifies with severity `error`.

## Considered Options

- **Reuse the inactivity-pause mechanism** (the issue's suggestion). Rejected: inactivity is idleness (idle-timeout coupling, per-family tail-trim, activity sensitivity); low disk is an environmental can't-write condition. The ADR 0021 transient-liveness suspension is the right precedent and already has a recovery driver.
- **A flat byte constant threshold** (e.g. pause < 2 GiB, resume > 3 GiB). Rejected: a single segment ranges 5 MB → ~4.5 GB, so a flat 2 GiB greenlights a 4.5 GB segment and fills the disk mid-segment *every* segment for high-res/high-bitrate users. Reserving `floor + one-segment estimate` is correct across the range and makes the mid-segment backstop genuinely rare.
- **A continuous 1 s `statvfs` poll during healthy capture.** Rejected: wasteful; the natural seam is "before opening the next file." The probe runs only in the degraded/suspended state.
- **Keep the `PrivacyCaptureSuspension` name** (as ADR 0021 chose, to avoid a wide rename). Rejected now: the name was already documented-misleading after `DisplayUnavailable`, and `LowDisk` is plainly not privacy *and* breaks the "mic stays alive" assumption; the rename is internal-only and small.
- **Timer-based give-up** (suspended > N minutes → stop) or **wait-forever with no stop**. Rejected: a timer needs an arbitrary constant and quits even while the app is healthy and space is stable; wait-forever ignores that a genuinely full disk degrades the whole app and the issue asks for a graceful stop. The spatial reserve-floor trigger needs no new magic number.
- **Temp-file-then-atomic-rename (item d)** and a **storage-capacity cap (item e)**. Deferred — out of scope for this change. "No corrupt segment" is met by discarding the partial on mid-write failure; the storage cap remains a separate follow-up to complement time-based retention.

## Consequences

- The recovery driver must restart the microphone session for `LowDisk`, since it is the only kind that stops the mic.
- `capture_suspension` is a single `Option` slot, so it holds one kind at a time; if display-unavailable and low-disk coincide, the low-disk critical-floor stop takes precedence (app-storage safety wins over a display that may still be asleep).
- A small free-space seam (`fs2::available_space`, already a desktop dependency) is added to the lifecycle; the probe is best-effort (an inability to *measure* never blocks capture — only a measured shortfall acts), mirroring the existing model-download disk preflight.
