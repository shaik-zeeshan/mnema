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

<div
  class="setting-row"
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
    <div class="setting-row__text">
      {#if crumb}
        <span class="setting-row__crumb">{crumb.group} › {crumb.section}</span>
      {/if}
      <span class="setting-row__label">{label}</span>
      {#if description}
        <span class="setting-row__description">{description}</span>
      {/if}
    </div>
    {#if echoing}
      <span class="setting-row__echo" role="status"><IconCheck aria-hidden="true" />Saved</span>
    {/if}
    {#if aside}
      <div class="setting-row__aside">{@render aside()}</div>
    {/if}
  </div>
  <div class="setting-row__control">
    {@render control()}
  </div>
</div>

<style>
  /* Rows are direct children of the card and sit flush; the card's padding
     comes from these rows. */
  /* Native System-Settings density: a 40px row with a 12px inset, not the
     16/20 airy card padding. The quiet majority of this direction is these
     rows, so they have to read as stock macOS rows — the instruments are the
     only things allowed to take up room. */
  .setting-row {
    position: relative;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--s-12);
    min-height: 40px;
    padding: var(--s-8) var(--s-12);
    min-width: 0;
  }

  /* Inset divider between consecutive rows — a 1px line at the top of each
     non-first row, inset L/R so it doesn't touch the card edges.
     `:global` on the sibling pair is required: each row is a separate
     <SettingRow> instance, so Svelte's scoper can't see them as adjacent and
     would prune (and strip) a purely-scoped `+` selector. The `.setting-row`
     class is unique to this component, so the global match is safe. */
  :global(.setting-row + .setting-row)::before {
    content: "";
    position: absolute;
    top: 0;
    left: var(--s-12);
    right: 0;
    height: 1px;
    background: var(--app-border);
    pointer-events: none;
  }

  /* `divider={false}` suppresses the divider that would otherwise sit above
     this row. */
  :global(.setting-row--no-divider)::before {
    display: none;
  }

  .setting-row--disabled {
    opacity: 0.38;
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
  /* The row-level half of G7's autosave story: a bare "Saved ✓" in the row
     that changed, plus a ~1.5s accent tint on the row itself. Bare, not a
     pill — the chip in the toolbar already carries the pill weight, and two
     pills saying the same thing is one too many. */
  .setting-row__echo {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    flex: 0 0 auto;
    color: var(--app-accent);
    font: var(--w-medium) var(--t-meta) / 1 var(--app-font-sans);
    white-space: nowrap;
  }

  .setting-row:has(.setting-row__echo) {
    background: color-mix(in srgb, var(--app-accent) 7%, transparent);
  }

  .setting-row__echo :global(svg) {
    width: 10px;
    height: 10px;
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
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--app-text-muted);
  }

  /* Regular weight, not semibold: a stock row's label is not a headline. */
  .setting-row__label {
    font: var(--w-regular) var(--t-ui) / 1.25 var(--app-font-sans);
    letter-spacing: var(--ls-ui);
    color: var(--app-text-strong);
  }

  .setting-row--warn .setting-row__label {
    color: var(--app-warn);
  }

  .setting-row__description {
    font: var(--w-regular) var(--t-meta) / 1.35 var(--app-font-sans);
    color: var(--app-text-muted);
    letter-spacing: var(--ls-meta);
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
