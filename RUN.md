# Running direction 04 — Command Deck

One of five phase-2 whole-app redesign directions. This branch
(`shaik-zeeshan/04-command-deck`) **never merges to main** — it exists so you can
use the direction as a real app before picking one.

## Launch

```sh
cd /Users/shaikzeeshan/orca/workspaces/mnema/04-command-deck
bun install                                    # once
bash scripts/prepare-mnema-cli-sidecar.sh debug # once
bun run tauri -- dev
```

Frontend only (no Tauri, IPC unstubbed — good for looking at chrome, not data):

```sh
bun --cwd=apps/desktop run dev
```

## The idea in one line

Mnema is driven from the keyboard, and the interface says so out loud: **a keycap
is a first-class visual element on every actionable thing**, and every window
carries a **deck** — a 28px bar fixed inside the window frame, context on the
left, live shortcut hints on the right.

## What to look at, per surface

*(filled in below once each surface landed — see the section at the end for the
exact per-surface checklist)*

## Spec

`docs/redesign/round4/04-command-deck/` — README + 7 mockup pages. Rendered
screenshots of the real app are in
`docs/redesign/round4/04-command-deck/shots-app/`.

The binding cross-direction decisions are `docs/redesign/round4/DECISIONS.md`
(G1–G11); where they contradicted this direction's mockups, the G-decision won.
Those cases are listed at the end of this file.
