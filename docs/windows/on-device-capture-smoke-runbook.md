# Windows On-Device Capture Smoke Runbook

_Last updated: 2026-06-08 (GitHub issue #84)_

This is the operator runbook for the human-in-the-loop (HITL) Windows capture
smoke pass. An operator must physically drive the machine — lock it (Win+L),
sleep/wake it, and idle/resume it — because no automation can synthesize a real
workstation lock, system suspend, or user-idle transition. The harnesses below
drive a **real** native capture, watch the runtime bookkeeping, and now also
assert the two new capture invariants this milestone introduced (#73, #74) so
the operator does not have to inspect artifacts by hand.

The green pass itself is **operator-deferred**: it is recorded here, consistent
with the project's deliberate on-device smoke deferral. Nothing in CI runs these
scenarios.

## Scope

In scope (three scenarios):

1. **Session lock** (#63) — Win+L, then unlock.
2. **System suspend** (#64) — sleep, then wake.
3. **Inactivity** (#62/#74) — idle past the threshold, then resume on input.

Explicitly **out of scope**: the *display-unavailable* scenario (monitor
disconnect/reconnect). It is tracked separately as **#78** — this machine has no
detachable display, so it cannot be exercised here. The transient-liveness
harness still accepts `--display-unavailable` for operators who do have a
detachable display, but it is not part of this pass.

## Auto-checked invariants

Both harnesses verify, after they stop and finalize, the milestone invariants:

- **#73 — Monotonic frame-index sidecar.** Every finalized screen segment
  (`*.mp4`) must have a sibling `*.frame-index.bin` sidecar that decodes and
  whose `video_offset_ms` values are non-decreasing. Implemented in
  `apps/desktop/src-tauri/src/native_capture/windows_smoke_invariants.rs`
  (`assert_all_screen_segments_have_monotonic_sidecars`), reusing
  `capture_screen::screen_segment_frame_index_path`,
  `decode_screen_segment_frame_index`, and
  `screen_segment_frame_index_offsets_are_monotonic`.
- **#74 — Inactivity tail hold-back.** A capture stopped *while
  inactivity-paused* must commit `.m4a` audio measurably **shorter** than the
  wall-clock capture window (the idle tail is discarded). A capture stopped
  **normally** must **not** be meaningfully shorter (the tail drains).
  Implemented as `assert_audio_tail_holdback` in the same module, reading
  committed `.m4a` duration through the Media Foundation `media-decode` seam
  (`media_decode::decode_to_mono_f32`; duration = samples / sample_rate).

If an invariant fails, the harness exits non-zero and prints which segment /
audio file failed and why.

## Prerequisites

- Build from **PowerShell** (not Bash) with the project build-env helper.
- A working screen + microphone (and, for lock/suspend, system audio) so the
  audio-continuity assertions have real audio to commit. Keep audio playing /
  speak into the mic during the run so committed files are non-empty.
- Run each command, watch stdout for the `ACTION REQUIRED` prompts, and perform
  the physical action within the printed timeout.

## Scenario 1 — Session lock (#63)

```powershell
cargo run --manifest-path apps/desktop/src-tauri/Cargo.toml -- --windows-transient-liveness-smoke --session-lock
```

Steps when prompted:
1. Confirm screen + microphone + system audio are recording.
2. Press **Win+L**.
3. Unlock and return to the desktop before the reconnect timeout.

Pass criteria (auto-checked):
- Screen enters `TransientLiveness { SessionLock }` while mic + system audio
  keep recording (audio continuity, stable source-session ids).
- Screen auto-resumes into a **fresh segment** (segment index advances).
- Every finalized screen segment has a monotonic frame-index sidecar (#73).
- This is a normal stop, so committed audio is **not** shorter than wall-clock.

## Scenario 2 — System suspend (#64)

```powershell
cargo run --manifest-path apps/desktop/src-tauri/Cargo.toml -- --windows-transient-liveness-smoke --system-suspend
```

Steps when prompted:
1. Put Windows to **sleep**.
2. **Wake** it manually before the timeout.
3. Keep the desktop unlocked and capture running.

Pass criteria (auto-checked):
- All requested families enter `TransientLiveness { SystemSuspend }` without
  ending the session.
- All families restart after wake; screen resumes into a fresh segment.
- Every finalized screen segment has a monotonic frame-index sidecar (#73).
- Normal stop: committed audio is not shorter than wall-clock.

## Scenario 3 — Inactivity (#62/#74)

Run it **twice** to cover both halves of acceptance criterion #5.

### 3a. Resume + normal stop (segment rotation + tail drains)

```powershell
cargo run --manifest-path apps/desktop/src-tauri/Cargo.toml -- --windows-inactivity-smoke
```

Steps: stop touching the keyboard/mouse until the harness reports the pause; the
harness then synthesizes a mouse move to resume. Pass criteria:
- Screen + audio pause on idle, resume on input, **segment index advances**,
  source-session ids preserved.
- Every finalized screen segment has a monotonic frame-index sidecar (#73).
- **Normal stop**: committed `.m4a` is **not** shorter than wall-clock (#74).

### 3b. Inactivity stop (tail held back)

```powershell
cargo run --manifest-path apps/desktop/src-tauri/Cargo.toml -- --windows-inactivity-smoke --stop-while-paused
```

Steps: stop touching the keyboard/mouse until the harness reports the pause; the
harness stops *during* the pause. Pass criteria:
- Inactivity pause observed.
- **Inactivity stop**: committed `.m4a` is **shorter** than wall-clock by more
  than `--audio-tail-tolerance-seconds` (default 1s) (#74).

Useful options: `--idle-timeout-seconds`, `--max-idle-wait-seconds`,
`--max-state-wait-seconds`, `--audio-tail-tolerance-seconds`, `--save-directory`.

## Scenario 4 — Audio-endpoint hotplug support gate

Regression check for the stale support-gate bug (mic connected after launch
stayed unusable until restart because the support probe was latched at first
call).

```powershell
cargo run -p capture-microphone --example smoke_support_hotplug
```

Steps: start the harness with the microphone unplugged (or disabled in
Settings -> System -> Sound), then plug it back in while the loop is running.
Pass criteria:
- Initial line reports `microphone supported = false`, permission `Unsupported`.
- Within ~2s of plugging the mic in, a `CHANGE` line flips to
  `microphone supported = true`, permission `Unknown` — **inside the same
  process, no restart**.
- Optionally unplug again and confirm it flips back to `false`.

Full-app variant: launch the desktop app with no mic, plug one in, and start a
recording with the microphone source enabled — it must start without an app
restart, and the tray Sources menu must re-enable Microphone after the plug-in.

## Recording results

For each run, capture: the command, the final `PASS`/`FAIL` line, the printed
output directory, and (on failure) the failing assertion. Suggested table:

| Scenario | Command | Result | Output dir | Notes / follow-up |
| --- | --- | --- | --- | --- |
| Session lock | `--windows-transient-liveness-smoke --session-lock` | PASS / FAIL | | |
| System suspend | `--windows-transient-liveness-smoke --system-suspend` | PASS / FAIL | | |
| Inactivity (normal stop) | `--windows-inactivity-smoke` | PASS / FAIL | | |
| Inactivity (stop while paused) | `--windows-inactivity-smoke --stop-while-paused` | PASS / FAIL | | |

File any failure as a follow-up issue, referencing the scenario, the failing
assertion text, and the output directory. The display-unavailable scenario stays
deferred under #78 until a detachable display is available.
