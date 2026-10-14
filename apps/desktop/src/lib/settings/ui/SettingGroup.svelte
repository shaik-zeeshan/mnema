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

  let { title, hint, id, actions, bare = false, nested = false, loading = false, children }: Props = $props();
</script>

<!-- Settings section (mockup `.st-section`): a head line — title + hint — over
     flat rows divided by hairlines. `id` is the deeplink + scroll-spy anchor
     and MUST stay on this outer <section>. -->
<section class="setting-group" {id}>
  <header class="setting-group__header">
    <h2 class="mx-h3 setting-group__title" class:setting-group__title--nested={nested}>{title}</h2>
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
    <div class="setting-group__card" class:setting-group__card--bare={bare}>
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

  .setting-group__actions {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    flex: none;
    margin-left: auto;
    align-self: center;
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
