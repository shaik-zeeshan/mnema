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
    /** Take both bento cells instead of one — the 2×1 footprint. Opt-in, for
        groups whose rows genuinely need the width (shortcut editors, connector
        tables). Everything else is 1×1. */
    wide?: boolean;
    /** The stack of <SettingRow>s. */
    children: Snippet;
  }

  // `bare` and `nested` are gone: both existed only for the shortcut category
  // lists, whose rows framed themselves and nested a group inside a group.
  // Those are now plain tile rows in their own top-level tile, so neither the
  // frame-dropping escape hatch nor the child-heading weight has a caller.
  let { title, hint, id, actions, titleExtra, cardClass, hintInline = false, onTitleClick, wide = false, children }: Props = $props();
</script>

<!-- A settings group IS a bento tile: `.tile` chrome (the constant 18px
     `.tile__h` header row — mono eyebrow left, actions right) over a
     `.pay--rows` payload whose rows bleed to both tile edges with separators
     inset by the tile pad. Identical object to a digest tile or a search
     result; only the contents differ.
     `id` is the deeplink anchor — it MUST stay on this outer <section>. -->
<section
  class="setting-group tile tile--static"
  class:setting-group--wide={wide}
  {id}
>
  <header
    class="setting-group__header tile__h"
    class:setting-group__header--inline={hintInline}
  >
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
          <span class="setting-group__title">{title}</span>
        {/if}
        {#if titleExtra}
          {@render titleExtra()}
        {/if}
      </div>
    </div>
    {#if hint && hintInline}
      <span class="setting-group__hint">{hint}</span>
    {/if}
    {#if actions}
      <div class="setting-group__actions">
        {@render actions()}
      </div>
    {/if}
  </header>

  <!-- Long prose hints are payload, not chrome: keeping them out of the header
       is what holds every tile's header on one baseline across the grid. -->
  {#if hint && !hintInline}
    <p class="setting-group__hint">{hint}</p>
  {/if}

  <div class="setting-group__card pay pay--rows {cardClass ?? ''}">
    {@render children()}
  </div>
</section>

<style>
  /* Geometry (tile padding, radius, fill, the 18px header row) comes from
     `.tile`/`.tile__h` in bento.css — this component only styles what is
     genuinely its own, plus the two tile properties it must override. */

  /* `.tile` clips to its radius; a settings tile must NOT — the app-exclusion
     combobox and every Select popover are positioned, not portaled, and were
     being cut at the tile's bottom edge. Nothing here relies on clipping: the
     rows are `.row--static`, so no hover fill can escape a rounded corner. */
  .setting-group {
    overflow: visible;
  }

  /* ── Section head ──────────────────────────────────────────── */
  .setting-group__header {
    display: flex;
    min-width: 0;
  }

  .setting-group__heading {
    display: flex;
    align-items: center;
    min-width: 0;
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

  /* Eyebrow/overline — the mono tile label, at the same weight and tone every
     tile header in the app uses (`.tile__h .t-label`). It is chrome: the row
     labels below it are the content, so it sits a step back, not forward. */
  .setting-group__title {
    font-family: var(--app-font-mono, ui-monospace, monospace);
    font-size: var(--t-label);
    font-weight: var(--w-semi);
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: var(--app-text-subtle);
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

  /* The hint is payload under the constant header row, not part of it — that is
     what keeps every tile on the grid opening at the same baseline. */
  .setting-group__hint {
    display: block;
    margin: 0 0 8px;
    font-size: var(--t-meta);
    color: var(--app-text-muted);
    letter-spacing: var(--ls-meta);
    line-height: var(--lh-meta);
  }

  .setting-group__header--inline .setting-group__hint {
    margin: 0 0 0 auto;
    font-family: var(--app-font-mono, ui-monospace, monospace);
  }

  .setting-group__actions {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 8px;
    margin-left: auto;
    flex-shrink: 0;
  }

  /* ── Card (wraps the rows) ─────────────────────────────────────
     Direction 01: the card is no longer a bordered box inside a section — the
     TILE is the box, and this is just its payload zone (`.pay--rows`, applied
     in the markup, bleeds the rows out to both tile edges). No frame, no
     background, no accent hairline: one surface per group, not two.
     NB: still no `overflow: hidden` anywhere on the tile — controls that open
     a positioned, non-portaled popover (the app-exclusion combobox, every
     Select) must be able to overflow it. */
  .setting-group__card {
    position: relative;
    /* `.pay--rows` already bleeds the rows to both edges; this is the same
       negative inset written where the component can guarantee it. */
    margin: 0 calc(var(--tile-pad) * -1) calc(var(--tile-pad) * -1);
  }

</style>
