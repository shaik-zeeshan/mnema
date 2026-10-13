# Developer window rebuild: implementation plan

Target: `docs/mockups/redesign/developer.html` (approved). It is drawn at 520×640 (min 440×480), with a mini mode of 300×120 that stays on top.
Settings entry: `docs/mockups/redesign/settings.html#s-developer`.
Out of scope: the main-window status popover (`status.html`).
Branch: `t3/developer-window-and-logging`. Baseline commit: **`c258f625`**. Every agent can read the old code there with `git show c258f625:<path>` after it is deleted.

Each slice below is a complete brief for one agent. That agent reads **this file's Standing rules plus its own slice section**, nothing else from this plan.

---

## Standing rules (every slice)

### Repo rules
- Read `AGENTS.md` first. Run every command from the repo root (`/Users/shaikzeeshan/.t3/worktrees/mnema/t3-e6217028`).
- Don't commit unless the founder asks.
- **Never run bare `cargo fmt`.** It churns ~16 unrelated files. To format, use `rustfmt --edition 2021 <the files you touched>`.
- Keep every file at **800 lines or fewer**. Split by responsibility, not by size.
- **New Tauri commands:**
  - Every new `#[tauri::command]` must be registered in `apps/desktop/src-tauri/src/lib.rs` (`invoke_handler`) and invoked from the frontend with the exact same name.
  - Wire structs use `#[serde(rename_all = "camelCase")]`.
  - TS mirrors are hand-written in `apps/desktop/src/lib/types/*.ts`. There is no codegen.
- **Window permissions:**
  - Window permissions go in `apps/desktop/src-tauri/capabilities/default.json`. That file is one capability covering every window label.
  - `windows::tests::every_app_window_label_is_covered_by_a_capability` must stay green. It also asserts named permissions, so add any new permission to its list.
- **Logging** (`AGENTS.md` "## Logging") applies to every new log line:
  - Error = gave up. Warn = recovers on its own. Info = a state change a user would recognise. Debug = everything else.
  - Never per poll.
  - Never log captured content: no window titles, URLs, OCR or transcript text, keys or tokens.
  - Log through `native_capture::debug_log::{log, log_info, log_warn, log_error}` (desktop) or `capture_runtime::*_log!` (capture crates). Never wrap them in a helper.
- **Plain language in UI copy:**
  - Say "Reading" (not OCR), "Speakers" (not diarization), "Meaning index" (not embeddings). "Transcription" is fine.
  - Raw ids, processor keys and error strings may appear only as secondary mono detail.
  - No ADR references and no "new" chips in UI text.

### Visual rules
- **The mockup wins on visual vocabulary.** Copy its markup and class structure: `dv-*` classes plus the kit's `mx-*`.
- Reuse existing components only for **behaviour**:
  - `lib/components/Segmented.svelte` renders `.mx-seg` with keyboard nav.
  - `use:tip` from `lib/components/tooltip.ts` stands in for the mockup's `data-tip`.
  - `lib/components/Switch.svelte` gives `.mx-switch` semantics.
  - `lib/components/WindowStrip.svelte` gives the lights and the drag region.
- Never substitute a component's look for a mockup element.
- **Kit:**
  - The global kit is already in the app: `lib/styles/kit.css` (`.mx-btn`, `.mx-seg`, `.mx-chip`, `.mx-spec`, `.mx-stat`, `.mx-notice`, `.mx-empty`, `.mx-pop`, `.mx-menu`, `.mx-switch`, `.mx-search`, `.mx-toast`, `.mx-spin`, …) and `lib/styles/shell.css` (`.mx-spacer`, …). Both are imported once in `routes/+layout.svelte`. Don't redefine them.
  - Popovers are raw `.mx-pop-anchor > .mx-pop[data-open] > ul.mx-menu` markup. Follow `lib/shell/ShortcutsPop.svelte`.
  - There is no shared toast helper. Hand-roll it like `lib/insights/SubjectDetail.svelte:56-63,461`.
- **Colours:**
  - Use only `var(--app-*|--chart-*|--mx-*)` tokens or `color-mix()` of them. Never hex.
  - Mono (`.num`, `--font-mono`) is only for data, times, ids and kbd.
  - Flat surfaces: a tinted fill plus a 1px border. No left bars. Shadows only on popovers.
  - One primary button per view.
- **Icons:**
  - Use lucide via `~icons/lucide/<name>`, the pattern already used in `lib/settings/panels/about/Developer.svelte`.
  - Pick the lucide icon the mockup's `<symbol>` was drawn from: pin, shrink, expand, ellipsis, copy, folder, file-text, refresh-cw, mic, volume-2, plug, monitor, inbox, cloud-off, triangle-alert, info, rotate-ccw, search, pause, play, square, circle, and for "Open in Timeline" the same icon `lib/shell/Titlebar.svelte` uses for Timeline.
  - Never use unicode glyphs as icons.
- **Where CSS lives:**
  - Shared developer primitives live in `lib/debug/developer.css`, which F1a owns.
  - Tab-specific CSS goes in the tab component's own `<style>` block.

### Frontend layout (created by F1a; later slices only fill their own files)
```
routes/debug/+page.svelte          shell: full window | mini, tab switch      (F1a, F1b may touch strip wiring only via DevStrip)
lib/debug/state/controller.svelte.ts  stores + nav + live/paused + poll()      (F1a)
lib/debug/developer.css            shared dv-* primitives                     (F1a)
lib/debug/DevStrip.svelte          strip + actions                            (F1a skeleton, F1b actions)
lib/debug/Verdict.svelte, verdict.ts (+ verdict.test.ts)                      (F1a)
lib/debug/HealthRow.svelte         .dv-row (dot · name · why · metric · chevron)  (F1a)
lib/debug/tabs/OverviewTab.svelte  (F2)   PipelineTab.svelte (F3a)   ProcessorView.svelte + JobInspector.svelte (F3b)
lib/debug/tabs/CaptureTab.svelte   (F4)   AiTab.svelte (F5)          LogsTab.svelte (F6)
lib/debug/MiniView.svelte          (F2 content; F1b does the window resizing)
```
- **Polling:** every poll goes through `controller.poll(fn, ms)`. It returns a teardown and skips while paused (strip pause), while the document is hidden, and while a previous call is still in flight.
- **Lifetime:** a tab starts its polls in its own `$effect`, so polls live only while the tab is mounted.
- **Never write a raw `setInterval`.**

### Polling budget (replaces ~6–7 IPC/s today, measured: health 1 s + features 4×1 s + OCR budget 1 s + idle/privacy 2×2 s)

| Where | Calls | Cadence |
|---|---|---|
| Always (verdict, full and mini) | `get_debug_health` | 2 s |
| Overview | nothing extra (rows come from health) | — |
| Pipeline | `get_processing_pipeline_status` + `get_semantic_index_status` | 2 s |
| Pipeline housekeeping | `get_retention_cleanup_status`, `list_frame_batches`, `list_app_jobs` | on open, then 30 s |
| Processor drill-in | `get_processing_pipeline_status` + `list_processing_jobs_by_processor` | 2 s |
| Capture | `get_capture_health` (new, B2) | 1 s (live meters) |
| Capture storage | `get_storage_usage` | on open |
| AI | `get_ask_ai_last_turn_usage` + `list_user_context_derivation_runs` + `list_cli_access_history` | 5 s; config loads once (`features.loadConfig`) |
| Logs | `tail_app_log` | 2 s |
| Mini | health + `get_processing_pipeline_status` (for the total queue) | 2 s |

- The worst case is about 1.5 IPC/s. Paused or hidden is 0.
- **Decision recorded:** lower cadence plus tab gating is smaller than one consolidated snapshot command. The only consolidation is B2's `get_capture_health`, which replaces the `get_idle_debug` + `get_capture_privacy_debug` pair.

### Verify by RENDERING (mandatory for every UI slice)
Grepping class names gave three false "matches the mockup" verdicts on the last debug redesign. Render the page, take a screenshot, and **look at the PNG with the Read tool**.

Harness, kept in your own scratchpad (don't commit it):
1. `bunx playwright install chromium` (once).
2. Start the dev server with `nohup bun --cwd=apps/desktop run dev > <scratch>/vite.log 2>&1 & disown`. It serves on **http://localhost:1420**.
3. Stub Tauri IPC with `page.addInitScript`:
   ```js
   window.__TAURI_INTERNALS__ = { invoke: async (cmd, args) => STUB(cmd, args), transformCallback: (c) => c, convertFileSrc: (x) => x,
     metadata: { currentWindow: { label: "debug" }, currentWebview: { label: "debug" } } };
   window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: async () => {} };
   ```
4. **`/debug` is gated.** `get_recording_settings` must return an object with `developerOptionsEnabled: true` (shape it from `RecordingSettings` in `lib/types/recording.ts`). Otherwise `+layout.svelte` redirects to `/`.
5. **Never return `null` for an unmocked command.** A `null` where an array is expected throws inside an `$effect` and silently freezes the whole page. Use these defaults:
   - `[]` for `list_*`
   - `false` for `has_pending*`
   - `null` only for `drain_*`
   - `{}` otherwise
   If a stray `.license-receipt` overlay from `+layout.svelte` swallows clicks, hide it.
6. **Shape stub data from the real TS types**, using the mockup's numbers for each state. Wrong-shaped data silently renders the empty path and looks like "missing UI".
7. Use `colorScheme: "dark"`, plus a second pass with `"light"`. Use viewport **520×640** for the full window and **300×120** for mini.
8. Screenshot the mockup at the same state for comparison: `file://<repo>/docs/mockups/redesign/developer.html?state=<state>` (add `&theme=light` for the light pass). Use `page.locator('#dv-win').screenshot()` for the full window and `#dv-mini` for mini.
9. Compare side by side and fix overflow, clipping, misalignment, wrong spacing, contrast and missing rows. Then report which states you rendered.

### Standing verify commands
- `bun run check` (repo root).
- Desktop Rust: `bash -c '. scripts/openblas-build-env.sh; cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml'`. On a clean checkout, run `bash scripts/prepare-mnema-cli-sidecar.sh debug` first.
- Lib tests, always filtered: `bash -c '. scripts/openblas-build-env.sh; cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --lib <filter>'`.
- Crate tests: `cargo test -p <crate> <filter>`.
- Frontend unit tests: `bun test apps/desktop/src/lib/debug`.
- **Known pre-existing failures (not yours):**
  - the `take_inflight` dead-code warning
  - 4 `resume_screen_from_inactivity_*` failures
  - flakes in native_capture global state, the ask_ai pidfile, and speaker_analysis helper start

---

## Wire contracts (fixed here so frontend and backend slices agree)

**B1 — `get_debug_health` v2** (`lib/types/debug.ts`):
```ts
export type DebugSeverity = "ok" | "info" | "warn" | "error" | "off";   // UI tone: error→danger, others same
export type DebugFeature =
  | "screen" | "microphone" | "systemAudio"
  | "reading" | "transcription" | "speakers" | "meaningIndex"
  | "aiEngine" | "userContext" | "disk" | "cliAccess" | "startup";      // returned in this order, always 12
export interface FeatureHealthDto {
  feature: DebugFeature; severity: DebugSeverity;
  reason: string;            // plain sentence, mockup voice ("Deepgram can’t be reached, retrying on its own")
  metric: string | null;     // short right-column text ("14 waiting", "182 GB", "3rd today", "1.8 s")
  metricAtMs: number | null; // when set, the UI renders a local HH:MM instead of `metric` ("used 14:28")
}
```

**B2 — `get_capture_health`** (new; `lib/types/debug.ts`):
```ts
export type CaptureSourceState = "off" | "recording" | "paused" | "waiting" | "reconnecting" | "failed";
export interface CaptureSourceHealth {
  requested: boolean; state: CaptureSourceState;
  reason: string | null;           // raw reason code, e.g. capture_display_unavailable
  levelDb: number | null;          // dBFS from the normalized peak, audio only
  idleMs: number | null;
  deviceName: string | null;       // optional; null if not cheaply known
}
export interface CaptureHealthDto {
  recording: boolean; startedAtMs: number | null;
  screen: CaptureSourceHealth; microphone: CaptureSourceHealth; systemAudio: CaptureSourceHealth;
  suspension: { kind: "privacyFilter" | "displayUnavailable" | "lowDisk"; reason: string; recoveryAttempts: number;
                restartRequired: boolean; sinceMs: number | null; lastErrorMessage: string | null } | null;
  systemAudioTap: { rebuildsToday: number;
                    lastReason: "deviceChanged" | "deviceDied" | "sampleRateChanged" | "excludeListMoved" | "silenceWatchdog" | null;
                    lastRebuildAtMs: number | null; lastDetail: string | null;  // e.g. "48000 → 16000 Hz"
                    nextSilenceCheckInMs: number | null; watchdogDelayMs: number | null; sampleRateHz: number | null } | null;
  disk: { freeBytes: number | null; reserveFloorBytes: number;
          pauseThresholdBytes: number | null; resumeThresholdBytes: number | null };   // thresholds null while not recording
}
```

**B4 — `retry_processing_job_now({ jobId })`** returns `ProcessingJobDto`. It only works on a `queued` job (including one waiting out a backoff) and errors otherwise.

---

## Tiers and parallel groups

| Tier | Slices (run in parallel within a tier) | Waits for |
|---|---|---|
| 0 | **B2** capture health · **B3** startup report · **W1** window config + capability · **F1a** window shell · **F8** Settings row + ⌘D | — |
| 1 | **B1** health rows v2 · **B4** retry-now · **F1b** strip actions + mini resize · **F3a** Pipeline tab · **F4** Capture tab · **F5** AI tab · **F6** Logs tab | B1←B2,B3,F1a · B4←B2,B3 · F1b←F1a,W1 · F3a,F5,F6←F1a · F4←F1a,B2 |
| 2 | **F2** Overview + not-recording + mini content · **F3b** processor drill-in, jobs, inspector | F2←B1,F1a · F3b←F3a,B4 |
| 3 | **X1** integration, dead code, full render pass | everything |

Expected size: about 4,600 changed lines added across 14 slices, and about 8,500 lines deleted (mostly in F1a).

### Conflict watch (shared files and who owns them)

| File | Owner | Others |
|---|---|---|
| `src-tauri/src/lib.rs` | B2 (registers `get_capture_health`) | B4 registers `retry_processing_job_now`. B4 must not run while B2 is open; it is in tier 1, so it is safe. B3 must not touch lib.rs (its report lives in a static). |
| `src-tauri/src/app_infra.rs` | B3 (`run_deferred_startup_blocking`) | B4 adds a command near `get_processing_job`. **Run B4 after B3.** |
| `crates/app-infra/src/lib.rs` | B3 (`run_startup_maintenance` return type) | B4 adds an `AppInfra` wrapper. **Run B4 after B3.** |
| `src-tauri/src/native_capture.rs`, `native_capture/{activity,runtime,system_audio,disk_space,segments}.rs` | B2 | B1 only calls B2's helper, so B1 runs after B2. |
| `src-tauri/src/debug_health.rs` | B1 | nobody else |
| `lib/types/debug.ts` | B2 (adds `CaptureHealthDto`), then B1 (replaces the health types) | B2 and B1 are sequential by tier. Frontend slices never edit it; if a field is missing, report it. |
| `lib/types/app-infra.ts` | B4 (adds `modelLocked`) | — |
| `routes/debug/+page.svelte`, `state/controller.svelte.ts`, `developer.css`, `HealthRow.svelte`, `Verdict.svelte` | F1a | Later slices don't edit them. A tab needs a new shared class → put it in the tab's `<style>` and let X1 hoist it if two tabs use it. |
| `DevStrip.svelte` | F1a (skeleton) → F1b (actions) | sequential |
| `MiniView.svelte` | F2 (content) | F1b only mounts or unmounts it through the page's `mode`, which F1a already wires |
| `state/capture.svelte.ts` | F4 | — |
| `state/pipeline.svelte.ts`, `state/detail.svelte.ts`, `detail/jobs.ts`, `detail/specs.ts` | F3a (pipeline store), F3b (detail, jobs, specs) | F1a re-keys `specs.ts` only (see F1a). After that, F3b owns it. |
| `state/features.svelte.ts` | F5 | F3a reads it but must not edit it. |
| `state/logs.svelte.ts`, new `log-line.ts` | F6 | F1a re-types `logs.svelte.ts`'s feature filter only (see F1a). |
| `log-filter.ts` | F1a (decouples it from `DebugFeature`) | F6 reads `LOG_FEATURE_PATTERNS` keys only |
| `format.ts` | F1a (deletes the old `*Class` helpers) | Tabs only import from it. New helpers go in the tab's own module. |
| `routes/+layout.svelte` | F1a (removes the debug `surface-titlebar` and `isNarrow`) | F8 does **not** touch it |
| `lib/shell/Titlebar.svelte`, `lib/settings/panels/about/Developer.svelte` | F8 | — |
| `capabilities/default.json`, `src-tauri/src/windows.rs` | W1 | — |

---

# Tier 0

## B2 — Capture health snapshot: per-source state, suspension, system-audio tap, disk thresholds
**Goal:** add one new command, `get_capture_health`, returning `CaptureHealthDto` (see Wire contracts). It feeds the Capture tab (F4) and the per-source, Disk and system-audio rows of the health rollup (B1). It also fixes the backend half of the macOS bug: today the screen suspension reason (display asleep, privacy filter) reaches the frontend only as `get_idle_debug.runtimeSources.screen.reason`, and nothing reads it on macOS.

**Read:**
- `developer.html?state=capture`, `?state=capture-asleep` and `?state=overview-issues`, including the notes column "Needs backend" items 1 and 2.
- `crates/capture-system-audio/src/rebuild.rs`: `RebuildReason` at :58-80, `request_rebuild` at :97-104, `rebuilds` at :303, `rebuild()` at :416-432, `rebuild_count()` at :398.
- `crates/capture-system-audio/src/watchdog.rs`: `ZeroWatchdog` at :35-71.
- `apps/desktop/src-tauri/src/native_capture/system_audio.rs`, which holds `SystemAudioFamily`.
- `native_capture/activity.rs`: `build_runtime_sources_status` at :364-431 and `AudioPeakReadMode`.
- `native_capture/runtime.rs`: `CaptureSuspension` at :36-66, `capture_suspension` at :165, `session_reports_running` at :317.
- `native_capture/disk_space.rs`: `RESERVE_FLOOR_BYTES`, `pause_threshold_bytes`, `resume_threshold_bytes` at :29-113.
- `native_capture/segments.rs`: `next_segment_estimate_for_runtime` at :965.
- `native_capture/lifecycle.rs`: `runtime()` at :1464, and `tick_system_audio` at :600-626, which holds the capture lock while rebuilding.
- `managed_storage_layout.rs`: `get_storage_usage`, which already reports `free_bytes`.

**Build:**
- **Crate** `capture-system-audio`. Add `pub struct TapHealth { rebuilds: u64, last_reason: Option<RebuildReason>, last_rebuild_at: Option<SystemTime>, last_detail: Option<String>, next_silence_check_in: Option<Duration>, watchdog_delay: Duration, sample_rate_hz: Option<u32> }` and `pub fn health(&self) -> TapHealth` on the session.
  - Record `last_reason`, the time and the detail inside `rebuild()`. For a sample-rate change, the detail is "48000 → 16000 Hz", from the values the listener already sees.
  - Compute the next check as `delay.saturating_sub(now - last_sound)` from `shared.lock_watchdog()`.
  - Make `RebuildReason::as_str` `pub`.
  - Add a unit test for `next_silence_check_in`.
- **"Rebuilds today"** has to outlive a session. Keep a process-global `AtomicU64` counter plus a local-day stamp. Put them in `native_capture/system_audio.rs` and bump them where a rebuild is observed.
  - Precedent: the process-global atomics in `activity.rs` (`record_delivery`).
  - Reset the counter when the local day changes.
  - `rebuildsToday` counts since local midnight **within this app run**. Say so in a doc comment.
- **`SystemAudioFamily`**: add a `health()` pass-through.
- **`native_capture.rs`**:
  - Add a `pub(crate) fn capture_health_snapshot(state: &NativeCaptureState, …) -> CaptureHealthDto`. B1 reuses it, so keep it the one seam.
  - Add the `#[tauri::command] get_capture_health` that wraps it.
  - Register it in `lib.rs`.
- **Source state mapping:**
  - Requested + session active → `recording`.
  - Inactivity-paused → `paused`.
  - Screen with a `DisplayUnavailable` suspension → `waiting`.
  - Privacy-filter suspension → `failed` when `restartRequired`, otherwise `waiting`.
  - `LowDisk` → `paused` on all sources.
  - System audio with a rebuild in the last 60 s → `reconnecting`. Make 60 s a named constant, a tunable ceiling.
  - Not requested or not recording → `off`.
  - **Always copy the suspension reason into `screen.reason`**, whatever `sessionActive` is. That is the bug.
- **Levels:** `levelDb = 20·log10(normalized_peak)`, clamped to −60 dB, or `null` when there is no sample.
  - **Read with `AudioPeakReadMode::Peek`, never `Take`.** A debug poll must not consume the inactivity detector's peak window.
- **Suspension:** add `since: Option<SystemTime>` to `CaptureSuspension`. Set it where the suspension is created and keep it across `recovery_attempts` increments. Expose `kind`, `reason`, `recoveryAttempts`, `restartRequired` (status == RestartRequired), `sinceMs` and `lastErrorMessage`.
- **Disk:**
  - `freeBytes` comes from `disk_space::measure_free_space` (or whatever `get_storage_usage` already calls). It is cheap; check that it does no directory walk.
  - `reserveFloorBytes = RESERVE_FLOOR_BYTES`.
  - The pause and resume thresholds come from `next_segment_estimate_for_runtime` while a runtime exists, otherwise `null`. That function is macOS-only, so return `null` elsewhere.
- **`deviceName`** is optional. Fill it only where the name is already held:
  - microphone: the selected device in recording settings
  - system audio: the tap's output device, if the session holds it
  - screen: `null` is fine
  Ships without it: the lane shows only the state word.
- **The Rust lock:**
  - Take the `NativeCaptureState` lock once and copy out what you need.
  - The tap rebuild runs under that same lock, so a `rebuilding_now` flag read through it is always false. Don't add one; `reconnecting` uses the 60 s rule instead.
- **TS:** add the B2 types from Wire contracts to `lib/types/debug.ts`. Don't touch the existing health types (B1 owns those).
- **Tests:**
  - A pure mapping function, `source_state(...)`, with a unit test covering display-unavailable → screen `waiting` with a reason while mic and system audio stay `recording`, and low-disk → all `paused`.
  - A `levelDb` test.

**Don't:**
- Change what `get_idle_debug` or `get_capture_privacy_debug` return (X1 decides whether they are still needed).
- Touch `debug_health.rs`.
- Add a TCC/SPI permission probe.
- Log per call.
- Touch the main-window status bar (`lib/shell/status-bar.ts` has the same blind spot, but that is the status-popover project).

**Files:** `crates/capture-system-audio/src/{rebuild,watchdog,lib}.rs`, `src-tauri/src/native_capture.rs`, `native_capture/{system_audio,activity,runtime,disk_space,segments}.rs` (as needed), `src-tauri/src/lib.rs` (registration only), `lib/types/debug.ts`. About **420 lines**.

**Verify:**
- `cargo test -p capture-system-audio`
- desktop `cargo check`
- `cargo test … --lib native_capture::` (filtered to your new tests)
- `bun run check`

**Dependencies:** none. **Parallel:** with B3, W1, F1a, F8. **Blocks** B1, F4 and B4 (the lib.rs registration).

## B3 — Startup maintenance report
**Goal:** keep what `mnema-deferred-startup` does as a `StartupReport` that B1's "Startup" row reads. Today every count is either logged or thrown away.

**Read:**
- `developer.html?state=overview-ok` and `?state=overview-issues`: the Startup row (ok copy "Nothing needed repair", metric "1.8 s").
- The notes item "Startup report".
- `src-tauri/src/app_infra.rs`: `run_deferred_startup_blocking` at :1788-1986.
- `crates/app-infra/src/lib.rs`: `run_startup_maintenance` at :468-497, which returns `Result<()>`.
- `processing/store.rs`: `ProcessingJobReclamationSummary` at :116.
- `src-tauri/src/lib.rs`: `run_deferred_startup` at :465-505, read-only.

**Build:**
- `run_startup_maintenance` returns a `StartupMaintenanceSummary` struct with these fields:
  - `stale_model_locks_cleared`
  - `frame_equivalence_backfilled`
  - `orphaned_app_jobs`
  - `orphaned_processing_requeued`
  - `orphaned_processing_failed`
  - `closed_batches_reconciled`
  - `open_batches_reconciled`
  Keep the counts the passes already return, instead of discarding them. Update its one caller.
- In `run_deferred_startup_blocking`:
  - Time the whole thing with `Instant`.
  - Collect the vault unlock outcome (bool plus an error code, never the message if it could hold a secret), the summary above, the workspace repair `removed` count and the OCR-disabled fail count.
  - Collect pass errors as short static labels (for example `"vault_unlock"` or `"workspace_repair"`).
  - Store the result in a `static STARTUP_REPORT: OnceLock<StartupReport>` with `pub(crate) fn startup_report() -> Option<StartupReport>`.
  - **Don't edit `lib.rs`.** That means the aggregate-device reap, which runs afterwards in `lib.rs`, is not in the report. That's acceptable; note it in a comment.
- Write one Info line at the end: `startup maintenance finished in {ms} ms (requeued=…, failed=…, workspaces_removed=…)`. It is a startup repair count, which AGENTS.md lists as Info. If equivalent per-pass Info lines already exist, merge them so it doesn't double-log.
- Add a unit test for the summary, if `run_startup_maintenance` is already tested in app-infra. Otherwise add a test for the `StartupReport` helper that builds the reason text. **B1 owns the copy**, so expose only data here.

**Don't:**
- Move any pass out of the deferred thread, or into the synchronous path (AGENTS.md invariant).
- Change pass order.
- Add a Tauri command (B1 reads the static).
- Touch DB migrations.

**Files:** `crates/app-infra/src/lib.rs`, `crates/app-infra/src/processing/store.rs` (only if the summary type needs exporting), `src-tauri/src/app_infra.rs`. About **180 lines**.

**Verify:**
- `cargo test -p app-infra startup`
- desktop `cargo check`

**Dependencies:** none. **Parallel:** with B2, W1, F1a, F8. **Blocks** B1 and B4 (which edit the same `app_infra.rs` and `crates/app-infra/src/lib.rs`).

## W1 — Window config and capability
**Goal:** the window opens at 520×640 with a minimum of 440×480, and the page is allowed to pin itself on top and shrink to mini mode.

**Read:**
- `developer.html`: the header comment and `?state=mini`.
- `src-tauri/src/windows.rs`: `AppWindow::Debug` at :254-266, and the test at :1586-1645.
- `capabilities/default.json`.

**Build:**
- In `AppWindow::Debug`:
  - `inner_size: (520.0, 640.0)`, `min_inner_size: (440.0, 480.0)`
  - title `"mnema · Developer"`
  - keep `gated_by_dev_options: true`, `decorations: false`, transparent, shadow, and corner radius 12
- Update the size comment: "mini mode lowers the min size at runtime (F1b)".
- Add these to `capabilities/default.json` `permissions`: `core:window:allow-set-always-on-top`, `core:window:allow-set-min-size`. `allow-set-size` is already there.
  - Extend the description sentence: the developer window pins itself on top and shrinks to mini.
  - Add both permission names to the test's asserted list.

**Don't:**
- Add a Rust command for mini mode. Frontend window APIs plus these two permissions are smaller.
- Touch any other window.

**Files:** `src-tauri/src/windows.rs`, `src-tauri/capabilities/default.json`. About **20 lines**.

**Verify:** `cargo test … --lib windows::tests` and desktop `cargo check`.

**Dependencies:** none. **Parallel:** with everything in tier 0.

## F1a — Window shell: page, controller, strip skeleton, verdict, tabs, shared CSS; delete the old window
**Goal:** replace the 11-card scroll and floating dock with the mockup's frame:
- the strip
- the verdict line
- the five-tab `.mx-seg` (Overview · Pipeline · Capture · AI · Logs)
- a scrolling `.dv-body`
- empty tab components that later slices fill

Also delete the old UI.

**Read:**
- `developer.html`: the whole `<style>` block (frame, strip, verdict, tabs, body, `.dv-sec`, `.dv-h`, `.dv-dot`, `.dv-row`, `.dv-list`, the `.mx-spec` overrides, `.dv-crumb`, `.dv-log`), and the script's `verdict()` / `renderVerdict()`. States: `?state=overview-ok`, `overview-issues`, `capture-asleep`, `not-recording`.
- The current `routes/debug/+page.svelte`, `lib/debug/state/*.svelte.ts`, `lib/debug/format.ts`, `lib/debug/log-filter.ts`, `lib/debug/detail/specs.ts`.
- `lib/components/WindowStrip.svelte`.
- `routes/+layout.svelte`: around :59 `isDebug`, :231-234 `showChildren`/`isNarrow`, :301-317 Escape, :395-412 the debug `surface-titlebar`, and the `.surface-titlebar*` / `.app-content--narrow` CSS around :961-1090.

**Build:**
- **`routes/debug/+page.svelte`** (≤200 lines):
  - Build the controller.
  - `mode: "full" | "mini"`, read from the controller.
  - Render `<DevStrip/>`, `<Verdict/>`, the tabs, and `<main class="dv-body">` with the active tab component, or `<MiniView/>` when `mode === "mini"`.
  - Health polling (2 s) runs in both modes.
  - **Esc:** in a drill-in (`nav.processor` or `nav.jobId` set), add a capture-phase window `keydown` that goes back one level and calls `preventDefault()`. The layout's close-on-Escape already ignores `defaultPrevented`. At the top level, Esc still closes the window.
- **`state/controller.svelte.ts`:**
  - Keep `capture`, `pipeline`, `health`, `features`, `detail`.
  - Add `nav = $state({ tab: "overview", processor: null as ProcessorKey|null, jobId: null as number|null, logNeedle: "" })` with helpers `go(tab)`, `openProcessor(p)`, `openJob(id)`, `back()`, `showInLog(needle)`.
  - Add `live = $state(true)` and `pausedAtMs`.
  - Add `mode`.
  - Add `poll(fn, ms)`: skip while `!live`, while the document is hidden, or while a call is in flight; fire immediately on start; return a teardown.
  - Remove the old pollers' call sites from the page. The stores keep their fetch functions.
- **`health.svelte.ts`:** poll cadence 2 s through `controller.poll`. The store API stays.
- **`DevStrip.svelte`** (skeleton):
  - Use `WindowStrip`. Add an optional `actions` snippet prop to `WindowStrip.svelte` (a right-aligned `.mx-strip__acts` slot); the update and access pages are unaffected.
  - Title "Developer".
  - The pause button is wired to `controller.live`: pause/play icon, live dot, tooltip "Pause live updates" / "Resume live updates".
  - Pin, mini and ⋯ are left for F1b. **Render no placeholder buttons.**
- **`Verdict.svelte` + `verdict.ts`:**
  - `verdict(rows: FeatureHealthDto[]) → { tone, text }`, where the tones are `danger | warn | info | off | ok`, mapping severity `error → danger`.
  - Copy:
    - "N things need a look" (or "1 thing needs a look") when any row is warn or error.
    - Otherwise, the first info row's `reason`.
    - Otherwise, "Not recording · finishing earlier work" when every capture row is `off`.
    - Otherwise, "Everything is running".
  - Right side, mono:
    - "paused at HH:MM:SS" while paused.
    - Otherwise "recording 2h 14m · <version>" or "not recording · <version>".
    - The duration comes from `capture.captureSession.startedAt` (already loaded). The version comes from `getVersion()` (`@tauri-apps/api/app`).
  - Type `severity` as `string` in `verdict.ts`, so it compiles before and after B1 changes the union.
  - Add `verdict.test.ts` (bun:test) for the four branches.
- **`HealthRow.svelte`:** the `.dv-row` button, with props `{ tone, name, why, metric, plain?, onclick }`. Its markup and CSS are exactly the mockup's `row()`.
- **`developer.css`:** port the mockup's shared rules:
  - strip actions, `.dv-verdict`, `.dv-dot` + tones, `.dv-tabs`, `.dv-body`
  - `.dv-sec`, `.dv-h`, `.dv-list`, `.dv-row*`, `.dv-spec-box`
  - the `.dv-body .mx-spec` / `.mx-notice` overrides
  - `.dv-crumb`, `.dv-log` (it's used by both the AI tab and the jobs list)
  Tone colour vars (`--tone`, `--tone-bg`, `--tone-bd`) come from kit.css `[data-tone]`; check they exist there and copy the mockup's mapping if not.
- **Tab stubs:** `tabs/{OverviewTab,PipelineTab,CaptureTab,AiTab,LogsTab}.svelte` and `MiniView.svelte`. Each is an `mx-empty mx-empty--compact` "Coming in <slice id>". Overview's stub renders the health rows as a flat `.dv-list` of `HealthRow`s, so the window stays usable mid-build.
- **Decoupling, so B1 can change `DebugFeature` without touching lib/debug:**
  - `log-filter.ts`: introduce `export type LogFeature = "capture"|"privacy"|"ocr"|"transcription"|"diarization"|"embeddings"|"aiRuntime"|"userContext"|"jobsAndStorage"` and key `LOG_FEATURE_PATTERNS` by it. Update `log-filter.test.ts` ("covers every LogFeature"). Re-type `state/logs.svelte.ts` to `LogFeature`.
  - `detail/specs.ts`: key it by `export type ProcessorKey = "reading" | "transcription" | "speakers" | "meaningIndex"`, mapping to the same processors and reprocess commands. Adjust `state/detail.svelte.ts` only as far as the type change needs.
- **`format.ts`:** delete the old-CSS emitters (`permissionBadgeClass`, `severityBadgeClass`, `severityDotClass`, `severityCardClass`, `jobStatusBadgeClass`, `ocrStatusBadgeClass`, `dispositionBadgeClass`) and the `DebugStat` type. Also delete `jobStateBadgeClass` in `detail/jobs.ts`. Keep the formatters.
- **Delete:**
  - `lib/debug/DebugDock.svelte`, `sections.ts`, `sections/*` (12 files)
  - `detail/{FeatureDetail,JobsTable,JobInspector}.svelte`, `LogTail.svelte`
  - all seven `lib/debug/debug-*.css` (2,482 lines)
  - the dead pager and debug-job-submit code in `state/pipeline.svelte.ts`: `JOBS_PAGE_SIZE`, the OCR event pagers, `clampPages`, `submit_debug_cpu_job`, `classify_hidden_segment_workspace`, the post-submit poll. Keep `fetchInfraStatus` and `fetchJobs`.
  - `state/features.svelte.ts`'s internal `setInterval` (F3a and F5 drive it via `controller.poll`). Keep its fetchers.
- **`+layout.svelte`:**
  - Delete the debug `surface-titlebar` block (:395-412) and its CSS, if no other window uses it (grep `surface-titlebar`).
  - Delete `isNarrow` / `.app-content--narrow`.
  - Keep the `/debug` dev-options gate and Escape-closes.

**Don't:**
- Build any tab's content.
- Add pin, mini or menu behaviour.
- Edit `lib/types/*`.
- Edit Rust.
- Keep "live · 1s poll" copy.
- Add a dock.

**Files:** see the Build list. About **650 lines added**, about **8,000 deleted**.

**Verify:**
- `bun run check`, `bun test apps/desktop/src/lib/debug`.
- Render `/debug` at 520×640, dark and light, with stub health rows in two states: all `ok`, and one `warn` plus one `error`. Compare the strip, verdict and tabs pixel-wise against `developer.html?state=overview-ok` and `?state=overview-issues` (`#dv-win`). The body differs until F2.
- Confirm in the render that the old dock and cards are gone.

**Dependencies:** none. **Parallel:** with B2, B3, W1, F8 (F8 doesn't touch `+layout.svelte`). **Blocks** every other F slice and B1.

## F8 — Settings › Developer: "Open developer window ⌘D" row; titlebar bug icon removed
**Goal:** match `settings.html#s-developer`. There's a "Developer window" row with the description "Health, queues, capture and logs in a small window you can keep beside the app." and a `mx-btn mx-btn--sm` "Open" with a `<kbd>` showing the effective ⌘D binding. The row is disabled (`is-off` look, `aria-disabled`, no click) while developer options are off. Also drop the redesign-removed titlebar bug icon.

**Read:**
- `docs/mockups/redesign/settings.html:1087-1100` and the toggle script at :1254-1256. Open `settings.html#s-developer`.
- `lib/settings/panels/about/Developer.svelte`, `lib/settings/ui/SettingRow.svelte`.
- `lib/surface-windows.ts:191` (`openDebugWindow`).
- `lib/global-shortcuts.ts` (`getEffectiveGlobalShortcut("openDebug")`), `lib/keyboard.ts` (`formatShortcut`).
- `lib/shell/Titlebar.svelte:184-195`.

**Build:**
- Add a new `SettingRow` right after "Enable developer options":
  - The button calls `openDebugWindow()`.
  - `disabled={!rec.draftDeveloperOptionsEnabled}`.
  - The kbd shows `formatShortcut(getEffectiveGlobalShortcut("openDebug"), platform)`. It must follow a rebind.
- Rewrite the stale "Enable developer options" description to one plain sentence: "Turns on the developer window and keeps detailed entries in the app log. Changes apply immediately." Mention neither "navigation bar" nor "Debug page".
- Remove the bug-icon button from `Titlebar.svelte` and its now-unused imports and props. Keep the `devEnabled` prop if `ShortcutsPop` still needs it. ⌘D and Settings remain the ways in; the status-popover footer link comes with that later project.

**Don't:**
- Restyle other rows.
- Touch the log rows.
- Touch `+layout.svelte`. The ⌘D handler stays as is.

**Files:** `lib/settings/panels/about/Developer.svelte`, `lib/shell/Titlebar.svelte`. About **50 lines**.

**Verify:**
- `bun run check`.
- Render `/settings` (main-window label stub) scrolled to the Developer section, with dev options on and off, against `settings.html#s-developer`. Render the main titlebar once to confirm the icon is gone.

**Dependencies:** none. **Parallel:** with all of tier 0.

---

# Tier 1

## B1 — `get_debug_health` v2: 12 plain rows
**Goal:** replace the 9 feature rollups with the mockup's 12 rows, in this order:

| # | Row | Mockup name |
|---|---|---|
| 1 | screen | Screen |
| 2 | microphone | Microphone |
| 3 | systemAudio | System audio |
| 4 | reading | Reading |
| 5 | transcription | Transcription |
| 6 | speakers | Speakers |
| 7 | meaningIndex | Meaning index |
| 8 | aiEngine | AI engine |
| 9 | userContext | User Context |
| 10 | disk | Disk |
| 11 | cliAccess | CLI access |
| 12 | startup | Startup |

Each row has the five severities and the `metric` / `metricAtMs` column (Wire contracts). Privacy folds into Screen. "Jobs & Storage" goes away: failed jobs are on their lane rows, and storage is on Disk.

**Read:**
- `developer.html`: the script's `BASE` / `SC` tables (the copy voice and metric shapes per scenario) and `?state=overview-ok`, `overview-issues`, `capture-asleep`, `not-recording`.
- `src-tauri/src/debug_health.rs` (all of it), `debug_status.rs` (`get_semantic_index_status`).
- B2's `capture_health_snapshot` (`native_capture.rs`) and B3's `startup_report()` (`app_infra.rs`).
- `cli_access.rs` (`list_cli_access_grants` / `list_cli_access_history` inner fns), `app_infra.rs:4224` (`get_cli_status_inner`).
- `ask_ai.rs:247` (last turn usage) and the user-context status / derivation runs readers.

**Build:**
- Rust: `DebugSeverity { Ok, Info, Warn, Error, Off }`.
  - Keep `roll_up` pure over a `DebugHealthInputs` struct.
  - Rank "worst" explicitly (error > warn > info > ok > off) instead of relying on derive order.
  - `FeatureHealthDto` gains `metric: Option<String>` and `metric_at_ms: Option<i64>`.
- **Rules.** Copy in quotes is the target voice; adapt the numbers:
  - **Screen:**
    - off: "Not recording"
    - info, inactivity paused: "Paused while you’re away"
    - info, `displayUnavailable`: "Waiting, display asleep", metric = minutes since `sinceMs`
    - warn, privacy filter retrying: "Privacy filter couldn’t be applied, retrying"
    - error, `restartRequired`: "Privacy filter failed, restart recording"
    - warn, low disk: "Paused, disk almost full"
    - warn, excluded apps configured but filter not applied (the old privacy rule)
    - ok: "Recording" (+ device name if any), metric = session duration "2h 14m"
  - **Microphone / System audio:**
    - off / ok (device name, metric = "−34 dB" from `levelDb`)
    - info when inactivity paused
    - warn on low disk
    - **System audio** warn when `reconnecting`: "Rebuilding after <plain reason>". The plain reasons:
      - sampleRateChanged → "headset switched to call quality"
      - deviceChanged → "output device changed"
      - deviceDied → "output device disappeared"
      - excludeListMoved → "an excluded app started or quit"
      - silenceWatchdog → "a long silence"
      Metric = "3rd today" (ordinal of `rebuildsToday`).
  - **Reading / Transcription / Speakers** (from `processing_pipeline_status`):
    - warn when `retrying > 0`: the first line of `lastError` made plain where obvious. A transient provider error reads "<Provider> can’t be reached, retrying on its own" (ADR 0048: never error). Metric "N waiting".
    - Otherwise error when `failedLast24h > 0`: "N frame(s)/recording(s) couldn’t be read in the last 24 h", metric "N failed".
    - Otherwise info when not recording and waiting > 0: "Finishing the last recording", metric "N waiting".
    - Otherwise ok: "Keeping up" / "Up to date", metric "N waiting".
    - Waiting = queued + retrying.
  - **Meaning index:**
    - error: load error or consecutive failures
    - warn: quarantined > 0
    - info: not recording and backlog > 0 ("Indexing what was just read")
    - ok: "12,480 passages searchable"
    - metric "N behind"
    - Off when semantic search is disabled.
  - **AI engine:**
    - off: user disabled
    - warn, not set up: `no_providers`, `no_default_model`, …
    - warn: `provider_unreachable:*` / `local_endpoint_unreachable` — "<provider> can’t be reached". **Never** "sign in again".
    - error: `needs_reconnect:*` ("Sign-in expired, reconnect in Settings")
    - ok: "<Provider kind>, ready"
    - `metricAtMs` = last Ask AI turn start
    - Use only the sync, no-network status. Never call `resolve_engine_config_live` from a poll.
  - **User Context:**
    - off: disabled
    - warn: enabled while the engine is unavailable
    - error: the last derivation run failed
    - ok: "Derived N min ago, digest fresh|stale"
    - `metricAtMs` = last run time
  - **Disk:**
    - warn, low-disk suspended: "Recording paused, disk almost full"
    - warn when free < resume threshold: "Getting low, pauses below X GB"
    - ok: "Plenty of room, pauses below X GB"
    - Fall back to the reserve floor when thresholds are null.
    - metric = free "182 GB"
  - **CLI access:**
    - off: CLI not installed ("Not set up")
    - ok: "N tools allowed, M blocked"
    - metric "N today": audit events since local midnight
    - Never warn on a refusal; that's the user's choice.
  - **Startup:**
    - info while `startup_report()` is `None`: "Running startup checks"
    - warn when the vault didn't unlock or any pass errored ("<label> failed, see Logs")
    - ok: "Nothing needed repair", or "Repaired: N jobs reclaimed, N workspaces cleaned"
    - metric = duration "1.8 s"
- Cost: build the snapshot through B2's helper. No new per-poll SQL beyond what runs today, plus one grants-file read and one audit count. Cache the CLI part for 30 s if it reads files.
- TS: replace `DebugSeverity`, `DebugFeature` and `FeatureHealthDto` in `lib/types/debug.ts` with the Wire contract. Remove the "9 of these but 11 sections" comment.
- Tests: update the `report.len() == 9` test to 12, and keep order stable. Add table tests for these mockup scenarios:
  - **issues:** system audio warn, transcription warn, reading error
  - **asleep:** screen info, mic and system audio ok
  - **off:** three capture rows off, reading info
  - **ok:** all ok
  - a ChatGPT `provider_unreachable` → warn, never error

**Don't:**
- Touch any `lib/debug/**` file. F1a decoupled them, and F2 renders.
- Add commands.
- Call the network.
- Log per poll.

**Files:** `src-tauri/src/debug_health.rs` (split into `debug_health.rs` + `debug_health/rules.rs` if it passes 800 lines), `lib/types/debug.ts`. About **500 lines**.

**Verify:**
- `cargo test … --lib debug_health`
- desktop `cargo check`
- `bun run check`. This must pass with F1a's code, because verdict severity is typed as `string`.

**Dependencies:** B2, B3, F1a. **Parallel:** with F1b, F3a, F4, F5, F6, B4.

## B4 — Retry one job now
**Goal:** the inspector's "Retry now" skips a waiting job's backoff.

**Read:**
- `developer.html?state=job` and the notes item "Retry one job now".
- `crates/app-infra/src/processing/store.rs`: `expire_processing_job_retry_backoff_for_test` at :2058-2069, the claim predicate at :1466/1505/1561.
- `crates/app-infra/src/lib.rs`: the reprocess fns at :1157-1250.
- `src-tauri/src/app_infra.rs`: `get_processing_job` at :5106, `ProcessingJobDto` at :693-713.
- `lib/types/app-infra.ts` (`ProcessingJobDto`).

**Build:**
- Promote the test fn to `pub fn clear_processing_job_backoff(job_id) -> Result<bool>`: `UPDATE processing_jobs SET next_attempt_at = NULL, updated_at = CURRENT_TIMESTAMP WHERE id = ?1 AND status = 'queued'`. Keep the test helper as a thin call to it.
- Add an `AppInfra` wrapper.
- Add `#[tauri::command] retry_processing_job_now(job_id: i64) -> Result<ProcessingJobDto, String>`, registered in `lib.rs`.
  - It errors with a plain message ("Only a waiting job can be retried now. Use Reprocess for a failed job.") when no row was updated.
  - It returns the refreshed DTO.
  - **Failed jobs keep using the existing `reprocess_*` commands.**
- One Info log line: `processing job {id} retry requested from developer window`. It is user-initiated, so it's a recognisable state change.
- TS: add the missing `modelLocked: boolean` to `ProcessingJobDto`; the Rust DTO already sends it.
- Add a store test: a queued job with a future `next_attempt_at` becomes claimable, and a failed job is untouched.

**Don't:**
- Reset `failure_count` or `attempt_count`.
- Change the reprocess semantics.
- Touch the model lock.

**Files:** `crates/app-infra/src/processing/store.rs`, `crates/app-infra/src/lib.rs`, `src-tauri/src/app_infra.rs`, `src-tauri/src/lib.rs`, `lib/types/app-infra.ts`. About **120 lines**.

**Verify:**
- `cargo test -p app-infra retry`
- desktop `cargo check`
- `bun run check`

**Dependencies:** B3 (same files), and B2 must be finished (`lib.rs`). **Parallel:** with B1 and every tier-1 F slice.

## F1b — Strip actions: keep on top, mini mode, ⋯ menu, Copy diagnostics
**Goal:** the right side of the strip matches the mockup (pause · pin · mini · ⋯), and the window really pins and shrinks.

**Read:**
- `developer.html`: `.dv-strip__acts`, `#dv-menu`, the mini frame `.dv-mini` + strip, and the click handlers at the bottom of the script. `?state=overview-issues` (open the ⋯ menu) and `?state=mini`.
- `lib/shell/ShortcutsPop.svelte` (the popover pattern).
- `lib/surface-windows.ts`.

**Build:**
- **Pin:** `aria-pressed` toggle, `getCurrentWindow().setAlwaysOnTop(v)`, tooltip "Keep on top".
- **Mini** (`controller.mode = "mini"`):
  - Remember the current outer size and the pin state.
  - `setMinSize(new LogicalSize(300, 120))`, `setSize(new LogicalSize(300, 120))`, `setAlwaysOnTop(true)`.
- **Expand** (clicking anywhere on the mini window, or its expand button):
  - `setMinSize(440, 480)`, restore the size, restore the pin state.
  - **Order matters:** lower the min before shrinking, and restore the size before raising the min.
- **Mini strip:** a page-local 28px strip inside `MiniView`'s frame (smaller lights, "Developer", pin glyph, expand button), per the mockup. `MiniView` content is F2's; F1b owns only the frame and the strip wrapper. Put that wrapper in `DevStrip.svelte` as a `compact` variant.
- **⋯ menu** (`mx-pop` + `mx-menu`):
  - **Copy diagnostics** `⇧⌘C` (also a window keydown):
    - App version.
    - The OS from `@tauri-apps/plugin-os` if it's already a dependency. Otherwise skip it.
    - The 12 health rows (feature, severity, reason, metric).
    - Pipeline status per processor plus the semantic index counts.
    - The last 200 lines of `tail_app_log({file:"rust", lines:200})`.
    - Write it as plain text with `navigator.clipboard.writeText` (the `clipboard-manager:allow-write-text` capability exists if the web API is refused).
    - Show a toast: "Diagnostics copied · health, versions, queues, last 200 log lines".
    - **Never include** the privacy snapshot (window titles or URLs) or the excluded-app list.
    - Put it in `lib/debug/diagnostics.ts` with a unit test on the formatter.
  - **Reveal logs in Finder:** `revealItemInDir(path)` from `@tauri-apps/plugin-opener`, using the `path` from a `tail_app_log` result. `opener:default` already allows it.
  - **Open app log:** `invoke("open_general_app_log")`.

**Don't:**
- Add Rust commands.
- Put Record/Stop in the strip (it lives once, in the Capture tab).
- Build `MiniView` content.

**Files:** `lib/debug/DevStrip.svelte`, `lib/debug/diagnostics.ts` (+ test), `lib/debug/window-mode.ts`. About **300 lines**.

**Verify:**
- `bun run check`, `bun test apps/desktop/src/lib/debug`.
- Render: the strip with the menu open (stub `data-open`), pin pressed, and paused (verdict shows "paused at …").
- Stub `plugin:window|set_size` / `set_min_size` / `set_always_on_top` in the IPC stub and assert they are called in the right order when toggling mini.
- Real-app smoke, if you can run `bun run tauri -- dev`: ⌘D, pin, mini, expand.

**Dependencies:** F1a, W1. **Parallel:** with B1, B4, F3a, F4, F5, F6.

## F3a — Pipeline tab: flow, processor rows, housekeeping
**Goal:** the Pipeline tab as drawn.
- **Flow strip:** 5 nodes (Capture · Reading · Transcription · Speakers · Meaning). Each shows a big number, a unit, 1–2 lines, and a backlog bar that fills toward 50 jobs, with node tone for warn/danger/info.
- **Processors:** a `.dv-list` of 4 `HealthRow`s in `plain` mode. Each shows the provider/model in plain words, the average time and a chevron, and calls `controller.openProcessor(key)`.
- **Housekeeping:** a `dl.mx-spec` with retention cleanup, frame batches, model locks and app jobs.

**Read:**
- `developer.html`: `.dv-flow`/`.dv-node` CSS, `FLOW` and `PROCS`/`AVG` in the script, and the Housekeeping markup. Render `?state=pipeline` (the issues scenario) and `?state=not-recording` → Pipeline tab.
- `lib/debug/state/features.svelte.ts` (`laneFor`, semantic, config loaders).
- `state/pipeline.svelte.ts`.
- `lib/types/debug.ts` (`ProcessorPipelineStatus`, `SemanticIndexStatusDto`), `lib/types/app-infra.ts` (`AppJobDto`, retention status type).
- `detail/specs.ts` (`ProcessorKey`).

**Build:**
- **`PipelineTab.svelte`:** poll `features`' pipeline-status + semantic fetch at 2 s via `controller.poll`, and housekeeping on mount plus every 30 s.
- **Node numbers:**
  - Capture: "N sources live", from `capture.captureSession` (requested and running sources). "142 files today" ships without it; there's no source for it.
  - Lanes: waiting = queued + retrying; lines "N running" / "N retrying" (warn) / "N failed" (`failedLast24h`, danger).
  - Meaning: `backlogCount` "behind", `vectorCount` "done".
  - Bar = min(waiting/50, 1).
- **Processor `why`:** provider and engine in plain words, from `features`' model and transcription config, for example "Parakeet, on this Mac" or "Deepgram, cloud". When a lane has `retrying > 0` and a cloud provider, use "<Provider>, cloud · can’t be reached". Metric = `averageCompletedSecondsLast24h` as "6.2 s avg". The Meaning row shows "nomic-embed, on this Mac" and no avg if it has none.
- **Housekeeping** (add fetchers to `state/pipeline.svelte.ts`):
  - **Retention cleanup:** `get_retention_cleanup_status`, giving "removed X older than N days" plus the time.
  - **Frame batches:** `list_frame_batches`, giving "N written today, none failed" or "N failed · <first error line>".
  - **App jobs:** `list_app_jobs`, giving "N running · <kind label>". Label it "App jobs" so it's never confused with processing jobs.
  - **Model locks:** no command exists. **Ships without it:** omit the row. Add it only if a later slice exposes `list_locked_processing_model_keys`.
- Drill-in: when `controller.nav.processor` is set, render `<ProcessorView/>` instead. That file is F3b's; create it as a stub "Coming in F3b" if absent.

**Don't:**
- Draw an attempt timeline.
- Edit `features.svelte.ts` (F5 owns it). Call its existing fetchers. If a fetcher is missing, add a local one in your tab file.
- Touch `detail.svelte.ts` (F3b).

**Files:** `lib/debug/tabs/PipelineTab.svelte`, `lib/debug/pipeline-view.ts` (pure node and row mapping + `pipeline-view.test.ts`), `state/pipeline.svelte.ts`, `tabs/ProcessorView.svelte` (stub only). About **450 lines**.

**Verify:**
- `bun run check`, `bun test apps/desktop/src/lib/debug`.
- Render the issues, ok and not-recording stub data against `?state=pipeline` (switch the mockup scenario through `overview-ok` → Pipeline tab and `not-recording` → Pipeline tab), dark and light, at 520×640 and at the 440 min width (the flow must not overflow).

**Dependencies:** F1a. **Parallel:** with B1, B4, F1b, F4, F5, F6.

## F4 — Capture tab (and the macOS suspension-reason fix, frontend half)
**Goal:**
- A run line: dot, "Recording" / "Not recording", "2h 14m · since 12:17", and the **only** Record/Stop button in the window.
- Three lanes: icon in the source colour, name, a state word in tone plus a detail, and on the right "every 2 s" for screen or a 12-bar meter plus dB for audio.
- The display-asleep info notice.
- The not-recording empty state.
- The "System audio connection" spec.
- "Kept out of recordings": app chips with a letter avatar and **app names, never bundle ids**, plus the web address mode line.
- Disk: a headroom bar on a 0–25 GB scale with pause and resume ticks and a legend, plus the "Mnema uses …" line.

**Read:**
- `developer.html`: Capture markup and CSS (`.dv-run`, `.dv-lane*`, `.dv-meter`, `.dv-apps`, `.dv-pline`, `.dv-head*`, `.dv-uses`). States `?state=capture`, `?state=capture-asleep`, `?state=not-recording` → Capture, `?state=overview-issues` → Capture (system audio reconnecting).
- `state/capture.svelte.ts` (start/stop with the session-generation guard, `recording_settings_changed`).
- B2's `CaptureHealthDto` in `lib/types/debug.ts`.
- `lib/types/recording.ts` (`ExcludedAppEntry.displayName`, `BrowserUrlMode`).
- `StorageUsage` in `lib/types/app-infra.ts`.

**Build:**
- In `state/capture.svelte.ts`:
  - Add `captureHealth` and `fetchCaptureHealth()` (`get_capture_health`).
  - **Delete** `startIdlePolling` and the `get_idle_debug` / `get_capture_privacy_debug` fetchers, if nothing else uses them (grep).
  - Keep start/stop, permissions, reconcile, wake resync and the listeners. The shell (F1a) decides which of reconcile and wake still run. Run reconcile only from this tab via `controller.poll(…, 5000)` while recording.
- **`CaptureTab.svelte`:** poll `get_capture_health` at 1 s. Load `get_storage_usage` on mount. Read privacy from `capture.recordingSettings`, which is kept fresh by the existing event.
- **Lane state word** from `state`:

| state | word | tone |
|---|---|---|
| recording | Recording | — |
| paused | Paused | info |
| waiting | Waiting | info |
| reconnecting | Reconnecting | warn |
| failed | Failed | danger |
| off | Off | `--off` look |

- **Lane detail** = device name, or the plain reason:
  - `capture_display_unavailable` → "display asleep"
  - `privacy_filter_apply_failed` → "privacy filter retrying"
  - `privacy_recovery_restart_required` → "privacy filter needs a restart"
  - `capture_low_disk` → "disk almost full"
  Plus "· 48 kHz" from the tap sample rate for system audio. Put the reason map in `capture-view.ts` with a test. **This is the frontend half of the bug fix:** the reason renders on macOS whatever `sessionActive` says.
- Screen right column: "every N s" from recording settings; while waiting, "N min" since the suspension started. Meter = 12 bars; lit = clamp((levelDb + 60) / 60 · 12).
- **Notice** (`mx-notice data-tone="info"`, monitor icon), only for `displayUnavailable`:
  - Title: "Resumes on its own when the display wakes".
  - Text: "Since HH:MM, checked N times. The last few minutes were saved; microphone and system audio keep recording."
  - Use `recoveryAttempts` and `sinceMs`. Drop "since" if null.
- **Low-disk and privacy suspensions:**
  - Low disk: a `mx-notice` with warn tone: "Recording paused until X GB is free".
  - Privacy `restartRequired`: danger tone, with a Stop button hint.
  - These aren't drawn in the mockup, so use the same pattern.
- **System audio connection** (only while recording and `systemAudioTap` is non-null):
  - "Rebuilt today: N times", note "each opens a new file". While reconnecting, append "· rebuilding now".
  - "Last reason: <plain reason>, <lastDetail>" plus the time.
  - "Silence check: next in N s", note "every <watchdogDelay>".
- **Kept out of recordings:** count = enabled entries; chips use `displayName` and its first letter. Web addresses: "off" → "not saved", "sanitized" → "saved without query strings", "full" → "saved in full".
- **Disk bar:**
  - `--v = min(free / 25 GB, 1)`; ticks at pause and resume (fall back to the floor only if null). Legend "0 · pause X · resume Y · 25 GB +".
  - `aria-label` "182 GB free; recording pauses below 5 GB and resumes at 10 GB".
  - Tone warn under resume.
  - "Mnema uses X: screen · audio (mic + system) · meaning index · models" from `StorageUsage`.
- **Record/Stop:** `capture.start()` / `capture.stop()`. The tooltip shows the effective ⌥⌘R binding (`getEffectiveGlobalShortcut("toggleRecording")`).

**Don't:**
- Show bundle ids, window titles or URLs. The old privacy snapshot JSON is gone for good.
- Show mic or system-audio levels anywhere but here.
- Duplicate Record/Stop elsewhere.

**Files:** `lib/debug/tabs/CaptureTab.svelte` (split into `CaptureLanes.svelte` + `CaptureTab.svelte` if it goes past about 400 lines), `lib/debug/capture-view.ts` (+ test), `state/capture.svelte.ts`. About **500 lines**.

**Verify:**
- `bun run check`, `bun test apps/desktop/src/lib/debug`.
- Render the four Capture states (ok, asleep, issues, off) dark and light against the mockup.
- Make sure the asleep render shows "Waiting · display asleep" on the screen lane.

**Dependencies:** F1a, B2. **Parallel:** with B1, B4, F1b, F3a, F5, F6.

## F5 — AI tab
**Goal:**
- An engine box: dot, "AI engine is available" or the plain reason, provider plus model in mono, and "Test connection".
- "Last Ask AI turn": four `mx-stat`s (Read, Wrote, Tool calls, Took), with the time and door on the header.
- "User Context": the digest freshness and build time on the header, "Derive latest now", and a `.dv-log` list of recent runs (window, status chip, result, tokens and time).
- "CLI access": "N tools allowed" on the header, and a `.dv-log` of recent requests (tool name, command in mono, allowed/refused chip, time).

**Read:**
- `developer.html`: AI markup and CSS (`.dv-engine`, `.dv-stats`, `.dv-log`). `?state=ai`.
- `state/features.svelte.ts` (`testAiConnection`, `runDerivationNow`, the config loaders).
- `lib/types/recording.ts` (`AiRuntimeStatus`, `UserContextStatus`), `lib/types/debug.ts` (`DerivationRun`, `AskAiTokenUsage`).
- `src-tauri/src/cli_access.rs` (`list_cli_access_history` → `BrokerAuditFile { events }`, `list_cli_access_grants`). Find or add the TS types next to the existing access-window types (grep `BrokerAudit` in `lib/`).
- `SHELL.md` "Engine status" table, for reason-code copy.

**Build:**
- In `features.svelte.ts`:
  - Remove any leftover internal timer.
  - Expose a `fetchAiLive()` covering last turn, runs (limit 4) and CLI history (latest 4).
  - Polled by `AiTab` at 5 s. Config loads on mount.
- **Engine reason copy:** follow SHELL.md's table.
  - `provider_unreachable:*` → warn "can’t be reached, resumes on its own", **never** "reconnect".
  - `needs_reconnect:*` → danger.
  - Disabled → off "AI is turned off".
- **Test connection:** a spinner while running, then the result inline as a quiet line under the box. Never show `rawJson`.
- **Stats:**
  - Read = `inputTokens` shown as "18.2k".
  - Wrote = `outputTokens`.
  - Tool calls.
  - Took = the duration (use the DTO's end/duration field; check the type).
  - The header shows the time plus the door ("Chat" / "Quick Recall") if the DTO carries it. Otherwise just the time.
  - Empty state: "No Ask AI turn since the app started".
- **Runs:**
  - The kind is "Activities" or "Conclusions" (map the run's stage field).
  - Chips: done, skipped, failed (danger).
  - Result: "6 activities", or the gate drop as "2 dropped as idle".
  - Tokens are mono.
- **Derive latest now:** a ghost button with a spinner. Refresh the list after it finishes.
- **CLI:** tool display name, command, outcome chip (refused = warn), time. The header count comes from the grants (allowed, not blocked).
- **ChatGPT token expiry is not drawn.** It ships without it; don't add it.

**Don't:**
- Call `resolve_engine_config_live` paths in a poll.
- Show raw JSON.
- Add a "default model" picker.

**Files:** `lib/debug/tabs/AiTab.svelte`, `lib/debug/ai-view.ts` (+ test for the reason-code → copy/tone map), `state/features.svelte.ts`, plus `lib/types/*` only if the CLI audit TS type truly doesn't exist (then add it to `lib/types/debug.ts`). **Coordinate:** B1/B2 own `types/debug.ts` in their tiers, so append at the end of the file in a separate block. About **380 lines**.

**Verify:**
- `bun run check`, `bun test apps/desktop/src/lib/debug`.
- Render `?state=ai` dark and light, plus an engine-offline variant and a no-turn-yet variant.

**Dependencies:** F1a. **Parallel:** with B1, B4, F1b, F3a, F4, F6.

## F6 — Logs tab
**Goal:** the tail fills the tab.
- Bar row 1: source segment "App log | Capture log" and level segment "All | Warnings | Errors".
- Bar row 2: a "Filter lines" `.mx-search`, a "Follow" switch, and Reveal in Finder.
- A horizontally scrolling chip row of features, "Everything" plus 9.
- Lines in mono grid columns `time · level · message`, with a bold prefix.
- WARN and ERROR rows tinted with tone backgrounds (no side bars). Click a line to copy it (the " copied" suffix for 1.2 s).
- The footer "Showing the last 500 lines · following".
- An empty state "No matching lines".

**Read:**
- `developer.html`: the Logs CSS (`.dv-logbar*`, `.dv-follow`, `.dv-feats`, `.dv-tail`, `.dv-line*`), `renderLogs()` and its click-to-copy. `?state=logs`.
- `state/logs.svelte.ts`, `log-filter.ts` (`LOG_FEATURE_PATTERNS`, keyed by `LogFeature` after F1a).
- A real line from `~/Library/Logs/day.mnema/rust.log`: `[2026-10-13][15:01:26][mnema_lib::native_capture::debug_log][INFO] message…`.

**Build:**
- **`log-line.ts`** (+ test): `parseLogLine(raw) → { time, level, target, message } | null`. It returns null for continuation lines; render those as message-only rows.
  - **rust.log timestamps are UTC.** Convert to local `HH:MM:SS`, keeping millis if present.
  - The prefix = the message's own leading tag (`system-audio-tap:`, `[app-infra]`, …) if present. Otherwise the last segment of `target`.
- Level filter: Warnings = WARN + ERROR; Errors = ERROR.
- The feature chips use `LOG_FEATURE_PATTERNS` keys. Their plain labels are Capture, Privacy, Reading, Transcription, Speakers, Meaning, AI, User Context, Jobs.
- Needle: initialise the filter from `controller.nav.logNeedle`. That's how the inspector's "Show in log" lands here.
- **Follow** on: stick to the bottom on each poll. Follow off: freeze the scroll position. The tail polls at 2 s via `controller.poll`; switching the file refetches at once.
- Reveal in Finder: `revealItemInDir(tail.path)`.
- Missing file (`exists:false`): a calm `mx-empty` "Nothing logged yet", and keep polling.

**Don't:**
- Add level parsing to `log-filter.ts` (F1a owns it; reading it is fine).
- Show raw JSON.
- Poll faster than 2 s.

**Files:** `lib/debug/tabs/LogsTab.svelte`, `lib/debug/log-line.ts` (+ test), `state/logs.svelte.ts`. About **400 lines**.

**Verify:**
- `bun run check`, `bun test apps/desktop/src/lib/debug`.
- Render `?state=logs` with the mockup's APP_LOG lines as stub `tail_app_log` output (written in the real rust.log format), dark and light.
- Also render the filtered-to-nothing empty state, and the window at its min height 480, where the tail must still fill the tab.

**Dependencies:** F1a. **Parallel:** with B1, B4, F1b, F3a, F4, F5.

---

# Tier 2

## F2 — Overview, not-recording state, mini content
**Goal:**
- **Overview:** the 12 rows grouped under "Needs a look" (warn and danger), "Right now" (info and off) and "Running fine" (ok). Each group has a count on its header, and groups with no rows are hidden. Within a group, rows sort by tone rank (danger, warn, info, off, ok). Every row drills into its tab.
- **Mini:** eight dots (Screen · Mic · Sys audio · Reading · Transcription · Speakers · Meaning · AI), then a footer line with the worst row as "Name: why" (or the verdict text) and "N queued".

**Read:**
- `developer.html`: `renderOverview`, `RANK`, `renderMini`, the `.dv-mini*` CSS. States `?state=overview-ok`, `overview-issues`, `not-recording`, `mini`.
- `lib/debug/HealthRow.svelte`, `verdict.ts`.
- B1's types.

**Build:**
- **`health-view.ts`** (+ test):
  - `FEATURE_VIEW: Record<DebugFeature, { name, mini?: string, tab }>`. The names are the mockup's plain names.
  - Tabs: capture rows and Disk → `capture`; the four processors → `pipeline`; AI engine, User Context and CLI → `ai`; Startup → `logs`.
  - A processor row with warn or error opens its drill-in directly (`openProcessor`) instead of only the tab.
  - `group(rows)` and `toneOf(severity)`.
  - Metric: `metricAtMs` ? local "HH:MM" (AI engine prefixes "used ") : `metric`.
- **`OverviewTab.svelte`:** replace F1a's stub with the grouped sections, `.dv-sec` style gap 18px.
- **Not-recording:** nothing special beyond B1's rows (off rows read "Not recording" with a hollow dot under "Right now").
- **`MiniView.svelte`:**
  - The dots and footer per the mockup.
  - It polls `get_processing_pipeline_status` at 2 s for "N queued" (sum of queued + retrying + running across lanes, plus the semantic backlog only if already fetched; don't add a call for it).
  - The whole body is a button that calls `controller` expand (F1b's function).

**Don't:**
- Show any row twice.
- Show levels or Record/Stop here.
- Change `HealthRow` (F1a's). Wrap it instead.

**Files:** `lib/debug/tabs/OverviewTab.svelte`, `lib/debug/MiniView.svelte`, `lib/debug/health-view.ts` (+ test). About **300 lines**.

**Verify:**
- `bun run check`, `bun test apps/desktop/src/lib/debug`.
- Render the four states dark and light: full at 520×640, and mini at 300×120 against `#dv-mini`.
- Mini check: eight dots fit on two rows with no clipping and the footer ellipsizes.

**Dependencies:** B1, F1a (and F1b, for the expand call; stub it if F1b isn't merged yet). **Parallel:** with F3b.

## F3b — Processor drill-in, jobs table, job inspector (and the open-in-timeline fix)
**Goal:**
- **Breadcrumb:** back button + "Pipeline › Transcription".
- **Spec:**
  - Average time ("6.2 s per 5-minute segment" for transcription, "per frame" for reading)
  - Failed ("N of M", last 24 h; if there's no total, "N in the last 24 h")
  - Provider ("Deepgram · cloud", note "your key" for BYO-key providers)
  - Model
- **Provider-unreachable notice** (warn, cloud-off icon) with "Test":
  - It shows when the lane has retrying jobs, a cloud provider, and the last error looks transient.
  - Copy: "Timed out at HH:MM. N jobs wait and retry on their own; waiting doesn’t count as a failed try."
  - "Test" uses `features.testDeepgram()` for Deepgram and `ai_runtime_test_connection` otherwise.
- **Filter `.mx-seg`:** All · Queued · Running · Retrying · Failed, with mono counts.
- **Jobs table:** subgrid columns Job (`#id` mono) · Recording · Status chip · Tries · Updated.
  - Under each row is a message line: "Next try in 42 s · <reason>" for retrying, the error first line for failed (danger text), "Waiting behind N retries" for queued.
  - Then "N more · same error" when the page is truncated.
- **Inspector** (replaces the list, Back returns):
  - `#id` in mono h3 plus a chip, and the subtitle "Transcription of the microphone, 09:55–10:00 today".
  - A danger notice: a plain title plus the raw error in `.dv-raw-err` mono.
  - Spec: Created, Last try, Tries ("N tries, N failed"), Provider.
  - Actions:
    - **Open in Timeline** (primary)
    - **Reprocess** (failed or completed)
    - **Retry now** (only for queued with a backoff, i.e. "retrying")
    - spacer
    - **Show in log** icon (`controller.showInLog("job " + id)`, switches to the Logs tab)
    - **Copy id** icon
  - A folded `details.dv-raw` "Raw payload" with the byte size. JSON appears **only** here.
- For **Meaning index** (no jobs), the drill-in shows only the semantic-index spec: passages searchable, behind, dimension, model loaded, load failures, quarantined, and the last load error inside a danger notice.

**Read:**
- `developer.html`: `.dv-crumb`, `.dv-filter`, `.dv-jobs*`, `.dv-job*`, `.dv-insp*`, `.dv-raw*`, `.dv-acts`. States `?state=processor` and `?state=job`.
- `state/detail.svelte.ts` (server-side status filter, stale-response guards, `reprocessSelected`), `detail/jobs.ts` (`jobState`, `nextAttemptLabel`, `jobProvider`), `detail/specs.ts`.
- Old inspector for behaviour only: `git show c258f625:apps/desktop/src/lib/debug/detail/JobInspector.svelte`.
- `src-tauri/src/lib.rs:116-143` (`open_capture_result_in_main_window`). Also look at how Quick Recall calls it (`lib/quick-recall/detailStore.svelte.ts:211`).

**Build:**
- In `detail.svelte.ts`:
  - Drop the "push a view inside /debug" nav. `controller.nav` owns it now.
  - Keep the job page, filter, selection and reprocess.
  - Poll via `controller.poll` at 2 s while `ProcessorView` is mounted.
  - Add `retryNow(id)` → `invoke("retry_processing_job_now", { jobId })`.
- **Recording column:**
  - From `subjectType` + `subjectId`: "Frame #918244" or "Segment #2291".
  - If the payload JSON carries the source and time range (check `jobProvider`'s sibling parsing), show "Microphone · 14:25–14:30".
  - Ships without enrichment: the plain id label.
- **Plain error titles:** a small map in `jobs.ts` (+ test) for the few known shapes:
  - transient connectivity → "<Provider> can’t be reached"
  - decode/moov/truncated → "The recording was cut off"
  - model missing → "The model isn’t installed"
  - anything else → "This job failed"
  The raw error always shows below in mono.
- **Open in Timeline:**
  - `invoke("open_capture_result_in_main_window", { kind, frameId, audioSegmentId, spanStartMs: null, alignedFrameId: null, query: null, byMeaning: null })`.
  - **No `goto`** in this window. This is the second known bug; the debug window itself must stay put.
- **Copy id:** `navigator.clipboard.writeText(String(id))`, with a tooltip flash "Copied".

**Don't:**
- Draw per-attempt history (retries update the row in place).
- Show the payload outside "Raw payload".
- Offer Retry now on failed jobs.
- Navigate the debug window to `/`.

**Files:** `lib/debug/tabs/ProcessorView.svelte`, `lib/debug/tabs/JobInspector.svelte`, `state/detail.svelte.ts`, `detail/jobs.ts` (+ tests), `detail/specs.ts`. About **600 lines**. Split the jobs table into `tabs/JobsTable.svelte` if `ProcessorView` passes about 350 lines.

**Verify:**
- `bun run check`, `bun test apps/desktop/src/lib/debug`.
- Render `?state=processor` and `?state=job` dark and light, at 520 and 440 widths (the table must not overflow; subject ellipsizes).
- In the stubbed IPC, assert that Open in Timeline invokes the command and that `location.pathname` stays `/debug`.

**Dependencies:** F3a, B4. **Parallel:** with F2.

---

# Tier 3

## X1 — Integration, dead code, full render pass
**Goal:** one coherent window, with no dead code or stale copy, matching every mockup state.

**Read:**
- `developer.html` (every `?state=`).
- This plan's polling budget.
- `git diff c258f625 --stat`.

**Build:**
- **Dead code:**
  - Grep for and remove anything under `lib/debug/**` and `lib/types/debug.ts` that nothing imports. Candidates: `pipeline.svelte.ts` app-infra-status bits, `features.svelte.ts` model-status loaders no tab uses, `IdleDebugInfo`/`CapturePrivacyDebugInfo`/`OcrBudgetDebug` TS types if unused.
  - Check whether any frontend still calls `get_idle_debug`, `get_capture_privacy_debug`, `get_ocr_budget_debug`, `submit_debug_cpu_job` or `classify_hidden_segment_workspace`. For each command no frontend calls (`grep -rn` across `apps/desktop/src`), **list it in the report and ask the founder before deleting the Rust command**. Some back the CLI or tests.
- **Stale copy:**
  - Grep the app for "Debug window", "Debug page", "debug surface", "live · 1s poll", "mnema · Debug", "Jobs & Storage", "anchors w/o vector", "detector source", "SQLCipher" in user-facing strings.
  - Fix them to "Developer" wording.
  - Delete `docs/agents/mockups/debug/` (superseded by `developer.html`). Grep for links to it first.
- **Hoist** any `dv-*` rule duplicated in two tab `<style>` blocks into `developer.css`.
- **Polling audit:** with the IPC stub counting calls, sit 10 s on each tab and in mini. Report calls/s per view against the budget table, then pause and confirm zero.
- **SUPPORTS.md:** if it mentions the debug window or Linux/Windows always-on-top behaviour, update it.
- **Full render pass:** dark and light, against the mockup's matching state. Fix every mismatch or list it with a reason. This covers all 11 states:
  - overview-ok
  - overview-issues
  - pipeline
  - processor
  - job
  - capture
  - capture-asleep
  - ai
  - logs
  - mini
  - not-recording
  Also render the 440×480 minimum and the ⋯ menu open.
- **Real app, if runnable** (`bun run tauri -- dev`):
  - ⌘D opens it at 520×640.
  - Settings › Developer "Open" works and is disabled with dev options off.
  - Pin, mini and expand work.
  - Open in Timeline drives the main window.
  - Escape backs out of a drill-in, then closes.

**Don't:**
- Redesign anything.
- Touch the main-window status bar or popover.
- Delete Rust commands without the founder's yes.

**Files:** whatever the sweep finds. Expect under 300 changed lines plus deletions.

**Verify:**
- All standing commands, plus `cargo test … --lib debug_health`, `cargo test … --lib windows::tests` and `bun test apps/desktop/src/lib/debug`.
- The render report with every PNG pair.

**Dependencies:** all slices.

---

## Bug ledger (where each known bug is fixed)

| Bug | Root cause | Fixed in |
|---|---|---|
| macOS screen suspension reason never renders | `CaptureSourcesSection.svelte:266/281` and `format.ts:132` read `reason` only when `sessionActive === null`, but macOS always sends a bool. `capture_health` also reports any suspension as "Recording / Ok" (`session_reports_running`). | B2 always carries the reason and suspension. B1's Screen row goes info or warn. F4's lane reads `state` + `reason`. The old section is deleted in F1a. |
| "Open in Timeline" navigates the debug window | `JobInspector.svelte:58-74` calls `goto("/")` after the (correct) `open_capture_result_in_main_window` | F3b (no `goto`; asserted in the render test). The old file is deleted in F1a. |
| `ProcessingJobDto` TS is missing `modelLocked` | hand mirror drift | B4 |

## Optional rows: what ships without them

| Row | Ships without it as |
|---|---|
| Pipeline › "142 files today" (Capture node) | the node shows only "N sources live" |
| Housekeeping › Model locks | the row is omitted (no command; add one later from `list_locked_processing_model_keys`) |
| Jobs table "Microphone · 14:25–14:30" | "Segment #2291" / "Frame #918244" |
| Lane device names ("MacBook Pro Microphone", "AirPods Pro") | the state word alone |
| System-audio "48 → 16 kHz" detail | the plain reason alone |
| Capture notice "since 14:21" | "Checked N times" alone |
| Disk pause/resume ticks while not recording | the reserve-floor tick only |
| Processor "Failed N of M" | "N in the last 24 h" |
| Startup row aggregate-device reap count | not in the report (it runs in `lib.rs` after the report is stored) |
| ChatGPT sign-in expiry | not drawn (per mockup) |
| AI "last turn" door ("Chat") | the time only |

## Decisions for the founder (only what the mockup leaves open)
1. **"Retry now" on a failed job.** The mockup shows it next to Reprocess on a failed job. For a failed job, both would do the same thing, so the plan offers Retry now only on a waiting (backoff) job, and Reprocess on failed or completed jobs. If you want it on failed jobs too, it becomes a second Reprocess button.
2. **Titlebar bug icon removal (F8).** The brief says the redesign drops it, but the redesigned titlebar still has it. The plan removes it, which leaves ⌘D and Settings as the only ways in until the status-popover footer link ships. If you want it kept until then, F8 drops that bullet.
