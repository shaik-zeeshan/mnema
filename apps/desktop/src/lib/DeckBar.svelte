<script lang="ts">
  /**
   * The deck — direction 04 "Command Deck"'s one added piece of chrome.
   *
   * A 28px bar pinned to the bottom of the window FRAME (last child of
   * `.app-shell`, outside `<main>`), carrying the route's context on the left
   * and its live shortcut hints on the right. Purely presentational: no drag
   * region, no focusable child, nothing that can steal focus from the surface.
   *
   * Routes publish into it via `setDeck()` (see `$lib/deck.svelte.ts`). With
   * nothing published the bar doesn't render at all, so a route that opts out
   * costs zero pixels.
   *
   * Named `DeckBar` rather than `Deck`: macOS's case-insensitive filesystem
   * makes `Deck.svelte` collide with the `deck.svelte.ts` store it imports.
   */
  import { deck, type DeckHint } from "./deck.svelte";

  /**
   * True on every main-window route, so the deck is never an empty bar before
   * a route has published. A route that publishes its own hints replaces these
   * wholesale — this is a fallback, not a merge, and it is a `$derived` read
   * rather than a seeding `$effect` so it can never race a route's publish.
   */
  const GLOBAL_HINTS: DeckHint[] = [
    { keys: "⌘⌥Space", label: "Quick Access" },
    { keys: "⌘,", label: "Settings" },
  ];

  const hints = $derived(deck.hints.length > 0 ? deck.hints : GLOBAL_HINTS);
</script>

{#if hints.length > 0 || deck.context || deck.status}
  <div class="deck">
    {#if deck.context}
      <!-- Context + hints are aria-hidden: they restate the route heading and
           the ⌘/ shortcut sheet, so announcing them again is noise. The status
           slot is NOT hidden — it is the only thing here that is real state. -->
      <span class="deck__ctx" aria-hidden="true">
        <!-- lucide "layers" — one generic surface glyph; the context string
             does the identifying, the glyph just anchors the row. -->
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="m12.83 2.18a2 2 0 0 0-1.66 0L2.6 6.08a1 1 0 0 0 0 1.83l8.58 3.91a2 2 0 0 0 1.66 0l8.58-3.9a1 1 0 0 0 0-1.83Z" />
          <path d="m22 17.65-9.17 4.16a2 2 0 0 1-1.66 0L2 17.65" />
          <path d="m22 12.65-9.17 4.16a2 2 0 0 1-1.66 0L2 12.65" />
        </svg>
        <span class="deck__ctx-text">{deck.context}</span>
      </span>
    {/if}

    <!-- Always mounted (even empty) so its `margin-left: auto` keeps the status
         slot right-aligned when a route publishes no hints. Clips from the
         right under width pressure — the last hints drop, which is what the
         800×600 mockup shows. -->
    <span class="deck__hints" aria-hidden="true">
      {#each hints as hint, i (`${hint.keys}-${hint.label}-${i}`)}
        {#if hint.separator}<span class="deck__sep"></span>{/if}
        <span class="hint"><span class="kbd">{hint.keys}</span><span>{hint.label}</span></span>
      {/each}
    </span>

    {#if deck.status}
      <!-- G7: settings autosave lives here. Never shrinks — being inside the
           window frame and unclippable is the reason this slot exists. -->
      <span
        class="deck__status"
        class:deck__status--ok={deck.status.tone === "ok"}
        class:deck__status--danger={deck.status.tone === "danger"}
        role="status"
        aria-live="polite"
      >
        {#if deck.status.tone === "ok"}
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round">
            <path d="M20 6 9 17l-5-5" />
          </svg>
        {/if}
        {deck.status.text}
      </span>
    {/if}
  </div>
{/if}

<style>
  .deck {
    flex: 0 0 var(--deck-h);
    height: var(--deck-h);
    display: flex;
    align-items: center;
    gap: var(--s-12);
    padding: 0 var(--s-12);
    background: var(--app-surface);
    box-shadow: inset 0 1px 0 var(--app-border);
    user-select: none;
    -webkit-user-select: none;
    overflow: hidden;
  }

  .deck__ctx {
    display: inline-flex;
    align-items: center;
    gap: var(--gap-inline);
    min-width: 0;
    flex: 0 1 auto;
    font: var(--w-medium) var(--t-meta) / 1 var(--app-font-sans);
    color: var(--app-text-muted);
  }
  .deck__ctx svg {
    width: 12px;
    height: 12px;
    flex: 0 0 auto;
    color: var(--app-text-subtle);
  }
  .deck__ctx-text {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* Hints yield ~100× faster than the context line, so a narrow window sheds
     shortcut hints (recoverable — ⌘/ still lists them) before it truncates the
     one label that says where you are. */
  .deck__hints {
    margin-left: auto;
    flex: 0 100 auto;
    min-width: 0;
    overflow: hidden;
    display: inline-flex;
    align-items: center;
    flex-wrap: nowrap;
    gap: var(--s-12);
  }
  .deck__hints :global(.hint) {
    flex: 0 0 auto;
  }
  .deck__sep {
    flex: 0 0 auto;
    width: var(--hairline);
    height: 14px;
    background: var(--app-border-strong);
  }

  .deck__status {
    flex: 0 0 auto;
    display: inline-flex;
    align-items: center;
    gap: var(--s-4);
    font: var(--w-medium) var(--t-meta) / 1 var(--app-font-mono);
    font-variant-numeric: tabular-nums;
    color: var(--app-text-subtle);
  }
  .deck__status svg {
    width: 11px;
    height: 11px;
  }
  .deck__status--ok {
    color: var(--app-accent);
  }
  .deck__status--danger {
    color: var(--app-danger);
  }
</style>
