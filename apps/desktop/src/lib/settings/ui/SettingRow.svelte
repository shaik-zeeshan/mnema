<script lang="ts">
  import type { Snippet } from "svelte";
  import IconCheck from "~icons/lucide/check";
  import { claimRowId, isRowEchoing, noteRowEdit } from "../state/row-echo.svelte";
  import { getSettingsSection, settingsFind } from "$lib/settings/state/settings-find.svelte";
  import { settingsInspector } from "../state/inspector.svelte";
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

  // Direction 02's inspector shows the FOCUSED setting (the rail it replaced was
  // navigation; this is not). A row becomes the subject when the user reaches
  // it — pointer or keyboard — and the accent inset bar marks which one that is.
  const focused = $derived(
    settingsInspector.subject?.label === label &&
      settingsInspector.subject?.section === section,
  );

  function inspect() {
    settingsInspector.focus({ label, description: description ?? null, section });
  }

  // The echo firing is the one moment we know a save is attributable to THIS
  // row, so it is also where the inspector's session history is written.
  $effect(() => {
    if (echoing) settingsInspector.noteChange(label);
  });
</script>

<div
  class="setting-row ss-srow"
  class:setting-row--full={full}
  class:ss-srow--wide={full}
  class:setting-row--warn={warn}
  class:setting-row--disabled={disabled}
  class:setting-row--no-divider={!divider}
  class:setting-row--miss={miss}
  class:is-focus={focused}
  {id}
  oninputcapture={() => noteRowEdit(rowId)}
  onchangecapture={() => noteRowEdit(rowId)}
  onclickcapture={() => { noteRowEdit(rowId); inspect(); }}
  onfocusincapture={inspect}
>
  <div class="setting-row__main">
    <div class="setting-row__text ss-srow__t">
      {#if crumb}
        <span class="setting-row__crumb">{crumb.group} › {crumb.section}</span>
      {/if}
      <span class="setting-row__label ss-srow__l">{label}</span>
      {#if description}
        <span class="setting-row__description ss-srow__s">{description}</span>
      {/if}
    </div>
    {#if echoing}
      <span class="setting-row__echo ss-echo" role="status"><IconCheck aria-hidden="true" />Saved</span>
    {/if}
    {#if aside}
      <div class="setting-row__aside">{@render aside()}</div>
    {/if}
  </div>
  <div class="setting-row__control ss-srow__v">
    {@render control()}
  </div>
</div>

<style>
  /* Row geometry is the direction skin's (`.ss-srow`, 34px floor, 6px/10px
     inset, hairline separations). Only what the skin has no opinion about
     lives here: the miss/warn/disabled states and the ⌘F breadcrumb. */
  .setting-row {
    position: relative;
    min-width: 0;
  }

  .setting-row--disabled {
    opacity: 0.38;
    pointer-events: none;
  }

  /* Header line: label/description column on the left, optional compact
     `aside` control (a Switch) on the right. In a `full` row this is the row's
     top line and the wide `control` content drops below it; in a normal row it
     sits opposite the `control` and is the only thing left of it. */
  .setting-row__main {
    display: flex;
    align-items: center;
    gap: 12px;
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

  .setting-row__echo {
    flex: 0 0 auto;
    white-space: nowrap;
  }

  .setting-row__echo :global(svg) {
    width: 11px;
    height: 11px;
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
    color: var(--app-text-subtle);
  }

  .setting-row--warn .setting-row__label {
    color: var(--app-warn);
  }

  .setting-row__control {
    min-width: 0;
    max-width: 100%;
  }

  /* Wide controls: the skin's `--wide` stacks the row; the control slot then
     fills the line beneath the label. */
  .setting-row--full .setting-row__main {
    align-items: flex-start;
    width: 100%;
  }

  .setting-row--full .setting-row__control {
    justify-content: stretch;
    flex-shrink: 1;
    width: 100%;
    margin-left: 0;
  }
</style>
