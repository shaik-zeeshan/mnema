# Onboarding input components

The chosen design for every input in onboarding. Open `index.html`.

Each input should feel like the Settings **capture rate** control
(`apps/desktop/src/lib/components/CaptureRateControl.svelte`) — you move it and
something moves back at you that tells you what you just chose. The bar it set:
a ladder of meaningful stops rather than a free value, a phrase readout rather
than a raw number, a live consequence visual, and a relative cost.

Two earlier rounds explored 55 variants across 8 input families; this directory
holds only what was picked.

| component | chosen design |
|---|---|
| the sentence (capture rate + retention + storage) | **ghost dial** — alternatives printed faintly around each word |
| excluded apps | **sentence-as-control** with inline app icons + `＋ Add` chip |
| feature switches | **the chain** + running-total strip + undo chip |
| providers | **recommended-with-a-reason**; OCR selector dropped from onboarding |
| model pickers | **family group** + variant sub-group + detail strip + budget bar |
| AI setup | **later-loudly** wrapping **connect-and-verify**, scan for local, rack from 2nd |

## Build

```sh
./build.sh          # cat parts/ into index.html, in onboarding flow order
```

No bundler. Each part is self-contained, so concatenation is the whole build.
`_shell-head.html` supplies the `--app-*` tokens, both themes, the `.ob-*` type
register and the `.cmp` frame; `_shell-foot.html` supplies the theme toggle and
builds the jump nav from whatever parts are present.

## Fragment contract

`parts/<slug>.part.html` is a fragment — no `<!doctype>`, `<html>`, `<head>`,
`<body>`. Exactly one `<section class="cmp" id="cmp-<slug>">` holding
`.cmp-head`, `.cmp-stage` (the live control), `.cmp-notes`, plus its own scoped
`<style>` and one IIFE `<script>`.

Isolation is mandatory — the parts share one page:
- Every CSS selector prefixed `#cmp-<slug>`. No bare element selectors, no `:root`.
- All JS in one IIFE. No `window.*` assignment. Query only within your section.
- Any `id` prefixed `<slug>-`. Don't define `--app-*`. Don't style the page.

The control renders in a 640px stage and must also survive ~560px — no
horizontal scroll, no clipped text.

`.cmp-notes` carries two things: **state drivers** (real buttons reaching every
state worth seeing — failure modes, locked, empty, over-disk; a reviewer must
reach them by clicking, not by editing code) and **implementation notes** (what
this replaces, file + line, what it deletes, what backend change it needs).

## Standing rules

- **Tokens only.** `var(--app-*)`, never a literal hex — a hardcoded `#3dffa0`
  breaks light theme. Both themes must work.
- **Motion is functional only.** Onboarding's six working screens carry no
  ambient motion (the bookends are the exception). Movement must be caused by
  the user's input or by real progress. Notably: do **not** copy the Settings
  capture-rate control's always-looping rAF sweep.
- **Density.** One line per row, one explanatory sentence per control. Provider
  names, model ids and byte sizes belong only on the Change-settings surface.
- **Numbers are computed, never pasted.** Leave a `console.assert` self-check
  that fails loudly if the arithmetic drifts.
- **Native elements first** — `<input type=range>`, `<details>`, `<popover>`,
  radio groups. Custom only where native genuinely can't. No new dependency.
- **Keyboard parity with mouse**, visible focus (`var(--app-ring)`),
  `prefers-reduced-motion` drops transitions but keeps state changes.
- No external requests of any kind — icons CSS-drawn or data-URI.

## Constants

Verified against the Rust manifests — do not re-type from docs, several older
figures in `docs/onboarding/` are stale.

| thing | value |
|---|---|
| capture disk anchor | 270 MB/day at one snapshot every 3 s (measured, n=1), linear in rate |
| capture rate ladder | `[0.1, 0.5, 1, 2, 3, 5, 10, 15, 30, 45, 60]` s, **default 2 s** → 405 MB/day |
| retention | `never` (default, first, never punished) / `days_30` / `days_14` / `days_7` |
| Whisper | tiny 77,691,713 · **base 147,951,465 (default)** · small 487,601,967 · medium 1,533,763,059 |
| Apple Speech | OS-managed, no download (`OS_MANAGED_OPTION_VALUE`) |
| Parakeet | **int8 670,619,803 (default)** · full 2,549,945,719 |
| speakrs speaker model | 419,482,724 — no picker anywhere, spent silently |
| nomic embed | 548,000,000 — approximate, so any total containing it reads "about" |
| default set | ≈ 1,115,434,189 B → "about 1.1 GB" |

Facts that must not be re-invented: Semantic Search has no cheap end (only *Off*
is a real saving); Parakeet int8 has no honest memory figure in any manifest;
"Apple Vision fast / Tesseract slower" has no basis in the repo — the defensible
difference is that this build ships Tesseract English-only (`eng` + `osd`).

## Verify

By rendering and looking at a screenshot, in both themes — never by grepping
class names. Grepping CSS/class names has produced false "matches the mockup"
verdicts here before.

```sh
SHELL_BIN=~/Library/Caches/ms-playwright/chromium_headless_shell-*/chrome-headless-shell-mac-arm64/chrome-headless-shell
"$SHELL_BIN" --headless --disable-gpu --no-sandbox --hide-scrollbars \
  --virtual-time-budget=6000 --window-size=1120,3000 \
  --screenshot=/tmp/out.png "file://$PWD/index.html"
```
