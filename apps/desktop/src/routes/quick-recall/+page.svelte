<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import SearchResultCard from "$lib/components/SearchResultCard.svelte";
  import { framePreviewAssetUrl } from "$lib/frame-preview";
  import { closeCurrentWindow } from "$lib/surface-windows";
  import type {
    SearchCaptureResponse,
    FrameSearchResultDto,
    AudioSearchResultDto,
    FrameScrubPreviewsDto,
  } from "$lib/types/app-infra";

  const MIN_QUERY_LENGTH = 2;
  const DEBOUNCE_MS = 250;

  let query = $state("");
  let inputEl = $state<HTMLInputElement | null>(null);

  let frames = $state<FrameSearchResultDto[]>([]);
  let audio = $state<AudioSearchResultDto[]>([]);
  let loading = $state(false);
  let errorMessage = $state<string | null>(null);
  // The query string that the currently-displayed results belong to.
  let resultsQuery = $state("");
  let thumbnailCache = $state(new Map<number, string>());

  // Generation token so stale (out-of-order) responses are discarded.
  let searchGeneration = 0;
  let debounceTimer: ReturnType<typeof setTimeout> | null = null;

  function clearDebounce(): void {
    if (debounceTimer !== null) {
      clearTimeout(debounceTimer);
      debounceTimer = null;
    }
  }

  function scheduleSearch(raw: string): void {
    clearDebounce();
    const trimmed = raw.trim();

    if (trimmed.length < MIN_QUERY_LENGTH) {
      // Invalidate any in-flight request and reset to the idle state.
      searchGeneration += 1;
      frames = [];
      audio = [];
      loading = false;
      errorMessage = null;
      resultsQuery = "";
      return;
    }

    debounceTimer = setTimeout(() => {
      void runSearch(trimmed);
    }, DEBOUNCE_MS);
  }

  async function runSearch(trimmed: string): Promise<void> {
    searchGeneration += 1;
    const generation = searchGeneration;
    loading = true;
    errorMessage = null;

    try {
      const response = await invoke<SearchCaptureResponse>("search_capture", {
        request: {
          query: trimmed,
          frameLimit: 5,
          frameOffset: 0,
          audioLimit: 5,
          audioOffset: 0,
          refinements: {},
        },
      });

      if (generation !== searchGeneration) {
        return;
      }

      frames = response.frames;
      audio = response.audio;
      resultsQuery = trimmed;
      loading = false;

      void loadThumbnails(response.frames, generation);
    } catch (error) {
      if (generation !== searchGeneration) {
        return;
      }
      frames = [];
      audio = [];
      resultsQuery = trimmed;
      loading = false;
      errorMessage = error instanceof Error ? error.message : String(error);
    }
  }

  async function loadThumbnails(
    frameResults: FrameSearchResultDto[],
    generation: number,
  ): Promise<void> {
    const frameIds = frameResults
      .map((result) => result.thumbnailFrameId)
      .filter((id) => !thumbnailCache.has(id));

    const uniqueIds = Array.from(new Set(frameIds));
    if (uniqueIds.length === 0) {
      return;
    }

    try {
      const response = await invoke<FrameScrubPreviewsDto>("get_frame_scrub_previews", {
        request: { frameIds: uniqueIds },
      });

      if (generation !== searchGeneration) {
        return;
      }

      const next = new Map(thumbnailCache);
      for (const entry of response.previews) {
        if (entry.preview) {
          next.set(entry.frameId, framePreviewAssetUrl(entry.preview.filePath));
        }
      }
      thumbnailCache = next;
    } catch {
      // Thumbnails are best-effort; the card falls back to its glyph.
    }
  }

  async function selectFrame(result: FrameSearchResultDto): Promise<void> {
    await invoke("open_capture_result_in_main_window", {
      kind: "frame",
      frameId: result.representativeFrame.id,
      audioSegmentId: null,
    });
    await closeCurrentWindow();
  }

  async function selectAudio(result: AudioSearchResultDto): Promise<void> {
    await invoke("open_capture_result_in_main_window", {
      kind: "audio",
      frameId: null,
      audioSegmentId: result.audioSegment.id,
    });
    await closeCurrentWindow();
  }

  $effect(() => {
    scheduleSearch(query);
  });

  let trimmedQuery = $derived(query.trim());
  let belowMinimum = $derived(trimmedQuery.length < MIN_QUERY_LENGTH);
  let hasResults = $derived(frames.length > 0 || audio.length > 0);
  let showEmpty = $derived(
    !belowMinimum && !loading && !errorMessage && !hasResults && resultsQuery.length > 0,
  );

  onMount(() => {
    inputEl?.focus();
  });

  onDestroy(() => {
    clearDebounce();
  });
</script>

<div class="quick-recall">
  <div class="quick-recall__field">
    <span class="quick-recall__glyph" aria-hidden="true">⌕</span>
    <input
      bind:this={inputEl}
      bind:value={query}
      class="quick-recall__input"
      type="text"
      autocomplete="off"
      autocapitalize="off"
      spellcheck="false"
      placeholder="Search your captures…"
      aria-label="Search your captures"
    />
    <kbd class="quick-recall__hint">esc</kbd>
  </div>

  <div class="quick-recall__results" role="listbox" aria-label="Search results">
    {#if belowMinimum}
      <p class="quick-recall__state">Type at least {MIN_QUERY_LENGTH} characters to search.</p>
    {:else if loading}
      <p class="quick-recall__state">Searching…</p>
    {:else if errorMessage}
      <p class="quick-recall__state quick-recall__state--error">{errorMessage}</p>
    {:else if showEmpty}
      <p class="quick-recall__state">No matches for “{resultsQuery}”.</p>
    {:else}
      {#if frames.length > 0}
        <div class="quick-recall__section">
          <span class="quick-recall__section-label">Screen</span>
          <div class="quick-recall__list">
            {#each frames as result (result.groupKey)}
              <SearchResultCard
                kind="frame"
                frame={result}
                thumbnailUrl={thumbnailCache.get(result.thumbnailFrameId) ?? null}
                onselect={() => void selectFrame(result)}
              />
            {/each}
          </div>
        </div>
      {/if}

      {#if audio.length > 0}
        <div class="quick-recall__section">
          <span class="quick-recall__section-label">Audio</span>
          <div class="quick-recall__list">
            {#each audio as result (result.groupKey)}
              <SearchResultCard
                kind="audio"
                audio={result}
                onselect={() => void selectAudio(result)}
              />
            {/each}
          </div>
        </div>
      {/if}
    {/if}
  </div>
</div>

<style>
  .quick-recall {
    height: 100vh;
    height: 100dvh;
    width: 100%;
    display: flex;
    flex-direction: column;
    box-sizing: border-box;
    background: var(--app-surface);
    border: 1px solid var(--app-border);
    border-radius: 12px;
    overflow: hidden;
    color: var(--app-text);
    font-family: inherit;
  }

  .quick-recall__field {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 16px 18px;
    flex-shrink: 0;
    border-bottom: 1px solid var(--app-border);
  }

  .quick-recall__glyph {
    font-size: 20px;
    line-height: 1;
    color: var(--app-text-muted);
    flex-shrink: 0;
    transform: rotate(-45deg);
  }

  .quick-recall__input {
    flex: 1;
    min-width: 0;
    border: none;
    outline: none;
    background: transparent;
    color: var(--app-text-strong);
    font-family: inherit;
    font-size: 18px;
    line-height: 1.4;
    padding: 0;
    caret-color: var(--app-accent);
  }

  .quick-recall__input::placeholder {
    color: var(--app-text-subtle);
  }

  .quick-recall__hint {
    flex-shrink: 0;
    font-family: inherit;
    font-size: 11px;
    line-height: 1;
    text-transform: lowercase;
    color: var(--app-text-muted);
    background: var(--app-surface-subtle);
    border: 1px solid var(--app-border);
    border-radius: 6px;
    padding: 4px 7px;
  }

  .quick-recall__results {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 12px 14px;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .quick-recall__section {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .quick-recall__section-label {
    font-size: 11px;
    line-height: 1;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--app-text-subtle);
    padding: 0 2px;
  }

  .quick-recall__list {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .quick-recall__state {
    margin: 0;
    padding: 8px 2px;
    font-size: 13px;
    line-height: 1.5;
    color: var(--app-text-muted);
  }

  .quick-recall__state--error {
    color: var(--app-accent);
  }
</style>
