# Debug page mockups (approved)

Static HTML mockups for the feature-organized `/debug` redesign. Open them in a
browser; they are self-contained (inline CSS, no assets, fake data).

These are **design intent, not code**. The real page is Svelte.

## Read this before "reconciling" a mockup with a component

**The mockup IS the app's design language — it is not in tension with it.** These
files are authored entirely in the app's own tokens (`--app-accent`,
`--app-surface-raised`, `--app-border-strong`, `--text-xs`…`--text-lg`, defined
in `routes/+layout.svelte`), and they were derived *from* the app's components:
the mockup's `.card` is byte-for-byte `SettingGroup`'s `.setting-group__card`,
and its `.group__title` is `SettingGroup`'s title rule.

So: **the mockup wins on visual vocabulary** — rows, bars, strips, badges,
chrome, spacing, type. Reuse a shared component only where it carries
*behaviour*: `Segmented` (tab semantics + keyboard nav), `tooltip.ts`'s `tip`,
`Switch`/`Select`/`Combobox` (form semantics), `SettingGroup` (card shell +
`onTitleClick` drill-in). Never substitute a component's *look* for a mockup
element that has no component equivalent.

> An earlier version of this README said "where a mockup class and a real
> component disagree, the real component wins." **That was wrong** and it caused
> a real fidelity regression: agents swapped the mockup's `.row`s for a terse
> `.kv-list`, dropped the centered `.panel` column entirely, and left the
> migrated sections looking like the pre-redesign page. It was resolving a
> conflict that does not exist. Do not reintroduce that rule.

Prefer `var(--text-*)` / `var(--app-*)` tokens over hardcoded px in anything you
touch. (Much of the migrated CSS still hardcodes px whose values happen to equal
the tokens — cosmetically identical, so not worth a mass sweep.)

| File | Level | Shows |
|---|---|---|
| `debug-mockup-a-feature-rail.html` | Summary (level 1) | Floating icon dock + per-feature summary cards |
| `debug-mockup-a-detail-transcription.html` | Detail (level 2) | Drill-down for one feature (Transcription) |

## Page layout (the thing that dominates the visual read)

`.scroll { padding: 16px 24px 48px 88px }` — the 88px left gutter clears the
fixed dock. Inside it, **one centered column**: `.panel { max-width: 680px;
margin: 0 auto; gap: 24px }`, holding the page head and every card. Dropping the
panel makes cards stretch edge-to-edge and the page stops reading as the mockup
no matter how right the cards are.

## What the mockups pin down

**Floating dock** — left, vertically centered, frosted pill, icon-only, one
health dot per icon (`ok` / `warn` / `err` / `idle`), hover tooltips, separators
between feature groups, live-poll pulse at the bottom. Clicking scrolls to the
matching summary section.

**Summary card** — status badge, provider/model rows, a 4-stat grid
(queued / running / failed / backlog), last error inline, action buttons.
`card--warn` / `card--danger` variants carry the severity.

**Detail view** — breadcrumb (`← Debug / Transcription`, Esc goes back), status
hero with a plain-language diagnosis and a 5-stat strip, `Segmented` sub-tabs
(Overview · Jobs · Config · Log tail), a filterable/paginated jobs table, and a
per-job inspector (subject, attempts vs counted failures, backoff, next attempt,
payload, actions).

## Known deliberate gaps

- The inspector shows `attempts / failures / last_error / next_attempt_at`, not a
  per-attempt timeline — retries update the job row in place, so only the latest
  error survives. A history table is out of scope (see `PLAN.md`).
- Embedding quarantine counts are in-memory; the UI labels them "since app start".
- Throughput numbers are derived frontend-side from successive polls.
