# Direction 03 — Layered Glass · how to run it

A whole-app skin of Mnema on the phase-1 machinery. **This branch never merges to main** — it
exists so you can use the direction as a real app before picking one.

## Launch

```sh
cd /Users/shaikzeeshan/orca/workspaces/mnema/03-layered-glass
bun install                                   # once
bash scripts/prepare-mnema-cli-sidecar.sh debug   # once (already done in this worktree)
bun run tauri -- dev
```

Port 1420 must be free — if another direction's worktree is already running, quit it first.

Browser-only preview (no Tauri backend, most surfaces render with empty data):
`bun --cwd=apps/desktop run dev`

## The idea, in one line

Depth comes from **layering**, not edges. Material is allowed exactly where macOS uses it —
toolbars, rails, drawers, HUDs, popovers, menus, toasts — and **content always lands on an
opaque plate**, so no paragraph's contrast ever depends on what is behind the window. A border
is only ever the rim of a piece of glass.

The ladder, if you want to name what you are looking at:

| level | what wears it | token |
|---|---|---|
| 1 | the window floor | `--app-bg` (opaque) |
| 2 | every readable thing: tile, group, result card, prose | `.plate` (opaque + `--sh-tile`) |
| 3 | toolbars, the settings rail, the audio drawer | `.glass-chrome` |
| 4 | Quick Access, the ask bar | `.glass-hud` |
| 5 | menus, popovers, toasts (densest) | `.glass-pop` |

## What to look at, per surface

**Title bar (everywhere).** Translucent chrome with a rim and a top highlight; content scrolls
*under* it — most visible on Overview at a small window size. The Timeline/Overview/Insights
switcher is an AppKit segmented control (⌘1 / ⌘2).

**Timeline (⌘1).** The chrome is one continuous material block over an opaque stage. The
signature move: the position readout and the date picker are **one glass capsule anchored over
the playhead** — it tracks the playhead's x as you scrub, and its chevron opens the jump menu
(Now / This morning / Yesterday → seven day rows with coverage bars → a month grid; days with no
recording are disabled). Coarse movement is the Hour/Day/Week zoom, so the capsule only ever
handles "jump". Hover the rail before clicking: a ghost playhead plus a time bubble. Open the
AudioDrawer — material shell, transcript on an opaque plate, because prose never sits on glass.

**Overview (⌘2).** A Sonoma widget board: one 16px gutter, four legal footprints, every tile the
same opaque plate with the same radius and shadow, only the *tint* changing with the tile's
subject. The moments strip is a payload zone — the frames bleed off the tile edge and are clipped
by its radius. Shrink the window to 800×600 and watch the grid scroll under the toolbar.

**Quick Access (⌥Space).** Stops pretending to be a second app window: one HUD floating over
whatever is behind it, results on opaque plates, overlay scrollbars. One field — typing offers
"Ask Mnema about '…'" as the first result row, which is the whole Search↔Ask bridge. Ask mode has
its own accent chrome, one ≤70ch reading column and a bottom-anchored composer.

**Ask about this screen (⌘⇧↵ from Quick Access).** The same window collapses to a 720×48 glass
bar at the top of the display and the dim drops — the point is to see your screen. The frame is
captured implicitly; the indication is doubled (a 44×26 thumbnail plus `Screen · hh:mm:ss` in the
bar, and an outline drawn on the screen itself). The answer arrives as a second, detached glass
panel below the bar; dismiss it and the bar survives. ⌘O grows the window back.

**Settings (⌘,).** The sidebar becomes a **floating translucent rail** — inset, rounded,
shadowed, with ⌘F search at the top and the live cost of what you enabled pinned to its bottom.
Under it, one opaque scroll pane with sticky section headers. Nothing lives at the bottom of the
window: autosave is a chip in the title bar plus a "Saved" echo in the row you actually changed.

## Screenshots

Rendered captures of every surface, both themes, at 1100×720 and 800×600 are listed in the
orchestrator's report. The mockups they were checked against are
`docs/redesign/round4/03-layered-glass/*.html` — open any of them directly in a browser.
