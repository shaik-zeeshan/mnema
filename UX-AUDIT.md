# UX & Interaction-Feedback Audit

Across ten surfaces, the Mnema desktop app is genuinely strong on the happy path: status transitions, loading states, empty states, double-fire guards, and keyboard/focus handling are well-built almost everywhere, and 89 individual flows were verified to give correct feedback. The audit confirmed **63 findings** — **1 critical, 2 high, 15 medium, 45 low** — and four candidate issues were investigated and refuted as false positives. The dominant theme is exactly the one flagged as the top concern: **silent failure on user-triggered actions**. The single highest-value defect is that every recording lifecycle action (start/stop/pause/resume) funnels failures into a `captureControls.error` field that is set in five places but **rendered nowhere**, so the most common real failures — permission denied, display unavailable, source-validation rejection — leave the Record button snapping back with zero explanation. The remaining critical/high items and a large share of the mediums are variants of the same pattern: a failed write or hand-off that the user cannot perceive, sometimes compounded by an error being routed into a *misleading* surface (a failed pin re-rendered as a full "Couldn't load" card).

## Severity scoreboard

| Severity | Count |
|---|---|
| Critical | 1 |
| High | 2 |
| Medium | 15 |
| Low | 45 |
| **Total findings** | **63** |
| Flows verified clean | 89 |
| Candidate issues refuted as false-positives | 4 |

## Critical & High findings

### [CRITICAL] App Shell + Recording Lifecycle — Recording start/stop/pause failures are invisible: the error is set but never rendered

- **Where:** `apps/desktop/src/lib/capture-controls.svelte.ts:138,190,210,233,247` (error set); `routes/+layout.svelte` (never read). Backend non-notifying reject at `native_capture.rs:1888`.
- **Trigger:** Click Record/Stop/Pause/Resume, or press the toggle/pause global shortcut, when the backend command rejects — permission denied, display unavailable, generic source-validation rejection.
- **What the user experiences:** The button label snaps back from "Starting…" to "Record" and nothing explains why recording did not start. The user cannot distinguish "it's recording" from "it silently failed."
- **Why it matters:** This is the canonical visibility-of-system-status / no-silent-failure violation, on the app's single most important action. `captureControls.error` (getter at `capture-controls.svelte.ts:64-66`) has **zero consumers** repo-wide; the backend only pushes notifications for the OCR/speech-detector preflight branches (`native_capture.rs:1916,1961`), so the common permission/display/source-validation path (`:1888`) has no UI signal at all.
- **Fix:** Render `captureControls.error` in the title bar as an inline error chip beside the status pill (`role="alert"`/`aria-live="assertive"`), or route lifecycle failures through `pushAppNotification` to the existing bell. Clear on the next successful `applyCaptureSession` (already done at line 168).

### [HIGH] Insights — A failed pin/dismiss wipes the entire Subjects list and shows a misleading "Couldn't load Subjects"

- **Where:** `apps/desktop/src/lib/insights/Subjects.svelte:329-330,344-345` (catch), rendered at `938-951`.
- **Trigger:** Click Pin or Dismiss on a conclusion inside an expanded subject row, and the write fails.
- **What the user experiences:** The whole loaded, scrolled, possibly-expanded list is replaced by a full-screen "Couldn't load Subjects." card whose "Try again" re-runs the *initial load*. The user loses their place and every expanded row.
- **Why it matters:** This is worse than a silent failure — it is *misattributed* feedback: a transient write error reads as a catastrophic surface failure, and the destructive list-wipe is the opposite of good status reporting. The `{#if loadError}` branch opens before the rows and is **not** gated on `!conclusions`, unlike `Context.svelte`, which deliberately routes the same case to a `message()` dialog.
- **Fix:** Mirror `Context.svelte`: on pin/dismiss failure show a Tauri `message()` dialog (or a transient inline toast on the row) and leave the list intact; reserve `loadError` for the initial fetch by gating the branch on `!conclusions`.

### [HIGH] Quick Recall — When Ask AI is unavailable, the fix is described but not actionable (a dead end)

- **Where:** `apps/desktop/src/routes/quick-recall/+page.svelte:3708-3719` (disabled button), `3763-3767` (hint `<p>`), `1024-1043` (`friendlyAskReason`).
- **Trigger:** A user with no/misconfigured provider sees the disabled "Ask AI" button and the hint line below it.
- **What the user experiences:** `friendlyAskReason` produces a genuinely actionable sentence ("Add a provider API key in Settings", "Choose a Reasoning Engine model in Settings"), but it renders as a non-interactive `<p class="quick-recall__ask-hint">` while the button is `disabled`. The user is told exactly what to do but given no way to do it from here.
- **Why it matters:** This is the "explained-with-a-fix-path, or a dead end?" test landing on dead end, and it is inconsistent with the sibling semantic-search hint (`3560-3571`), which **is** a button calling `openSemanticSearchSettings()`. `openSettings` is already imported (line 12).
- **Fix:** Make the hint a button (or add an inline "Open Settings" affordance) that calls `openSettings()` routed to the AI/Reasoning-Engine pane, matching the semantic-search hint pattern.

## Medium & Low findings

### Medium (15)

| Surface | Title | Location | One-line fix |
|---|---|---|---|
| Dashboard/Timeline | Broker "open capture in app" silently does nothing on failure | `routes/+page.svelte:6790-6797, 3948-3985` | try/catch the drain; on null `get_frame` show "That capture is no longer available." |
| Dashboard/Timeline | `jumpToFrame` errors written only to popover-scoped `pickerError`, invisible when picker closed | `routes/+page.svelte:6287-6296, 6938-6939` | Route jump failures through the always-visible `frameActionStatus`/app banner; reserve `pickerError` for picker-originated jumps. |
| Dashboard/Timeline | Deleted captures vanish from the timeline with no on-dashboard acknowledgment | `routes/+page.svelte:6739-6771` | Surface "This capture was deleted — showing the nearest frame" when the active frame is in the deleted set. |
| Dashboard/Audio | Transcript/speaker failures render text but are never announced (no `role="alert"`) | `routes/+page.svelte:7776-7778, 8055-8056, 8029-8031, 8004-8005` | Wrap each error `<p>` in `role="alert"`, matching media-error treatment at 7634/7721. |
| Dashboard/Audio | Transcript processing status pill/body not in a live region — async completion invisible to AT | `routes/+page.svelte:7751-7773, 8049-8058` | Put the status pill in `aria-live="polite"` and set `aria-busy` while running. |
| Quick Recall | Opening an external answer link can fail with no user feedback | `quick-recall/+page.svelte:995-1011` | Await `openUrl` in try/catch and surface a failure dialog matching `openCapturedUrl`. |
| Insights | A failed pin/dismiss replaces the whole SubjectDetail with "Couldn't load this subject" | `SubjectDetail.svelte:326-328,341-343` (rendered 406-419) | Route write failures to a dialog/toast; gate `loadError` on `!view`. |
| Insights | Clicking a history row whose load throws silently shows an empty "new chat" pane | `Chat.svelte:310-328` | Track a per-thread load error; render an error state with Retry instead of the empty-thread invite. |
| Insights | Overview category/focus corrections fail as a silent no-op | `Overview.svelte:870-872,890-892` | On failure show a toast and/or apply-then-revert optimistically. |
| Insights | Engine-data load failure misrepresented as the "still learning" empty state, no retry | `Overview.svelte:729-737 → 954-960 → 1412-1426` | Add an `engineError` state mirroring `freeError` with a Retry. |
| Settings | Autosave failure discards the error message — user sees only a generic "save failed" dot | `controller.svelte.ts:735 → SettingsRail.svelte:221-223` | Render `recError` as a dismissible inline banner near the panel with the message + Retry; reconcile the control to last-saved; add backoff to the auto-retry. |
| Settings | Inconsistent busy feedback: some action buttons swap label text only, skip ButtonSpinner/aria-busy | `UserContext.svelte:182-189,211-218; Privacy.svelte:65-67,75-77; Access.svelte:102-109` | Standardize on `<ButtonSpinner/>` + `aria-busy`. |
| Onboarding | Transcription/Speaker model-status load failures have no Retry (OCR/Semantic do) | `TranscriptionBody.svelte:112-113; SpeakersBody.svelte:106-107` | Add a Retry button calling `loadTranscriptionModelStatus()`/`loadSpeakerModelStatus()` with a Retrying… state. |
| Access Request | Watchdog can show a misleading "didn't complete" error after a grant that succeeded | `access/request/+page.svelte:139-146, 168-185` | Resolve approve/cancel on an explicit backend ack; on watchdog fire after a resolved grant, show neutral "Access was granted — you can close this window." |
| Form Controls | Custom resolution/bitrate validation message hidden behind a hover-only tooltip | `FieldWarning.svelte:64-102` (ScreenResolutionControl `:64`, VideoBitrateControl `:52`) | Render the first error message inline/as persistent helper text; reserve hover-tooltip for soft warnings. |

### Low (45)

**App Shell + Recording Lifecycle**
- Source-pill toggle persistence failure is silent · `capture-controls.svelte.ts:389-393` · surface the failure (toast/inline error tone) when a source toggle fails to persist.
- Clearing notifications silently no-ops on backend failure · `notifications.svelte.ts:49-55` · try/catch; keep the item and surface a brief error.
- Privacy-recovery "Restart" button has no in-progress label and fails silently · `+layout.svelte:966-975, 530-536` · add "Restarting…" label and surface stop/restart failure.
- Notification load failure is indistinguishable from "no notifications" · `notifications.svelte.ts:36-42` · retain a recoverable error state instead of collapsing to no-bell.
- `runNotificationAction` swallows failures from openSettings/clear · `+layout.svelte:272-278` · try/catch; don't clear the notification if navigation failed.
- Notifications never show their age/timestamp · `+layout.svelte:1062-1075` · render a relative timestamp from `createdAtUnixMs`.
- Clearing the last notification drops keyboard focus to `<body>` · `+layout.svelte:268-270,1008,742-760` · move focus to a stable nearby control (settings button).

**Dashboard — Timeline**
- Persistent preview decode failure can retry-loop with no terminal error · `routes/+page.svelte:616-641, 4121-4123` · cap retries per frameId; set a terminal `frameActionStatus` error and stop refetching.
- OCR overlay copy/selectability discoverable only by hover; no copy-all · `routes/+page.svelte:7171-7194, 9965-10037` · add a persistent signifier + a "copy all recognized text" action.
- Copy/Download frame actions have no in-flight state · `routes/+page.svelte:3785-3834, 7114-7129` · set a "Copying…/Saving…" status and disable during the async.

**Dashboard — Audio**
- Long transcript poll/media load show static text with no motion — can read as stuck · `routes/+page.svelte:7628-7632, 7762-7763, 8051-8054, 8623-8626` · add a spinner/indeterminate bar on the loading/running pill.
- "missing" transcript state shows pill copy "unavailable", contradicting the Run button · `routes/+page.svelte:7770-7771` · add an explicit "not run"/"ready" pill state.
- Seeking to first transcript segment while paused gives no active-highlight · `routes/+page.svelte:2172-2176, 8012-8024` · resolve active segment from the clicked index, not gated on `currentTime>0`.

**Quick Recall**
- Opening a search result that fails to hand off shows nothing (core action, no error path) · `quick-recall/+page.svelte:266-303, 392-401` · try/catch the invoke; surface an error dialog before/instead of closing.
- "Continue in Chat" fails silently when the hand-off invoke rejects · `quick-recall/+page.svelte:1522-1534` · surface a plugin-dialog error on the catch.
- Copy-answer failure is swallowed — no feedback when clipboard write fails · `quick-recall/+page.svelte:1681-1708` · show a brief failure cue / red flash on the catch.
- Stopping a streaming answer strands it: composer and "Continue in Chat" both vanish · `quick-recall/+page.svelte:1806-1813, 938-940, 1511-1513` · keep the session resident on stop, or render an explicit "Stopped — start a new question" affordance.
- Search result-count changes are not announced to AT · `quick-recall/+page.svelte:3896-4008` · add a visually-hidden `aria-live="polite"` status announcing count/loading/no-matches/error.

**Insights**
- Overview pin/dismiss reverts with no explanation on failure · `Overview.svelte:907-912,921-925` · pair the revert with a brief toast.
- Active conversation row in the rail is signified by text color alone · `RailHistory.svelte:333-336` · add a second signal (accent-bg tint and/or 3px inset bar).
- Answer Source card open-in-timeline fails silently · `Chat.svelte:684-696` · surface a brief dialog/toast on failure.
- Chart-level accessible labels are generic or missing (Sparkline/MiniBars/Timeline) · `Sparkline.svelte:47; MiniBars.svelte:56-80; Timeline.svelte:114-148` · add contextual aria-labels (subject + trend; group/row label+value; fallback caption).

**Settings**
- All autosave feedback lives in the bottom-left rail footer, remote from the control · `SettingsRail.svelte:219-237` · add a per-row/per-control "saved" micro-affordance or a transient toast near the edit.
- User Context "Refresh" button has no loading/disabled state — double-fireable · `UserContext.svelte:190-196` · replace with the `ReloadButton` primitive.
- "Wipe User Context" (most destructive, irreversible) styled as a quiet ghost button · `UserContext.svelte:211-216` · promote to `btn--danger`.
- Storage "Browse" folder picker has no error path · `Storage.svelte:41-61` · add a catch surfacing `storageLocationError`.

**Onboarding**
- Welcome-phase load failure has no retry path — first-run onboarding can become uncompletable · `onboarding-lifecycle.ts:94-98; WelcomeScreen.svelte:68-70` · add a Retry calling `c.load()` to the welcome error banner.
- Storage "Browse…" folder picker swallows a dialog rejection · `StorageBody.svelte:16-32` · add a catch setting `controller.errorMessage`.

**Access Request**
- Request expiry/timeout never shown and not handled when it lapses mid-decision · `access/request/+page.svelte:18, 305-378, 168-185` · show request age/expiry; map expired-request rejection to a specific "no longer valid" message.
- Successful grant gives no positive receipt — the window just disappears · `access/request/+page.svelte:168-185, 395-425` · emit a brief notification/in-window receipt naming tool + scope + expiry.
- Affirmative button label doesn't escalate with consent weight on all-retained grants · `access/request/+page.svelte:411-423, 342-352` · make the label/treatment reflect `isBroadScope` (e.g. "Allow full-history access").

**Debug**
- Start/Stop failure shows no feedback near the button — error lands at page bottom · `debug/+page.svelte:1271-1276, 487/501, 2514-2522` · render an inline error chip in the action-row.
- Manual refresh buttons silently no-op when not recording · `debug/+page.svelte:161,170,200-225,1492-1827` · disable when `!isCapturing` (tooltip) or surface a transient "no active session" note.
- Background reconcile/wake-resync swallow errors — can leave "Recording" status stale · `debug/+page.svelte:635-637, 673-675` · after N consecutive misses, surface a "status may be stale" note.
- Event listener registrations have no rejection handler · `debug/+page.svelte:540-545, 547-564, 685-690` · add `.catch` to each `listen()` chain; set a one-time `lastError` for failure-reporting listeners.
- Submit debug CPU job allows empty inputs while Classify guards them · `debug/+page.svelte:2375 vs 2250` · disable submit on empty fields (pick one validation convention).
- Event tables lack skeleton/loading rows; refresh can't distinguish in-flight from never-loaded · `debug/+page.svelte:2119-2218` · add 3-5 skeleton rows / `aria-busy` on first load.

**Form Controls (design system)**
- Same validation error signalled in two conflicting colors (red border + amber badge) · `ScreenResolutionControl.svelte:57,62,64; VideoBitrateControl.svelte:49,52` · keep border and badge in one semantic family.
- Invalid field not programmatically associated with its error message · `Input.svelte:26-38; Stepper.svelte:79-102; FieldWarning.svelte:9-22` · give FieldWarning a stable id and wire the field's `aria-describedby`/`aria-errormessage`.
- Focus indicator is a low-opacity glow with no solid outline on glow-only controls · `Slider.svelte:160-162; Segmented.svelte:186-188; RetentionPicker.svelte:145-147; RadioGroup.svelte:139-142` · add a 2px solid accent outline (or raise ring opacity) on focus.
- Disabled styling magnitude inconsistent across the control family · `Slider/Combobox/RadioGroup 0.38, Stepper 0.35 vs `--app-disabled-opacity` 0.4` · route all disabled states through the token + uniform `pointer-events:none`.
- Segmented/RetentionPicker keyboard nav depends on a clicked button being focused (WKWebView doesn't) · `Segmented.svelte:112-126; RetentionPicker.svelte:78-94` · on click also move focus to the clicked segment.
- ThemeModeControl failure only logs to console and silently reverts · `ThemeModeControl.svelte:51-56` · push an app notification on catch in addition to reverting.
- Stepper gives no feedback when +/- hits the min or max bound · `Stepper.svelte:48-51,68-76,108-116` · disable + at max and − at min.
- Checkbox check-mark pops in without animation while the box animates · `Checkbox.svelte:62-78,148-153` · add a ~120ms scale/opacity transition on the glyph (respect reduced-motion).

## Flows verified clean (no issue)

**App Shell + Recording Lifecycle** — Start→running→stop happy path (pulsing dot + accent + `aria-live`, loading labels, double-fire guards); pause/resume (amber resume style + paused pill); Timeline⇄Insights toggle active state (accent bg + weight + `aria-current`); notification popover keyboard access (Tab-trap, Esc-close, focus management, `aria-expanded`/`controls`); per-source live indicators (`role="status"`, distinct glyphs, 2s poll); button/control interaction states (hover/focus-visible/active/disabled + reduced-motion).

**Dashboard — Timeline** — Active-frame preview load failure (`prettifyFramePreviewError` → `role=status` banner); empty state (glyph + title + hint + Record CTA); loading/pagination/load-error (spinner, "loading…", `role=alert` retry banner); OCR run/rerun states (running/empty/missing/error + rerunning label); date-picker month load + no-frames-on-day; copy/download success+error + redaction confirm; jump-to-latest header button (surfaces via the always-visible timeline error banner).

**Dashboard — Audio** — Media load failure (`role=alert`); play/pause decode failure (`role=alert` + `aria-pressed` integrity); reprocess transcription loading + double-fire guard; speaker-analysis retry visibility + loading; segment bars as real buttons (hover/focus-visible/selected, `aria-pressed`); lane error/empty/loading with Retry; no silent catches for sighted users (every catch assigns a rendered `$state` error var).

**Quick Recall** — Search loading skeletons; no-results empty state (correctly gated off parse-error); search error + Retry; malformed-filter/paused-results (`role=alert` + calm body state); open captured page (shared `openCapturedUrl` dialog feedback + `opening` latch); copy-answer success micro-interaction; Ask AI streaming phase visibility (every phase distinct, self-heals via snapshot); Ask AI cancel/Stop acknowledgement; turn error + retry; thumbnail load fallback to glyph; keyboard navigation of results; full button interaction states.

**Insights** — Chat send→stream→done + direct-invoke error rendered; Chat cancel (terminal done update); three distinct Chat empties; loading skeletons; model picker switch + per-provider unavailable + Retry; Subjects/SubjectDetail pin/dismiss success + busy spinners; Subjects realtime refresh-pill; Overview re-read (busy "reading…" + `digestError`); Overview free-tier usage-chart error + Try again; Context add/edit/delete (inline + dialog routing + confirm); conversation delete (confirm + undo + animated removal); charts start at 0 / single-hue ramp / labeled legends; rail primary-nav active state (3 signals + `aria-current`); Overview range stepper (disabled `atLatest`, gen-token guarded).

**Settings** — Provider API key save (spinner/`aria-busy`/`role=alert`); persistent keychain badge (defensive merge, cold-load re-probe); AI runtime Test connection; model downloads (OCR/Transcription/Speaker/Semantic — progress, cancel, error, delete-confirm); retention tightening (preview + `ask()` confirm + revert); wipe User Context (warning confirm); semantic-search enable toggle (revert + warn hint); microphone controller state matrix; app update (spinners, progress, dismissible error, copy-to-clipboard); rail nav active + scroll-spy; keyboard shortcut rebinding (listening state, inline conflicts, `role=alert` block banner).

**Onboarding** — Permission grant outcome reflected back (status pill re-derive + Re-check); dependency-locked toggles with working unlock buttons (hoisted out of inert subtree); finale attention-gated finish + escape hatch (`canSkipToDashboard`); inline AI provider key save/clear (double-invoke guard + badge + `prov-hint.err`); default-model picker connect/validate; per-feature model download (start/cancel/error + collapsed-row progress badge); Welcome "Begin setup"/"Apply recommended" busy states.

**Access Request** — Load skeleton (`role=status` `aria-live`); load-failure recovery (`role=alert` + non-double-firing Retry); approve/deny pending state (both buttons disabled, "Allowing…/Denying…"); action-error visibility (`role=alert`); 6s infinite-hang watchdog; trust cues (focus on Deny, Enter unbound, accent-outline Allow); informed-consent disclosure; broad-scope warning hierarchy; Segmented scope/duration controls; empty state; reduce-motion/a11y (dialog roles, Tab-trap).

**Debug** — OCR table numeric column alignment (`.cell-num` tabular-nums); sticky table headers; async-error announcement across all 18 user-triggered fetches (`role=alert` `aria-live`); Start/Stop loading + disabled + re-reconcile; submit-job success feedback; classify-workspace validation + 3 states; jobs pagination + clamped page + empty state; Overview all-clear gated on `overviewProbesLoaded`.

**Form Controls** — Switch toggle (4 states + checked-hover + thumb-slide + label linkage); Select/Combobox popover (Esc-close, focus-return, empty state, open state); Slider keyboard + drag (`aria-valuetext`); Stepper text-entry + spin (invalid + invalid:focus + spinbutton ARIA); Input focus/invalid/invalid:focus/disabled; RadioGroup/RetentionPicker/Segmented roving-tabindex selection (unit-tested index math).

## Recommended fix order

1. **Render `captureControls.error` (CRITICAL).** Surface recording start/stop/pause failures in the title bar or via the notification bell — closes the highest-traffic silent failure.
2. **Stop misattributing pin/dismiss write failures (HIGH ×2).** Gate `loadError` on `!conclusions`/`!view` in `Subjects.svelte` and `SubjectDetail.svelte` and route write failures to a dialog/toast — they currently destroy the list/inspector and mislabel themselves as load failures.
3. **Make the Ask-AI-unavailable hint actionable (HIGH).** Turn the described fix into a one-click Open-Settings affordance.
4. **Close the remaining hand-off silent failures (MEDIUM/LOW cluster).** Broker "open capture" (`6790-6797`), Quick Recall open-result/Continue-in-Chat/external-link/copy, Insights Answer-Source open, Overview corrections/engine load, history-row load — wrap each in try/catch and surface a dialog/notification.
5. **Fix the autosave error channel (MEDIUM).** Render the captured `recError` message near the panel with Retry, reconcile the control to last-saved, and add backoff to the silent ~450ms auto-retry loop.
6. **Route closed-channel errors to a visible surface (MEDIUM).** `jumpToFrame`'s popover-scoped `pickerError` and the source-pill/restart/clear-notification paths all write to dead/hidden state.
7. **Announce async transcript/speaker completion + failure to AT (MEDIUM).** Add `role="alert"`/`aria-live` to the audio-drawer transcript errors and status pill, matching the frame domain.
8. **Recoverability gaps (MEDIUM/LOW).** Add Retry to onboarding Transcription/Speaker model-status and the Welcome-phase load failure; fix the Access Request watchdog so a slow-but-successful grant is never reported as failed.
9. **Consistency & polish sweep (LOW).** Standardize busy spinners (Settings/onboarding), the disabled/focus-ring recipe and validation-color semantics across the control family, the WKWebView click-then-keyboard focus fix, chart/rail a11y labels, and the smaller missing micro-interactions.
