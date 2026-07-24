# Warm Paper / Story-First Shell — Design Spec

Converged 2026-07-23 from a Littlebird-inspired design exploration. **Pinned reference:
[`main-surface/story-first-v5.html`](main-surface/story-first-v5.html)** (final shell, light + dark, capture model).
The [`app-match/`](app-match/) set is the app-accurate anatomy reference per surface; the five
`direction-*.html` files and `main-surface/story-first{,-v2,-v3,-v4}.html` / `vertical-spine.html`
are exploration history — mood boards, not specs.

## Vision

Mnema's story layer becomes the product's face; the machinery becomes the receipt layer.
The window opens onto *your day, written* — not a frame scrubber. Littlebird's friendliness
(one rail, story-first, template-first creation) grafted onto what only Mnema has:
mechanically-detected meetings, evidence for every claim, local-first capture.

## Visual system — "Warm Paper"

- **Light (default face):** cream three-layer stack — `#ece8dc` desk → `#faf8f2` shell → `#fffdf7` cards.
- **Dark sibling:** warm charcoal, never blue — shell `#17150f`, rail `#131109`, card `#1e1b13`, ivory ink.
- **Accent:** botanical green `#1f6f4a` / `#2a8a60` (+ pale `#e7f1e9` tints); lifted to `#5cbd8d`/`#7ed3a8` on dark.
- **Type split (the Mnema signature):** serif (Iowan/Palatino stack) for human narrative — greetings,
  titles, digest, activity prose; **mono strictly for machine data** — timestamps, durations, provenance,
  status, condition glyphs. Warm-red reserved for the live/record dot; amber for timed-pause states.
- Condition/origin glyphs stay typographic: ◉ meeting-ends, ▣ app-opened, ◷ schedule. Never emoji.
- Implemented via the existing `[data-theme]` machinery; current dark terminal theme is superseded,
  not deleted, until Warm Paper Dark reaches parity.

## Surface model (final)

- **Rail:** Today · Meetings · Subjects · Triggers, plain entries (no group labels, no sub-tabs),
  then `＋ New chat`, then chat history in the **tightened-B** treatment: search + `all ▾` origin scope
  in one row, origin as a small glyph before the title, times hidden until hover, 8.5px group labels.
  Engine/model footer stays.
- **Overview no longer exists** — the Today front page *is* the overview.
  **Context folds into Subjects** (both are "what Mnema believes").
- **Today (home):** serif greeting → **digest paragraph** (3–5 real sentences narrating the day,
  ↻ re-read, four lede stats in a quiet mono row) → meeting chips → **Ask-Mnema composer** with
  2–3 suggestion chips drawn from real recent activity → the journal river.
- **River entries = borderless ledger prose** (alternates-board treatment A): hanging mono time gutter,
  tinted small-caps category word leading a serif sentence, quiet mono receipt line; sub-5-min activities
  are single lines; recap-ready is an inline green link. No card boxes, no edge bars.
- **Evidence on demand:** an activity's `▸ N frames · receipt` opens a drawer scoped to that span —
  frame stage with OCR overlays, pinned readout, span-scoped scrubber, mic/system-audio lanes,
  play-at-moment, "Heard in this span" transcript strip. Footer link "Browse full timeline ›" is the trust door.
- **Raw Timeline survives** as a full surface behind a clock **icon button** in the titlebar-right cluster
  (sibling of theme + settings). No Timeline|Insights|Triggers segmented switcher anywhere.

## Capture model — "the record"

Recording is the day's default state, not an activity. No Record/Stop grammar, no per-source
titlebar glyphs.

- **Titlebar pill** (after traffic lights): `● On the record · since 8:47 AM` (rec-tint, pulsing) /
  `◌ Off the record · 12m left` (amber, counting down, hover → `resume now`) /
  `○ Off the record · since 6:41 PM` (muted). Clicking opens the **capture popover**: per-source rows
  (screen / mic / system-audio — device, mode, live/off), session clock, today's stats line.
- **One quiet action button:** `Go off the record ▾` ↔ `Back on the record`.
- **Timed pause menu:** For 15 minutes / For 1 hour / Until I turn it back on, + `Stop for today`.
  Timed options auto-resume and show their resume time; the failure mode being designed against is
  "paused for a sensitive call, forgot to resume, lost the afternoon."
- **The live edge carries the state in the story's vocabulary:** capturing → breathing
  `◦ On the record — writing this hour…` entry; paused → an away-gap-styled note forming in real time
  (`— off the record since 2:12 PM · resumes 2:27 PM —`). The user sees the hole forming in their story.

## Meetings (new surface)

Keyed off **detected meetings** (ADR 0057 mic-holds), not trigger runs — a meeting with no recap
trigger still appears with its transcript. Anatomy per `app-match/meetings.html`:

- **List:** day-grouped rows — inferred title (`TITLE INFERRED` tag where recap-named), time + duration,
  diarized speaker chips ("You", named voices), provenance line (`detected via mic-hold · zoom.us`),
  state: recap ready / transcript only / processing (readiness wait) / skipped (quiet).
- **Detail:** Summary / Transcript / Notes tabs. Summary is the trigger-run document when one exists
  (eyebrow `◉ TRIGGER RUN · MEETING RECAP`, firing/readiness footer, "Open run conversation");
  Transcript is speaker-labeled turns with wall-clock mono times; Notes is the user's own text.
- **No live transcript in v1** — batch pipeline + readiness wait stands; processing rows say so honestly.

## Triggers — gallery-first creation

The wizard gains a **Template step ahead of the existing 01→03 flow** (`app-match/triggers.html`):
a card gallery (Meeting Recap, Daily Digest, "When I open Figma, brief me", Weekly Review, …) where
every card is expressible as the real `TriggerCondition` wire type × a prefilled prompt. Picking a card
prefills and lands on **Review** with a removable `from template · <name>` chip (same mechanic as Import);
`Start from scratch` routes to step 01 Condition. List page and Review step anatomy unchanged from the
shipped triggers feature, restyled Warm Paper.

## North star (not now)

`main-surface/vertical-spine.html` — the whole day on one vertical time spine with filmstrip evidence
riding beside every block and expand-in-place. The receipt drawer's scoped evidence panel is deliberately
the component the spine would later inline. Revisit after story-first proves out in daily use.
