# Redesign → real app: what is NEW, what is OLD

A map, not a plan. It compares the mockups in this folder with `apps/desktop/src`, `apps/desktop/src-tauri/src`, `crates/capture-types` and `crates/app-infra`. App paths are relative to `apps/desktop/src/` unless they start with `src-tauri/` or `crates/`.

Tags: **[UI]** frontend only · **[DERIVED]** new data computed from the existing DB, no LLM · **[PROMPT]** LLM prompt or schema change · **[MIGRATION]** new DB column or table · **[BACKEND]** new Rust command or logic · **[NEW FEATURE]** a capability the app lacks · **[DECISION]** needs a founder ruling.

---

## 1. Foundation

**Tokens: keep.** `tokens.css` has exactly the same `--app-*`, `--cat-*`, `--focus-*` and `--chart-*` names as `routes/+layout.svelte` (lines ~1591–1970; I diffed the names). Nothing changes there. Additions [UI]:
- Scales: `--s-1..8` (4·8·12·16·24·32·48·64), `--r-xs/sm/md/lg/xl/pill`, `--h-sm` 26 / `--h-md` 32, `--ease-expo`, `--ease-quart`, `--t-fast/med/slow`.
- `--mx-titlebar-h` 38px replaces `--app-titlebar-height` 36px.
- Derived mixes, declared per `[data-theme]`: `--mx-hairline`, `--mx-wash`, `--mx-wash-strong`, `--mx-selected`, `--mx-glass`, `--mx-dot`, `--mx-sheen`.
- Skip `--mx-highlight`. It is a dead no-op.
- **Where:** a new global stylesheet, e.g. `lib/styles/kit.css`, imported once from `+layout.svelte`. `+layout.svelte` is already 3,299 lines, so the kit does not go into it.

**Fonts** [UI]:
- Today the whole app runs on a mono stack: `--app-font-mono: "Berkeley Mono", "TX-02", "Monaspace Neon"…` (`+layout.svelte:1685`) is set as the root `font-family` (`:1975`). 38 files reference `--app-font-mono`.
- New: Hanken Grotesk (interface) + Spline Sans Mono (record), both OFL.
- **Files:** copy `fonts/hanken-grotesk-latin-wght-normal.woff2`, `fonts/spline-sans-mono-latin-wght-normal.woff2` and `fonts/OFL.txt` into `apps/desktop/static/fonts/`. Use the mockup's `@font-face` rules from `kit.css` (variable weight, latin `unicode-range`). Nothing loads from a CDN; the app is offline.
- **Why not npm:** `apps/web` gets the same faces from `@fontsource-variable/hanken-grotesk` and `@fontsource/spline-sans-mono`, but its mono there is not the variable cut. Copying the two files adds no dependency.
- **Wiring:** add `--font-sans` and `--font-mono`. The root becomes `--font-sans`. Repoint `--app-font-mono` → `var(--font-mono)` so the 38 files keep working.
- **Cleanup:** audit those 38 files. Wherever mono carries a sentence, button, nav item, label or field text, switch it to sans; keep mono only for data, time, kicker and kbd.
- Delete the Berkeley/TX-02/Monaspace stack.
- Optional: list the two fonts in the About notices. `src-tauri/src/third_party_notices.rs` is model-only today.

**Type roles** [UI]: add global classes `mx-display` 40, `mx-h1/title` 28, `mx-h2/heading` 20, `mx-h3/subheading` 16, `mx-prose` 14/1.6, body 13, `mx-body-sm`, `mx-label` (now **sans**), `mx-data`/`.num` (mono, tabular), `mx-kicker(--live)`, `mx-stat__value`. They replace the per-component `font-size: var(--text-*)` plus mono declarations scattered through `+layout.svelte` and `lib/insights/*`.

**Flat rules** [UI]:
- Depth is a surface step plus a 1px border only.
- No bevels. Only `routes/+page.svelte` and `routes/+layout.svelte` use `inset 0 1px 0`.
- No drop shadows except popovers and tooltips.
- Selected = `--mx-selected` fill plus stronger text, everywhere.
- **No left-bar selection.** Audit `border-left`/`inset 2px|3px 0 0` in `lib/insights/{Overview,Context,DayTimeline}.svelte`, `lib/timeline/{JumperCalendar,JumperTimeList}.svelte`, `lib/quick-recall/DetailPane.svelte`, `lib/onboarding/{CaptureSentence,FeatureSwitches}.svelte`, `lib/AnswerProse.svelte` (a blockquote may legitimately keep its bar) and `+layout.svelte`.
- Green covers about 10% of the screen: live/now, matches, selection marks, ONE primary button, focus.

**Kit component → what it replaces**

| Kit | Replaces (OLD) | Note |
|---|---|---|
| `.mx-btn` (+`--primary/--ghost/--danger`, `--sm`, `--icon`, `__accent`) | There is **no shared button today**. Local `.btn`/`.btn--ghost`/`.btn--sm` copies live in `routes/+page.svelte` (~6967–7020), `lib/settings/settings-controls-fields.css`, `lib/insights/{Overview,Context,Subjects,Chat,ConclusionHero}.svelte`, `lib/timeline/{TimelineJumper,JumperTimeList,SpeakerRepairPanel,DrawerStatePanels}.svelte`, `lib/settings/panels/data/Storage.svelte`, `lib/settings/panels/intelligence/McpOAuthConnect.svelte`, `lib/components/AppPrivacyExclusion*.svelte`, `routes/update`, `routes/access/request`, `lib/debug/debug-controls.css` | [UI] One global class, no wrapper component. 32/26px, flat, ONE primary per view. |
| `.mx-seg` (`--md`, `--mono`) | `lib/components/Segmented.svelte` (keep `segmented-nav.ts` keyboard logic) | [UI] Restyle in place; add a `mono` prop for data values (1m/2m/5m, fps). |
| `.mx-choice` / `.mx-choice--list` | `lib/components/RadioGroup.svelte` | [UI] Tiles (≤4 options) or list rows, real hidden radios, no circles. The 5 call sites are in §6. |
| `[data-date-input]` day / period / range | Overview's hand-rolled `.date-stepper` + D/W/M `Segmented` (`lib/insights/Overview.svelte:1184`), `lib/insights/JournalDateStepper.svelte` | [UI] New `DateInput.svelte`: capsule + ruler + popover. `mxHeat` density is [DERIVED] from capture per day. *range* is used only by Chat scope (§5). |
| `.mx-cal` | `lib/timeline/JumperCalendar.svelte` look | [UI] Restyle only; see §3. |
| `.mx-switch` | `lib/components/Switch.svelte`, and `Checkbox.svelte` inside Settings | [UI] |
| `.mx-input` | `lib/components/Input.svelte` | [UI] |
| (no kit class) `.st-select`, `.st-combo` | `Select.svelte`, `Combobox.svelte`, `ActionSelect.svelte` | [UI] Keep the components; snap them to `--h-md` and flat border. |
| (none) | `Stepper.svelte`, `Slider.svelte` | [UI] The mockup turns Segment Duration (Slider) into `mx-seg--mono` and Idle timeout into a select. Keep the components where Settings still needs them. |
| `data-tip` | `lib/components/tooltip.ts` (`use:tip`) | [UI] Restyle only. |
| `.mx-pop` / `.mx-menu` | Notifications popover in `+layout.svelte` | [UI] |
| `.mx-chip`, `.mx-kicker`, `.mx-stat`, `.mx-panel`, `.mx-empty`, `.mx-skel` | Per-file chips/cards, `lib/insights/Skeleton.svelte` | [UI] |

## 2. App shell (`routes/+layout.svelte`)

| Area | OLD (today) | NEW (mockup) |
|---|---|---|
| Titlebar left | Record controls `titlebar__record--start/pause/stop`, source pills `titlebar__source*`, privacy warning `titlebar__privacy-warning` (~970–1185) | Traffic-light inset plus an **icon** Timeline/Insights pill with a sliding ink (`mx-nav`). ⌘1/⌘2 [UI]. |
| Surface switch | Text `surface-toggle` buttons (~1189) | Icon tabs with tooltips "Timeline ⌘1" and "Insights ⌘2" [UI]. |
| Centre | `titlebar__search` button that opens the **Quick Recall window** | A single `mx-cmd` field. It recalls by default; **Tab** flips to Ask, Esc resets. ↵ in ask (or ⌘↵) goes to Chat with `?q=`; ↵ in recall opens the Quick Recall window pre-filled with the text (§8 #2: no in-page filtering). ⌘K focuses recall, ⌘J focuses ask (never ⌥Space / ⌥⌘Space — ⌥⌘Space stays the global Quick Recall window). [UI] + see decisions. |
| Right | Notifications, keyboard-help `?`, settings gear, `ThemeModeControl` (`:1438`), debug | Chats icon, notifications (bell + badge), settings. **Theme toggle removed** (Settings → Appearance only). No help icon. [UI] |
| Status bar | **None exists** | New 30px `<footer>`: REC timer + pause/stop (22px ghost icons) · lanes screen/mic/system on/off (toggles while off) · spacer · flag / engine pill when unhealthy · "today 6h 12m" · storage size · "local only" · **?** keyboard-help mini button (22px ghost, opens the shortcuts `mx-pop`; decision 24). When not recording: "NOT RECORDING" + "Record". [UI]; "today Xh" is [DERIVED] from capture segments. |
| Dedicated windows | `surface-titlebar` with its own `ThemeModeControl` (`:1486`) | Drop the theme control there [UI]. |

**Engine status only when unhealthy** [UI]:
- Today it always shows in `lib/insights/RailFooter.svelte` ("engine · <model>" / "engine off · Enable"), fed by `engineOn`/`shortModel()` in `routes/insights/+page.svelte`. `lib/insights/Context.svelte` also shows a tier badge.
- New: nothing at all while healthy. When unhealthy, one `mx-engine` pill in the status bar ("engine off" / "engine needs reconnect", linking to Settings → Providers) plus one notification row.
- Feed it from the reason codes that already exist: `UserContextStatus.engine_available/reason`, `ask_ai_availability`, and `needs_reconnect:` / `provider_unreachable:`.
- `provider_unreachable` must not read as "reconnect" (ADR 0058).

**Status-bar and bell gaps found while drawing states** (details in `backend.html` §1/§3/§5):
- [BACKEND] Display unavailable is not on `CaptureSession`; `capture_display_unavailable` only reaches the frontend via the debug `get_idle_debug` runtime sources (`native_capture/activity.rs:384-402`), and no notification is pushed. Today the titlebar shows it as "starting…" (`+layout.svelte:437-443`).
- [BACKEND] Mic / system-audio `RuntimeSourceStatus.reason` is always `None` (`activity.rs:409-428`): tap rebuild, zero-watchdog and device loss are invisible, so "System audio reconnecting" has no source.
- [BACKEND] No engine-health `AppNotification` and no engine-status event; `AppNotificationAction.tab` can't target Providers / License / Storage / Privacy (`lib/notifications.svelte.ts:5-8`).
- [BACKEND] No low-disk early warning; free bytes and the resume threshold aren't exposed (`native_capture/disk_space.rs:136`). The drawn banner doesn't need them.
- [UI] Start-capture failure is a one-off native dialog (`lib/capture-controls.svelte.ts:45-47`); the status bar needs to keep the error.
- [UI][BACKEND] The status bar's "Screen Recording off · Open System Settings" flag (and the stage notice) also floats the **permission helper window** — see Onboarding below (decision 20). Today no main-window surface offers Open System Settings for Screen Recording at all.

## 3. Timeline (`routes/+page.svelte`, `lib/timeline/*`): visuals, plus three new interactions

- **Restyle** [UI]: rail, OCR/stage actions (`timeline__ocr-btn`, `timeline__stage-action-trigger`, `timeline__stage-play-moment`…) become `mx-btn --ghost/--icon --sm`.
- **"latest"** (`timeline__jump-latest`, `timeline__picker-global-latest` with `btn--accent`) becomes `mx-btn --sm` with the accent icon and `<kbd>L</kbd>`.
- **Compact jumper trigger** [UI]:
  - Only the trigger in `TimelineJumper.svelte` changes: one 28px `.mx-date__btn` showing calendar · sans day · hairline · mono time. The `J` badge moves into the tooltip.
  - The trigger opens the popover; it has no drag or wheel stepping.
  - **Keep** `JumperCalendar.svelte` (restyle to `.mx-cal`) and `JumperTimeList.svelte` (hour list). The mockup keeps that two-pane popover. SHELL.md's migration row ("`.tl-jump` → `data-date-input="day"` capsule") contradicts its own Date-input section; fix the mockup doc.
- **Audio drawer speaker strip** [NEW FEATURE][UI]:
  - The app has none of this. `AudioDrawer.svelte` only counts `distinctSpeakers` for status text.
  - New: a strip under `DrawerHeader.svelte` with one toggle chip per speaker (mark · name · state "you · auto" / "maybe Daniel" / "unnamed" · talk-time %).
  - Selecting a chip dims other turns (opacity 0.38) and their bars in `WaveformScrubber.svelte`.
  - An "only this voice" filter hides the other turns in `TranscriptReader.svelte`.
  - A waveform hover tooltip shows the current speaker.
  - All of it can come from turns already loaded ([DERIVED] client-side); no new IPC.
- **Repair panel** (`SpeakerRepairPanel.svelte` exists; restyle plus flow changes) [UI]:
  - Cluster summary.
  - A "Sounds like X" card with **Confirm X / Not X**. Confirm exists only on the reader chip today.
  - `mx-seg` scope.
  - **One search box filtering an `mx-choice--list` of saved people, plus "Create '<name>' new person"**. This replaces the separate name input and the link `<select>`.
  - "Move this line to" as a choice list, and a single primary commit.
  - Check that the existing name/link commands cover create-from-search before calling this [UI]-only.
- **Recall on the timeline** — **decided 2026-10-10: not built** (§8 #2). No match dots, no in-page filtering, no ↵-stepping; the rail's interaction stays as it is. Search stays in the Quick Recall window (`routes/quick-recall`, `lib/quick-recall/*`); the titlebar field's ↵ in recall mode opens Quick Recall pre-filled.
  - What ships instead — a **landing chip** [UI][BACKEND]: the query travels with the hit. Add `query: Option<String>` to the open payload (`src-tauri/src/lib.rs:72-127`), send it from `lib/quick-recall/searchStore.svelte.ts:328/345` and `routes/quick-recall/+page.svelte:111`; the Timeline (`+page.svelte:3679-3724`, payload type `:462`) shows a dismissible chip `from Recall · "tap"` and one `.tl-matchmark` at the landed frame. Dismiss clears both. ≈5 files.
  - Pre-fill: `summon_quick_recall_window` needs an optional query the Quick Recall page reads on focus (not verified whether one exists).
  - Why: the rail is frame-counted (8px per frame, ≤5,000 loaded ≈ 2.8 h at 2 s, live refresh every 1.5 s), so whole-day dots can't be drawn without a time-indexed rail; `search_capture` returns ≤50 merged, relevance-sorted groups and "by meaning" hits have no exact frame, so dots would be a second, keyword-only search. Revisit only if the rail becomes time-indexed.
- ~~**Speaker colors** [DECISION]:~~ — **Resolved (2026-10-10).** [UI]
  - `SPEAKER_COLOR_PALETTE` (`lib/insights/receipt-audio.ts:297-302`) becomes `[--cat-meetings, --cat-research, --cat-entertainment, --cat-learning]` — green (`--cat-creating`, 18° from the accent) out, teal in; four colours, `--cat-communication` stays reserved for the owner. Not the mockup's five: putting communication in the cycle collides with the pin and breaks `audio-drawer-view.test.ts:129-133` (cycle at four).
  - The "You" pin never matched because `assignSpeakerMarks` (`lib/timeline/audio-drawer-view.ts:276-289`) keys by `String(clusterId)` and the pin compares to `"You"`; the receipt path (`:379`) keys by display name and only works while the owner is still named "You". Fix: key the pin on `person_profiles.is_account_owner` (DTO `isAccountOwner`, `app-infra.ts:393`) — the drawer already has `profiles` (`AudioDrawer.svelte:89`) and each group's `personId` (`audio-drawer-view.ts:103`); the receipt tests the same flag. One new test (owner cluster → communication). The mockup (`timeline.html:1291-1299`) moves to four colours with the owner pinned lavender.
- **Why a segment has no transcript** [BACKEND]: an admission refusal writes no job row (`crates/app-infra/src/lib.rs:214`), so model-missing and never-queued look the same; a cloud-unreachable job is requeued with free-text `last_error` (`processing/runtime.rs:115-123`, `store.rs:1984`) and the Timeline shows it as running (`+page.svelte:1627-1629`). Needs a reason on the job DTO. Frontend bug alongside: an untranscribed segment falls to `not-run` (`audio-drawer-view.ts:633`) and shows "Speaker analysis hasn't run" (`DrawerStatePanels.svelte:102`).
- **Start of history** [UI]: `list_frames` returns a bare page, so the end is an empty page — enough. Retention copy reads the policy from `get_recording_settings`.
- **Screen-off sessions** [UI]: the rail is frame-indexed, so an audio-only session draws an empty lane (`+page.svelte:6738-6769`). Audio segments are already listable.

## 4. Insights (`routes/insights/+page.svelte`, `lib/insights/*`)

**Shell** [UI]:
- Delete the left rail: `InsightsRail.svelte`, `RailHistory.svelte`, `RailFooter.svelte`, `RailResizer.svelte`. Replace it with a sticky `.in-bar` row: underline tabs Overview · Journal (`5d`) · Subjects (`8`) · Context (`5`), with the active view's controls on the same row.
- Remove `"chat"` from `InsightsTab` (line 27). Chat moves out (§5).
- ~~Today derivation-off steers all four surfaces to Chat (lines ~123–128). The mockup says engine off means "Insights shows capture-only views" instead [UI][DECISION].~~ — **Resolved (2026-10-10): no full-page pitch and no steer, for any cause.** Delete the `engineGated` `.gate` branch (`routes/insights/+page.svelte:107-110, 339-350`) and the `derivationOff` steer effect (`:119-128`). Never set up / off by you / derivation off each show one `mx-notice` (reason-keyed copy, `insights.html:620-639`) with its action over capture-only content; `has_providers` from the 2026-10-09 Insights plan still picks the copy but no longer a wall (amends that plan's "`!has_providers` → pitch" in one line). **Overview** greys its capture-derived figures (`get_usage_charts` reads only `frames`) and prints "needs AI" in the engine-owned slots (read, categories, focus). **Journal differs:** a day with nothing written up (AI off *before* that day) has no cards to grey — it takes the **capture-run shape**: "N tracked. Nothing written up.", grey capture runs on the spine cut at gaps and labelled by app (`.in-sp__run`), stats Tracked + Activities "— · needs AI", an in-place empty under "What you did", Apps from capture, no Day mix (settled 2026-08-20: no engine → capture runs). AI switched off mid-day keeps the cards written before (mockup state `engine-off-midday`). Subjects / Journal keep their centred empty with the same CTA when never set up.

**Overview**:
- Period picker = `data-date-input="period"` with the D·W·M switch, matching the real `RangeMode = "day"|"week"|"month"` (`lib/insights/activity-helpers.ts:50`) and `shiftAnchor` [UI]. It steps one period and never goes past now.
- Header row:
  - Kicker "The read · week 41". The ISO week number is [UI].
  - `mx-display` headline.
  - Ghost "Ask about this week ⌘J" with a sparkle, label following the period, prefilled and scoped. It replaces `.ask-entry` "Ask about your history" (`Overview.svelte:1951`) [UI].
- Stats [DERIVED]:
  - Deep-focus delta vs the previous range (new; Tracked already has `trackedDelta`).
  - Top-category % of time.
  - Focus split bar Deep / Mixed / Scattered.
  - "Where the week went" as **per-day** stacked category columns, or 2-hour blocks in day mode. Today `StackedBar` draws a single bar.
- Restyle only: "What changed", "Needs attention", Time in apps (`MiniBars`), `Heatmap`.

**Journal**:
- Day spine [UI]+[DERIVED]: a horizontal axis with an activity lane, a focus lane and a now-line; hovering cross-links to the cards. It replaces the Morning/Afternoon/Evening bands (`bandRiver`, `journal-view.ts`).
- **Newest-first** list [UI]. Today `journal-view.ts:20,31` is oldest-first with follow-to-bottom and a "↓ now" button; drop those.
- **Inline receipt** [UI]: the card expands in place (Play, 1×/8×/16×, speaker turns, "Open in Timeline"), reusing `ReceiptViewer.svelte`. It replaces the `ActivityReceipt.svelte` modal (`role="dialog"`).
- Day stepping moves to `data-date-input="day"` (deletes `JournalDateStepper.svelte`). The side column (This week day bars, Day mix, Apps) is new [DERIVED] via `get_usage_charts` plus activities.

**Subjects**:
- Conviction map (one row per subject: dot now, tail to 14 days ago, ticks for other conclusions). It replaces per-row `Sparkline` as the hero [UI]+[DERIVED] from existing conclusion history (`subjectTimeline.ts`).
- Stat tiles [UI].
- Headline: **recommend a template over the tiers** (e.g. "{strong} settled. {rising} forming fast — {fading} fading.") [UI]. A new LLM pass would be [PROMPT][BACKEND].
- ~~The mockup must use the app's tiers. `subjectsTiers.ts` has 4 (strong ≥0.68, forming ≥0.30, shaping, fading) where the mockup has 3 (75/40) [DECISION if 3 is wanted].~~ — **Resolved (2026-10-10): four tiers stay; the mockup already draws them at 68 / 30 / 15 (`insights.html:1315-1316`), so the premise was stale.** 0.68 is confirmed as the Strongly-held cut (closes `PLAN.md:45`; at formation 0.68 and 0.75 draw the same line — 3 supports = 0.66, 4 = 0.78 — and differ only in demotion speed under the 30-day half-life). Two drifts folded in: [UI] `ConclusionStrip.svelte:50-55` imports the shared cuts instead of its own 0.68 / 0.45; the mockup's Fading test becomes "every conclusion below the 15% floor" to match `Subjects.svelte:247`.

**Context**:
- Layout restyle: topic chips Work/Preferences/People/Goals plus ⌘↵, a "What the dossier is built from" mix bar, and Guardrail chips [UI].
- Authored and Inferred counts already exist (`statements.length`, the conclusions list) [UI].
- ~~**"Steering N · conclusions backed" and "backed by '…'" are not exposed.** No authored→conclusion link exists; `Context.svelte:95–101` just takes the top 3 by confidence. Building it is [BACKEND][MIGRATION]; otherwise drop the stat [DECISION].~~ — **Resolved (2026-10-10): dropped.** No link is built; the mockup's "Strongest inferred views · by confidence" (top 3, the same pick `Context.svelte` makes) with "say it here to make it standing" is the replacement [UI].
- **Topic becomes a fixed choice** — decided 2026-10-10 (§8 #9): Work · Preferences · People · Goals, or none, as `mx-btn--sm aria-pressed` chips in the composer and edit rows (replaces the free-text `topic-input`, `Context.svelte:344-351`); `user_context_add_authored` / `user_context_update_authored` (`commands.rs:688, 718`) accept only those four or null. Column stays `TEXT`, no migration (no installed users); the prompt line `(topic: X)` is unchanged. The mix bar's per-topic rows count authored statements client-side [UI].
- "Guarded" is a static count, but `crates/app-infra/src/user_context/guardrail.rs` has **5** `SensitiveCategory` values while the mockup draws 6 chips. Fix the mockup.

### Digest (Overview "The read" + Journal day lede): verified detail

**Reality today**
- Table `user_context_digests` (migrations `0029_user_context_digests.sql`, `0030_user_context_digest_headline.sql`).
- `UserContextDigest {rangeKind, rangeStartMs, rangeEndMs, narrative, headline?, generatedAtMs}` (`crates/capture-types/src/user_context.rs:234–246`).
- Generated on demand by `get_user_context_digest` / `regenerate_user_context_digest` (`src-tauri/src/user_context/commands.rs:444/475`).
- Cached by activity fingerprint; a stale entry is still served within the freshness floor of 1h day / 6h week / 24h month (`digest.rs:74–84`).
- Needs at least 2 activities (`MIN_DIGEST_ACTIVITIES`, `:67`).
- Prompt: 2–4 sentences in second person; headline ≤6 words, no terminal punctuation (`:43–53`), clamped to 80 chars (`HEADLINE_CHAR_CAP`, `:607`).
- Model input, one line per activity: `[day | duration | category | focus] title — summary`. Summaries are clipped to 240 chars. **No start times, no app names** (`format_activity_line`, `:252–275`).
- `Activity {title, summary, category, focus, startedAtMs, endedAtMs, evidence[]}` (`user_context.rs:91–106`). Apps are **not** persisted on the activity. Evidence is `ActivityEvidenceRef {subject_type: "frame"|"audio_segment", subject_id, …}`.

**Gaps (mockup → what it takes)**

| Mockup element | Tag | How |
|---|---|---|
| Sentence headline + de-emphasized turn clause (`<em>— until Thursday's calls…</em>`) | [PROMPT] | **Resolved (2026-10-10): no migration.** `DigestNarrative` gains `turn` (serde default ""); the prompt asks for a full-sentence headline with *no dash in it* plus an optional turn; the store writes `headline — turn` into the existing `headline` column; `HEADLINE_CHAR_CAP` 80 → 140 (`digest.rs:607`); the lede greys everything after the first " — ". The output guardrail (`:941`) sees the joined string. The fingerprint already carries a shape tag — bump `v2` → `v3` (`store.rs:2767`) so cached rows regenerate. |
| Prose citing clock times ("before the 11:00 call") | [PROMPT] | **Resolved (2026-10-10): times yes, apps not now.** Add local `HH:MM–HH:MM` to each activity line (`digest.rs:252-275`; offset recovered from `range_start_ms` as today). App names are *not* added: derivation already sends per-frame `app=`/`url=`/minute timestamps to the same engine (`derivation.rs:302-357`), so there is no new exposure, and no digest sample cites an app (the "in Zed" quote was a kit sample). The guardrail (`:749-755`) filters title + summary only and never saw apps or times anyway. Revisit apps (frame evidence → `search_documents.app_name`, `0017`) only if the prose needs them. |
| Per-activity app chips, "2 speakers" | [DERIVED] | Same frame→app join, and `audio_segment` evidence → speaker turns (`0010_speaker_analysis.sql`). Expose on the activity DTO or a batch command. |
| Deep-focus delta vs previous range, top-category %, day mix, week bars, "through HH:MM" | [DERIVED] | Previous-window activities plus `get_usage_charts`. "through" = last covered activity end / `covered_until_ms`. |
| Kicker "week 41", "Digest so far" (when `atLatest`), newest-first, inline receipt, spine, empty copy "Too little captured on {day} to write a digest." | [UI] | `atLatest` comes from the period picker; the empty state shows below 2 activities. |
| "Still writing up the last 12 minutes" (inside mockup prose, `insights.html:918`) | [UI] | **Never LLM text**: a cached digest would freeze a stale "12 minutes". Render it from `pending.sinceMs` (`lib/insights/journal-day.ts:44`) as a live kicker "Writing up 15:08 → now". |
| Subjects headline | [UI] (recommended) | Template over tiers; see Subjects above. |
| Context "Steering" / "Guarded" stats | n/a | Not exposed; see Context above. |

**The mockup must change where it contradicts reality**
- "Away · lunch" is impossible: gaps carry no labels (the app shows "away — no capture"). Use **"Away · 1h 10m"**.
- ~~**Deep-focus metric** [DECISION]: `lede-stats.ts:100–111` gives the share of *activities* with focus=deep. The mockup weights by *minutes*. Pick one and use it in both views.~~ — **Resolved (2026-10-10): by activity (count), as shipped and as the mockup already draws** ("Deep focus · by activity", `insights.html:903-909, 1189-1191`; the bar says "of activities"). The premise was stale: minutes appear only in the colour-only focus heat. The one real outlier is the app's Focus modal (`FocusDetailModal.svelte:69-73`, span minutes) — move its Deep / Mixed / Scattered split to the activity count so "Deep" means one computation [UI].
- ~~**"Tracked" definition** [DECISION]: Overview uses Σ `timePerApp[].activeMs` (`lede-stats.ts:98`); the mockup Journal uses Σ activity spans. These differ on the same day.~~ — **Resolved (2026-10-10): usage-charts app active time (screen only), everywhere — as shipped in both views through `lede-stats.ts:98` and as the mockup draws ("Tracked · app time" in both ledes; its `0.93` at `:791` is fake-data generation, not a rule).** The premise was stale. Rule of record: `docs/user-context/CONTEXT.md:376-381` — Tracked deliberately ≠ Σ activity spans (overlaps, zero-length activities, idle inside spans, unassigned frames, the pending region). Σ spans is never labelled "tracked" (week-bar tooltips show "Xh · N activities"). The 10-09 plan's `capture_presence` + 60 s tracked refresh while recording stand.
- **The spine must be dynamic.** The mockup is fixed at 08:00–20:00. Use the first-to-last capture or activity span, show past days' real span (not "08:00 – 20:00"), and stack overlapping activities into sub-lanes.
- **"So far" prose lags.** The digest is cached up to 1h for a day, so show "written HH:MM" from `generatedAtMs` next to the live "through" stat.

**State gaps** (from the feedback-state pass):
- [BACKEND] `get_user_context_digest` returns `None` for derivation off, engine not ready, <2 activities and a guardrail trip alike (`digest.rs:726, 732, 761, 951`). Model errors already return `Err`. Add a reason enum so "too few", "off" and "failed" render differently.
- ~~[BACKEND][DECISION] Per-topic mix bar: inferred conclusions have no topic and the authored topic is free text — same root as "Steering" (decision 9).~~ — **Resolved (2026-10-10):** the rows count *authored* statements by topic (`insights.html:1478`), so inferred topics are not needed; the fixed four-topic choice above makes them countable. The inferred share is the grey segment of the total bar only.
- Authored context — **decided 2026-10-10** (§8 #15) [UI][BACKEND]: statements are **not** filtered through the keyword guardrail (it would silently delete deliberate instructions like "I have ADHD — keep answers short", and it is noisy both ways: "marketing campaign" → Politics, "psychiatrist" → nothing). Instead the egress is made honest and the cap visible:
  - Copy: fix the four spots implying statements stay on the Mac — `Context.svelte:311-312`, `:638-640` ("never inferred or surfaced — even if you mention them here" → "Mnema never infers these categories about you; what you write here is sent as written"), `Providers.svelte:283-286` ("the assembled dossier — never leaves this machine"), `UserContext.svelte:65`.
  - Provider line under the Context composer: "Goes to <provider> each time Mnema forms conclusions · stays on this Mac with a local model." Needs the default provider's *kind* on `AiRuntimeStatus` (`ai_runtime.rs:21-29`; reuse the `worker.rs:1123-1131` lookup) + the TS mirror (`recording.ts:212`) — Insights only sees an instance id today. No save-time dialog.
  - Cap: raise `AUTHORED_CONTEXT_CHAR_CAP` (`derivation.rs:1094`) to 4,000 (≈1,000 tokens per pass, next to a ≈36k-char activity block in the same prompt). Per-statement limit 280 chars, enforced in the textarea (`Context.svelte:330`) and in the add/update commands (`commands.rs:683/714`). Move the selection loop (`derivation.rs:1103-1121`) into a `pub(crate)` fn that returns the included ids; `list_user_context_authored` adds `used: bool` per statement (`capture-types` `AuthoredContext` + its shape test, `recording.ts:266`); the UI dims unused rows and shows "N of M statements used — older ones aren't sent". Sort by `updated_at_ms DESC` instead of `created_at_ms` (`store.rs:2336-2343`) so editing a statement keeps it in. ≈5 files, no migration.
- [UI] Bugs today: `DayTimeline.svelte` `loadRange` swallows errors, so a failed load shows "Nothing captured on {day}" (`JournalRiver.svelte:214`); the month heatmap labels one row per day by weekday (`Overview.svelte:496`).

## 5. Chat: new standalone surface

- **Where** [UI]: a `/chat` route in the **Main** window, reached from the titlebar Chats icon, ⌘J, Ask-mode ↵ and "Ask about …" buttons.
  - No new `AppWindow` (`src-tauri/src/windows.rs:71` has Onboarding, Main, CliAccessRequest, Debug, QuickRecall, Update), so no capability change.
  - `lib/insights/Chat.svelte` moves out of Insights.
  - Quick Recall's "Open in Chat" (`open_conversation_in_chat`) and the `insights_open_conversation` listener in `+layout.svelte` retarget to `/chat`.
- **Chat list** [UI]: replaces `RailHistory.svelte`; logic stays in `conversationStore.svelte.ts` (`historyGroups`, `search_conversations`, rename, delete).
  - Remove the mockup's category dots: `Conversation`/`ConversationSummary` in `crates/capture-types/src/conversation.rs` have no category.
  - The mockup's **Pinned** group and pin action — **decided 2026-10-10: build** (§8 #12) [NEW FEATURE][MIGRATION][BACKEND]. Copy the pinned-conclusions pattern (`0025_user_context_dismissals.sql:18`, `user_context/store.rs:2079 set_pinned`, `user_context_set_pinned`): new additive migration `conversations.pinned INTEGER NOT NULL DEFAULT 0`; `conversation/store.rs` selects it, sorts `pinned DESC, updated_at_ms DESC` in both list and search (required — the list loads only 60 rows, `conversationStore.svelte.ts:138`, so an older pinned chat would otherwise drop out), adds `set_pinned`; `ConversationSummary.pinned` + TS mirror; `set_conversation_pinned` command (`conversation/commands.rs`, registered in `lib.rs`); `conversationStore` `togglePin` + a "Pinned" group first in `historyGroups`; a pin hover button in `RailHistory.svelte` (no context menu exists) and the thread-header pin icon. **Rules:** pinning never touches `updated_at_ms` / `last_activity_at_ms` (it must not jump to "Today" or extend retention); pinned chats are still deleted by the retention policy like any other (no `pinned = 0` exception in `capture_retention.rs:873`). ≈8 files. Naming: code already uses "pin" for the per-chat model pin (`0033`) — the new column/command are `pinned` / `set_conversation_pinned`; `set_conversation_engine` is untouched.
- **New-chat empty state** [UI]: a time-of-day greeting, a privacy line and "Try asking" suggestions. It replaces "Ask the engine about your activity" and `EXAMPLE_QUESTIONS` (`Chat.svelte:389`). Suggestions that also set scope depend on the context panel below.
- **Model picker** [UI]: restyle `ModelPicker.svelte` / `ModelPickerMenu.svelte` (search, grouped, typed id, retry already exist). Add a cloud/local chip from provider kind and the footer "applies to this chat only · Settings → AI".
- **Answers** [UI]: `mnema-bars`, `mnema-dossier` and `mnema-timeline` already exist (`src-tauri/src/ask_ai/answer_view.rs`), as do sources via `AnswerSourceCard.svelte`, `AnswerProse.svelte` and tool-activity disclosures. Restyle to the mockup's steps line ("N steps · 3.4s · searched this week"), the sources row, and a per-answer token bar from the existing post-hoc `ContextTokens` update.
- **Context panel** — **decided 2026-10-10: build the slim version** (§8 #11) **[NEW FEATURE][BACKEND]**: scope = time range + an "About you" switch, plus the "Send to <provider>" line. **No per-source toggles, no pre-send token meter** (the exact post-answer `ContextTokens` number stays — see Answers above).
  - Today Ask AI scopes nothing: `AskAiStartRequest` (`src-tauri/src/ask_ai.rs:971-997`) and `AskAiFollowupRequest` (`:1001-1010`) carry no range; the model picks its own `from`/`to` per tool call.
  - (a) Add optional `scope { fromMs, toMs, aboutYou: bool }` to both requests and pass it to `run_ask_ai_turn` (`:1720`). Each turn is rebuilt from scratch, so the frontend resends the scope every turn — no migration (persisting a per-thread scope would need one; not now).
  - (b) In the tool executor (`:1922-1996`), before `broker_request_from_tool`: fill in `from`/`to` when the model omits them (`search`, `recall_context`) and narrow them when they exceed the scope (`timeline`, `activities`). One pure clamp function + one test. In `build_ask_ai_tools` (`:1317`): leave out `recall_context` when `aboutYou` is false. One line about the scope in the prompt (`:1185`).
  - (c) Out-of-scope tools: the scope governs *captured history* only. App control stays as it is (reads no data). `fetch_url` and `mcp__*` are not limited by it and are **named** in the "Goes to" line ("+ web fetch · 2 connectors — not limited by scope"); nothing is hidden or disabled.
  - Privacy line (verified against the tool structs — Mnema's own tools return text only, never image/audio bytes or file paths): "Only text: screen words, transcript lines, app and window names, page addresses, and Mnema's notes about you. Never images or audio." When web fetch / MCP are on, append the connectors line above (MCP results pass through as-is).
  - Why no source toggles: `activities` and `recall_context` are built from screen *and* audio together (`crates/app-infra/src/user_context/capture_source.rs:3-10, 41-52`), so "Microphone off" can't be applied to them honestly. Why no meter: what is sent depends on what the model chooses to read (≤12 tool calls × ≤100 results, ≤24k chars per page/MCP result), and Anthropic/OpenAI/ChatGPT report no window (`ai_runtime.rs:46-54, 484-490`) — a pre-send number could only say "up to".
  - "About you" = inferred conclusions + derived activities (`recall_context`), not the authored statements (those never reach Chat directly; see #15).
- **Turn states** (drawn in `chat.html`; details in `backend.html` §1):
  - [BACKEND] A user stop persists as `done` (`ask_ai.rs:2400-2424`); `TurnView` has no stopped flag (`capture-types/src/conversation.rs:177`).
  - [BACKEND] Turns left `streaming` on app quit are never reconciled, so the composer stays on Stop (`Chat.svelte:460`). Needs a startup pass → `interrupted`.
  - [BACKEND] `TurnUpdate::Error` is a bare string (`conversation.rs:229`); engine-setup errors send raw codes like `needs_reconnect:chatgpt` (`ask_ai.rs:1810-1823`) and `Chat.svelte:1360-1363` shows them in one generic card. Add an error `kind`.
  - [BACKEND][MIGRATION] Token bar / steps line / model: no context window for ChatGPT (`ai_runtime.rs:488`), Anthropic or Ollama; `contextTokens` not persisted; tool steps lack count and duration (`conversation.rs:156`); the model isn't recorded per turn.
  - [PROMPT][BACKEND] Inline `[n]` citations don't exist; sources are capped 6 frames / 4 audio (`ask_ai.rs:346-347`) and cited ids aren't checked against retention ("source no longer stored").
  - [NEW FEATURE][BACKEND] Regenerate — **decided 2026-10-10: replace the latest answer in place; never offered on earlier turns; the old answer is discarded (no versions)** (§8 #14). Mechanism: a store method `delete_last_turn(conversation)` (guarded to the trailing index — `conversation_turns` has `UNIQUE (conversation_row_id, turn_index)`, `0028:46`, and `save_turn` refuses to overwrite finished rows, `store.rs:133-148`) + a Tauri command, then re-send the same question through the normal path so it takes the same `turn_index` back (`ask_ai.rs:1770` numbers by row count). Frontend: reset that turn's `version` to 0 so the new stream isn't dropped as stale (`Chat.svelte:1100-1112`). Retry on a failed last turn (`Chat.svelte:694-704`) becomes the same path — this fixes the existing bug where a mid-answer failure leaves an `error` row, so Retry's new turn is numbered n+1 while the screen expects n ("Thinking…" never clears; after reload the question shows twice). ≈4 files, no migration. Quick Recall's follow-up Retry (`quick-recall/+page.svelte:1360-1375`) has the mirror-image mismatch — route it through the same command.
  - [UI] Bugs today: `refreshHistory` turns a failure into an empty list (`conversationStore.svelte.ts:146-148`); an unknown chat id opens an empty chat under that id (`Chat.svelte:426-429`).

## 6. Settings (`routes/settings/+page.svelte`, `lib/settings/*`)

- **Full-window focus mode** [UI]:
  - Today `/settings` renders under the main titlebar (`showMainTitlebar` includes settings; `app-content--settings`, `+layout.svelte:1520`).
  - New: hide the titlebar and status bar on `/settings`. `SettingsRail.svelte` becomes a full-height 252px sidebar with a 52px drag strip for the traffic lights, then "Back to app" (Esc), search (`/`), groups with icon tiles and sub-links, and a REC + lanes + version footer.
  - The content gets a 52px drag header with breadcrumb and "All changes saved"; save status moves out of the rail footer.
- **RadioGroup → `mx-choice`** [UI]:
  - `panels/capture/Capture.svelte:165` (voice detection) → tiles.
  - `panels/capture/Privacy.svelte:35` (browser URL mode) → tiles.
  - `panels/intelligence/Transcription.svelte:204` (provider) → list.
  - `panels/intelligence/Ocr.svelte:77` (provider) → `mx-seg`.
  - ~~`panels/about/About.svelte:126` (update channel) → `mx-seg`.~~ — not drawn (decision 16: stable only; the hidden picker stays hidden).
  - Then delete `RadioGroup.svelte`.
- **Segmented → `mx-seg --md`**: Audio, Ocr, Transcription, UserContext, ScreenResolution. Use `--mono` for data values (segment duration, capture rate, delete window). `VideoBitrateControl` becomes a select with "Custom…". `RetentionPicker` becomes a seg plus a ghost "Run cleanup now".
- **Answer questions automatically** [UI][BACKEND]: new `mx-choice` row in Ask AI (Always · Only with local models, default · Never). Details and the new settings key under Quick Recall below.
- **Theme** [UI]: Appearance (`panels/general/Appearance.svelte`) is the only theme control, a plain `mx-seg` System/Light/Dark. Delete `ThemeModeControl.svelte` once the titlebar copies go.
- **Delete Recent Capture row** [UI] — **decided 2026-10-10 (§8 #13): build it, tray unchanged.** New in Settings → Storage, as a danger box with `mx-seg--mono` 1/5/15 min and an `mx-btn--danger` button, with the note "Also in the menu bar". Confirm through plugin-dialog `ask()` with the tray's wording (`status_bar.rs:586-610`), call the existing command, and show a one-line receipt under the row ("Deleted the last 5 minutes") — the tray gives none.
  - Today it exists only in the tray (`src-tauri/src/status_bar.rs`).
  - The command already exists: `delete_recent_capture({ request: { windowSeconds } })`, which accepts only 60/300/900 (`src-tauri/src/app_infra.rs:4182`). Nothing in the frontend calls it.
  - Confirm through `@tauri-apps/plugin-dialog`.
- **Labels / mockup gaps**: the mockup renames Capture→Sources, Audio→Microphone, Access→CLI Access, About→Updates [UI].
  - It omits sections the app has: MCP connectors, Voice enrollment, OCR preprocessing/upscale, mic/system sensitivity, activity sources, system-audio access warning. Keep them, restyled.
  - Its storage meter with per-source breakdown is [DERIVED] — and confirmed missing: no storage-usage number is returned anywhere (`Storage.svelte` shows only the location).
- **State gaps** (from `settings.html` states):
  - [UI] No screen / mic permission rows in Settings, and voice enrollment shows raw errors (`VoiceEnrollment.svelte:153-155`). `get_capture_permissions` already exists.
  - [BACKEND] Ollama "not running" vs "not installed" vs "running, 0 models" all collapse to `local_endpoint_unreachable` (`state/ai-runtime.svelte.ts:69-102`).
  - [BACKEND] A full disk during a model download is a raw string (`Ocr.svelte:291`); only semantic search has a typed `InsufficientDiskSpace` (`semantic_search_models.rs:256`).
  - ~~[DECISION] The update channel picker is hidden on purpose (`showUpdateChannelPicker = false`, `About.svelte:45`) but the mockup draws it.~~ — **Resolved (2026-10-10): dropped from the mockup.** Only stable builds ship (24 tags, zero `-preview`; `preview/latest.json` is a 404; prereleases build with a placeholder licence key, `docs/licensing/ENV.md:79-81`). The row, the footer's "· stable" and the "selected channel" copy are gone from `settings.html` / `update.html`; the app keeps the flag `false` and the dormant channel code + release lane. Show the row again only when a real preview release exists and prerelease licensing is decided.
  - No license deactivate command or UI (only `reset_license_devices`); not drawn, so no work unless wanted.

## 7. Becomes dead after the redesign

- `lib/insights/InsightsRail.svelte`, `RailHistory.svelte` (content moves to the chat list), `RailFooter.svelte`, `RailResizer.svelte`.
- `lib/insights/JournalDateStepper.svelte`; Overview `.date-stepper` + D/W/M `Segmented` + `.ask-entry`; `bandRiver` bands and follow-to-bottom / "↓ now" in `journal-view.ts` / `JournalRiver.svelte`.
- The `ActivityReceipt.svelte` modal shell (the viewer survives inline).
- `"chat"` in `InsightsTab`; derivation-off steer-to-Chat.
- In `+layout.svelte`:
  - `titlebar__record*`, `titlebar__source*`, `titlebar__privacy-warning*` (moved to the status bar).
  - `surface-toggle`, `titlebar__search*`, `titlebar__theme`.
  - The keyboard-help titlebar button (ruled out 2026-10-10 — the **?** moves to the far right of the status bar; the sheet becomes an `mx-pop`).
  - The `surface-titlebar` theme control and `--app-titlebar-height`.
- `lib/components/RadioGroup.svelte`, `ThemeModeControl.svelte`; `Checkbox.svelte` inside Settings.
- Every local `.btn*` definition listed in §1.
- Left-bar selection styles; `inset 0 1px 0` bevels.
- The Berkeley Mono / TX-02 / Monaspace font stack.

## 8. Open decisions

1. ~~Quick Recall window vs titlebar field~~ — **Resolved:** both stay. The titlebar `mx-cmd` field is in-window recall/ask; the Quick Recall window stays on ⌥⌘Space as the confirmed answer-first bar + Quick Look, and Ask answers in place there ("Continue in Chat ↵" hands off). Tab = search ⇄ ask in both, → accepts the ghost completion.
2. ~~Recall-on-timeline (in-page match dots and chip): build it, or keep results in Quick Recall?~~ — **Resolved (2026-10-10): keep results in Quick Recall; no match dots or in-page filtering.** The titlebar field's ↵ in recall mode opens the Quick Recall window pre-filled. The hit's query travels to the Timeline as a dismissible `from Recall · "…"` chip with one mark at the landed frame (≈5 files, §3). Dots only if the rail ever becomes time-indexed.
3. ~~Status bar: where do source **toggles** (select lanes while stopped), the privacy-visual-capture warning and the keyboard-help button go? The mockup's lanes are display-only.~~ — **Resolved (2026-10-10).** Toggles: lanes are toggles while not recording and display-only while recording (SHELL.md status-bar table; same rule as today's `toggleSourceSelected`). Privacy: `mx-flag` warn "privacy filter failed · retrying" / danger "restart to resume screen · Restart" at the right of the status bar (today's `privacyVisualCaptureStatus`, `+layout.svelte:416-484`). Keyboard help: a 22px ghost **?** at the far right of the status bar (after "local only"), tooltip "Keyboard shortcuts · /", opening today's sheet (`+layout.svelte:1540`, groups from `lib/keyboard-help.svelte.ts`) restyled as a kit `mx-pop` anchored above it; `/` and `?` (`global-shortcuts.ts:115-121`) unchanged. The titlebar keeps no help icon.
4. ~~Speaker palette without `--cat-creating` (green ≈ accent), plus the "You" pin fix.~~ — **Resolved (2026-10-10): `--cat-learning` replaces green (four colours, lavender reserved); the pin keys on `is_account_owner`, not the name, in both the drawer and the receipt.** Detail in §3.
5. ~~Deep-focus metric: activity count (`lede-stats.ts:100–111`) or minutes.~~ — **Resolved (2026-10-10): activity count, as shipped and drawn; the Focus modal moves to count.** Detail in §4 Digest.
6. ~~"Tracked": app active time (`timePerApp`) or activity sum.~~ — **Resolved (2026-10-10): app active time, as shipped in both views and drawn.** Detail in §4 Digest.
7. ~~Digest headline: allow full sentences and add `headline_turn` (prompt + migration)? Send times and app names to cloud models?~~ — **Resolved (2026-10-10): full-sentence headline + turn as one joined string (no migration; clamp 140; fingerprint v3); local clock times on every line; app names not sent — no new exposure either way.** Detail in §4 Digest.
8. ~~Subjects: keep the app's 4 tiers (0.68/0.30) or move to the mockup's 3 (75/40)?~~ — **Resolved (2026-10-10): four tiers at 0.68 / 0.30 / 0.15 (the mockup already matched); 0.68 confirmed.** Detail in §4 Subjects.
9. ~~Context "Steering": build the authored→conclusion link ([BACKEND][MIGRATION]) or drop the stat?~~ — **Resolved (2026-10-10): dropped**; "Strongest inferred views" replaces it; topic becomes a fixed four-way choice so the per-topic mix bar is a client-side count. Detail in §4 Context.
10. ~~Engine off: Insights shows capture-only views instead of steering to Chat?~~ — **Resolved (2026-10-10): capture-only views with one notice, no pitch, no steer.** Overview greys capture figures; a Journal day with nothing written up takes the capture-run shape. Detail in §4 Shell.
11. ~~Chat context panel: build backend scoping + token estimate? What happens to out-of-scope tools (app control, fetch_url, MCP)?~~ — **Resolved (2026-10-10): build the slim panel.** Scope = time range + "About you" switch, resent per turn (no migration); no per-source toggles, no pre-send meter — the exact post-answer token count stays. App control unchanged; web fetch and MCP are not limited by the scope and are named in the "Goes to" line. Privacy line rewritten to the verified text-only list. Detail in §5.
12. ~~Chat pinning (new column) — yes or no?~~ — **Resolved (2026-10-10): build it**, copying the pinned-conclusions pattern (≈8 files, one additive migration). Pinned sorts first; pinning never bumps timestamps; pinned chats still follow the retention policy (no exception). Detail in §5.
13. ~~Delete Recent Capture in Settings in addition to the tray?~~ — **Resolved (2026-10-10): yes, both.** Same command, same confirm wording, a one-line receipt in Settings; the menu-bar submenu stays. Detail in §6.
14. ~~Chat Regenerate: replace turn *n* in place, or append a new turn? (Today only a failed last turn retries.)~~ — **Resolved (2026-10-10): replace in place, latest answer only, old answer discarded; never offered on earlier turns.** `delete_last_turn` + re-send (≈4 files, no migration); Retry on a failed last turn uses the same path, which fixes its turn-numbering bug. Detail in §5.
15. ~~Authored context: filter it through the guardrail before it goes to the provider (or warn on save)? Show a signal when the 2,000-char cap drops older statements, or raise the cap?~~ — **Resolved (2026-10-10): no keyword filter on the user's own words; honest copy (4 spots) + a "Goes to <provider>" line under the composer; cap raised to 4,000 with a 280-char limit per statement, a visible "N of M used" with unused rows dimmed, and last-edited ordering.** Detail in §4 state gaps.
16. ~~Update channel picker: show it (flip `showUpdateChannelPicker`) once preview builds exist, or drop it from the mockup?~~ — **Resolved (2026-10-10): dropped from the mockup; the flag stays `false`.** Stable only today; the row returns when preview builds exist. Detail in §6 state gaps.

---

## Feedback states

Every surface now draws its empty / loading / error / offline / off / permission states, built from the kit's **States** vocabulary only (SHELL.md › States; living reference `index.html` › States). No page-local toast, empty or banner skins.

**Hook convention.** `?state=<name>` renders one named state; a mockup-only switcher (bottom-left or under the window, labelled as not product UI) reloads with it and keeps `&theme=` and the view hash. `access.html` uses `?view=` instead. In a `?state=` render, timers and downloads hold still so screenshots are stable.

**Kit parts used.** `mx-empty(--center|--compact)`, `mx-skel*`, `mx-spin`, `mx-progress(--indeterminate)`, `mx-inline`, `mx-notice`, `mx-banner`, `mx-toast`, `mx-notifs`/`mx-notif`, `mx-badge`, `mx-flag`, `mx-engine(--error)`, `mx-rec--*`, `mx-lane--*`; tone via `data-tone`.

| Surface | States drawn | Parts |
|---|---|---|
| Timeline + shell (`timeline.html`) | Page: first load, never recorded, starting, load error, stale refresh, not recording, permission missing (+ helper window), screen off (audio only), display asleep, landed from Quick Recall (chip + one mark), paging, start of history. Status bar: off (lanes as toggles), permission, read-only / trial, starting, idle, paused (idle / you), low disk, stopped (disk full), privacy filter retry / restart, system audio blocked / reconnecting, engine not set up / offline. Preview + OCR + audio lane + drawer (processing, waiting for network, model missing, not transcribed, off, no speech, failed, playback error) + speakers + repair + jumper + bell. | rec, lane, flag, banner, engine, skel, spin, empty, notice, toast, notifs |
| Insights (`insights.html`) | Any view: loading, AI off, not set up, needs reconnect, offline, derivation off, first day. Overview: read writing / re-reading / failed / too few, empty range, load error. Journal: digest writing, being written, nothing captured, receipt loading / audio-only / expired. Subjects: none yet, sparse, no match, staged, detail states. Context: empty, adding, add failed. | engine, notice, empty, skel, spin |
| Chat (`chat.html`) | Streaming (thinking / tools / writing), reattached, stopped, interrupted, regenerating, source gone; turn failures (failed, rate limited, too long, empty, needs reconnect, unreachable); engine (local down, not set up, off); model picker (listing, failed, no match); context panel; chat list (loading, empty, error, no results, rename, deleted); opening / failed / deleted chat. | inline, notice, empty, skel, toast |
| Quick Recall (`recall.html`) | Idle, recent, first run, searching, indexing behind, meaning off / building, search failed, bad operator, no results, audio pending, not recording, Quick Look (+ frame missing), auto-answer gate, ask streaming / answered / stopped / error, no provider, AI off, needs reconnect, unreachable, offline. | empty, notice, inline, skel |
| Settings (`settings.html`) | Save failed; per-section loading; resolution checking; AI on with nothing ready; Ollama unreachable; MCP empty; Ask AI unavailable; model downloads; Apple Speech denied; no saved voice; storage resolving; cleanup running; CLI missing; license checking / revoked / out of window; update checking / failed. | inline, notice, progress, skel, spin, empty |
| Onboarding (`onboarding.html`) | Permissions (first ask, denied, denied before, needs relaunch — holds, resumed, helper floating, mic denied, system audio listening / blocked); storage (fits, tight, read-only, offline, full); AI (held, verifying, invalid, none, empty, ready); Setup (downloading, held, failed, offline, disk full, cancelled); Voice (waiting, off, busy, too quiet, too short, two voices, saved); Finale (starting, downloads, failed, idle, audio-only, helper floating). | notice, inline, rec, spin |
| Update (`update.html`) | available, checking, up to date, check failed, downloading, download failed, installing, install failed, ready, incompatible, outside window. | inline, notice, progress, spin |
| CLI Access (`access.html`, `?view=`) | request (inferred / unnamed), widen, allowing, save failed, timed out, blocked meanwhile, loading, load failed, none waiting; manage list loading / error / empty / CLI missing. | notice, inline, empty, skel |

Backend gaps these states exposed are in `backend.html` §1–§3; frontend bugs found in the current app are in `backend.html` §5.

---

## CLI Access (`access.html`)
- [UI] REPLACED `apps/desktop/src/routes/access/request/+page.svelte`: 520-wide window leads with the tool name + source chip (Declared / From env / Inferred / Unnamed — warn-tinted when inferred/unnamed), the `mnema …` command, and an "identity can't be verified" note. Scope = `mx-choice--list` tagged asked-for / current / needed; options below the minimum disabled, minimum pre-selected; All-retained shows a danger note and relabels the button. "Standing until idle · 30 days" replaces "does not expire". Focus starts on Deny; Esc denies.
- [UI] ~~Allow is a secondary button with a green lock icon, not the green primary — confirm that consent should carry no green pull.~~ — **Resolved (2026-10-10): confirmed.** Allow = `mx-btn` + `mx-btn__accent` lock icon, no green fill, in both the first-request and scope-widen views; the window has no `--primary` (the kit's one-per-view rule is a ceiling — banners and this window's error states have none either). Replaces today's one-off `.btn--allow` accent outline (`access/request/+page.svelte:1299-1312`, the 2026-06-25 "inverted consent colour" fix). Focus-on-Deny, Enter unbound and Esc-denies stay as they are (ADR 0059).
- [UI] NEW scope-widen variant: "Already allowed Last day → now asks for Last 7 days", with copy that issued results keep working (row widened in place, id survives) and Deny keeps the old scope.
- [BACKEND] ~~NEW Block on first request: `block_client` only flips existing rows, so blocking an unseen tool needs a writer that creates a blocked row — conflicts with "upsert_grant_for_identity is the only row-creating writer" (CLAUDE.md / ADR 0059).~~ — **Resolved (2026-10-10): add `block_identity`** in `brokered_access.rs`, a second row-creating writer for standing rejections only. Under one `with_grants_lock`: row exists → flip `blocked` (+ `blocked_at_unix_ms`) only; no row → create `{id: random, scope: LAST_DAY, blocked: true, blocked_at: now, last_used: 0}`. It never touches an existing `scope` or `id` (a blocked row never issues opaque ids, so the id-preservation reason behind the single-writer rule doesn't apply). Plus a `broker_authorization_channel` command that answers the pending request with `blocked`, its registration in `lib.rs`, and the button. Reword CLAUDE.md / AGENTS.md line 27 + the `brokered_access.rs` comment: two writers create rows — `upsert_grant_for_identity` (grants; the only writer that changes a scope or clears `blocked`) and `block_identity` (rejections; never touches a scope or id). One-line amendment to ADR 0059. Same change: Enable in Settings on a row with `last_used_at_unix_ms == 0` (blocked before ever granted) deletes the row so the tool's next request opens the normal consent window instead of silently granting last-day access; the `--no-prompt` path reports "blocked" (today it says "approval required, retryable"). ≈4 files, no migration (grants live in `broker-grants.json`).
- [UI] REPLACED `apps/desktop/src/lib/settings/panels/data/Access.svelte`: install row, "Tools with access", "Blocked", "Recent activity", empty state. Block has no confirmation; Unblock confirms and restamps last use (matches backend).
- [DERIVED] NEW "Lapses in N days" meter (last use + `BROKER_GRANT_IDLE_TTL_MS` − now) and "last ran `cmd`" (newest audit event per tool). Add `identitySource` to the TS `BrokerGrant` type (backend already sends it).
- [DECISION] No executable path / signer / team id exists — identity is self-declared by design (ADR 0059); only the source chip is shown. "Revoke" is drawn as Block, which is what `mnema access revoke` does.
- Mockup bug: `settings.html#s-access` still shows nonexistent "search · read" scopes (scopes are time windows) and a separate Revoke button — superseded by `access.html?view=manage`.

## Update window (`update.html`)
- [UI] REPLACED `apps/desktop/src/routes/update/+page.svelte`: three bands (identity, notes, controls); buttons don't move between states. Data line `0.1.23 → 0.1.24 · date · size`. Available (Later / Install and restart) → Downloading (byte progress, "Continue in background", busy primary) → Ready (Restart now). Copy from `apps/desktop/src/lib/update-window.ts`.
- [BACKEND] NEW pre-download size: the feed has no size field; `contentLengthBytes` only exists during download.
- ~~[UI][DECISION] NEW "Outside your update window" state (`availableOutOfWindow`): nothing stops working; shows licensee, window end, newest covered build, renewal value; actions "Refresh license" (existing re-check) and "Renew for a year" (`RENEWAL_CHECKOUT_URL`).~~ — **Resolved (2026-10-10): build it as drawn minus the "Newest covered" row** (no field exists; the app reads only `latest.json` — revisit with a release index). Licensee + window end come from `Licensed { name, email, update_through_ms }` (`capture-types/licensing.rs:20-26`; `name` may be ""). Refresh license = the existing re-check; Renew = `RENEWAL_CHECKOUT_URL` (`licensing.ts:70-72`; set the live link before selling, `ENV.md:64`).
- ~~[BACKEND][DECISION] Today that state never opens the window (`decide_remote_update` sets `notify: false`); opening it once per version is a ruling. The "running build newer than your window" case isn't drawn.~~ — **Resolved (2026-10-10): open it once per remote version, persisted.** `decide_remote_update` (`app_updates.rs:440-457`) notifies out of window too; that path opens the window and skips `push_update_available_notification` (its copy says "ready to install"). Gate: a `notified_out_of_window_version` field beside `channel` in `app-update-settings.json` (`:105-117`, serde default) — persisted because nothing but a renewal ends the state; the in-window gate (`notified_version`, `:797-811`) stays in memory and keeps reopening until installed. Licensing `publish` (`licensing.rs:138-152`) also kicks a silent `run_update_check(.., false)` so a Refresh or a renewal flips a stale `availableOutOfWindow` to Available at once (today it lingers up to 2 h). The fresh-install-past-window trigger (`running_build_window_gate`, `:404-433`) stays Settings-only, not drawn. ≈4 files, no migration.
- [BACKEND] State gaps: no re-check action from the window (it only installs / restarts); check-, download- and install-failed are one `failed` state, and `error.kind` can't tell a network failure while checking from one while downloading; ~~no "newest covered build" field~~ (row dropped, decision above).
- [UI] Renew exists only in Settings → License (`lib/licensing-panel.ts:21`); the window needs its own Renew link.

## Onboarding (`onboarding.html`)
Flow and copy are locked (#195); only the visual layer changes. Paths under `apps/desktop/src/routes/onboarding/` and `apps/desktop/src/lib/onboarding/`.
- [UI] NEW shell: `onboarding-shell.css` + the `+page.svelte` strip become a 52px bar with a flow ruler (8 ticks; Change settings and Voice drawn as short "optional" ticks) and a hairline action footer. REPLACES the playhead `.ob-ph-*`.
- [UI] REPLACED `lib/onboarding/CaptureSentence.svelte`: sentence-with-dials → 4 rows (rate = 11-stop `mx-seg--mono`; Keep = `mx-seg`, Everything first; folder = path + Choose…; verdict panel with plan line, verdict, fix buttons). `capture-sentence.ts` (`sentenceVerdict`) kept as is.
- [UI] REPLACED `lib/onboarding/ExcludedApps.svelte`: sentence → `mx-btn--sm aria-pressed` toggles (crossed out when recorded) + icon-only Add.
- [UI] REPLACED `screens/YourSettingsScreen.svelte`: 2×4 manifest of ticks + values, mono facts row.
- [UI] REPLACED `screens/ChangeSettingsScreen.svelte`: tabs → `mx-seg--md`; `Providers.svelte` cards → `mx-choice` tiles with download/memory meters; `ModelPickers.svelte` → `mx-seg` + spec strip + budget bar; `AiSetup.svelte` fork → two `mx-choice` tiles; `FeatureSwitches.svelte` → `mx-switch` in source colours with tree elbows (no left bars).
- [UI] REPLACED `PermissionsScreen`, `SetupScreen`, `VoiceScreen`, `FinaleScreen`: `.ob-btn` → `mx-btn` (one green primary per view); Finale recording pill → `mx-rec`.
- [UI] Motion only on Welcome (one settling rewind) and Finale (letter-by-letter reveal, pulsing rec dot); both honour reduced motion. Working-screen segmented thumbs jump, not slide.
- [UI] ~~Permissions currently holds Continue on Screen Recording (`screenGate`) — contradicts the locked "Permissions gates nothing". Mockup follows the locked rule.~~ — **Resolved (2026-10-10): a denial gates nothing; only the relaunch hold stays; every "Open System Settings" floats a permission helper window.** The third gate was added by slice 6 (`b2338d31`, a day after the locked plan) on a premise ADR 0052 had already retired; the start request already ANDs the screen source with the grant (`onboarding-start-request.ts:26`) and the Finale already has the idle and audio-only outcomes. Change: `screenGate` returns `ready: true` on a denial (label "Continue without screen recording"); keep `needsRelaunch` → "Relaunch to continue" (a grant made in-process fails on a stale stream at the Finale; one in-app click, resumes at Permissions); `gates.ts` header back to two gates; `permissions-gate.test.ts:36` flips; the ruler string "only the screen is required" goes. Finale: screen denied + mic granted = audio-only session, not idle.
- [UI][BACKEND] NEW **permission helper window** (`AppWindow::PermissionHelper`, label in `AppWindow::ALL` + `capabilities/default.json`): a small always-on-top, non-activating panel — reuse the Quick Recall NSPanel reclass (`windows.rs:654-740`) — floated by every "Open System Settings" (Permissions, Finale idle/audio-only, the status-bar flag in §2) with the app icon, "Turn on Mnema in the <Screen Recording | Microphone | Screen & System Audio Recording | Accessibility> list" and a live check that polls `get_capture_permissions` (~1 s) and closes the window on grant. **First cut without drag-out**; later, a native `NSDraggingSession` carrying the bundle's file URL (or `tauri-plugin-drag`) for the not-listed case — after a Deny macOS already lists Mnema with its switch off. Kit part `mx-helper` (SHELL.md).
- [UI] ~~Change settings still holds Back while AI features are on with no provider — a third hold beyond the two locked gates.~~ — **Resolved (2026-10-10): drop the hold; keep the in-place warning.** (Correction: the hold *was* in PLAN.md's gating table — "AI features cannot be left on with no credentials" — under a heading that says two gates.) Its premise, a silent failure, is retired: the same state is allowed and loud after onboarding (status-bar `mx-engine` "not set up" pill, Insights "AI not set up", Settings → Providers banner; reason codes `no_providers` / `no_default_model` / `no_provider_key` exist), and the auto-enable rule (connecting a provider switches the row on, `ChangeSettingsScreen.svelte:92-107`) means only a deliberate flip reaches it. Change: delete the `aiBlocked` exit branch (`ChangeSettingsScreen.svelte:196-212`; the in-place print on the AI section stays), Your settings' AI row gets an "on · nothing connected yet" value (`YourSettingsScreen.svelte:78`), plan row → soft warning. Mockup: footer chip + "Add a key" + "Turn AI features off" removed; state `change-ai-unready`.
- [UI] ~~Ruler gives Change settings slot 5; today both settings screens read 4/8.~~ — **Resolved (2026-10-10): own slot, as drawn.** Change settings = 5 / 8 · round trip with a short tick (lit while there), Voice 7 · optional (short), Setup 6, Finale 8. Today's 4 / 8 is a faithful port of the chosen #195 mockup (which also labels the round trip 4 / 8 and jumps to 6) and leaves the fifth tick never lit on any path. Change: `changeSettings.position = 5` (`onboarding-flow.svelte.ts:81`) + two short ticks in `onboarding-shell.css` / the `+page.svelte` strip. The count stays eight; the 4 → 6 jump on the straight path stays (only a seven-count would remove it — not taken).
- [UI] ~~Finale: the real app starts capture on arrival ("It has already started"); mockup draws that, not a "Press record" button.~~ — **Resolved (2026-10-10): confirmed.** Capture starts on arrival (locked #195; `FinaleScreen.svelte:1-20, 114-137` already does it); the Finale is evidence, not a summary; "Press record" was the pre-#195 accordion's finale (deleted in slice 17). Start → on Your settings is the explicit press; Voice's last button stays "Continue". No change.
- Drawn: `perm-relaunch` (holds Continue — "Relaunch to continue"), `perm-helper` / `finale-helper` (helper window floating), `finale-audio-only` (screen denied, microphone recording).
- [BACKEND] None for the figures — they come from existing manifests/formulas (~405 MB/day at 2 s, ~1.1 GB downloads). The states pass found these:
  - [BACKEND] Downloads have no pause / resume, offline state or partial resume; a full disk is typed only for semantic search.
  - [UI] A failed download leaves its feature on — only Cancel turns it off (`SetupScreen.svelte:210, 246`).
  - [BACKEND] Voice: a busy mic comes back as a raw error; "too quiet" is `NoSpeech` (`voice_enrollment.rs:35-43`), which the drawn copy already matches.
  - [UI] The relaunch resume persists only the step (`onboarding-flow.svelte.ts:97, 457`), so capture choices made before it are lost; "needs relaunch" is a frontend guess. The relaunch hold ("Relaunch to continue", `permissions-gate.ts:48`) stays — the one hold the Permissions decision above keeps.

## Quick Recall (`recall.html`)
Confirmed design: an answer-first bar that grows with its content, with Quick Look opening under it. Paths are under `apps/desktop/src/routes/quick-recall/+page.svelte` and `apps/desktop/src/lib/quick-recall/`.
- [UI] REPLACED the window shape. The 1120×720 two-pane layout (results | `DetailPane.svelte`) becomes one ~800px bar, and **the window grows and shrinks with its content** (confirmed): 60px idle, at most ~420px with results, ~570px in Quick Look (its stage is a fixed height, so scrubbing never resizes). `+page.svelte` gets a new shell that measures its content and resizes the window to it; `ResultsList.svelte` and `DetailPane.svelte` lose their two-pane layout.
- [BACKEND] Window requirement: `AppWindow::QuickRecall` (`src-tauri/src/windows.rs:279`) becomes a frameless, transparent bar sized to its content — ~800 wide, height set per state (idle 60 / results ≤420 / Quick Look ~570). Today it is 1120×720 with min 960×600: drop the min size and the 1120×720 default.
  - Resize from the frontend with `getCurrentWindow().setSize(new LogicalSize(800, h))` whenever the measured content height changes (or one Rust command that sets the frame).
  - Anchor it at a fixed top position (horizontally centred, top edge at ~15% of the screen) so it grows **downward**. `summon_quick_recall_window` calls `window.center()` today, which would re-centre a different height on every summon — replace it with the fixed anchor. If `setSize` keeps the bottom-left origin (Cocoa), re-anchor the top with `setPosition` in the same step, or do both in the Rust command.
  - Permissions: `core:window:default` does **not** grant `allow-set-size` (CLAUDE.md gotcha). The `default` capability in `capabilities/default.json` already lists `core:window:allow-set-size` and covers the `quick-recall` label — keep it; add `core:window:allow-set-position` if the frontend re-anchors. A Rust command needs neither.
  - The window is the bar's shadow edge: with no transparent margin, keep `shadow: true` (native) rather than the mockup's CSS shadow.
- [UI] REPLACED the field. `.quick-recall__field`, the Filter button and "Ask AI ⌃↵" become one bar: a mode chip (search ⇄ sparkle **Ask**, plus an "auto" tag when a detected question is being answered automatically) and text that tints recognised phrases in place, then turns them into pills (`mx-btn--sm` + `aria-pressed`) after a 700 ms pause. `FilterPicker.svelte` → a popover per pill (options with result counts, Edit as text, Remove ⌫); ⌫ at the start of the text moves into the pills. `SyntaxHelp.svelte` is dropped: the footer shows the real operator query (`pricing page app:Arc`) and the keys that work right now.
- [UI] NEW natural language → operators (new `lib/quick-recall/natural-tokens.ts`, beside `query-tokens.ts` / `filter-chips.ts`). "in Arc" → `app:Arc`; "yesterday", "last Tuesday", "this week" → `date:`, with weekdays resolved to an ISO day on the frontend (`crates/app-infra/src/search/dates.rs` only knows today/yesterday/Nd/ISO/this-week/last-week); "on a call" → `source:mic`. Pills serialise through `filter-chips.ts` as today.
- [BACKEND] NEW time-of-day filter: "afternoon" → `hours:12-18`. `crates/app-infra/src/search/query.rs` only parses app/source/date/after/before, so this needs a new operator, read in local time.
- [UI] REPLACED results. The list, the 8-day `TimelineStrip.svelte` and `DetailPane.svelte` become two parts:
  - A **best moment** card: a crop of the frame zoomed onto the match at native pixels (or the matched stretch of the waveform for audio), title, time range, snippet, a "keyword" / "by meaning" chip, Open in Timeline as the one primary, Quick Look, and Open page.
  - **Ranked one-line rows** (app tile, title, snippet with marks, a "meaning" tag, time). `SearchResultCard.svelte` becomes the row.

  Fetch limits are unchanged (24 frames / 12 audio, `result-sections.ts`); one visible cap replaces the two section caps (decision 9 below).
- ~~[DECISION] One ranked list merges frames and audio; today they are separate sections with separate ranks. This needs a comparable score or a simple interleave rule.~~ — **Resolved (2026-10-10): merge by `rank` with the broker's rule, on the frontend.** Both kinds are scored by the same `bm25(search_documents_fts, 5.0, 1.0)` over one FTS5 table (`retrieval.rs:407/560`), so keyword-only the scores are directly comparable; under hybrid both kinds are RRF-fused per kind on the same scale (`retrieval.rs:306-351`), so the merge is a position interleave that pulls up hits found both by keyword and meaning. [BACKEND] Add `rank: f64` to `FrameSearchResultDto` / `AudioSearchResultDto` (`app_infra.rs:557-610`, `From` impls `:894-940` drop it today) and to the TS mirror (`lib/types/app-infra.ts:230-287`). [UI] `result-sections.ts` merges the two fetched lists two-pointer: lower rank first, tie → newer, then frame (the broker's `frame_outranks_audio`, `brokered_access.rs:3507-3540, 3644-3650`, tested). Best moment = top of the merged list, either kind; `ResultsList.svelte` loses its Screen/Audio sections; selection walks the one list. No search-engine change; no kind weight unless the BM25 length bias (short transcript spans vs whole-screen OCR) is measured first — ADR 0036 rejects score calibration.
- [UI] NEW Quick Look, anchored to the bar. How to open it:
  - Space once focus is in the results (↓ moves from the field into the list). Space while typing in the field is a normal space.
  - The Quick Look button on the best moment, the eye button on a row (shown on hover/selection), or a click on an answer citation.
  - It works on the best moment, any row, or an answer citation. The preview scales up from its source (crop, row, citation chip), and the bar stays on top and stays editable.

  What it shows:
  - **Screen:** the whole frame with matched words outlined (solid = keyword, dashed = by meaning), a 7-frame strip 15 s apart that ←/→ scrubs, and a side panel with that frame's text (matching lines in strong text), the why-chip, Open this frame, Copy text and Page.
  - **Audio:** the matched line set large over a zoomed waveform; the strip is the transcript turns, plus a transcript panel and "On screen then" (`alignedFrame` is already on `AudioSearchResultDto`).

  ↑/↓ moves to other results. Esc/Space returns to the list with the same selection. Typing closes it and edits the query.
- [UI] Final keymap (confirmed):
  - **Field:** typing edits the query; → with the caret at the end accepts the inline ghost completion (unchanged from `search-keys.ts:201`; the dimmed suffix and a "→ complete" footer hint are drawn); Tab = search ⇄ ask (Tab no longer accepts the ghost — → does); ↵ = open the selected moment / continue the answer in Chat; ⌘↵ = open the page (or Chat from an answer); ⌘C = copy; ⌫ at the start walks into the pills; Esc stops a stream, then clears, then closes.
  - **List (after ↓):** ↑/↓ move; Space = Quick Look; ↵ = open; on the answer, ←/→ walk the citations. Typing returns to the field.
  - **Quick Look:** ←/→ scrub ±15 s (audio: transcript lines); ↑/↓ other results; ↵ opens that frame; ⌘↵ page; ⌘C copy; Esc or Space back to the list.
- [UI] Hand-off: ↵ in Quick Look opens **that** frame — the frame id of the scrubbed neighbour, not the hit's representative — through the existing `open_capture_result_in_main_window` (`frameId`). Audio passes the turn's `spanStartMs`. Then the window closes. ⌘↵ = `open_captured_url`; ⌘C copies the frame's OCR text (`list_processing_results`, as `detailStore.svelte.ts` already does) or the quote.
- [UI] REPLACED Ask:
  - A question (trailing `?` or a leading who/what/when…) is detected; Tab forces either way.
  - When auto-answer is on for the current model, the answer streams in place after a 750 ms pause over the unchanged `ask_ai` session, `TurnView` and typed blocks. When it's off, the hero slot shows a quiet "Press Tab to ask <model>" row (↵ or a click also asks) and the moments stay below; nothing is sent until the user asks. The `mnema-timeline` block sits beside the prose so the answer stays within ~420px, and the evidence rows below are numbered by citation.
  - "Continue in Chat ↵" in the answer header replaces the "← Back" header and the follow-up composer; Esc stops.
  - Citations are selectable from the list: ←/→ walks them and lights the cited evidence row. Space (or a click) opens Quick Look on the cited frame or audio; closing it returns to the answer with that citation still selected.
- [UI] NEW setting "Answer questions automatically" in Settings → Ask AI (`lib/settings/panels/intelligence/AskAi.svelte`), an `mx-choice`: **Always** · **Only with local models** (default) · **Never** (press Tab to ask). Auto-answer spends a model call on every question the user pauses on (today asking is explicit ⌃/⌘↵, ADR 0025); the default keeps cloud calls — money and an upload — explicit. "Local" = the resolved model's provider kind `is_local()` (Ollama / Llamafile, `capture-types/src/recording.rs:593`).
- [BACKEND] New app setting key: `quickRecallAutoAnswer: "always" | "localOnly" | "never"` on `AccessSettings` beside `askAiEnabled` (`capture-types/src/recording.rs:514`, serde default `localOnly`), plus `UpdateAccessSettingsRequest` (`:1333`) and the TS mirror in `lib/types/recording.ts`. JSON settings, no migration. Whatever the setting, an edit must cancel the in-flight `ask_ai` turn rather than queue a second one.
- [UI] REPLACED empty states. Idle = the bare bar with "↓ recent". ↓ shows Recent and Try, with recognised phrases already tinted. No results = one sentence plus ways out as selectable rows with counts: "Drop 'Arc' · 1 moment", "Any time — not just …", "All of Tue, not just the afternoon", "Ask instead".
- ~~[NEW FEATURE] Recent queries: no query history exists today. Store the last few locally. [DECISION] Queries are sensitive: should Delete Recent Capture clear them?~~ — **Resolved (2026-10-10): no — recent queries take the conversations' contract.** [BACKEND] One JSON key `quick_recall.recent_queries` in the encrypted `app_settings` table (the last 8 of `{text, ask, at}`; searches and asks in one list, no join with the conversation store), read/written by two new commands (`app_settings` has no generic accessor — each key gets its own, like `user_context.local_offset_minutes`). Retention prunes entries older than its cutoff beside the conversations (`capture_retention.rs:492-500`); Wipe User Context deletes the key (`user_context/commands.rs:797-801`); Delete Recent Capture leaves it alone (its transaction, `app_infra.rs:3759-3983`, never names typed text — same as conversations, migration `0028` header). Never localStorage: plaintext outside the SQLCipher DB and unreachable from Rust. [UI] ⌫ on a selected Recent row removes that one query; the footer shows "⌫ remove" only while a Recent row is selected. No migration.
- [NEW FEATURE][BACKEND] Indexing line in the no-results state ("Still reading your last 4 min — 38 frames queued"). It needs the same new user-facing command as before: `count_queued_or_running_processing_jobs_for_processor` (`crates/app-infra/src/lib.rs`) plus the backlog from `get_semantic_index_status` (`src-tauri/src/debug_status.rs`).
- [BACKEND] Data the drawing needs (details in `backend.html`):
  - **Match boxes per hit.** OCR geometry is stored (`processing_results.structured_payload_json`, `OcrBoundingBox`), but `FrameSearchResultDto` returns none. Needed for the zoomed crop and the outlines.
  - **Neighbouring frames for the strip:** `list_frame_summaries_in_range` + `get_frame_scrub_previews` (both exist — verify).
  - **Counts per pill option:** today only `hasMore*`.
  - **Keyword vs meaning** already exists (`foundByMeaning`).
  - **Why results may be partial** (first run, meaning off, audio pending): `SearchCaptureResponse` (`lib/types/app-infra.ts:230`) can't tell an empty index from no matches; a failed semantic fetch silently falls back to keywords (`search/retrieval.rs:183`); audio is only indexed once transcribed (`search/projection.rs:249-254`). OCR backlog and semantic build progress are debug-only (`debug_pipeline.rs:37`, `debug_status.rs:87`) — the indexing command above covers them.
  - **Recording state** needs no new command: `get_capture_permissions` returns the session; listen to `native_capture_session_changed`.
- [UI] Bugs today: Quick Recall ignores the `missingReason` it already receives (`frame_preview.rs:567`, `detailStore.svelte.ts:115-124`); `friendlyAskReason` has no `needs_reconnect:` / `provider_unreachable:` cases (`+page.svelte:568-587`); a search failure shows the raw "failed to search captured content: …" (`src-tauri/src/app_infra.rs:5044`).
- Becomes dead: `TimelineStrip.svelte`, `timeline-dots.ts` (+ test), `SyntaxHelp.svelte`, the picker half of `FilterPicker.svelte` / `filterSurfaces.svelte.ts`, and the Tab ghost-accept in `search-keys.ts` (→ ghost-accept stays).
- Shortcut (resolved): the Quick Recall window keeps the real default ⌥⌘Space (`src-tauri/src/keyboard_bindings.rs`); the main-window field never advertises ⌥Space or ⌥⌘Space (SHELL.md).
