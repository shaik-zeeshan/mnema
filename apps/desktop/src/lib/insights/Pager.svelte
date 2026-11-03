<script lang="ts">
  // A section's pager, drawn INSIDE its kicker heading (mockup r2-A):
  //   // FORMING · 69  ───────────  11–20 OF 69  ‹ ›
  // The range text opens a tiny rows-per-page menu (10 / 25 / 50 / Show all).
  // Pure view: the caller owns `page`/`size` and any ←/→ keys. Renders nothing
  // when the section fits on its smallest page.
  import { tick } from "svelte";
  import { pageLabel, SHOW_ALL, TIER_PAGE_SIZES } from "$lib/insights/subjectsTiers";
  import IconPrev from "~icons/lucide/chevron-left";
  import IconNext from "~icons/lucide/chevron-right";
  import IconCheck from "~icons/lucide/check";

  interface Props {
    /** From `pageSlice`: 0-based page, page count, and the visible row range. */
    page: number;
    pages: number;
    from: number;
    to: number;
    total: number;
    onPage: (page: number) => void;
    /** Current rows per page (SHOW_ALL = everything). */
    size: number;
    /** Rows-per-page choices; `null` = a fixed size with no menu. */
    sizes?: readonly number[] | null;
    onSize?: (size: number) => void;
    /** What the section is, for screen readers ("Forming", "Evidence"). */
    label: string;
  }
  let { page, pages, from, to, total, onPage, size, sizes = TIER_PAGE_SIZES, onSize, label }: Props = $props();

  let open = $state(false);
  let rootEl = $state<HTMLElement | null>(null);
  let rangeEl = $state<HTMLButtonElement | null>(null);
  const shown = $derived(total > (sizes?.[0] ?? size));
  const choices = $derived(sizes ? [...sizes, SHOW_ALL] : []);

  function pick(s: number): void {
    open = false;
    onSize?.(s);
    void tick().then(() => rangeEl?.focus());
  }
  function onDown(e: PointerEvent): void {
    if (open && !rootEl?.contains(e.target as Node)) open = false;
  }
  function onKey(e: KeyboardEvent): void {
    if (open && e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation(); // Esc closes the menu, not the page under it
      open = false;
      rangeEl?.focus();
    }
  }
</script>

<svelte:window onpointerdown={onDown} onkeydowncapture={onKey} />

{#if shown}
  <!-- Tabbing out closes the menu, or it would stay open unseen and eat the next Esc.
       A null relatedTarget (a click; WebKit doesn't focus clicked buttons) is onDown's job. -->
  <span
    class="in-pg"
    bind:this={rootEl}
    onfocusout={(e) => {
      if (open && e.relatedTarget && !rootEl?.contains(e.relatedTarget as Node)) open = false;
    }}>
    {#if sizes && onSize}
      <span class="in-pg__rw">
        <button
          type="button"
          class="in-pg__range"
          bind:this={rangeEl}
          aria-haspopup="menu"
          aria-expanded={open}
          aria-label="{label}: showing {pageLabel({ from, to, total })}. Rows per page"
          onclick={() => (open = !open)}>{pageLabel({ from, to, total })}</button>
        {#if open}
          <span class="in-pg__menu" role="menu" aria-label="Rows per page">
            <span class="in-pg__menu-k" aria-hidden="true">Rows per page</span>
            {#each choices as s (s)}
              <button type="button" role="menuitemradio" aria-checked={s === size} onclick={() => pick(s)}>
                {s === SHOW_ALL ? `Show all ${total}` : s}
                <IconCheck width="13" height="13" aria-hidden="true" />
              </button>
            {/each}
          </span>
        {/if}
      </span>
    {:else}
      <span class="in-pg__range in-pg__range--static">{pageLabel({ from, to, total })}</span>
    {/if}
    {#if pages > 1}
      <span class="in-pg__steps">
        <button type="button" class="in-pg__step" disabled={page === 0} aria-label="{label}: previous page" onclick={() => onPage(page - 1)}>
          <IconPrev width="15" height="15" aria-hidden="true" />
        </button>
        <button type="button" class="in-pg__step" disabled={page >= pages - 1} aria-label="{label}: next page" onclick={() => onPage(page + 1)}>
          <IconNext width="15" height="15" aria-hidden="true" />
        </button>
      </span>
    {/if}
  </span>
{/if}

<style>
  /* Inherits the heading's kicker type: mono, xs, uppercase, subtle. */
  .in-pg { display: inline-flex; align-items: center; gap: 2px; flex: none; }
  .in-pg__rw { position: relative; display: inline-flex; }
  .in-pg__range {
    height: 22px; padding: 0 6px; border: 0; border-radius: var(--r-xs); background: none; cursor: pointer; white-space: nowrap;
    font: inherit; letter-spacing: inherit; text-transform: inherit; color: var(--app-text-subtle); font-variant-numeric: tabular-nums;
    transition: background-color var(--t-fast) var(--ease-quart), color var(--t-fast) var(--ease-quart);
  }
  .in-pg__range--static { display: inline-flex; align-items: center; cursor: default; }
  button.in-pg__range:hover, .in-pg__range[aria-expanded="true"] { background: var(--mx-wash-strong); color: var(--app-text-strong); }
  .in-pg__steps { display: inline-flex; gap: 2px; margin-right: -3px; }
  .in-pg__step {
    width: 22px; height: 22px; display: inline-flex; align-items: center; justify-content: center; padding: 0;
    border: 0; border-radius: var(--r-xs); background: none; color: var(--app-text-muted); cursor: pointer;
    transition: background-color var(--t-fast) var(--ease-quart), color var(--t-fast) var(--ease-quart);
  }
  .in-pg__step:hover:not(:disabled) { background: var(--mx-wash-strong); color: var(--app-text-strong); }
  .in-pg__step:disabled { opacity: 0.3; cursor: default; }
  .in-pg__range:focus-visible, .in-pg__step:focus-visible, .in-pg__menu button:focus-visible { outline: 2px solid var(--app-accent); outline-offset: 1px; }
  .in-pg__menu {
    position: absolute; z-index: 20; right: 0; top: calc(100% + 4px); min-width: 140px; padding: 4px; display: grid; gap: 1px;
    border: 1px solid var(--app-border-strong); border-radius: var(--r-md); background: var(--app-surface-raised);
    box-shadow: 0 14px 34px -14px rgba(0, 0, 0, 0.5); text-transform: none; letter-spacing: 0;
    animation: in-pg-pop var(--t-med) var(--ease-expo);
  }
  .in-pg__menu-k { margin: 5px 8px 6px; font: 500 10px / 1 var(--font-mono); letter-spacing: 0.08em; text-transform: uppercase; color: var(--app-text-faint); }
  .in-pg__menu button {
    display: flex; align-items: center; justify-content: space-between; gap: var(--s-3); height: 26px; padding: 0 8px;
    border: 0; border-radius: var(--r-sm); background: none; cursor: pointer; text-align: left;
    color: var(--app-text-muted); font: 500 var(--text-sm) / 1 var(--font-sans);
  }
  .in-pg__menu button:hover { background: var(--mx-wash-strong); color: var(--app-text-strong); }
  .in-pg__menu button[aria-checked="true"] { color: var(--app-text-strong); }
  .in-pg__menu button[aria-checked="false"] :global(svg) { visibility: hidden; }
  @keyframes in-pg-pop { from { opacity: 0; transform: translateY(-4px); } }
  @media (prefers-reduced-motion: reduce) {
    .in-pg__range, .in-pg__step { transition: none; }
    .in-pg__menu { animation: none; }
  }
</style>
