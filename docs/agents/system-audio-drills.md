# System Audio — Manual Drills

The hardware-in-the-loop half of [ADR 0052](../adr/0052-system-audio-is-an-independent-capture-family-on-core-audio-process-taps.md)'s verification. The watchdog/backoff state machine, the exclude-list diff, and the permission heuristic are unit-tested (`cargo test -p capture-system-audio`); everything below needs a real Mac, real speakers, and a human. Nothing here is automated, and none of it has been run by an agent.

**Drill 1 (privacy parity) is a merge blocker.** The rest gate the release together with the soak in the last section.

---

## Setup (once)

1. **Turn on verbose logging.** Settings → About → *Developer & Logs* → **Enable developer options**. Nearly every `system-audio-tap:` line — including all the ones below — is Debug-level, so a release build logs none of them without this. A `bun run tauri -- dev` build always logs verbosely.
2. **Know your log.** `rust.log` lives at `~/Library/Logs/<bundle-id>/rust.log` — `com.shaikzeeshan.mnema` for a packaged build, `com.shaikzeeshan.mnema.dev` for a dev build. **Its timestamps are UTC**, so add your offset before correlating with anything you observed on the clock (IST: +5:30).
3. **Know your grep.** One prefix covers every tap event: `capture_system_audio::LOG_PREFIX` = `system-audio-tap:`.
   ```sh
   tail -f ~/Library/Logs/com.shaikzeeshan.mnema/rust.log | grep 'system-audio-tap:'
   ```
4. **Know your output.** System-audio segments land at
   `~/.mnema/recordings/YYYY/MM/DD/audio/sysaudio_<uuid>-segment-####.m4a`
   (a mid-segment rebuild adds `-<unix-ms>`; `mic_…` files are the microphone's and are not part of these drills). Adjust the root if you moved the save directory.
5. **Two handy checks** on any produced `.m4a`:
   ```sh
   afinfo <file.m4a>                                  # sample rate + duration
   ffmpeg -hide_banner -i <file.m4a> -af volumedetect -f null - 2>&1 | grep max_volume
   ```
   `max_volume: -91.0 dB` (or lower) is digital silence. Anything around `-20 dB` is audible content.

---

## Drill 1 — Privacy-exclusion parity (**merge blocker**)

The one coupling that was easy to lose: ScreenCaptureKit's content filter silenced privacy-listed apps' **audio** as well as hiding their windows. The tap must do the same via its exclude list. A regression here silently records apps the user believes are excluded, and nothing in the app will say so.

Compare against the Slice 1 baseline recorded under SCK before the swap. If that baseline is missing, this drill has no bar to clear — re-run it on the pre-swap build first.

1. Add a browser to the privacy list (Settings → Capture → *Privacy* → **Excluded Apps**). Chrome is the interesting one: its audio comes from a **helper process**, not the parent, and the exclude list resolves by bundle id.
2. Start a recording with system audio on.
3. Play something loud and unmistakable in the excluded browser (a music video). Let it run ~30 s.
4. Play something equally loud from a **non-excluded** app (Music, QuickTime). Let it run ~30 s.
5. Stop the recording.

**Expected:**
- The excluded browser's audio is **absent** — the segment covering step 3 is silent (`max_volume` at the noise floor).
- The non-excluded app's audio **is** present in the segment covering step 4. (This half matters as much: a tap that records nothing also "passes" step 3.)
- Behaviour matches the SCK baseline exactly, including the helper-process case.

**Log:**
```sh
grep 'system-audio-tap: started tap generation' ~/Library/Logs/com.shaikzeeshan.mnema/rust.log
```
`excluding N process object(s)` — `N` must cover Mnema's own process **plus** the excluded browser's audio processes (so `N ≥ 2` with one excluded browser playing audio).

**Also drill the live edit:** with a recording running, add the browser to the privacy list *mid-recording*. Expect `exclude list moved, rebuild needed` followed by `rebuilding tap generation (reason=exclude_list_moved, ...)`, and the browser silent from that point on.

---

## Drill 2 — Output device switch mid-recording

Exercises both the device-change listener and the tap-follows-the-format rule (built-in speakers are typically 44.1 kHz, AirPods 48 kHz).

1. Start a recording with system audio on, playing audio through the **built-in speakers**.
2. Mid-segment, switch the system output to **AirPods** (or any other output device).
3. Keep audio playing for ~30 s. Stop.

**Expected:** two valid `.m4a` files for that segment index (the second carrying a `-<unix-ms>` suffix), both openable, **at different sample rates**, with no lost audio beyond the rebuild instant. Verify with `afinfo` on both.

**Log:**
```
system-audio-tap: rebuilding tap generation (reason=default_output_device_changed, rebuild=1)
system-audio-tap: started tap generation: 48000 Hz, 2 ch, excluding N process object(s)
system-audio-tap: sample_format_stabilized observed=<n> streak=<n> sample_rate_hz=48000 channels=2
```

---

## Drill 3 — Bluetooth disconnect

The device-death path, and the one most likely to produce a rebuild *storm*.

1. Start recording with system audio on, output going to AirPods.
2. Put the AirPods back in the case (or power the device off) mid-recording. macOS falls back to the built-in output.
3. Play audio again through the built-in output for ~30 s. Reconnect the AirPods. Stop.

**Expected:** one rebuild per real transition — `reason=device_died` and/or `reason=default_output_device_changed` — audio recording again on the fallback device within the rebuild, and again after reconnect.

**Watch for:** more than a couple of rebuilds per transition. A flapping device that out-paces the listener → rebuild cycle is the storm risk the ADR flagged; the fix (debouncing device-change events) is deliberately **not** implemented until observed. Count them:
```sh
grep -c 'system-audio-tap: rebuilding tap generation' ~/Library/Logs/com.shaikzeeshan.mnema/rust.log
```

---

## Drill 4 — `killall coreaudiod`

Kills the Core Audio server out from under a live tap. This is the crude version of the wedge the whole rebuild engine exists for.

1. Start recording with system audio on, audio playing.
2. `sudo killall coreaudiod` (it restarts itself within a second or two).
3. Keep audio playing for **at least 90 s** afterwards — long enough for the zero-watchdog's first 30 s trip and one backoff step.
4. Stop.

**Expected:** Mnema does not crash, the session stays alive, screen and microphone are untouched, and system audio is recording again within roughly one watchdog window. Expect `reason=device_died` and/or `reason=zero_watchdog`.

**Also expected on restart:** if Mnema itself crashed at any point, the *next* launch reaps the aggregate devices it left behind — `system-audio-tap: destroyed N stale aggregate device(s) from a previous process` (Info-level, so it shows even without developer options; the per-UID `destroyed stale aggregate mnema-system-audio-<pid>-<uuid>` lines beside it are Debug).

---

## Drill 5 — Permission denial and the hint

Process taps have their own TCC category and **no API to query it**, so the app can only infer denial from a tap that has only ever delivered silence. This drill is the only way to see that inference fire.

1. Quit Mnema.
2. Reset the grant:
   ```sh
   tccutil reset SystemAudioCaptureRequests com.shaikzeeshan.mnema
   ```
   (Use `com.shaikzeeshan.mnema.dev` for a dev build. If that service name is rejected by your macOS version, `tccutil reset All <bundle-id>` works but also resets screen/mic — expect to re-grant those too.)
3. Launch Mnema and start a recording with system audio on — **or** use onboarding's system-audio **Grant** button, which runs a throwaway tap purely to raise the prompt.
   **Expected:** the macOS "Screen & System Audio Recording" prompt appears.
4. **Deny it.**
5. Keep recording, with audio playing, for **more than 60 s** (`SILENT_SESSION_AFTER_MS`).

**Expected:**
- Recording proceeds. Screen and microphone are unaffected; nothing errors.
- The system-audio `.m4a` files are silent.
- Permission state moves to *possibly blocked*, and a **dismissible** "system audio may be blocked" hint appears, deep-linking to Privacy & Security → Screen & System Audio Recording. Click it: the pane opens.
- Dismissing the hint sticks.

**Log:**
```
system-audio-tap: permission evidence moved to silent_session after 60123ms of tap
```

6. Now grant it in the pane, restart the recording, play audio, and confirm the evidence moves once and only once:
```
system-audio-tap: permission evidence moved to sound_heard after ...
```
State should read *assumed working* and stay there — a later quiet session must **not** re-accuse (the evidence is monotonic).

**False-positive check:** a Mac that genuinely plays nothing for 60 s of recording earns the same hint. That is known and accepted (hence "may be", hence dismissible). If the soak shows it firing on honest users, the upgrade is to require several silent sessions — not to reach for the private TCC SPI the ADR rejected.

---

## Drill 6 — Screen independence

The decoupling itself. Cheap, and it is what three of the ADR's user stories are about.

- **Audio-only session:** turn screen **and** microphone off, system audio on. Start recording. **Expected:** the session starts and produces `sysaudio_…` segments. The Settings toggle and the tray item must both allow this — neither may gate system audio on screen capture.
- **Display sleep / lock:** start a recording with screen + system audio, play audio, then lock the screen (`Ctrl+Cmd+Q`) or let the display sleep for a few minutes. **Expected:** system audio keeps recording straight through, exactly as the microphone does; only the screen suspends and resumes ([ADR 0021](../adr/0021-recover-from-display-unavailable-as-transient-liveness.md), amended 2026-07-15). The audio recorded during the sleep is present and audible — a gap here is the regression.
- **Low disk:** the one suspension that *does* stop the tap ([ADR 0040](../adr/0040-low-disk-safety-is-a-transient-liveness-capture-suspension-kind.md)) — it is about the volume, not the display. If you exercise it, expect system audio to stop with everything else and resume with everything else.

---

## Soak (multi-day, macOS 26) — release gate

Record normally for several days on macOS 26 with system audio on and developer options enabled, then read the log. The point is not that a rebuild happened — rebuilds are the design — but that they stayed **bounded** and that audio was never silently lost.

```sh
LOG=~/Library/Logs/com.shaikzeeshan.mnema/rust.log

# Total rebuilds, and the breakdown by reason.
grep -c 'rebuilding tap generation' "$LOG"
grep -o 'reason=[a-z_]*' "$LOG" | sort | uniq -c | sort -rn

# Rebuilds per hour (lines start `[YYYY-MM-DD][HH:MM:SS]`, UTC) — a storm is a
# cluster, not a total.
grep 'rebuilding tap generation' "$LOG" | cut -c1-16 | uniq -c

# Rebuilds that failed and are retrying on the backoff.
grep 'tap rebuild failed' "$LOG"

# Format churn across generations.
grep 'sample_format_stabilized' "$LOG"
```

**Watch list:**
- **Rebuild storms.** The backoff bounds the watchdog (30 s → 600 s, six an hour at the cap), but the *device* triggers are unbounded: a flaky Bluetooth device flapping faster than the listener → rebuild cycle would show as a tight cluster of `default_output_device_changed` / `device_died`. If seen, debounce device-change events (deliberately not built until observed).
- **`zero_watchdog` trips while audio was actually playing.** That is the macOS-26 wedge being caught — which is the feature working — but the *frequency* is the number that matters for the release call.
- **`zero_watchdog` trips at the 600 s cap all day.** Expected on a quiet Mac: real silence is indistinguishable from a wedge. Harmless (a rebuild during zeros loses nothing by construction), but it should stop as soon as sound returns.
- **Segment fallout.** Spot-check that rebuild-produced `-<unix-ms>` files are openable and non-empty, and that `segment ... closed without usable audio` only appears for segments that were paused for inactivity throughout.
- **Silent-forever.** The absence of any `permission evidence moved to sound_heard` on a machine that definitely played audio is the loudest possible signal — treat it as a denial or a wedge, not as a quiet week.
