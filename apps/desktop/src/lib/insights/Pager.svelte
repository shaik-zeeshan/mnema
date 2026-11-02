<script lang="ts">
  // One numbered pager for any Insights list: ‹ 1 … 4 5 6 … 12 ›, "21–40 of 138",
  // and an optional rows-per-page choice. Pure view: the caller owns `page` and
  // `size` (and the ←/→ keys), so a list keeps its place when it re-renders.
  import type { Snippet } from "svelte";
  import { pageLabel, pageNumbers } from "$lib/insights/subjectsTiers";
  import IconPrev from "~icons/lucide/chevron-left";
  import IconNext from "~icons/lucide/chevron-right";

  interface Props {
    /** From `pageSlice`: 0-based page, page count, and the visible row range. */
    page: number;
    pages: number;
    from: number;
    to: number;
    total: number;
    onPage: (page: number) => void;
    /** Rows-per-page choices (e.g. 20/50/100); omit for a fixed size. */
    sizes?: readonly number[];
    size?: number;
    onSize?: (size: number) => void;
    /** Marks the pager whose list ←/→ currently turns (a green rule). */
    active?: boolean;
    /** What the list is, for screen readers ("Subjects", "Evidence"). */
    label: string;
    /** Extra inline content after the position (e.g. "selected: page 3"). */
    extra?: Snippet;
  }
  let { page, pages, from, to, total, onPage, sizes, size, onSize, active = false, label, extra }: Props = $props();

  const nums = $derived(pageNumbers(page, pages));
</script>

<nav class="in-pg" class:in-pg--active={active} aria-label="{label} pages">
  {#if pages > 1}
    <button type="button" class="in-pg__b" disabled={page === 0} aria-label="Previous page" onclick={() => onPage(page - 1)}>
      <IconPrev width="14" height="14" aria-hidden="true" />
    </button>
    {#each nums as n, i (n ?? `gap-${i}`)}
      {#if n === null}
        <span class="in-pg__gap" aria-hidden="true">…</span>
      {:else}
        <button
          type="button"
          class="in-pg__b"
          aria-current={n === page ? "page" : undefined}
          aria-label="Page {n + 1}"
          onclick={() => onPage(n)}>{n + 1}</button>
      {/if}
    {/each}
    <button type="button" class="in-pg__b" disabled={page >= pages - 1} aria-label="Next page" onclick={() => onPage(page + 1)}>
      <IconNext width="14" height="14" aria-hidden="true" />
    </button>
  {/if}
  <span class="in-pg__pos">{pageLabel({ from, to, total })}</span>
  {#if extra}{@render extra()}{/if}
  {#if sizes && onSize && total > sizes[0]}
    <span class="in-pg__sizes" role="group" aria-label="Rows per page">
      {#each sizes as s (s)}
        <button type="button" class="in-pg__b" aria-pressed={s === size} onclick={() => onSize(s)}>{s}</button>
      {/each}
    </span>
  {/if}
</nav>

<style>
  .in-pg { display: flex; align-items: center; flex-wrap: wrap; gap: 2px var(--s-2); position: relative; }
  .in-pg--active::before { content: ""; position: absolute; left: 0; right: 0; top: -1px; height: 2px; border-radius: 1px; background: var(--app-accent); }
  .in-pg__b {
    min-width: 26px; height: 26px; padding: 0 6px; display: inline-flex; align-items: center; justify-content: center;
    border: 0; border-radius: var(--r-sm); background: none; cursor: pointer;
    color: var(--app-text-muted); font: 500 var(--text-sm) / 1 var(--font-mono); font-variant-numeric: tabular-nums;
    transition: background-color var(--t-fast) var(--ease-quart), color var(--t-fast) var(--ease-quart);
  }
  .in-pg__b:hover:not(:disabled) { background: var(--mx-wash-strong); color: var(--app-text-strong); }
  .in-pg__b[aria-current="page"], .in-pg__b[aria-pressed="true"] {
    background: var(--app-surface); color: var(--app-text-strong); box-shadow: inset 0 0 0 1px var(--app-border-strong);
  }
  .in-pg__b:disabled { opacity: 0.3; cursor: default; }
  .in-pg__b:focus-visible { outline: 2px solid var(--app-accent); outline-offset: 1px; }
  .in-pg__gap { width: 18px; text-align: center; color: var(--app-text-faint); font: var(--text-sm) / 1 var(--font-mono); }
  .in-pg__pos { margin-left: var(--s-1); font: 500 var(--text-xs) / 1 var(--font-mono); color: var(--app-text-subtle); font-variant-numeric: tabular-nums; white-space: nowrap; }
  .in-pg__sizes { display: inline-flex; gap: 2px; margin-left: auto; }
  @media (prefers-reduced-motion: reduce) { .in-pg__b { transition: none; } }
</style>
