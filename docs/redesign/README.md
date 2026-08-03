# Full-app redesign

**The design is [`mockups/design.html`](mockups/design.html).** Its binding decision record
is [`DECISIONS.md`](DECISIONS.md); the implementable system is [`system.css`](system.css).
That is the whole directory.

```
open docs/redesign/mockups/design.html
```

How it got here, in three lines: round 1 (five shells) got the grid right and the
architecture wrong; round 2 (five de-boxed visual systems) fixed the architecture and was
rejected whole on feel (*"make sure they feel like a native app"*); round 3 (five native
macOS idioms) was narrowed to Grouped (System Settings idiom) and Material (Music/Control
Center idiom), which the design merges — grouped fills as the one surface system, material
scroll-under chrome on top. The fifteen round mockups and their briefs were deleted once the
design converged; this file carries everything from them that still binds.

## What the design is

13's grouped calm as the base (single-level borderless fills, one radius — a settings group,
a bento tile and the CLI card are the same surface) with 14's contributions re-skinned onto
it: the Overview bento tile grid, the important-moments strip, and content scrolling
edge-to-edge under the material toolbar. The Overview digest cites frames and conversations
inline; the **Conversations tile** presents audio as conversations ("Launch sync · 38 min ·
5 speakers" → diarized transcript), never as minutes — the competitor-research outcome in
`DECISIONS.md`. Recording chrome is the **state pill** (one capsule: dot + elapsed + cost,
native popover for transport, tray keeps full transport) — adopted 2026-08-03. Settings
icons follow the shipping monochrome Lucide convention.

**How to review it:** frame 00 (what each parent contributed) · 01 Timeline · 04 the bento
Overview at three widths · 05 the conversation UI · 07 the surface switch · 08 the Quick
Look grid · 11 the state pill (all nine states, popover, degradation ladder, and the old
cluster it replaced) · the Native audit after frame 16 · frames 17/18/19 (component→code
map, type specimen, annotated where-it's-used) · Deviations and Border count at the end.
Light/dark toggle top-right; both themes are fully designed.

## The rules that bind (formerly the round briefs)

**Architecture — settled.**
- Two surfaces in the main window: **Timeline** and **Overview (AI)**, peers in a switcher
  (`⌘1`/`⌘2`); the user picks which one the app opens on. Recording chrome lives on both —
  AI never outranks the record control.
- **Search and Ask both live in the Quick Look window** (today's `quick-recall`, ⌘⌥Space,
  1120×720 fixed). The grid is 3-up, 349×196, fixed 16:9, row-aligned, no masonry; results
  hand off to Main focused on Timeline at that instant.
- **Same window set as ships.** Settings is an in-main route. No new windows.
- Main window 1100×720 default, 800×600 floor; design proves 800×600 / 1100×720 / 1440×900.

**The timeline is FROZEN.** `apps/desktop/src/routes/+page.svelte` (markup from ~line 6043)
renders exactly as it ships — thin bar → stage → fixed-height rail-wrap (8px-per-frame tick
rail, app bands, sibling two-row mic/sys audio lane, pointer-following readout) → AudioDrawer.
The only permitted change is adopting the shared type roles and spacing constants. The mic/sys
lane stays: it is a coverage/liveness indicator (mic + system audio keep recording while the
screen sleeps — ADR 0021/0052), not a transcript surface. Improvement ideas live in the
design's Deviations as proposals, never as work.

**De-boxing (eight rules, condensed).** A bordered element never contains another bordered
element; no border sits flush against another; captions have no container; depth is surface
step, spacing, or (floating only) shadow — never an edge; no left-accent-bar callouts; panels
separate by one hairline or nothing; rounded corners only on things that float or clip an
image; count your borders (~12 ceiling on a 1100px window, stated in the file — the ceiling
counts **containers, not control rings**: a pill's or segmented control's own outline is free;
grill 2026-08-03). Clarification: a **single-level borderless filled group** (the System
Settings idiom) is a surface step and is allowed.

**The native bar (nine points).** HIG window anatomy · AppKit controls drawn accurately
(accent-filled default button, real focus ring, `:active` depress) · NSMenu anatomy for
menus/popovers · materials only where macOS uses them, content on opaque surface · full-row
accent selection, never underlines · native density (28px rows, 13px default) · one
`--accent`-driven tint layer · overlay scrollbars, default cursor on lists, `tabular-nums` ·
nothing web-shaped. The design's Native audit block states where each point is proven.

**Errors are toasts** — bottom-right, stack to three, overlay content, never reflow, errors
never auto-dismiss; everything archived in the bell. Inline validation only in reserved space;
modal only when the app cannot continue. **The title bar degrades on a designed ladder**:
cost → elapsed → single glyph → dot alone; the dot and the switcher never go.

## Implementing

**`system.css` is the source of truth.** Its colour tokens are byte-identical to
`+layout.svelte`'s `:root` today, so the block pastes in without changing any existing rule.
It adds the type ramp, named spacing constants, control metrics, elevation and motion rules,
and the shared primitives (`.btn` / `.input` / `.toast` / `.kbd` / `.pill`) that replace
`.btn` being re-declared in six files' scoped styles. The design carries a verbatim copy plus
a marked `/* direction-specific */` block.

| token | px | lh | tracking | weight | family | for |
|---|---|---|---|---|---|---|
| `--t-label` | 10 | 1.4 | +.02em | 510 | mono | machine labels, column heads, kbd, units. Never a sentence. |
| `--t-meta` | 11 | 1.35 | +.01em | 400 | either | timestamps, counts, helper lines, frame captions |
| `--t-ui` | **13** | 1.25 | −.006em | 400 | sans | **the default** — buttons, rows, labels, nav, menus |
| `--t-read` | 14 | 1.55 | −.008em | 400 | sans | prose only: transcripts, AI answers, errors. Max 70ch |
| `--t-title` | 17 | 1.3 | −.016em | 590 | sans | screen and section titles, dialog headings |
| `--t-display` | 22 | 1.2 | −.02em | 590 | either | **one per screen** — the readout clock, a hero number |

Six sizes, three weights (400/510/590), 1.25 for UI and 1.55 for prose; no 12/15/16/20/26px,
no weights 300/680. Body moves 12 → 13px (macOS Body). The shipping `--text-*` family is
replaced by these roles; the migration map is in `system.css` §2.

Sequence (grill-final, 2026-08-03): fix the dark surface steps in `system.css` → land
`system.css` into `+layout.svelte` → build the shared `.btn` (frame 17 is the component→code
map) → the state pill → the per-source mid-session mask (user intent routed through the
existing per-source paused-flag seam; popover toggles render disabled-while-recording until
it lands) → the surface switcher + "open Mnema on" setting (default: Timeline) → move search
into Quick Look + the grid (the Overview Ask field is a *launcher* into Quick Look, never an
answer surface) → the toast system → the conversations read-time JOIN
(`user_context_activities` × `speaker_turns`) + the moments focus/duration heuristic →
**the Overview bento last, built against real data**. Full decision record: `DECISIONS.md`
"Grill" section.

Known gaps, recorded in `system.css` itself: dark surface steps too tight for region-scale
separation (**fix first** — see sequence); the rest fix-when-hit: no text-over-image guidance;
no oversized-input role (a big search field borrows `--t-title`); object-size ramp incomplete.
