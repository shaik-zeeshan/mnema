<script lang="ts">
  import type { Snippet } from "svelte";

  interface Props {
    label: string;
    description?: string;
    /** The right-side (or, with `full`, full-width) control slot. */
    control: Snippet;
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
    id,
    warn = false,
    disabled = false,
    full = false,
    divider = true,
  }: Props = $props();
</script>

<div
  class="setting-row"
  class:setting-row--full={full}
  class:setting-row--warn={warn}
  class:setting-row--disabled={disabled}
  class:setting-row--no-divider={!divider}
  {id}
>
  <div class="setting-row__text">
    <span class="setting-row__label">{label}</span>
    {#if description}
      <span class="setting-row__description">{description}</span>
    {/if}
  </div>
  <div class="setting-row__control">
    {@render control()}
  </div>
</div>

<style>
  /* Rows are direct children of the card and sit flush; the card's padding
     comes from these rows. */
  .setting-row {
    position: relative;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    padding: 14px 18px;
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
    left: 18px;
    right: 18px;
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

  .setting-row__text {
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
    flex: 1 1 auto;
  }

  .setting-row__label {
    font-size: 13.5px;
    font-weight: 550;
    letter-spacing: 0.01em;
    color: var(--app-text-strong);
    line-height: 1.3;
  }

  .setting-row--warn .setting-row__label {
    color: var(--app-warn);
  }

  .setting-row__description {
    font-size: 11px;
    color: var(--app-text-muted);
    letter-spacing: 0.01em;
    line-height: 1.45;
    max-width: 420px;
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
    gap: 10px;
  }

  .setting-row--full .setting-row__control {
    justify-content: stretch;
    flex-shrink: 1;
    width: 100%;
  }
</style>
