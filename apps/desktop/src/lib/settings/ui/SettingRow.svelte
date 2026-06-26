<script lang="ts">
  import type { Snippet } from "svelte";
  import ButtonSpinner from "$lib/settings/ui/ButtonSpinner.svelte";

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
    /**
     * Near-the-control autosave cue. Pass the owning domain's saving / just-saved
     * flags and the row renders a quiet, transient "Saving…/Saved" line beside (or,
     * in a `full` row, below) the control — so an edit is confirmed at the point of
     * interaction instead of only at the remote rail footer. The rail footer remains
     * the single announced status (`aria-live`); this cue is visual-only to avoid
     * spamming multiple live regions when several same-domain rows light at once.
     */
    saving?: boolean;
    saved?: boolean;
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
    saving = false,
    saved = false,
  }: Props = $props();

  const showAutosaveCue = $derived(saving || saved);
</script>

<div
  class="setting-row"
  class:setting-row--full={full}
  class:setting-row--warn={warn}
  class:setting-row--disabled={disabled}
  class:setting-row--no-divider={!divider}
  {id}
>
  <div class="setting-row__main">
    <div class="setting-row__text">
      <span class="setting-row__label">{label}</span>
      {#if description}
        <span class="setting-row__description">{description}</span>
      {/if}
    </div>
    {#if aside}
      <div class="setting-row__aside">{@render aside()}</div>
    {/if}
  </div>
  <div class="setting-row__control" class:setting-row__control--with-cue={showAutosaveCue}>
    {@render control()}
    {#if saving}
      <span class="setting-row__autosave-cue"><ButtonSpinner />Saving…</span>
    {:else if saved}
      <span class="setting-row__autosave-cue setting-row__autosave-cue--ok">Saved</span>
    {/if}
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
    padding: 16px 20px;
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
    left: 20px;
    right: 20px;
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
    gap: 16px;
    min-width: 0;
    flex: 1 1 auto;
  }

  .setting-row__text {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
    flex: 1 1 auto;
  }

  .setting-row__aside {
    display: flex;
    align-items: center;
    flex-shrink: 0;
  }

  .setting-row__label {
    font-size: var(--text-md);
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

  /* When the autosave cue is present, allow the control slot to wrap so the cue
     can sit beside a compact control or drop onto its own line under a wide one
     (see the `--full` rule below). Only applied with the cue so existing
     single-control rows keep their nowrap layout. */
  .setting-row__control--with-cue {
    flex-wrap: wrap;
  }

  /* Near-the-control autosave cue: a quiet, transient "Saving…/Saved" line so the
     user sees their edit persist without looking down to the remote rail footer. */
  .setting-row__autosave-cue {
    display: inline-flex;
    align-items: center;
    font-size: 12px;
    line-height: 1.4;
    color: var(--app-text-subtle);
  }

  .setting-row__autosave-cue--ok {
    color: var(--app-accent, var(--app-text-subtle));
  }

  /* In a `full` row the control fills the width, so push the cue onto its own
     line beneath it (left-aligned) instead of squeezing it against the control. */
  .setting-row--full .setting-row__autosave-cue {
    flex-basis: 100%;
  }
</style>
