# Round 4 — five whole-app mockups

Founder feedback (2026-08-05) on everything so far: **the converged design as a whole is not
it — but the bento Overview is.** "Whatever design we have done till now, I don't like it,
but I did like the bento design. It is not the whole app. I need a whole app." The previous
session changed only the Overview and left the rest of the app as-is; this round designs the
**entire app**, five ways, each in its own folder.

## What the founder said — binding for every direction

1. **Keep/build on the bento Overview** (`../mockups/design.html` frame 04). It may be
   re-skinned per direction but the tile-grid idea — heterogeneous tiles: digest, capture
   state, conversations, context, moments strip — stays.
2. **Timeline layout is settled** — "a perfect layout for viewing and scrolling through the
   history." Keep the shipping model: thin bar → one big stage → fixed-height tick rail with
   app bands (newest anchored RIGHT) → sibling two-row mic/sys audio lane → pointer-following
   readout → AudioDrawer. **Audio Drawer is good — keep it.** What SHOULD be improved:
   - the **date-range selector** (how you jump/navigate across days of history), and
   - the secondary controls around the rail (the hour/readout affordances, the re-run/refresh
     control) — polish, not reinvention.
3. **Quick Access (⌘⌥Space): Search and Ask AI should be clearly DIFFERENT.** Today they read
   as the same surface. Differentiate them visually and behaviorally (mode identity, layout,
   result presentation) while both staying in the one Quick Access window.
4. **NEW FEATURE — "Ask about the current frame" (every direction must design it):**
   from Quick Access Ask AI, the window gets out of the way and shows what's beneath/behind
   it — the user's current screen. The current frame is captured, previewed as attached
   context, and sent to the AI with the user's question. Design the full anatomy: how the
   palette collapses (e.g. to a floating bar), how the frame is previewed beneath the input,
   how "AI can see this" is communicated, and the answer state.
5. **Settings is good but improvable.** Two ordered fixes in every direction:
   - the left **sidebar is "a bit weird"** — rethink settings navigation (better rail,
     searchable single scroll, sticky group headers, horizontal top-level… your call), and
   - the **autosave indicator at the bottom clips off-screen** — the saved/saving state must
     always be fully visible (inline near the changed control, toast, header chip — never a
     bottom bar that can fall off the viewport).
6. **Settings custom inputs — be creative, but restrained.** Don't stop at stock HTML
   controls. Each setting *type* can get an input that expresses what it means: a disk-budget
   slider that shows GB/day cost, a retention picker that shows what survives, a duty-cycle
   control, a shortcut recorder, a model picker with size/RAM badges, a consequence-preview
   toggle row. BUT: "adding too much there would make it pretty big — we don't want that."
   A handful of high-value custom inputs, not a themepark. **Onboarding is untouched** (it's
   good) — custom inputs are for Settings only.

## Carried over from rounds 1–3 (still binding, do not re-litigate)

- **Native macOS feel** (round 2 was rejected whole on this). The nine-point native bar in
  `../README.md`: HIG window anatomy, accurately drawn AppKit-like controls, NSMenu anatomy
  for menus/popovers, materials only where macOS uses them, full-row accent selection,
  native density (28px rows, 13px default type), one accent tint, overlay scrollbars,
  `tabular-nums`, nothing web-shaped.
- **De-boxing**: no box-in-box, no border flush against another, captions have no container,
  depth = surface step/space, ~12 bordered *containers* ceiling per window (control bezels
  are free). Single-level borderless filled groups (System Settings idiom) are the surface
  system.
- **Architecture**: two main-window surfaces — Timeline + Overview (AI) — peers in a
  switcher (⌘1/⌘2); Search + Ask live in Quick Access (1120×720 fixed); Settings is an
  in-main route; no new windows. Main window 1100×720 default, 800×600 floor.
- **Recording chrome = the state pill** (dot + elapsed + cost; popover with transport;
  degradation ladder cost → elapsed → glyph → dot). See design.html frame 11.
- **Audio is conversations, not minutes**, everywhere except the timeline's mic/sys
  coverage lane (which stays — it's a liveness differentiator).
- **Errors are toasts** — bottom-right, overlay, never reflow; errors never auto-dismiss.
- **Monochrome Lucide-style icons** — multicolor only for real third-party app icons.
- **Type ramp** from `../system.css`: t-label 10 mono / t-meta 11 / t-ui 13 (default) /
  t-read 14 / t-title 17 / t-display 22 (one per screen); weights 400/510/590; sans = human,
  mono = machine. Directions may adapt *values* coherently but must keep a role-based ramp
  and stay internally consistent.
- Confirms/alerts are native `plugin-dialog` — never draw styled confirm chrome.

## Deliverables — per direction folder `NN-<slug>/`

Every page is a **self-contained HTML file** (all CSS inline, no external assets, opens via
`file://`), with a **page-identification masthead** at the top (outside the rendered app
window): direction name, page number + page name, what to look at. Both themes designed
(light/dark toggle like design.html). Frames render at true size (1100×720 main window,
1120×720 Quick Access).

| file | contents |
|---|---|
| `README.md` | direction statement: the idea in 5 lines, what it does with each founder ask |
| `01-overview.html` | the bento Overview at 1100×720 **and** 800×600 |
| `02-timeline.html` | Timeline with improved date-range selector + secondary controls; AudioDrawer open state |
| `03-quick-access-search.html` | Quick Access in Search mode (results grid) + empty/no-match |
| `04-quick-access-ask.html` | Quick Access in Ask mode **+ the current-frame feature** (collapsed state, frame-attached state, answering state) |
| `05-settings-general.html` | the settings shell (new navigation + always-visible autosave) showing General + Capture content |
| `06-settings-intelligence.html` | Intelligence settings (AI providers, transcription, OCR, semantic search) — the custom-inputs showcase |
| `07-components.html` | component sheet + type/spacing specimen + the direction's UI/UX pattern self-audit table |

Real app content to mirror (use realistic data, not lorem): settings sections are General
(Appearance/Shortcuts/Startup), Capture (Video/Audio/Privacy/Capture), Intelligence
(Providers/Ask AI/Transcription/Speakers/OCR/Semantic Search/User Context/MCP), Data
(Storage/Access), About (License/Developer). Reference renderings live in
`../mockups/design.html` (frame 01 timeline, 04 bento, 08/09 Quick Look, 12 settings).

## The five directions

| # | slug | seed |
|---|---|---|
| 1 | `01-bento-native` | The bento is the app's one organizing idiom. Overview as liked; settings groups, quick-access results and even the ask-answer render as tiles on the same grid rhythm. Calm grouped fills, System-Settings bones. |
| 2 | `02-studio-shell` | Pro-app (Final Cut/Logic) bones: compact toolbar, optional right inspector for context/details. Settings = searchable single scroll with sticky section headers (no sidebar at all). Densest of the five. |
| 3 | `03-layered-glass` | Control-Center/Sonoma-widget materials: floating layered panels over content, material chrome. Quick Access as a true HUD; current-frame ask as a translucent overlay riding on the frame. |
| 4 | `04-command-deck` | Keyboard-first. Quick Access is the app's centerpiece; Search vs Ask get strong distinct mode identities (shape, accent, layout). Main window minimal around the content; settings rows with inline kbd-driven controls. |
| 5 | `05-tactile-instruments` | The custom-input showcase direction: settings controls as small instruments (retention dial, disk-budget gauge, duty-cycle meter) with the most restraint elsewhere — instruments only where a value has a physical meaning. |

All five stay native-macOS in feel; the seed is where each spends its novelty budget.
