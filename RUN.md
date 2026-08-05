# Direction 02 · Studio Shell — how to run it

This worktree is one of five phase-2 redesign candidates. **It never merges to main and gets no
PR.** It exists so the direction can be judged by being used, not by being read.

Spec it was built to: `docs/redesign/round4/02-studio-shell/` (README + 6 numbered pages +
`07-components.html`), with `docs/redesign/round4/DECISIONS.md` (G1–G11) overriding the mockups
wherever the two disagree.

## Launch

```sh
# once per fresh checkout of this worktree
bun install
bash scripts/prepare-mnema-cli-sidecar.sh debug

# the app
bun run tauri -- dev
```

UI-only iteration (no Tauri backend, IPC calls fail — good enough for layout work):

```sh
bun --cwd=apps/desktop run dev     # vite; the port auto-bumps past 1420 if taken
```

Checks: `bun run check` (whole repo) and `bun --cwd=apps/desktop test`.

## The thesis, in one paragraph

Pro-app bones. **Four fixed pieces and one scrolling region**: a 38px title bar, a 30px contextual
tool strip, the content, and a 24px status strip welded to the bottom window edge — plus a **256px
right inspector on every surface**. Chrome is dense (22px controls) so content gets the room;
content rows never go below 28px. Hairlines replace card edges. Because the strips and the
inspector are *structural*, live capture state and save state cannot scroll or clip out of view —
which is the founder's autosave complaint answered by geometry rather than by copy.

## What to look at, per surface

### Everywhere — the shell
- The **status strip** at the bottom edge. It carries the capture state, the capture rate, the
  measured daily and projected monthly pace, the queue, and disk free. Resize the window as small
  as it goes: the strip does not clip. Every figure is a measured fact or it is **absent** — you
  will see fields simply missing on a machine that has not recorded a full day yet. That is G8
  working, not a bug.
- The **inspector** on the right of every surface, and how the same 256px panel changes subject:
  frame metadata on Timeline, the selected tile row on Overview, the result's record in Quick
  Access, the focused setting on Settings.
- The **tool strip** under the title bar changes contents per surface and never holds anything a
  keyboard user cannot reach.

### Timeline (`⌘1`)
- The **position pill anchored to the playhead** — it reads out `Mon, Aug 3 · 14:32 ▾` *and* opens
  the jump menu. Position only; it never answers "how wide".
- The **jump menu**: Now / This morning / Yesterday, then seven day rows with a coverage bar each,
  then the month grid. Days with no recording are disabled — you cannot land on an empty day.
- The **Hour · Day · Week** segmented control owns span, and nothing else. There is no Month level
  and no type-a-date field (G5, G6).
- Hover the rail before clicking: a ghost playhead and a time bubble preview where the click lands.
- Layout order is frozen: stage → rail → audio lane → readout → drawer.

### Overview (`⌘2`)
- The bento, re-skinned flat and dense: a hairline under each tile header instead of a card edge,
  12px inset equal to a 12px gutter.
- **Select a row inside a tile** — it fills the inspector. That is what lets every tile stay a
  headline and stops any tile from growing a second column.
- This Week and Ask history are real reads. **Open Threads is the daily digest's own prose**, not
  an extracted entity (G11).
- Shrink to 800×600 and check the degradation.

### Quick Access (`⌥Space`)
- **One field.** There is no Search/Ask toggle. "Ask AI about '…'" arrives as a **ranked row in the
  results** — and is promoted to the selection when search finds nothing. Choosing it transforms
  the surface (G4).
- Search and Ask differ in **shape**, not tint: Search is a results grid with a record in the
  inspector and a bottom bar that always states what ⏎ does; Ask is a single 70ch prose column with
  cited moments on a horizontal media rail and the composer at the bottom.
- **Ask about the current screen**: the same window collapses to a 560px bar pinned top-centre. The
  control pill is a *separate* object — dismiss the answer and Stop is still there. The frame rides
  as a **chip inside the sentence**, deletable like a word, and the display is outlined on screen
  where the thing being seen actually is (G3).

### Settings (`⌘,`)
- **The navigation rail is gone.** One scrolling page, a filter field in the tool strip, sticky
  section headers carrying the section name and its position in the total.
- Type in the filter (`⌘F`): the page cuts to the matching rows, each with its breadcrumb and its
  *live* control.
- Focus a row: the inspector shows what that setting costs and when it takes effect — the panel is
  detail, not navigation.
- **Autosave in three places, none of them clippable**: the row echoes "Saved ✓" (*what*), the
  status strip carries the save state at the window edge (*whether*), and the inspector shows what
  the change did. There is no save bar and no Undo (G7).
- Intelligence carries the six custom inputs — rate slider, retention ladder, duty-cycle bar, model
  rows with a fit verdict, shortcut recorder, consequence toggle. Note what they *don't* say: no
  temperature, no minute-precise ETA, and an external shortcut conflict never names the other app
  (G8, G9).
