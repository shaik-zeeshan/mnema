<script lang="ts">
  import type { Snippet } from "svelte";

  interface Props {
    label: string;
    description?: string;
    /** The right-side (or, with `full`, full-width) control slot. */
    control: Snippet;
    /**
     * Optional compact control (e.g. a Switch) pinned beside the label/
     * description. Use this — not `control` — for the primary toggle when
     * `control` carries wide, full-width content below (a disclosure callout,
     * status card, conditional fields).
     */
    aside?: Snippet;
    /** Inline status / error / result lines under the description, in the
     *  copy column (mockup `.st-row .mx-inline`), so the control stays right. */
    below?: Snippet;
    /** Leading capture-family dot before the label (mockup `.st-src`). */
    source?: "screen" | "mic" | "sysaudio";
    /** Inline content right after the label, on its line (e.g. an `mx-chip`). */
    labelExtra?: Snippet;
    /** Optional anchor id for deeplink scroll-to. */
    id?: string;
    /** Tint the label for an attention/warning state. */
    warn?: boolean;
    /** Dim + block interaction without removing the row. */
    disabled?: boolean;
    /** Render the control full-width beneath the label (for wide controls). */
    full?: boolean;
    /** Show the hairline above this row. Defaults true; pass false to suppress. */
    divider?: boolean;
  }

  let {
    label,
    description,
    control,
    aside,
    below,
    source,
    labelExtra,
    id,
    warn = false,
    disabled = false,
    full = false,
    divider = true,
  }: Props = $props();
</script>

<!-- Obsidian-style row (mockup `.st-row`): copy left, control right, a
     hairline below. `full` = `.st-row--stack`: the control drops to its own
     full-width line. -->
<div
  class="setting-row {source ? `setting-row--src-${source}` : ''}"
  class:setting-row--full={full}
  class:setting-row--warn={warn}
  class:setting-row--disabled={disabled}
  class:setting-row--no-divider={!divider}
  {id}
>
  <div class="setting-row__main">
    <div class="setting-row__text">
      <span class="setting-row__label">
        {#if source}<span class="setting-row__src" aria-hidden="true"></span>{/if}
        {label}
        {#if labelExtra}{@render labelExtra()}{/if}
      </span>
      {#if description}
        <span class="setting-row__description">{description}</span>
      {/if}
      {#if below}
        <div class="setting-row__below">{@render below()}</div>
      {/if}
    </div>
    {#if aside}
      <div class="setting-row__aside">{@render aside()}</div>
    {/if}
  </div>
  <div class="setting-row__control">
    {@render control()}
  </div>
</div>

<style>
  .setting-row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    align-items: center;
    gap: var(--s-6);
    padding: 14px 0;
    border-bottom: 1px solid var(--mx-hairline);
    min-width: 0;
  }

  /* `divider={false}` drops the hairline between this row and the row above
     it — which, rows owning the line BELOW them, lives on that row. */
  :global(.setting-row:has(+ .setting-row--no-divider)) {
    border-bottom-color: transparent;
  }

  .setting-row--disabled {
    opacity: var(--app-disabled-opacity);
    pointer-events: none;
  }

  /* Copy column + the optional compact `aside` control beside it. */
  .setting-row__main {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    align-items: center;
    gap: var(--s-6);
    min-width: 0;
  }

  .setting-row__text {
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
  }

  .setting-row__below {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 6px;
    margin-top: 3px;
  }

  /* A long line wraps its text, not the icon onto a line of its own. */
  .setting-row__below :global(.mx-inline) {
    flex-wrap: nowrap;
    align-items: flex-start;
  }

  .setting-row__below :global(.mx-inline > svg) {
    margin-top: 3px;
  }

  .setting-row__below:empty {
    display: none;
  }

  .setting-row__aside {
    display: flex;
    align-items: center;
    gap: var(--s-2);
  }

  .setting-row__label {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    font: 500 var(--text-md)/1.35 var(--font-sans);
    color: var(--app-text-strong);
  }

  .setting-row--warn .setting-row__label {
    color: var(--app-warn);
  }

  /* `source`: the family's colour on the leading dot AND on a checked switch
     in the row (mockup `.st-src` + `.st-switch--*`). */
  .setting-row--src-screen { --src: var(--app-source-screen); }
  .setting-row--src-mic { --src: var(--app-source-mic); }
  .setting-row--src-sysaudio { --src: var(--app-source-sysaudio); }

  .setting-row__src {
    flex: none;
    width: 8px;
    height: 8px;
    border-radius: 2px;
    background: var(--src);
  }

  .setting-row[class*="setting-row--src-"] :global(.mx-switch:checked) {
    background: var(--src);
  }

  .setting-row__description {
    font-size: var(--text-base);
    line-height: 1.5;
    color: var(--app-text-muted);
  }

  .setting-row__control {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: var(--s-2);
    min-width: 0;
  }

  /* `.st-row--stack`: one column, the control on its own full-width line. A
     tall copy block pins its aside control to the label line. */
  .setting-row--full {
    grid-template-columns: minmax(0, 1fr);
    gap: var(--s-3);
  }

  .setting-row--full .setting-row__main {
    align-items: start;
  }

  .setting-row--full .setting-row__control {
    justify-content: stretch;
  }
</style>
