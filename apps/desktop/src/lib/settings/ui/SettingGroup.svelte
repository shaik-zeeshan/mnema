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

<!-- `id` is the deeplink + scroll-spy anchor — it MUST stay on this outer
     scrollable <section>, never on the inner card. -->
<section class="setting-group" {id}>
  <header class="setting-group__header" class:setting-group__header--inline={hintInline}>
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

  <div class="setting-group__card {cardClass ?? ''}" class:setting-group__card--bare={bare}>
    {@render children()}
  </div>
</section>

<style>
  .setting-group {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  /* ── Section head (above the card) ─────────────────────────── */
  .setting-group__header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 12px;
    padding: 0 4px;
  }

  .setting-group__heading {
    display: flex;
    flex-direction: column;
    gap: 4px;
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
    font-size: var(--text-xs);
    font-weight: 700;
    letter-spacing: 0.13em;
    text-transform: uppercase;
    color: var(--app-text-strong);
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
    /* --text-sm is 11px — same value the mockup's `.group__hint` names. */
    font-size: var(--text-sm);
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

  /* ── Card (wraps the rows) ─────────────────────────────────── */
  /* NB: no `overflow: hidden` here. Controls that open a dropdown/popover
     anchored inside a row (e.g. the app-exclusion combobox, which is
     positioned, not portaled) must be able to overflow the card; clipping
     them was cutting the menu at the card's bottom edge. The rows are
     transparent and the accent hairline below is inset within the card
     bounds, so nothing relies on clipping for the rounded-corner look. */
  .setting-group__card {
    position: relative;
    background: var(--app-surface-raised);
    border: 1px solid var(--app-border);
    border-radius: 12px;
  }

  /* Bare: no frame — children (which carry their own borders) sit flush. */
  .setting-group__card--bare {
    background: none;
    border: 0;
    border-radius: 0;
  }

  .setting-group__card--bare::before {
    display: none;
  }

  /* Faint top-edge accent hairline — Mnema signature, inset L/R. */
  .setting-group__card::before {
    content: "";
    position: absolute;
    top: 0;
    left: 14px;
    right: 14px;
    height: 1px;
    background: linear-gradient(
      90deg,
      transparent,
      color-mix(in srgb, var(--app-accent) 12%, transparent) 22%,
      color-mix(in srgb, var(--app-accent) 12%, transparent) 78%,
      transparent
    );
    pointer-events: none;
  }
</style>
