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
    /** Show the bottom divider. Defaults true; pass false on the last row. */
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
  .setting-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    padding: 12px 0;
    border-bottom: 1px solid var(--app-border);
    min-width: 0;
  }

  /* Last row in a group shouldn't trail a divider. */
  .setting-row:last-child,
  .setting-row--no-divider {
    border-bottom: none;
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
    font-size: 12px;
    font-weight: 600;
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

  /* Wide controls: drop the control onto its own full-width line below the
     label so a combobox / input group isn't crushed into the right gutter. */
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
