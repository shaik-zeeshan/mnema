# Onboarding rework #195 — shared implementation brief

Read alongside repo-root `PLAN.md` (plan of record) and `docs/onboarding/mockups/README.md`.
This file is the shared contract between the slices so screens do not drift from each other.

## Design source of truth

`docs/onboarding/mockups/chosen-cinematic-rewind.html` — the chosen "Cinematic / Rewind"
direction, rendered as a design-brief document. Each screen is a `.win` element inside a
`.frame` section; port the `.win` content, **not** the surrounding annotation chrome (`.cap`,
`.note`, the state cards below each frame — those cards document *other states of the same
screen* and must all be implemented, but as states, not as extra cards on the page).

**Superseded for six screens by `docs/onboarding/mockups/revision-2.html`** — Capture &
Storage (03), the four *Change settings* sections (05), Setup (06), Voice (07) and the
Finale (08). Where the two disagree, revision 2 wins: its controls are the real input
components, inlined live from `input-components/parts/`, and it draws the real window
frame. Welcome, Permissions and Your settings still come from the file above.

Line ranges in that file:

| Screen | Frame lines |
|---|---|
| Global `<style>` (tokens, `.win`, shared primitives) | 3–563 |
| Flow map (reference only, not a screen) | 588–753 |
| 01 Welcome | 754–805 |
| 02a Permissions — Microphone denied | 806–883 |
| 02b Permissions — System audio | 884–965 |
| 03 Capture & Storage (+ scoped style 1084–1137) | 966–1083 |
| 04 Your settings | 1140–1204 |
| 05 Change settings | 1205–1348 |
| 06 Setup (+ scoped style 1442–1507) | 1349–1441 |
| 07a Voice — ready, take rejected | 1510–1597 |
| 07b Voice — model still downloading | 1598–1643 |
| 08a Finale — running | 1644–1721 |
| 08b Finale — nothing granted | 1722–1792 |
| Bookend motion notes | 1793+ |

## Design tokens — use the app's, do not hardcode

The mockup's `.app { --app-* }` block **is** the app's real token set. Those variables are
defined for real in `apps/desktop/src/routes/+layout.svelte` (dark ~line 1662, light ~1860).

So: copy the mockup's *layout, spacing, type scale, copy and structure*, but reference colours
as `var(--app-accent)`, `var(--app-text-muted)`, `var(--app-border)`, `var(--app-warn)`,
`var(--app-danger)` and so on. Never paste a hex from the mockup into a component. The app is
theme-aware; a hardcoded `#3dffa0` breaks the light theme.

Type scale tokens already exist: `--text-xs: 10px` … `--text-xl: 20px`.

## Density rule — enforced, not advisory

The founder's note was a **visual density** complaint, not a request to cut features.

1. **One line per row.** No row wraps at the frame's width. If it wraps, cut words — not font size.
2. **No screen carries more than ~7 lines of content**, excluding the heading and the action row.
3. **One explanatory sentence per screen, maximum.** Not one per option, not one per row. Prose
   attached to individual options is the main source of the density — it does not belong.
4. **~32px between groups.** Let the screens breathe.
5. Provider names, model identifiers, per-row byte sizes, caveats and parentheticals live
   **only on Change settings**, which is deliberately dense and is the reason the rest can be light.

Motion is scoped to the bookends — **Welcome and Finale only**. The six working screens carry no
ambient motion, only functional feedback (progress bar, level meter, focus states).

## What must not be lost

- Both hard gates and their real error copy.
- The AI features row present and **visibly unticked** — consent is never pre-ticked.
- System audio never shown as confirmed — macOS gives no API to read that grant.
- Continue on Setup live on arrival and never disabled.
- All four Finale states.
- Real numbers, **computed, never pasted**: ~400 MB/day at the 2 s default, and a download total
  rendered from `workListBytes()`. The mockup's "729 MB" and "Who's speaking · 31 MB" are stale
  placeholders — speakrs is **419 MB**, so the real default set is **~1.1 GB**. The nomic figure
  is `approx_download_bytes` by design, so any total containing it is approximate ("about 1.1 GB")
  and must never be presented as exact. The measured basis (270 MB/day at one snapshot every 3 s,
  pause-on-inactivity on) survives as a single footnote on Capture & Storage.

## Component conventions

- Confirmations, alerts and file dialogs use `@tauri-apps/plugin-dialog` — never
  `window.confirm` / `alert`.
- Settings control convention (also applies here): Segmented for small static enums and tiers;
  Select/Combobox for long or searchable lists; RadioGroup only when options need per-option
  descriptions. Onboarding deliberately uses Segmented where Settings uses RadioGroup — that is
  an accepted exception, not a bug.
- Reusable controls already exist in `apps/desktop/src/lib/components/` and
  `apps/desktop/src/lib/settings/` — reuse them before writing new ones.
- Keep every source file under 800 lines; split by responsibility if a screen grows past it.
- Prefer Svelte component-scoped `<style>` over appending to a shared CSS file, so parallel
  screen work does not collide.

## Verification — mandatory

UI work is verified by **rendering and looking at a screenshot**, never by grepping CSS or class
names. Grep-based verification has produced false "matches the mockup" verdicts on this repo
before. Run the dev server, capture the screen you built, and compare it against the same frame
rendered from the mockup HTML. Check for overflow too — several screens were built here that
type-checked cleanly and still ran past the frame. The onboarding window is **1120×800**,
minimum **920×620** (`apps/desktop/src-tauri/src/windows.rs:200`); screenshot both sizes, in
both themes.

Typecheck: `bun --cwd=apps/desktop run check`.
