<script lang="ts">
  import type { Snippet } from "svelte";

  interface Props {
    title: string;
    /** One line of prose read inline after the title (mockup `.st-section__hint`). */
    hint?: string;
    /** Optional anchor id for deeplink scroll-to. */
    id?: string;
    /** Optional snippet pinned to the right of the head line (e.g. a reload button). */
    actions?: Snippet;
    /** Optional snippet rendered inline right after the title (e.g. a status
        badge). Used by the Debug page's feature cards. */
    titleExtra?: Snippet;
    /** Optional extra class on the body element. Used by the Debug page's
        severity-tinted card hairlines (`setting-group__card--warn`). */
    cardClass?: string;
    /** Push `hint` to the head line's right edge instead of reading it right
        after the title. Used by the Debug page, whose hints are short machine
        status (`processor: ocr`). */
    hintInline?: boolean;
    /** When set, the title becomes the group's drill-in affordance: a button
        with a trailing chevron. Used by the Debug page's feature cards. */
    onTitleClick?: () => void;
    /** Drop the hairline above the rows, for children that carry their own
        borders (the keybinding lists). */
    bare?: boolean;
    /** A child heading under a parent section (smaller, muted): the shortcut
        category lists under "Keyboard Shortcuts". */
    nested?: boolean;
    /** Show skeleton rows (mockup `.st-skel-row`) in place of the rows. */
    loading?: boolean;
    /** The stack of <SettingRow>s. */
    children: Snippet;
  }

  let { title, hint, id, actions, titleExtra, cardClass, hintInline = false, onTitleClick, bare = false, nested = false, loading = false, children }: Props = $props();
</script>

<!-- Settings section (mockup `.st-section`): a head line — title + hint — over
     flat rows divided by hairlines. No card: the Debug page re-dresses the body
     (`.setting-group__card`) as a card in its own debug-cards.css. `id` is the
     deeplink + scroll-spy anchor and MUST stay on this outer <section>. -->
<section class="setting-group" {id}>
  <header class="setting-group__header" class:setting-group__header--inline={hintInline}>
    {#if onTitleClick}
      <button type="button" class="mx-h3 setting-group__title setting-group__title--link" onclick={onTitleClick}>
        {title}<span class="setting-group__title-chevron" aria-hidden="true">›</span>
      </button>
    {:else}
      <h2 class="mx-h3 setting-group__title" class:setting-group__title--nested={nested}>{title}</h2>
    {/if}
    {#if titleExtra}
      {@render titleExtra()}
    {/if}
    {#if hint}
      <p class="setting-group__hint">{hint}</p>
    {/if}
    {#if actions}
      <div class="setting-group__actions">
        {@render actions()}
      </div>
    {/if}
  </header>

  {#if loading}
    <div aria-busy="true">
      <span class="mx-sr">Loading {title}…</span>
      {#each [0, 1, 2] as i (i)}
        <div class="setting-group__skel">
          <div class="mx-skel-lines"><div class="mx-skel"></div><div class="mx-skel"></div></div>
          <div class="mx-skel setting-group__skel-ctl"></div>
        </div>
      {/each}
    </div>
  {:else}
    <div class="setting-group__card {cardClass ?? ''}" class:setting-group__card--bare={bare}>
      {@render children()}
    </div>
  {/if}
</section>

<style>
  .setting-group {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }

  /* Head line (`.st-section__head`): title, then the hint on the same
     baseline; a long hint wraps under itself, not under the title. */
  .setting-group__header {
    display: flex;
    align-items: baseline;
    gap: var(--s-3);
    padding-bottom: var(--s-2);
    min-width: 0;
  }

  .setting-group__title {
    flex: none;
    font-size: var(--text-md);
  }

  .setting-group__hint {
    flex: 1 1 auto;
    min-width: 0;
    margin: 0;
    font-size: var(--text-base);
    line-height: 1.5;
    color: var(--app-text-subtle);
  }

  /* `hintInline`: short status pinned to the right edge, kept on one line. */
  .setting-group__header--inline .setting-group__hint {
    flex: none;
    margin-left: auto;
    white-space: nowrap;
  }

  .setting-group__actions {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    flex: none;
    margin-left: auto;
    align-self: center;
  }

  /* A right-pinned hint already took the slack; the actions follow it. */
  .setting-group__header--inline .setting-group__hint + .setting-group__actions {
    margin-left: 0;
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
    transition: color var(--t-fast) var(--ease-quart);
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

  /* Child heading: one level down from the section title above it. */
  .setting-group__title--nested {
    font-weight: 500;
    font-size: var(--text-base);
    color: var(--app-text-muted);
  }

  /* Rows (`.st-rows`): a border above, each row brings its own hairline
     below. NB: no `overflow: hidden` — positioned dropdowns anchored in a row
     (the app-exclusion combobox) must overflow it. */
  .setting-group__card {
    position: relative;
    border-top: 1px solid var(--app-border);
  }

  .setting-group__card--bare {
    border-top: 0;
  }

  /* `loading` (mockup `.st-skel-row`). */
  .setting-group__skel {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 160px;
    gap: var(--s-6);
    align-items: center;
    padding: 16px 0;
    border-bottom: 1px solid var(--mx-hairline);
  }

  .setting-group__skel .mx-skel-lines {
    max-width: 420px;
  }

  .setting-group__skel-ctl {
    justify-self: end;
    width: 30px;
    height: 18px;
  }
</style>
