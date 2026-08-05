# Round 3 — founder feedback (2026-08-03)

> **Status:** acted on. `mockups/design.html` implements everything below (the keeps,
> the Conversations tile, the state-pill proposal in its frame 11). The 13 spacing fixes are
> applied, plus a nested-`<p>` markup bug in 13's frame 05 cite chips found during convergence.

Verbatim intent, interpreted. The five round-3 directions narrowed to **two: 13 Grouped and
14 Material**. No winner picked yet; the likely end state is a convergence of the two.

## What was liked (binding for whatever comes next)

**From 14 (Material):**
- The Overview bento — Control-Center tiles showing *different kinds* of things: today's
  conversation/digest, capture state, disk cost today.
- The Context tile — good *because* it shows everything (facts count + newest fact), keep that.
- The important-moments strip — "the main things you did today, shown as frames" — called out
  as really good. Keep real frames as the unit.

**From 13 (Grouped):**
- The Overview digest with **frame citations** — prose that cites today's actual moments.
- Subjects and conversations presentation.
- The chat / conversation UI.
- The recording-chrome states (frame 11) — good, but see fixes below.
- The CLI access dialog.

## Open questions the founder raised

1. **Recording controls in the top navbar** — "not sure about them." Not rejected; uncertain.
   See recommendation below.
2. **The audio (mic) display** — "should it be displayed with all the things? Is there a better
   way? Check how the competitors do it." See research below.

## Fixes ordered

- 13's frame 11 (recording chrome) + frame 14 (toasts): spacing, icon sizing, padding — icons
  drawn at one size in rows cleared for another, uneven padding at intersections. (Polish pass
  applied 2026-08-03.)
- **Settings icons stay the shipping style — monochrome, never multicolor.** "Current icons
  and style looks great; in design having multiple color doesn't look great." The mockups'
  System-Settings-style colored icon badges in the settings nav are replaced with the app's
  real convention: monochrome Lucide-family line glyphs, 16px, color inherited from the row
  (`section-icons.ts` + the rail CSS own this in the app). Applied to 13 and 16 on 2026-08-03.
  Third-party *app* icons (privacy exclude list: 1Password, Signal…) keep their real colors —
  that is app identity, not decoration.

## Competitor research — how audio is displayed (2026-08-03)

Sources: Kevin Chen's Rewind app teardown (kevinchen.co), Limitless help docs
(help.limitless.ai — Lifelog/transcript articles), screenpipe.com/about + GitHub readme.

- **Rewind (now Limitless desktop)** — audio is **meeting-scoped, not ambient**. It asks to
  record when you join a call; the transcript attaches to that meeting and appears beside the
  screen recording in the history browser. The timeline scrubber itself is screen-only (app
  icons + favicons). No persistent audio lane.
- **Limitless (Pendant/Lifelog)** — audio-first product; the day view is a **list of
  summarized conversations** ("lifelogs"): a summary header, transcript expands under a
  disclosure, speakers editable inline. Audio playback exists per entry. Again: no waveform
  lane — the unit is *the conversation*, not the recording.
- **Screenpipe** — DVR-style screen timeline; audio is transcribed continuously but surfaces
  as **playback + transcript attached to a moment** and as search results with clips. No
  parallel audio lane in the timeline visual.
- **Microsoft Recall** — captures no audio at all (screen snapshots only).

**Pattern: nobody draws a persistent audio lane next to the screen timeline. Audio enters
every competitor's UI as *conversations attached to moments* — summarized, diarized,
expandable — or as search results.**

## Recommendation (Claude, 2026-08-03)

1. **Keep the mic/sys lane on the Timeline.** It is not a transcript surface — it is a
   *coverage/liveness* indicator: it shows when audio exists so you can scrub to it, and it is
   the only honest way to show that mic + system audio keep recording while the screen is
   dark/locked (ADR 0021/0052 behaviour no competitor can even express). It is also part of
   the frozen shipping timeline. This is a differentiator, not clutter.
2. **Everywhere else, adopt the competitor unit: the conversation, not the minutes.** Replace
   14's "Audio Today — Microphone 1h 12m / System 2h 03m" tile with a **Conversations** tile:
   "Design review · 41 min · 3 speakers", tap → diarized transcript (13's conversation UI).
   Digest cites conversations the same way it cites frames. Quick Look already treats audio
   results as grid cards with transcript snippet — keep that.
3. **Recording controls: keep state in the title bar, collapse transport into it.** One calm
   pill (state dot + elapsed/GB per the degradation ladder); click opens a popover with
   pause/stop/sources — same anatomy as macOS menu-bar screen-recording controls — and the
   tray keeps full transport. Rewind/Screenpipe hide recording in the menu bar entirely, but
   Mnema's settled rule is that recording state must stay visible in-window; the pill keeps
   the visibility while removing the button row the founder is unsure about.
   → **ADOPTED 2026-08-03** ("update the design"): the pill is the recording chrome across
   all of 16; the old cluster survives only in frame 11's for-the-record comparison row.
   The pill primitive and the monochrome-icon rule are now in `system.css` §6.

---

# Grill — design pressure-test (2026-08-03)

`mockups/design.html` was grilled in fourteen questions across two rounds; every answer
below is a founder decision ("agreed", with one modification on the conversations data
source). Backend claims were verified against the code, the design against rendered
screenshots at all three widths, both themes.

## Recording chrome

- **Two-click in-window stop is accepted.** Pill → popover → Stop; the popover's first item
  is Stop/Pause. Panic is already covered outside the window: the app ships four global
  shortcuts including ToggleRecording and Pause/Resume (`keyboard_bindings.rs`), and the
  tray has full transport.
- **Popover source toggles ship in two steps.** Verified: mid-session per-source machinery
  exists internally (inactivity pause flags per source; the mic is genuinely released and
  restored mid-session for voice enrollment) but there is **no user-facing path** — the only
  user control writes settings for the next session, and the tray hard-disables source
  toggles while recording. Decision: build a user-scoped per-source mask as its own slice
  right after the pill chrome, routed through the existing paused-flag seam. Until it lands,
  popover toggles render disabled-while-recording (today's tray semantics). Tray and
  shortcut labels must always match the popover — one behavior everywhere.

## Surfaces

- **Out-of-box default surface: Timeline.** ("User picks the main" still stands; this is
  the value before they pick.)
- **Frozen boundary confirmed as drawn:** the new title bar (switcher + pill) sits *above*
  the shipping thin bar; thin bar down is frozen, re-typed only. Nothing is absorbed.
- **Materials are CSS.** `backdrop-filter` samples the app's own content, which is all
  scroll-under-chrome needs; NSVisualEffectView (behind-window vibrancy) is deferred
  indefinitely. Scroll-under being Overview-only — the frozen Timeline is opaque — is
  accepted as honest, not a compromise.
- **800×600 drop order accepted with one change: the captured-hours hero stays.** The
  Capture tile drops "270 MB today" instead — the hero is the screen's one `--t-display`
  use. Everything else (This-Week tile, Ask-history tile, speaker counts, one-sentence
  digest) drops as drawn.
- **The Overview Ask field is a launcher, not a surface.** Typing + return opens Quick Look
  in Ask mode; history rows reopen the conversation in Quick Look; an answer never renders
  in Overview. The Search+Ask-only-in-Quick-Look rule holds unamended.

## Conversations & moments (the backend branch)

- **Verified: no conversation aggregation exists.** "Conversation" in the codebase means AI
  chat threads. `speaker_turns` has the raw material (cluster ids, timestamps) but nothing
  groups it, counts speakers per window, or titles an audio grouping.
- **The conversation unit rides `user_context_activities`** (founder's modification: extend
  the activity rather than build a sessionization entity). An activity *is* a conversation
  when a **read-time JOIN** against overlapping `speaker_turns` shows meaningful turn
  coverage; speaker count = distinct clusters in the overlap; turns spilling past the
  activity's end extend the *displayed* duration without mutating the activity row. No
  migration, no prompt change, retroactive over existing data. The LLM keeps its job (title,
  boundaries); whether speech happened inside them is a fact the database knows.
- **"Meaningful" = total overlapping turn time ≥ 2 minutes** (grill 2026-08-04). One knob,
  no minimum speaker count — a 1-cluster entry still shows as "1 speaker" rather than
  hiding a merged-diarization meeting. Accepted consequence: system-audio speech from media
  passes the bar (an hour of a talk video shows as "1h · 1 speaker"); media-vs-conversation
  filtering is explicitly not attempted in v1. Tune the knob when it feels wrong.
- **Freshness lag accepted for v1.** The derivation worker beats every 2–10 min over
  2–30 min windows, so a finished conversation appears within ~5–40 min. No in-progress
  row; the tile shares the digest's "updated HH:MM" semantics.
- **Moments strip v1 = headline frames + a dumb rule.** Verified: each activity already has
  an engine-nominated headline frame (`is_headline` evidence) plus focus level and duration.
  The strip is the day's activities' headline frames ordered by a focus+duration heuristic.
  No ranking infrastructure; tune the rule if it feels wrong.

## Rules & system

- **The border ceiling counts containers, not control rings.** A pill's or segmented
  control's own outline is free. This replaces the Settings-frame-at-13 annotation — the
  frame is legitimately under the ceiling. README's de-boxing rule updated to match.
- **`system.css` dark surface steps get fixed before implementation starts** — every
  dark-mode region separation depends on them, and retuning surface tokens after components
  ship touches everything. The other three recorded gaps (text-over-image, oversized-input
  role, object-size ramp) are fix-when-hit at the component that needs them.

## Dialogs (grill 2026-08-04)

- **Confirmations and alerts always use `@tauri-apps/plugin-dialog`** — the CLAUDE.md rule
  stands unamended. Where a mockup draws a styled confirm or alert, do **not** follow its
  chrome; carry over only the content (title, message, button labels). The in-DOM
  `Dialog.svelte` exists solely for rich-content sheets the native plugin cannot render
  (the Settings sheet's forms, the CLI consent card). No existing confirm migrates.

## Implementation sequence (final)

Dark-step fix in `system.css` → land `system.css` into `+layout.svelte` → shared `.btn` →
state pill → per-source mask slice → switcher + "open Mnema on" setting (default Timeline)
→ Quick Look search + grid + Ask-field launcher wiring → toasts → conversations JOIN +
moments heuristic → **Overview bento last, built against real data** (its headline tiles
are why it exists; it never ships hollow).
