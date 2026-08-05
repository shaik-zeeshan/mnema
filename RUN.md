# Running direction 04 — Command Deck

One of five phase-2 whole-app redesign directions. This branch
(`shaik-zeeshan/04-command-deck`) **never merges to main and has no PR** — it
exists so you can use the direction as a real app before picking one.

## Launch

```sh
cd /Users/shaikzeeshan/orca/workspaces/mnema/04-command-deck
bun install                                     # once
bash scripts/prepare-mnema-cli-sidecar.sh debug # once
bun run tauri -- dev
```

Frontend only (no Tauri — chrome and layout only, no real data):

```sh
bun --cwd=apps/desktop run dev
```

## The idea in five lines

1. Mnema is driven from the keyboard and the interface says so out loud: a
   **keycap is a first-class visual element** on every actionable thing.
2. Every window carries a **deck** — a 28px bar fixed inside the window frame,
   context on the left, live shortcut hints on the right. It is the direction's
   one added piece of chrome, and it is why the settings save-state can never
   clip off-screen.
3. **Quick Access is the centrepiece.** One field, no mode toggle: Ask AI is the
   top-ranked row, and taking it transforms the surface.
4. The main window is minimal chrome around content: two bars and nothing else.
5. Settings has **no sidebar** — five horizontal tabs over one scroll, and ⌘F is
   the real navigation.

## What to look at, per surface

**Everywhere — the deck (bottom bar).** Context on the left changes with what
you are looking at; the hints on the right are live and every one of them is a
key that actually works. Nothing speculative was drawn.

**Timeline (⌘1).** The position pill is *anchored to the playhead* — it reads
out where you are and opens the jump menu, one control doing two jobs, never
sitting somewhere other than the thing it reads. Hover the rail to see the ghost
playhead + time bubble before you commit. Zoom (Hour/Day/Week) wears its ⌥1–⌥3
caps. Open the jump menu: real NSMenu anatomy, per-day captured hours in the key
column, and days with no recording are disabled so you can never land empty.
Open the audio drawer (⌥A or the headphones button) for the floating-panel
treatment.

**Overview (⌘2).** Hold **⌃** — every tile header's keycap raises contrast and
the grid becomes the app's shortcut map. Tap any of ⌃M ⌃D ⌃R ⌃S ⌃C ⌃J ⌃W ⌃K to
focus that tile, ⏎ to open it. Resize to 800×600: Storage, This Week and Ask
drop, the `6:42` hero stays.

**Quick Access (⌘⌥Space).** Type. The **Ask AI row is the top hit** — ⌘⏎ takes
it. Type something with no matches: the ask row is promoted to selection and its
key becomes plain ⏎. Take it and watch the whole surface change identity —
accent header band, an `ASK` token pinned in the field (⌫ deletes it and you're
back in search), one reading column, cited moments as a media rail.

**Ask about this screen (⌘⇧A).** The window collapses to a floating bar. Context
is a chip in a sentence — an excluded app is named, and a non-vision model says
so before you type. The answer arrives as a detached second panel; ⌘⌫ drops the
frame, ⌘O grows back.

**Settings (⌘,).** No rail. Five tabs, ⌃1–⌃5. Press **⌘F** and type — every
matching row in every section comes to you with its breadcrumb *and its live
control*, so you change the setting from the search result. Change anything and
watch both autosave signals: `Saved ✓` beside the control, and the timestamped
state in the deck. Go to Intelligence for the six instruments (cost slider,
retention ladder, OCR duty bar, model fit rows, shortcut recorder, consequence
toggles) — each names a real quantity on *this* Mac.

## Where the G-decisions overrode the mockup

`docs/redesign/round4/DECISIONS.md` (G1–G11) outranks this direction's mockups.
Six places where the mockup pixels were deliberately not copied:

| mockup draws | shipped instead | why |
|---|---|---|
| a type-a-date field in the jump menu | quick targets + 7 day rows + month grid | G6 dropped it from v1 |
| a settings Undo (deck hint + panel copy) | nothing | G7 — Undo is out for v1 |
| °C claims and "backlog clears in ≈22 min" | both duty halves, split in seconds, real backlog count | G8 — no thermals, no minute-precise ETAs |
| retention `7d·30d·90d·1y·∞` | the app's real 7/14/30/Forever | G8 — 90d and 1y do not exist in `RetentionPolicy` |
| "taken by CleanShot X" | "this shortcut is taken by another app — try a different combination" | G9 — macOS cannot enumerate other apps' hotkeys |
| a 42×26 thumbnail in the current-frame bar | a chip in a sentence | G3 — context is a chip, never a thumbnail |

Plus one substrate rule: the mockups still carry the pre-fix dark
`--app-text-faint: #33334a`; the app keeps phase 1's `#6c6c88`.

## Spec and verification

- Mockups: `docs/redesign/round4/04-command-deck/` (README + 7 pages).
- Render-verification screenshots (99 PNGs, both themes, 1100×720 + 800×600 +
  Quick Access at 1120×720): `docs/redesign/round4/04-command-deck/shots-app/`.
  **Deliberately not committed** — 16 MB of artifacts, not deliverables; they
  live in this worktree only.
- `bun --cwd=apps/desktop run check` → 0 errors. `bun --cwd=apps/desktop test` →
  1227 pass.
