<script lang="ts">
  import type { Snippet } from "svelte";
  import IconCheck from "~icons/lucide/check";
  import { claimRowId, isRowEchoing, noteRowEdit } from "../state/row-echo.svelte";
  import { getSettingsSection, settingsFind } from "$lib/settings/state/settings-find.svelte";
  import { rowMatchesQuery, sectionBreadcrumb } from "../settings-index";

  interface Props {
    label: string;
    description?: string;
    /** The right-side (or, with `full`, full-width) control slot. */
    control: Snippet;
    /**
     * Optional compact control (e.g. a Switch) pinned beside the label/
     * description in the header. Use this — not `control` — for the primary
     * toggle when `control` carries wide, full-width content below (a
     * disclosure callout, status card, conditional fields). The toggle then
     * bounds the description's measure instead of stranding on its own line.
     */
    aside?: Snippet;
    /** Optional anchor id for deeplink scroll-to. */
    id?: string;
    /** Tint the row for an attention/warning state. */
    warn?: boolean;
    /** Dim + block interaction without removing the row. */
    disabled?: boolean;
    /** Render the control full-width beneath the label (for wide controls). */
    full?: boolean;
    /** Show the inset divider above this row. Defaults true; pass false to suppress. */
    divider?: boolean;
  }

  let {
    label,
    description,
    control,
    aside,
    id,
    warn = false,
    disabled = false,
    full = false,
    divider = true,
  }: Props = $props();

  // Row-level "Saved ✓" echo (G7). Every row gets it for free: touching any
  // control inside the row claims the echo, and the shared save-state chip
  // hands it out when the next save lands. Capture-phase so a control that
  // stops propagation still registers.
  const rowId = claimRowId();
  const echoing = $derived(isRowEchoing(rowId));

  // ⌘F row filtering (G7). The row knows its own label; its section comes from
  // the panel section's context, and its synonyms from the index keyed on that
  // pair. A miss is hidden by CSS (`display: none`) rather than unmounted, so
  // the control keeps its state and stays the same live control when it returns.
  const section = getSettingsSection();
  const miss = $derived(
    settingsFind.active && !rowMatchesQuery(section, label, settingsFind.query),
  );
  // A hit renders its breadcrumb, so the row still says where it lives when the
  // surrounding panels are filtered down to scattered rows.
  const crumb = $derived(
    settingsFind.active && !miss && section ? sectionBreadcrumb(section) : null,
  );
</script>

<!-- A settings row IS a tile row (`.row`): the same object the digest and search
     tiles list their contents with, so its separator is inset by the tile pad
     and it shares the app's 40px row metric. -->
<div
  class="setting-row row row--static"
  class:setting-row--full={full}
  class:setting-row--warn={warn}
  class:setting-row--disabled={disabled}
  class:setting-row--no-divider={!divider}
  class:setting-row--miss={miss}
  {id}
  oninputcapture={() => noteRowEdit(rowId)}
  onchangecapture={() => noteRowEdit(rowId)}
  onclickcapture={() => noteRowEdit(rowId)}
>
  <div class="setting-row__main">
    <div class="setting-row__text row__txt">
      {#if crumb}
        <span class="setting-row__crumb">{crumb.group} › {crumb.section}</span>
      {/if}
      <span class="setting-row__label row__lbl">{label}</span>
      {#if description}
        <span class="setting-row__description row__sub">{description}</span>
      {/if}
    </div>
    {#if echoing}
      <span class="setting-row__echo" role="status"><IconCheck aria-hidden="true" />Saved</span>
    {/if}
    {#if aside}
      <div class="setting-row__aside">{@render aside()}</div>
    {/if}
  </div>
  <div class="setting-row__control row__val">
    {@render control()}
  </div>
</div>

<style>
  /* Geometry (the 40px row, the tile-pad inset, and the separator inset by
     that same pad) comes from `.row` in bento.css — the shared tile-row
     primitive. Only `justify-content` is this component's own: the text
     cluster and the control push apart.
     `divider={false}` suppresses the separator `.row + .row` would draw above
     this row. `:global` is required — each row is a separate <SettingRow>
     instance, so Svelte's scoper can't see them as adjacent and would prune a
     purely-scoped `+` selector. `.setting-row` is unique to this component,
     so the global match is safe. */
  .setting-row {
    justify-content: space-between;
    min-width: 0;
  }

  /* Three real classes so this reliably outweighs `.row + .row::before` (0,2,1)
     wherever the two stylesheets happen to land in the cascade. */
  :global(.setting-row.row.setting-row--no-divider)::before {
    display: none;
  }

  .setting-row--disabled {
    opacity: var(--opacity-disabled);
    pointer-events: none;
  }

  /* Header line: label/description column on the left, optional compact
     `aside` control (a Switch) on the right. In a `full` row this is the row's
     top line and the wide `control` content drops below it; in a normal row it
     sits opposite the `control` and is the only thing left of it. Either way
     the `flex: 1 1 auto` text + `flex-shrink: 0` aside split bounds the
     description against the control beside it. */
  .setting-row__main {
    display: flex;
    align-items: center;
    gap: var(--s-12);
    min-width: 0;
    flex: 1 1 auto;
  }

  .setting-row__text {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    flex: 1 1 auto;
  }

  /* Transient "Saved ✓" echo — locality tells you WHICH row saved (the chip in
     the top strip tells you whether). Accent pill, matching the chip's weight. */
  .setting-row__echo {
    display: inline-flex;
    align-items: center;
    gap: var(--s-4);
    flex: 0 0 auto;
    height: 18px;
    padding: 0 var(--s-6);
    border-radius: var(--r-sm);
    background: var(--app-accent-bg);
    color: var(--app-accent);
    font-family: var(--app-font-mono, ui-monospace, monospace);
    font-size: var(--t-label);
    font-weight: var(--w-medium);
    letter-spacing: var(--ls-label);
    text-transform: uppercase;
    white-space: nowrap;
  }

  .setting-row__echo :global(svg) {
    width: 9px;
    height: 9px;
    fill: none;
    stroke: currentColor;
    stroke-width: 2.4;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  .setting-row__aside {
    display: flex;
    align-items: center;
    flex-shrink: 0;
  }

  /* ⌘F hit breadcrumb — where this row lives, since the filtered view has torn
     it out of its usual neighbourhood. */
  .setting-row__crumb {
    font-family: var(--app-font-mono, ui-monospace, monospace);
    font-size: var(--t-label);
    letter-spacing: var(--ls-label);
    text-transform: uppercase;
    color: var(--app-text-subtle);
  }

  /* Native density: 13px regular. The row label is content, so it carries the
     strong tone — the mono eyebrow above it is the one that steps back. */
  .setting-row__label {
    /* `.row__lbl` truncates to one line — right for a digest tile, wrong for a
       settings label in a half-width column. */
    white-space: normal;
    overflow: visible;
    text-overflow: clip;
    font-size: var(--t-ui);
    font-weight: var(--w-regular);
    letter-spacing: var(--ls-ui);
    color: var(--app-text-strong);
    line-height: var(--lh-ui);
  }

  .setting-row--warn .setting-row__label {
    color: var(--app-warn);
  }

  /* Present-tense consequence line: what is happening now, not what the option
     means in the abstract. Wraps (unlike the one-line `.row__sub` a digest tile
     uses), so the flex split below bounds it against the control. */
  .setting-row__description {
    font-size: var(--t-meta);
    color: var(--app-text-muted);
    letter-spacing: var(--ls-meta);
    line-height: var(--lh-meta);
    white-space: normal;
    overflow: visible;
    text-overflow: clip;
    /* Fill the text column. The flex split (`.setting-row__text` is
       `flex: 1 1 auto`, `.setting-row__control` is `flex-shrink: 0`) already
       reserves room for a beside control, so 100% wraps against the toggle —
       not the card edge — when the toggle sits inline. */
    max-width: 100%;
  }

  .setting-row__control {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 8px;
    flex-shrink: 0;
    min-width: 0;
    max-width: 100%;
  }

  /* Wide controls (mockup `.row.stack`): drop the control onto its own
     full-width line below the label. */
  .setting-row--full {
    flex-direction: column;
    align-items: stretch;
    gap: 12px;
  }

  /* In a `full` row the header can be tall (multi-line description), so pin the
     aside control to the top — aligned with the label, not floating against the
     middle of the paragraph. */
  .setting-row--full .setting-row__main {
    align-items: flex-start;
  }

  .setting-row--full .setting-row__control {
    justify-content: stretch;
    flex-shrink: 1;
    width: 100%;
  }
</style>
