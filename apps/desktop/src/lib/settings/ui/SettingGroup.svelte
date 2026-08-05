<script lang="ts">
  import type { Snippet } from "svelte";

  interface Props {
    title: string;
    hint?: string;
    /** Optional anchor id for deeplink scroll-to. */
    id?: string;
    /** Optional snippet rendered at the right of the header (e.g. a reset button). */
    actions?: Snippet;
    /** Optional snippet rendered INLINE after the title, on the same line (e.g.
        a status badge). Opt-in — omit it and the title line is unchanged, which
        is what Settings wants. Used by the Debug page's feature cards, whose
        mockup puts the severity badge in the group title rather than in
        `actions`' right-aligned rail. */
    titleExtra?: Snippet;
    /** Optional extra class on the card element. Opt-in; used by the Debug
        page's severity-tinted card hairlines (`setting-group__card--warn`). */
    cardClass?: string;
    /** Put `hint` on the RIGHT of the title, on one baseline-aligned row,
        instead of stacked beneath it. Opt-in — omit it and the header is
        unchanged, which is what Settings wants: its hints are long prose
        descriptions that only read as a block under the title. The Debug
        page's hints are short machine status (`processor: ocr`), which its
        mockup pins to the header's right edge. */
    hintInline?: boolean;
    /** When set, the title becomes the group's drill-in affordance: it renders
        as a button with a trailing chevron and calls this on click. Opt-in —
        omit it and the title stays inert text, which is what Settings wants.
        Used by the Debug page's feature cards to push their detail view. */
    onTitleClick?: () => void;
    /** Drop the card chrome (border, background, accent hairline) so children
        sit directly on the page. Used by the keybinding lists, whose rows
        already carry their own borders — the parent frame is redundant. */
    bare?: boolean;
    /** Render the section title as a nested/child heading (smaller, lighter,
        inset) so a group sitting under a parent section reads as its child,
        not a fifth equal-weight sibling. Used by the shortcut category lists. */
    nested?: boolean;
    /** The stack of <SettingRow>s. */
    children: Snippet;
  }

  let { title, hint, id, actions, titleExtra, cardClass, hintInline = false, onTitleClick, bare = false, nested = false, children }: Props = $props();
</script>

<!-- `id` is the deeplink anchor — it MUST stay on this outer scrollable
     <section>, never on the inner card.

     Direction 02: the section title is the skin's `.ss-subh` (10px mono
     uppercase) and the rows below it are one hairline-separated `.ss-setgrp`
     block, not a bordered card. The group's own structure is unchanged — only
     which classes it wears. -->
<section class="setting-group" {id}>
  <header class="setting-group__header ss-subh" class:setting-group__header--inline={hintInline}>
    <div class="setting-group__heading">
      <!-- The title and anything inline after it share one row so a trailing
           badge sits beside the title rather than under it. With no
           `titleExtra` this is a one-item flex row — identical to the bare
           title it replaced. -->
      <div class="setting-group__title-line">
        {#if onTitleClick}
          <button type="button" class="setting-group__title setting-group__title--link" onclick={onTitleClick}>
            {title}<span class="setting-group__title-chevron" aria-hidden="true">›</span>
          </button>
        {:else}
          <span class="setting-group__title" class:setting-group__title--nested={nested}>{title}</span>
        {/if}
        {#if titleExtra}
          {@render titleExtra()}
        {/if}
      </div>
      {#if hint}
        <span class="setting-group__hint">{hint}</span>
      {/if}
    </div>
    {#if actions}
      <div class="setting-group__actions">
        {@render actions()}
      </div>
    {/if}
  </header>

  <div class="setting-group__card ss-setgrp ss-grp {cardClass ?? ''}" class:setting-group__card--bare={bare}>
    {@render children()}
  </div>
</section>

<style>
  .setting-group {
    display: flex;
    flex-direction: column;
  }

  /* ── Section head (above the rows) ─────────────────────────────
     `.ss-subh` supplies the type + the 14px/16px/6px inset; this only splits
     the heading from the trailing actions. */
  .setting-group__header {
    justify-content: space-between;
    align-items: flex-start;
    gap: 12px;
  }

  .setting-group__heading {
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
  }

  /* `hintInline`: the whole header becomes one baseline-aligned row —
     title left, hint right (then `actions`, if any, further right). Reached
     only via the opt-in prop, so every other caller keeps the stacked
     heading above byte-for-byte. */
  .setting-group__header--inline {
    align-items: baseline;
  }

  .setting-group__header--inline .setting-group__heading {
    flex: 1;
    flex-direction: row;
    align-items: baseline;
    justify-content: space-between;
    gap: 12px;
  }

  /* Short status, not prose: keep it on its line and let the title's flex
     row give up the space instead. */
  .setting-group__header--inline .setting-group__hint {
    white-space: nowrap;
    flex-shrink: 0;
  }

  /* Title row: the title plus whatever `titleExtra` renders beside it. */
  .setting-group__title-line {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    min-width: 0;
  }

  /* Eyebrow/overline: kept smaller than the row labels by design, but pushed to
     the strong text tone so it registers on the squint test rather than reading
     as the faintest line on the page. */
  .setting-group__title {
    font-family: var(--app-font-mono, ui-monospace, monospace);
    font-size: var(--t-label);
    font-weight: 510;
    letter-spacing: var(--ls-label);
    text-transform: uppercase;
    color: var(--app-text-subtle);
    white-space: nowrap;
  }

  /* Drill-in title: same type as the inert one, plus a hit target and a
     chevron that says there is a level below. */
  .setting-group__title--link {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 0;
    border: 0;
    background: none;
    cursor: pointer;
    text-align: left;
    transition: color 0.12s;
  }

  .setting-group__title--link:hover {
    color: var(--app-accent);
  }

  .setting-group__title--link:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
    border-radius: 4px;
  }

  .setting-group__title-chevron {
    font-weight: 400;
    opacity: 0.7;
  }

  /* Nested/child section title: a parent section (e.g. "Keyboard Shortcuts")
     owns the strong eyebrow; the category groups beneath it are its children,
     so lighten + inset their titles to read one level down rather than as
     equal-weight siblings. */
  .setting-group__title--nested {
    font-weight: 600;
    letter-spacing: 0.1em;
    color: var(--app-text-muted);
    padding-left: 8px;
  }

  .setting-group__hint {
    /* --t-meta is 11px — same value the mockup's `.group__hint` names. */
    font-size: var(--t-meta);
    color: var(--app-text-muted);
    letter-spacing: 0.01em;
    line-height: 1.5;
  }

  .setting-group__actions {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 8px;
    flex-wrap: wrap;
    flex-shrink: 0;
  }

  /* ── The rows block ────────────────────────────────────────────
     Hairlines instead of card edges is the direction's de-boxing rule, so the
     card frame and its accent hairline are gone; `.ss-grp` supplies the flat
     surface + radius and `.ss-setgrp` the 16px page inset.

     NB: still no `overflow: hidden`. Controls that open a dropdown anchored
     inside a row (the app-exclusion combobox is positioned, not portaled) must
     be able to overflow this block. */
  .setting-group__card {
    position: relative;
  }

  /* Bare: no surface at all — children carry their own separations. */
  .setting-group__card--bare {
    background: none;
  }
</style>
