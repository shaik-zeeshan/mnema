<!-- The Quick Access inspector — direction 02's one right-hand panel, here
     holding the SELECTED RESULT's record. It is what lets the result cells stay
     pictures: the snippet, the URL, the revisit span and the actions all live
     here instead of on the tile.

     It reads the search store directly (same singleton the list uses) and is
     always mounted, placeholder included — a panel that appears and disappears
     is a layout that jumps (page 03's own note). -->
<script lang="ts">
  import { quickRecallSearch as search } from "$lib/quick-recall/searchStore.svelte";
  import { parseSearchSnippet } from "$lib/search-snippet";
  import { formatTimestampCompact, parseCapturedAt } from "$lib/format-time";

  const selected = $derived(search.selectedResult);

  // "2 of 14" — the selection's rank in the flattened visible order.
  const position = $derived(
    search.selectedIndex >= 0 && search.resultCount > 0
      ? `${search.selectedIndex + 1} of ${search.resultCount}`
      : null,
  );

  const thumbnail = $derived(
    selected?.kind === "frame"
      ? (search.thumbnailCache.get(selected.frame.thumbnailFrameId) ?? null)
      : null,
  );

  // Elapsed wall-clock between two capture timestamps, coarse on purpose (G8:
  // a number ships only where the fact is real, and seconds here are noise).
  function span(startTs: string, endTs: string): string | null {
    const start = parseCapturedAt(startTs);
    const end = parseCapturedAt(endTs);
    if (isNaN(start.getTime()) || isNaN(end.getTime())) return null;
    const seconds = Math.max(0, Math.round((end.getTime() - start.getTime()) / 1000));
    if (seconds < 60) return `${seconds} s`;
    const minutes = Math.floor(seconds / 60);
    if (minutes < 60) return `${minutes} min`;
    return `${Math.floor(minutes / 60)} h ${minutes % 60} min`;
  }

  function durationMs(ms: number): string {
    const seconds = Math.max(0, Math.round(ms / 1000));
    const m = Math.floor(seconds / 60);
    const s = seconds % 60;
    return `${m} min ${s.toString().padStart(2, "0")} s`;
  }
</script>

<aside class="ss-insp ss-insp--wide result-inspector">
  <div class="ss-insp__h">
    <span class="result-inspector__title">Result</span>
    {#if position !== null}
      <span class="ss-tstrip__spacer"></span>
      <span class="t-meta is-mono is-num">{position}</span>
    {/if}
  </div>

  <div class="ss-insp__b">
    <div class="result-inspector__preview" class:is-empty={thumbnail === null}>
      {#if thumbnail !== null}
        <img src={thumbnail} alt="" />
      {:else}
        <svg
          width="18"
          height="18"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="1.6"
          stroke-linecap="round"
          stroke-linejoin="round"
          aria-hidden="true"
        >
          <rect x="4.5" y="4.5" width="15" height="15" rx="1.5" />
          <path d="M4.5 9.5h15M4.5 14.5h15M9.5 4.5v15M14.5 4.5v15" />
        </svg>
      {/if}
    </div>

    {#if selected === null}
      <div class="ss-kv"><span class="ss-kv__k">Title</span><span class="ss-kv__v">—</span></div>
      <div class="ss-kv"><span class="ss-kv__k">App</span><span class="ss-kv__v">—</span></div>
      <div class="ss-kv"><span class="ss-kv__k">Seen</span><span class="ss-kv__v">—</span></div>
      <p class="ss-insp__empty">
        Select a result to see where it came from, how long you looked at it, and
        why it matched.
      </p>
    {:else if selected.kind === "frame"}
      {@const frame = selected.frame}
      <div class="ss-kv ss-kv--stack">
        <span class="ss-kv__k">Title</span>
        <span class="ss-kv__v result-inspector__strong"
          >{frame.windowTitle ?? frame.appName ?? "Screen"}</span
        >
      </div>
      <div class="ss-insp__sec">Where it came from</div>
      <div class="ss-kv">
        <span class="ss-kv__k">App</span>
        <span class="ss-kv__v">{frame.appName ?? "Unknown app"}</span>
      </div>
      {#if frame.url}
        <div class="ss-kv">
          <span class="ss-kv__k">URL</span>
          <span class="ss-kv__v is-mono result-inspector__url">{frame.url}</span>
        </div>
      {/if}
      <div class="ss-kv">
        <span class="ss-kv__k">Seen</span>
        <span class="ss-kv__v is-mono">{formatTimestampCompact(frame.groupStartAt)}</span>
      </div>
      {#if span(frame.groupStartAt, frame.groupEndAt) !== null}
        <div class="ss-kv">
          <span class="ss-kv__k">On screen</span>
          <span class="ss-kv__v is-mono">{span(frame.groupStartAt, frame.groupEndAt)}</span>
        </div>
      {/if}
      <div class="ss-insp__sec">Why it matched</div>
      <div class="ss-kv">
        <span class="ss-kv__k">Source</span>
        <span class="ss-kv__v"
          >{frame.foundByMeaning ? "Meaning" : "Text on screen"} · {frame.matchCount}
          {frame.matchCount === 1 ? "hit" : "hits"}</span
        >
      </div>
      <div class="ss-kv ss-kv--stack">
        <span class="ss-kv__k">Snippet</span>
        <span class="ss-kv__v result-inspector__snippet"
          >{#each parseSearchSnippet(frame.snippet) as segment}{#if segment.marked}<mark
                >{segment.text}</mark
              >{:else}{segment.text}{/if}{/each}</span
        >
      </div>
      {#if frame.hasSecretRedactions}
        <div class="ss-kv">
          <span class="ss-kv__k">Redacted</span>
          <span class="ss-kv__v is-mono"
            >{frame.secretRedactionCount}
            {frame.secretRedactionCount === 1 ? "secret" : "secrets"}</span
          >
        </div>
      {/if}
      <div class="result-inspector__acts">
        <button
          type="button"
          class="btn btn--sm btn--primary result-inspector__act"
          onclick={() => search.openResultAt(search.selectedIndex)}
        >
          Show in Timeline<span class="kbd result-inspector__key">⏎</span>
        </button>
        {#if frame.url}
          <button
            type="button"
            class="btn btn--sm result-inspector__act"
            onclick={() => search.openSelectedResultUrl()}
          >
            Open page<span class="kbd result-inspector__key">⌘O</span>
          </button>
        {/if}
      </div>
    {:else}
      {@const audio = selected.audio}
      <div class="ss-kv ss-kv--stack">
        <span class="ss-kv__k">Said</span>
        <span class="ss-kv__v result-inspector__snippet"
          >“{#each parseSearchSnippet(audio.snippet) as segment}{#if segment.marked}<mark
                >{segment.text}</mark
              >{:else}{segment.text}{/if}{/each}”</span
        >
      </div>
      <div class="ss-insp__sec">Where it came from</div>
      <div class="ss-kv">
        <span class="ss-kv__k">Source</span>
        <span class="ss-kv__v"
          >{audio.sourceKind === "microphone" ? "Microphone" : "System audio"}</span
        >
      </div>
      <div class="ss-kv">
        <span class="ss-kv__k">Heard</span>
        <span class="ss-kv__v is-mono">{formatTimestampCompact(audio.absoluteStartAt)}</span>
      </div>
      <div class="ss-kv">
        <span class="ss-kv__k">Length</span>
        <span class="ss-kv__v is-mono"
          >{durationMs(audio.spanEndMs - audio.spanStartMs)}</span
        >
      </div>
      <div class="ss-insp__sec">Why it matched</div>
      <div class="ss-kv">
        <span class="ss-kv__k">Source</span>
        <span class="ss-kv__v"
          >{audio.foundByMeaning ? "Meaning" : "Transcript"} · {audio.matchCount}
          {audio.matchCount === 1 ? "hit" : "hits"}</span
        >
      </div>
      {#if audio.hasSecretRedactions}
        <div class="ss-kv">
          <span class="ss-kv__k">Redacted</span>
          <span class="ss-kv__v is-mono"
            >{audio.secretRedactionCount}
            {audio.secretRedactionCount === 1 ? "secret" : "secrets"}</span
          >
        </div>
      {/if}
      <div class="result-inspector__acts">
        <button
          type="button"
          class="btn btn--sm btn--primary result-inspector__act"
          onclick={() => search.openResultAt(search.selectedIndex)}
        >
          Open conversation<span class="kbd result-inspector__key">⏎</span>
        </button>
      </div>
    {/if}
  </div>
</aside>

<style>
  /* The header label is the kit's mono caps; the panel body is the kit's. Only
     the preview well, the snippet's reading treatment and the action stack are
     this surface's own. */
  .result-inspector__title {
    color: var(--app-text-strong);
  }

  .result-inspector__preview {
    position: relative;
    display: grid;
    place-items: center;
    margin: 10px 10px 8px;
    aspect-ratio: 16 / 9;
    border-radius: 5px;
    overflow: hidden;
    background: var(--app-surface-subtle);
    box-shadow: 0 0 0 var(--hairline) var(--app-border);
    color: var(--app-text-faint);
  }

  .result-inspector__preview img {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
    object-position: top center;
  }

  .result-inspector__strong {
    font-size: var(--t-ui);
  }

  .result-inspector__url {
    font-size: var(--t-label);
  }

  .result-inspector__snippet {
    color: var(--app-text);
    line-height: 1.5;
  }

  .result-inspector__snippet mark {
    border-radius: 2px;
    padding: 0 1px;
    background: color-mix(in srgb, var(--app-accent) 26%, transparent);
    color: var(--app-text-strong);
  }

  .result-inspector__acts {
    display: flex;
    flex-direction: column;
    gap: 5px;
    padding: 12px 10px 0;
  }

  .result-inspector__act {
    justify-content: flex-start;
    width: 100%;
  }

  /* The shortcut sits hard right of its own action, so the panel reads as a
     list of "this key does this". On the accent-filled primary it has to carry
     its own contrast — the global `.kbd` is built for quiet surfaces. */
  .result-inspector__key {
    margin-left: auto;
  }

  .btn--primary .result-inspector__key {
    background: color-mix(in srgb, var(--app-accent-contrast) 22%, transparent);
    color: var(--app-accent-contrast);
  }
</style>
