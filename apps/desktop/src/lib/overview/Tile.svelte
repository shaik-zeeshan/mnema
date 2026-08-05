<script lang="ts">
  // One bento tile. The header carries the key that focuses it — direction 04's
  // rule that the grid doubles as the app's shortcut map.
  //
  // The keycap is only ever rendered for a key the page actually binds: `kbd`
  // is passed by the page, which owns the handler. A keycap for a key nothing
  // listens to is a lie, so there is no default value here.
  import type { Snippet } from "svelte";

  interface Props {
    /** Matches `data-tile`; the page's key handler focuses by this id. */
    id: string;
    title: string;
    kbd?: string | null;
    /** Right-aligned count/timestamp in the header. */
    more?: string | null;
    span?: 1 | 2 | 4;
    media?: boolean;
    /** The tile's destination. `null` = nothing to open (no chevron, Enter is
     *  inert); the tile is still focusable so its key does something real. */
    open?: (() => void) | null;
    /** Announced destination, e.g. "Open Insights". */
    openLabel?: string | null;
    children: Snippet;
  }

  let {
    id,
    title,
    kbd = null,
    more = null,
    span = 1,
    media = false,
    open = null,
    openLabel = null,
    children,
  }: Props = $props();

  function onKeydown(event: KeyboardEvent): void {
    if (!open) return;
    if (event.target !== event.currentTarget) return;
    if (event.key !== "Enter" && event.key !== " ") return;
    event.preventDefault();
    open();
  }

  function onClick(event: MouseEvent): void {
    if (!open) return;
    // Inner rows own their own clicks; only bare tile chrome opens the tile.
    if ((event.target as HTMLElement | null)?.closest("button, a")) return;
    open();
  }
</script>

<!-- A tile is a focusable group, not a button: it holds buttons of its own, so
     it cannot BE one. It takes focus (⌃-key, Tab) and Enter opens it — the
     composite-widget pattern the two rules below don't model. -->
<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<section
  class="tile"
  class:tile--2={span === 2}
  class:tile--4={span === 4}
  class:tile--media={media}
  data-tile={id}
  role="group"
  tabindex="0"
  aria-label={open && openLabel ? `${title} — ${openLabel}` : title}
  onkeydown={onKeydown}
  onclick={onClick}
>
  {#if !media}
    <div class="tile-h">
      <span class="t-label">{title}</span>
      {#if more}<span class="more t-meta">{more}</span>{/if}
      {#if kbd}<span class="kbd tile-k" class:tile-k--far={!more}>{kbd}</span>{/if}
    </div>
  {/if}
  {@render children()}
</section>
