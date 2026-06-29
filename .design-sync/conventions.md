# Mnema visual language

Mnema is a local-first activity-recall desktop app with a **terminal-console
aesthetic**: a monospace body, a compact type scale, near-black surfaces, and a
single bright neon-green accent. This sync ships only the **design tokens and
base CSS** — Mnema's product UI is built in Svelte 5 + Tauri, so the original
components are not available here. Build new screens with your own elements and
style them entirely against the tokens below; the result will read as Mnema.

## The styling idiom: CSS variables, not utility classes

There is **no utility-class framework and no component library** in this system.
Style every element with the `var(--token)` custom properties defined in
`styles.css` and `tokens/theme.css`. Do not invent class names or import a
Tailwind/Chakra-style vocabulary — it will not resolve. Apply tokens via inline
styles or your own scoped CSS, e.g.
`background: var(--app-surface); color: var(--app-text); border: 1px solid var(--app-border);`.

## Theming

Two themes share one token vocabulary. **Dark is the default** (`:root`). For
light, set `data-theme="light"` on a wrapping `<html>`/root element — every
token flips in one place. Never hard-code a hex; always go through a token so
both themes stay coherent.

## Token families (real names — read `tokens/theme.css` for the full set)

- **Page / text:** `--app-bg`, `--app-fg`, `--app-surface`,
  `--app-surface-raised`, `--app-surface-hover`, `--app-border`,
  `--app-border-strong`, `--app-text-strong`, `--app-text`, `--app-text-muted`,
  `--app-text-subtle`, `--app-text-faint` (decorative only — never body text).
- **Accent (brand neon green):** `--app-accent`, `--app-accent-strong`,
  `--app-accent-bg`, `--app-accent-border`, `--app-accent-glow`,
  `--app-accent-contrast` (dark ink for text on an accent fill).
- **Semantic status:** `--app-warn*`, `--app-danger*`, `--app-info*`,
  `--app-neutral*` (each with `-bg`/`-border`/`-strong` variants).
- **Capture sources:** `--app-source-screen*`, `--app-source-mic*`,
  `--app-source-sysaudio*`.
- **Charts/insights:** grayscale ramp `--chart-grey-1..5`, category palette
  `--cat-creating|communication|meetings|research|learning|organizing|personal|entertainment`,
  focus heat `--focus-deep|mid|distracted`.
- **Type scale:** `--text-xs` 10px · `--text-sm` 11px · `--text-base` 12px ·
  `--text-md` 13px (body default) · `--text-lg` 16px · `--text-xl` 20px.
- **Font:** `--app-font-mono` (Berkeley Mono → ui-monospace fallback; the named
  faces are licensed and not shipped, so previews render the monospace tail —
  that is expected and on-brand).
- **Effects:** `--app-ring` / `--app-ring-danger` (focus rings),
  `--app-shadow-popover`, `--app-disabled-opacity` (0.4), `--app-busy-opacity`
  (0.6).

## Idiomatic snippet

```html
<div style="
  background: var(--app-surface-raised);
  border: 1px solid var(--app-border);
  color: var(--app-text);
  font-family: var(--app-font-mono);
  font-size: var(--text-md);
  padding: 12px 16px;
  border-radius: 8px;
">
  <h2 style="color: var(--app-text-strong); font-size: var(--text-lg);">Session</h2>
  <p style="color: var(--app-text-muted); font-size: var(--text-sm);">2 sources active</p>
  <button style="
    background: var(--app-accent);
    color: var(--app-accent-contrast);
    border: 1px solid var(--app-accent-border);
    font-family: inherit;
    padding: 6px 12px;
    border-radius: 6px;
  ">Open</button>
</div>
```

**Where the truth lives:** `styles.css` (base layer + import) and
`tokens/theme.css` (every token, both themes). Read them before styling.
