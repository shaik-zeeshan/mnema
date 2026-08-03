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
