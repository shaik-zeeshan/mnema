# Direction 01 — Bento Native · how to run it

This worktree is one of five whole-app redesign candidates. It **never merges to main**
and has no PR. It exists so you can use the direction as a real app before picking one.

Branch: `shaik-zeeshan/01-bento-native`.

## Launch

```bash
cd /Users/shaikzeeshan/orca/workspaces/mnema/01-bento-native
bun run tauri -- dev
```

The setup toll is already paid in this worktree (`bun install`, `THIRD_PARTY_LICENSES.md`,
`scripts/prepare-mnema-cli-sidecar.sh debug`). Nothing in this direction touches Rust, so a
cold `cargo` build is the only slow part and only on the first launch.

Browser-only (no Tauri, faster iteration, stubbed IPC):

```bash
bun --cwd=apps/desktop run dev      # then open the printed URL
```

## What the direction is

The tile grid stops being an Overview layout and becomes the whole app's organizing idiom:
**one cell unit, one 16px gutter, four legal footprints (1×1, 2×1, 2×2, 4×1), and tile chrome
that is identical everywhere** — an 18px header row, mono eyebrow left, meta right, on one
baseline — while the payload under it is completely free and may bleed past the inset to be
clipped by the tile radius. That one move is what stops a bento reading as a form: a digest, a
waveform, a search result, a settings group and an AI answer become the same object wearing
different contents.

Surfaces are grouped borderless **fills** (the System Settings idiom) on an opaque window.
Materials appear in chrome only — toolbar, menus, popovers, the screen HUD. One accent.
Native density. Depth is a surface step; a shadow means the thing genuinely floats.

The direction layer lives in one file: `apps/desktop/src/lib/bento/bento.css`, imported from
`routes/+layout.svelte`. It adds tokens and primitives *on top of* the shared phase-1 design
system — it does not fork the palette, the type ramp, or `.btn/.input/.kbd/.pill`.

## What to look at, surface by surface

Each surface has a mockup page to compare against, in
`docs/redesign/round4/01-bento-native/`. Open the HTML directly — the pages are
self-contained and carry a light/dark toggle in the top-right.

| Surface | Open with | Compare against |
|---|---|---|
| Overview | ⌘2 | `01-overview.html` |
| Timeline | ⌘1 | `02-timeline.html` |
| Quick Access — Search | ⌘⌥Space | `03-quick-access-search.html` |
| Quick Access — Ask | ⌘⏎ from Search, or the first result row | `04-quick-access-ask.html` |
| Ask about this screen | ⌘⇧O | `04-quick-access-ask.html` (states 1–3) |
| Settings — General/Capture | ⌘, | `05-settings-general.html` |
| Settings — Intelligence | ⌘, then the Intelligence tab | `06-settings-intelligence.html` |
| Every control and state | — | `07-components.html` |

Rendered screenshots of the app itself, both themes, at 1100×720 and 800×600, are in
`docs/redesign/round4/01-bento-native/shots-app/`.

### The five things worth judging

1. **The Overview bento** (⌘2) — the direction's thesis at full strength. Nine tiles, four
   footprints, one gutter, zero borders. Watch the moments strip bleed off the right edge and
   the conversation waveforms run into the tile's bottom radius.
2. **The readout as the jump control** (⌘1, click the position pill under the rail) — one pill
   doing two jobs: it reads out where you are, and it opens a bento jump panel. Days with no
   recording are disabled, so you can never land on an empty day. Coarse movement is the
   Hour/Day/Week zoom, never a second date field.
3. **Search and Ask as opposite postures in one window** (⌘⌥Space) — no segmented control.
   Search is quiet: neutral field, mono match count, one homogeneous grid. Ask is the only
   accent-filled field in the app, and its answer is a heterogeneous tile composition rather
   than a grid. Ask is reachable as the first result row.
4. **Ask about this screen** (⌘⇧O) — the window collapses to a control pill with the panel
   below it. What the model can see is outlined on the screen itself; the captured frame
   attaches as a chip inside the prompt sentence, deleted the way you delete a word.
5. **Settings without a rail** (⌘,) — five toolbar tabs, a sticky sub-bar carrying scoped ⌘F
   and the autosave chip at the *top* where a short window cannot clip it, and groups as bento
   tiles in two columns. Change something and watch for the row-level "Saved" echo.

## Deviations from the mockup, and why

Where the mockups and the round-4 decisions (`docs/redesign/round4/DECISIONS.md`, G1–G11)
disagreed, the decisions won.

**Forced by a G-decision:**

- **G8, honest numbers.** No temperature claim anywhere — the OCR duty-cycle bar shows both
  halves of the cycle and the real backlog and says nothing about °C. No minute-precise ETAs.
  Dropped because nothing measures them: the search field's "0.08 s", the Quick Access empty
  state's frame and hour counts, the ask row's "2,140 frames read", the Overview Storage tile's
  "270 MB today / 34.2 GB on disk / 90-day keep", the retention ladder's "34.2 GB kept now",
  and the model rows' "slow on battery". What replaced them is measured on your machine.
- **G6.** The jump menu has no type-a-date field.
- **G4.** No Search/Ask segmented control; Ask is the first result row and ⌘⏎.
- **G7.** No settings save bar and no settings Undo.
- **G9.** Shortcut-conflict copy never names an external app.
- **G10.** The semantic coverage meter renders only when semantic search is enabled.
- **G11.** Open Threads is the digest's own sentence, not a structured tile.

**Stated deviations the direction itself argues for** (see `07-components.html`): tile radius
12 and inset 14 rather than 10/16; the AudioDrawer floating above the rail-wrap instead of
covering it; retention living under Capture rather than Data.

**Judgement calls worth knowing about:**

- **The digest's inline frame citations are not rendered.** They are page 01's signature move,
  but the digest carries narrative prose and no frame references — picking which clause points
  at which frame would be a fabricated citation. Real citations need the digest to emit
  grounded frame refs, which is a backend change.
- **Danger styling was kept on Settings' destructive actions.** The mockup reserves danger for
  Delete Recent Capture, but that action does not live in Settings, and Settings has nine
  genuinely destructive confirm-gated actions. Demoting them removes a safety signifier rather
  than a decoration.
- **Select is a push-bezel NSPopUpButton while Combobox keeps a recessed well.** They look like
  mismatched siblings but this is native-correct and matches the mockup's own `.pop` vs
  `.input`: you choose from one and type into the other.
- **The collapsed Quick Access window is centred, not pinned top-centre.** The centring is a
  Rust constant, and this direction is a skin with no Rust changes.
- **The timeline has no separate always-red "now" tick.** The rail already paints the active
  tick record-red; a second permanent red mark would put two reds on one rail.
