<script lang="ts">
  import { onMount, onDestroy, tick } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { getCurrentWindow } from "@tauri-apps/api/window";
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

  // Roving selection over the flattened result list (frames first, then audio).
  // -1 means nothing highlighted. The search input keeps DOM focus the whole
  // time; selection is surfaced via aria-activedescendant + a `selected` class.
  let selectedIndex = $state(-1);

  // The window is reused across summons (hidden, not destroyed), so its state
  // persists while it's closed. Re-summoning within 5s resumes where you left
  // off; once it has been closed for 5s, reset to the empty search state so the
  // next summon is fresh. Clearing only happens while the window is *not* open.
  const IDLE_CLEAR_MS = 5000;
  let windowFocused = $state(true);
  let idleClearTimer: ReturnType<typeof setTimeout> | null = null;

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
      selectedIndex = -1;
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
      // Auto-highlight the top hit so a hurried Enter opens it (spotlight-style).
      selectedIndex = response.frames.length + response.audio.length > 0 ? 0 : -1;

      void loadThumbnails(response.frames, generation);
    } catch (error) {
      if (generation !== searchGeneration) {
        return;
      }
      frames = [];
      audio = [];
      resultsQuery = trimmed;
      loading = false;
      selectedIndex = -1;
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

  // ---------------------------------------------------------------------------
  // Keyboard navigation (search mode)
  //
  // Results render as a single flattened list (frames first, then audio) so the
  // arrow keys can roam across both sections. `selectedIndex` indexes into that
  // flattened order; the helpers below translate it back to a concrete result.
  // ---------------------------------------------------------------------------

  let resultCount = $derived(frames.length + audio.length);
  const OPTION_ID_PREFIX = "qr-opt-";
  let activeOptionId = $derived(
    selectedIndex >= 0 ? `${OPTION_ID_PREFIX}${selectedIndex}` : undefined,
  );

  // Open the flattened result at `index`, mapping it back to a frame or audio.
  function openResultAt(index: number): void {
    if (index < 0 || index >= resultCount) {
      return;
    }
    if (index < frames.length) {
      void selectFrame(frames[index]);
    } else {
      void selectAudio(audio[index - frames.length]);
    }
  }

  function moveSelection(delta: number): void {
    if (resultCount === 0) {
      return;
    }
    // Wrap around the ends; a first ArrowDown from -1 lands on the top result.
    const base = selectedIndex < 0 ? (delta > 0 ? -1 : 0) : selectedIndex;
    selectedIndex = (base + delta + resultCount) % resultCount;
  }

  function handleSearchKeydown(event: KeyboardEvent): void {
    if (event.isComposing) {
      return;
    }

    // Tab (and ⌘/Ctrl+Enter) pivots to Ask AI, carrying the current query as
    // the seed. Shift+Tab is left to native focus traversal.
    if (
      askAvailable &&
      ((event.key === "Tab" && !event.shiftKey) ||
        (event.key === "Enter" && (event.metaKey || event.ctrlKey)))
    ) {
      event.preventDefault();
      void activateAskAi();
      return;
    }

    // ⌘/Ctrl+1–9 jumps straight to the Nth result.
    if ((event.metaKey || event.ctrlKey) && /^[1-9]$/.test(event.key)) {
      const index = Number(event.key) - 1;
      if (index < resultCount) {
        event.preventDefault();
        openResultAt(index);
      }
      return;
    }

    switch (event.key) {
      case "ArrowDown":
        event.preventDefault();
        moveSelection(1);
        break;
      case "ArrowUp":
        event.preventDefault();
        moveSelection(-1);
        break;
      case "Home":
        if (resultCount > 0) {
          event.preventDefault();
          selectedIndex = 0;
        }
        break;
      case "End":
        if (resultCount > 0) {
          event.preventDefault();
          selectedIndex = resultCount - 1;
        }
        break;
      case "Enter":
        if (selectedIndex >= 0) {
          event.preventDefault();
          openResultAt(selectedIndex);
        }
        break;
    }
  }

  // Put the cursor on whatever field is live for the current mode. Called on
  // mount and every time the (reused) window regains focus on a fresh summon.
  function focusActiveField(): void {
    if (mode === "ask") {
      if (askSubmitted) {
        askAreaEl?.focus();
      } else {
        askInputEl?.focus();
      }
      return;
    }
    inputEl?.focus();
    // Select any leftover query so typing immediately replaces it.
    inputEl?.select();
  }

  // Escape steps back: in Ask AI mode the first press returns to search (the
  // layout's window handler closes the window on a second press from search).
  function handleRootKeydown(event: KeyboardEvent): void {
    if (
      event.key === "Escape" &&
      mode === "ask" &&
      !event.metaKey &&
      !event.ctrlKey &&
      !event.altKey &&
      !event.shiftKey &&
      !event.isComposing
    ) {
      event.preventDefault();
      event.stopPropagation();
      void backToSearch();
    }
  }

  // ---------------------------------------------------------------------------
  // Ask AI
  //
  // Ask AI pivots from the current Quick Search query into a PI-driven answer
  // seeded with redacted broker results for that same query. State is fully
  // ephemeral: a fresh window summon recreates the component, and returning to
  // search mode resets everything below. Nothing is persisted.
  // ---------------------------------------------------------------------------

  type AskAiAvailability = {
    available: boolean;
    reason: string | null;
  };

  type AskAiStatusEvent = {
    conversationId: string;
    phase: "seeding" | "thinking" | "tool";
    seededResultCount?: number;
    tool?: string;
  };

  type AskAiDeltaEvent = {
    conversationId: string;
    text: string;
  };

  type AskAiDoneEvent = {
    conversationId: string;
  };

  type AskAiErrorEvent = {
    conversationId: string;
    message: string;
  };

  type AskAiPhase = "seeding" | "thinking" | "streaming" | "done" | "error";

  let mode = $state<"search" | "ask">("search");

  // Availability is resolved on mount. Until it resolves the affordance is
  // treated as unavailable so we never render a dead button that errors.
  let askAvailability = $state<AskAiAvailability | null>(null);

  // The question being asked (and the read-only header once submitted), plus
  // the editable input used when Ask AI is opened with no seed query.
  let askQuestion = $state("");
  let askInput = $state("");
  let askInputEl = $state<HTMLTextAreaElement | null>(null);
  let askSubmitted = $state(false);

  // Streaming state for the active conversation.
  let askConversationId = $state<string | null>(null);
  let askPhase = $state<AskAiPhase>("seeding");
  let askAnswer = $state("");
  let askErrorMessage = $state<string | null>(null);
  let askSeededResultCount = $state<number | null>(null);
  // Current brokered tool activity label (e.g. "Searching your captures"),
  // shown as a working line while the agent gathers more context mid-answer.
  let askToolActivity = $state<string | null>(null);
  // True between ask_ai_start resolving and a terminal done/error event.
  let askStreaming = $state(false);

  let askAnswerEl = $state<HTMLDivElement | null>(null);
  // The Ask AI answer region; focused on entry so Escape (back-to-search) and
  // scroll keys are captured even when the seeded path renders no text input.
  let askAreaEl = $state<HTMLDivElement | null>(null);

  function friendlyAskReason(reason: string | null): string {
    switch (reason) {
      case "ask_ai_disabled":
        return "Enable Ask AI in Settings";
      case "pi_not_found":
        return "Set up PI to use Ask AI";
      case "pi_version_too_old":
        return "Update PI to use Ask AI";
      case "pi_auth_missing":
        return "Sign in to PI to use Ask AI";
      case "pi_no_provider":
        return "Configure a PI provider to use Ask AI";
      default:
        return "Set up PI to use Ask AI";
    }
  }

  let askAvailable = $derived(askAvailability?.available === true);
  let askUnavailableHint = $derived(
    askAvailability && !askAvailability.available
      ? friendlyAskReason(askAvailability.reason)
      : null,
  );

  async function loadAskAvailability(): Promise<void> {
    try {
      askAvailability = await invoke<AskAiAvailability>("ask_ai_availability");
    } catch (error) {
      // Treat a failed availability probe as unavailable rather than erroring.
      askAvailability = {
        available: false,
        reason: error instanceof Error ? error.message : String(error),
      };
    }
  }

  // Begin an Ask AI conversation. `question` is what gets answered; `seedQuery`
  // seeds the broker search (the prior Quick Search query, or null).
  async function startAsk(question: string, seedQuery: string | null): Promise<void> {
    const trimmedQuestion = question.trim();
    if (trimmedQuestion.length === 0) {
      return;
    }

    // Cancel any in-flight stream before starting a new one.
    if (askStreaming && askConversationId !== null) {
      await cancelActiveAsk();
    }

    const conversationId = crypto.randomUUID();
    askConversationId = conversationId;
    askQuestion = trimmedQuestion;
    askSubmitted = true;
    askPhase = "seeding";
    askAnswer = "";
    askErrorMessage = null;
    askSeededResultCount = null;
    askToolActivity = null;
    askStreaming = true;

    try {
      await invoke<void>("ask_ai_start", {
        request: {
          conversationId,
          question: trimmedQuestion,
          seedQuery: seedQuery && seedQuery.trim().length > 0 ? seedQuery.trim() : null,
        },
      });
    } catch (error) {
      // A start that never streamed: ignore stale (superseded) failures.
      if (askConversationId !== conversationId) {
        return;
      }
      askStreaming = false;
      askPhase = "error";
      askErrorMessage = error instanceof Error ? error.message : String(error);
    }
  }

  async function cancelActiveAsk(): Promise<void> {
    const conversationId = askConversationId;
    if (conversationId === null || !askStreaming) {
      return;
    }
    askStreaming = false;
    try {
      await invoke<void>("ask_ai_cancel", { request: { conversationId } });
    } catch {
      // Cancellation is best-effort; the conversation is being abandoned.
    }
  }

  // Activate the Ask AI affordance from search mode.
  async function activateAskAi(): Promise<void> {
    if (!askAvailable) {
      return;
    }

    const seed = trimmedQuery;
    mode = "ask";

    if (seed.length > 0) {
      // Seeded: immediately submit the current query as the question. Focus the
      // answer region (no text input renders) so Escape/scroll keys are caught.
      askInput = "";
      void startAsk(seed, seed);
      await tick();
      askAreaEl?.focus();
    } else {
      // Unseeded: show an empty ask input for the user to type a question.
      askInput = "";
      askSubmitted = false;
      askQuestion = "";
      await tick();
      askInputEl?.focus();
    }
  }

  // Submit the typed question from the unseeded ask input (Enter). The input is
  // replaced by the read-only question, so move focus to the answer region.
  async function submitAskInput(): Promise<void> {
    const typed = askInput.trim();
    if (typed.length === 0) {
      return;
    }
    void startAsk(typed, null);
    await tick();
    askAreaEl?.focus();
  }

  function handleAskInputKeydown(event: KeyboardEvent): void {
    // Enter submits; Shift+Enter inserts a newline.
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      void submitAskInput();
    }
  }

  // Return to search mode, abandoning any in-flight stream and resetting all
  // ephemeral ask state.
  async function backToSearch(): Promise<void> {
    await cancelActiveAsk();
    mode = "search";
    askConversationId = null;
    askQuestion = "";
    askInput = "";
    askSubmitted = false;
    askPhase = "seeding";
    askAnswer = "";
    askErrorMessage = null;
    askSeededResultCount = null;
    askToolActivity = null;
    await tick();
    inputEl?.focus();
  }

  $effect(() => {
    scheduleSearch(query);
  });

  // Keep the streaming answer scrolled to the latest token.
  $effect(() => {
    if (askPhase === "streaming" && askAnswer.length > 0 && askAnswerEl) {
      askAnswerEl.scrollTop = askAnswerEl.scrollHeight;
    }
  });

  // Keep the highlighted result within the scroll viewport as it moves.
  $effect(() => {
    if (mode !== "search" || selectedIndex < 0) {
      return;
    }
    document
      .getElementById(`${OPTION_ID_PREFIX}${selectedIndex}`)
      ?.scrollIntoView({ block: "nearest" });
  });

  let trimmedQuery = $derived(query.trim());
  let belowMinimum = $derived(trimmedQuery.length < MIN_QUERY_LENGTH);
  let hasResults = $derived(frames.length > 0 || audio.length > 0);
  let showEmpty = $derived(
    !belowMinimum && !loading && !errorMessage && !hasResults && resultsQuery.length > 0,
  );

  // A search or Ask AI operation is in flight — that counts as activity, so the
  // idle countdown is suspended while it runs.
  let operationRunning = $derived(loading || askStreaming);

  // There is something to reset: anything other than a pristine, empty search
  // box. Clearing the pristine state would be a no-op, so the timer only arms
  // when content (query, results, error, or an Ask AI view) is present.
  let hasClearableState = $derived(
    mode === "ask" ||
      trimmedQuery.length > 0 ||
      hasResults ||
      errorMessage !== null ||
      resultsQuery.length > 0,
  );

  // Reset the window to the just-summoned empty search state and refocus.
  async function clearState(): Promise<void> {
    await cancelActiveAsk();
    clearDebounce();
    // Invalidate any in-flight search so a late response can't repopulate.
    searchGeneration += 1;
    mode = "search";
    query = "";
    frames = [];
    audio = [];
    resultsQuery = "";
    errorMessage = null;
    loading = false;
    selectedIndex = -1;
    askConversationId = null;
    askQuestion = "";
    askInput = "";
    askSubmitted = false;
    askPhase = "seeding";
    askAnswer = "";
    askErrorMessage = null;
    askSeededResultCount = null;
    askToolActivity = null;
    await tick();
    inputEl?.focus();
  }

  function clearIdleTimer(): void {
    if (idleClearTimer !== null) {
      clearTimeout(idleClearTimer);
      idleClearTimer = null;
    }
  }

  function scheduleIdleClear(): void {
    clearIdleTimer();
    idleClearTimer = setTimeout(() => {
      idleClearTimer = null;
      void clearState();
    }, IDLE_CLEAR_MS);
  }

  // Arm the countdown once the window is closed (unfocused) with clearable
  // content and nothing running. Re-opening it (focus) cancels the timer and
  // preserves the state for a quick resume.
  $effect(() => {
    if (!windowFocused && hasClearableState && !operationRunning) {
      scheduleIdleClear();
    } else {
      clearIdleTimer();
    }
  });

  onMount(() => {
    focusActiveField();
    void loadAskAvailability();

    let destroyed = false;
    let unlistenStatus: (() => void) | undefined;
    let unlistenDelta: (() => void) | undefined;
    let unlistenDone: (() => void) | undefined;
    let unlistenError: (() => void) | undefined;
    let unlistenFocus: (() => void) | undefined;

    // The window is hidden/re-shown rather than recreated across summons, so
    // re-grab focus each time it becomes key — onMount alone fires only once.
    getCurrentWindow()
      .onFocusChanged(({ payload: focused }) => {
        windowFocused = focused;
        if (focused) {
          void tick().then(focusActiveField);
        }
      })
      .then((fn) => {
        if (destroyed) fn();
        else unlistenFocus = fn;
      });

    listen<AskAiStatusEvent>("ask_ai_status", (event) => {
      if (event.payload.conversationId !== askConversationId) return;
      // A "tool" status is mid-answer activity: surface the tool label without
      // touching askPhase, so any already-streamed answer text stays visible.
      if (event.payload.phase === "tool") {
        askToolActivity = event.payload.tool ?? "Working";
        return;
      }
      if (typeof event.payload.seededResultCount === "number") {
        askSeededResultCount = event.payload.seededResultCount;
      }
      // Don't regress out of streaming once tokens have started arriving.
      if (askPhase !== "streaming") {
        askPhase = event.payload.phase;
      }
    }).then((fn) => {
      if (destroyed) fn();
      else unlistenStatus = fn;
    });

    listen<AskAiDeltaEvent>("ask_ai_delta", (event) => {
      if (event.payload.conversationId !== askConversationId) return;
      // The model resumed answering: clear any in-progress tool activity.
      askToolActivity = null;
      askPhase = "streaming";
      askAnswer += event.payload.text;
    }).then((fn) => {
      if (destroyed) fn();
      else unlistenDelta = fn;
    });

    listen<AskAiDoneEvent>("ask_ai_done", (event) => {
      if (event.payload.conversationId !== askConversationId) return;
      askStreaming = false;
      askToolActivity = null;
      askPhase = "done";
    }).then((fn) => {
      if (destroyed) fn();
      else unlistenDone = fn;
    });

    listen<AskAiErrorEvent>("ask_ai_error", (event) => {
      if (event.payload.conversationId !== askConversationId) return;
      askStreaming = false;
      askToolActivity = null;
      askPhase = "error";
      askErrorMessage = event.payload.message;
    }).then((fn) => {
      if (destroyed) fn();
      else unlistenError = fn;
    });

    return () => {
      destroyed = true;
      unlistenStatus?.();
      unlistenDelta?.();
      unlistenDone?.();
      unlistenError?.();
      unlistenFocus?.();
    };
  });

  onDestroy(() => {
    clearDebounce();
    clearIdleTimer();
    void cancelActiveAsk();
  });
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="quick-recall" onkeydown={handleRootKeydown}>
  {#if mode === "search"}
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
      role="combobox"
      aria-expanded={resultCount > 0}
      aria-controls="quick-recall-results-list"
      aria-activedescendant={activeOptionId}
      onkeydown={handleSearchKeydown}
    />
    {#if askAvailable}
      <button
        type="button"
        class="quick-recall__ask-button"
        onclick={() => void activateAskAi()}
        aria-label="Ask AI"
      >
        Ask AI <span class="quick-recall__ask-key" aria-hidden="true">⇥</span>
      </button>
    {:else}
      <button
        type="button"
        class="quick-recall__ask-button quick-recall__ask-button--disabled"
        disabled
        aria-label={askUnavailableHint ?? "Ask AI unavailable"}
        title={askUnavailableHint ?? "Ask AI unavailable"}
      >
        Ask AI
      </button>
    {/if}
  </div>

  {#if askUnavailableHint}
    <p class="quick-recall__ask-hint">{askUnavailableHint}</p>
  {/if}

  <div
    id="quick-recall-results-list"
    class="quick-recall__results"
    role="listbox"
    aria-label="Search results"
  >
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
        <div class="quick-recall__section" role="presentation">
          <span class="quick-recall__section-label">Screen</span>
          <div class="quick-recall__list" role="presentation">
            {#each frames as result, i (result.groupKey)}
              <SearchResultCard
                kind="frame"
                frame={result}
                thumbnailUrl={thumbnailCache.get(result.thumbnailFrameId) ?? null}
                id={`${OPTION_ID_PREFIX}${i}`}
                selected={selectedIndex === i}
                onselect={() => void selectFrame(result)}
              />
            {/each}
          </div>
        </div>
      {/if}

      {#if audio.length > 0}
        <div class="quick-recall__section" role="presentation">
          <span class="quick-recall__section-label">Audio</span>
          <div class="quick-recall__list" role="presentation">
            {#each audio as result, i (result.groupKey)}
              <SearchResultCard
                kind="audio"
                audio={result}
                id={`${OPTION_ID_PREFIX}${frames.length + i}`}
                selected={selectedIndex === frames.length + i}
                onselect={() => void selectAudio(result)}
              />
            {/each}
          </div>
        </div>
      {/if}
    {/if}
  </div>
  {:else}
  <div class="quick-recall__field quick-recall__field--ask">
    <button
      type="button"
      class="quick-recall__back"
      onclick={() => void backToSearch()}
      aria-label="Back to search"
    >
      ← Back
    </button>
    {#if askSubmitted}
      <span class="quick-recall__ask-question" title={askQuestion}>{askQuestion}</span>
    {:else}
      <textarea
        bind:this={askInputEl}
        bind:value={askInput}
        class="quick-recall__ask-input"
        rows="1"
        autocomplete="off"
        autocapitalize="off"
        spellcheck="false"
        placeholder="Ask anything about your captures…"
        aria-label="Ask AI a question"
        onkeydown={handleAskInputKeydown}
      ></textarea>
    {/if}
  </div>

  <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
  <div
    bind:this={askAreaEl}
    class="quick-recall__results quick-recall__answer-area"
    aria-live="polite"
    tabindex="-1"
  >
    {#if !askSubmitted}
      <p class="quick-recall__state">Type a question and press Enter to ask.</p>
    {:else if askPhase === "error"}
      <p class="quick-recall__state quick-recall__state--error">
        {askErrorMessage ?? "Ask AI failed."}
      </p>
    {:else}
      {#if askSeededResultCount !== null && askSeededResultCount > 0}
        <p class="quick-recall__seeded">
          Seeded with {askSeededResultCount}
          {askSeededResultCount === 1 ? "result" : "results"}
        </p>
      {/if}

      {#if askPhase === "seeding"}
        <p class="quick-recall__state quick-recall__state--working">
          <span class="quick-recall__dot" aria-hidden="true"></span>
          Searching your captures…
        </p>
      {:else if askPhase === "thinking" && askToolActivity === null}
        <p class="quick-recall__state quick-recall__state--working">
          <span class="quick-recall__dot" aria-hidden="true"></span>
          Thinking…
        </p>
      {:else}
        {#if askPhase === "streaming" || askPhase === "done"}
          <div bind:this={askAnswerEl} class="quick-recall__answer">{askAnswer}{#if askStreaming}<span class="quick-recall__caret" aria-hidden="true"></span>{/if}</div>
        {/if}
        {#if askToolActivity !== null}
          <p class="quick-recall__state quick-recall__state--working">
            <span class="quick-recall__dot" aria-hidden="true"></span>
            {askToolActivity}…
          </p>
        {/if}
      {/if}
    {/if}
  </div>
  {/if}

  <div class="quick-recall__footer" aria-hidden="true">
    {#if mode === "search"}
      {#if resultCount > 0}
        <span class="quick-recall__hint-item"><kbd>↑</kbd><kbd>↓</kbd> navigate</span>
        <span class="quick-recall__hint-item"><kbd>↵</kbd> open</span>
        {#if askAvailable}
          <span class="quick-recall__hint-item"><kbd>⇥</kbd> Ask AI</span>
        {/if}
        <span class="quick-recall__hint-item"><kbd>esc</kbd> close</span>
      {:else}
        {#if askAvailable}
          <span class="quick-recall__hint-item"><kbd>⇥</kbd> Ask AI</span>
        {/if}
        <span class="quick-recall__hint-item"><kbd>esc</kbd> close</span>
      {/if}
    {:else if !askSubmitted}
      <span class="quick-recall__hint-item"><kbd>↵</kbd> ask</span>
      <span class="quick-recall__hint-item"><kbd>esc</kbd> back</span>
    {:else}
      <span class="quick-recall__hint-item"><kbd>esc</kbd> back</span>
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

  .quick-recall__footer {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 8px 14px;
    border-top: 1px solid var(--app-border);
    background: var(--app-surface-subtle);
  }

  .quick-recall__hint-item {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 10.5px;
    line-height: 1;
    color: var(--app-text-subtle);
    white-space: nowrap;
  }

  .quick-recall__footer kbd {
    font-family: inherit;
    font-size: 10px;
    line-height: 1;
    text-transform: lowercase;
    color: var(--app-text-muted);
    background: var(--app-surface);
    border: 1px solid var(--app-border);
    border-radius: 5px;
    padding: 3px 5px;
    min-width: 9px;
    text-align: center;
  }

  .quick-recall__answer-area:focus {
    outline: none;
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

  .quick-recall__ask-button {
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-family: inherit;
    font-size: 12px;
    line-height: 1;
    color: var(--app-text);
    background: var(--app-surface-subtle);
    border: 1px solid var(--app-border);
    border-radius: 6px;
    padding: 6px 9px;
    cursor: pointer;
    transition: border-color 0.12s ease, color 0.12s ease;
  }

  .quick-recall__ask-button:hover {
    border-color: var(--app-accent);
    color: var(--app-text-strong);
  }

  .quick-recall__ask-key {
    font-size: 11px;
    color: var(--app-text-muted);
  }

  .quick-recall__ask-button--disabled,
  .quick-recall__ask-button:disabled {
    color: var(--app-text-subtle);
    cursor: not-allowed;
  }

  .quick-recall__ask-button--disabled:hover {
    border-color: var(--app-border);
    color: var(--app-text-subtle);
  }

  .quick-recall__ask-hint {
    margin: 0;
    padding: 6px 18px 0;
    font-size: 11px;
    line-height: 1.4;
    color: var(--app-text-subtle);
    flex-shrink: 0;
  }

  .quick-recall__field--ask {
    gap: 12px;
  }

  .quick-recall__back {
    flex-shrink: 0;
    font-family: inherit;
    font-size: 12px;
    line-height: 1;
    color: var(--app-text-muted);
    background: var(--app-surface-subtle);
    border: 1px solid var(--app-border);
    border-radius: 6px;
    padding: 6px 9px;
    cursor: pointer;
    transition: border-color 0.12s ease, color 0.12s ease;
  }

  .quick-recall__back:hover {
    border-color: var(--app-accent);
    color: var(--app-text-strong);
  }

  .quick-recall__ask-question {
    flex: 1;
    min-width: 0;
    font-size: 16px;
    line-height: 1.4;
    color: var(--app-text-strong);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .quick-recall__ask-input {
    flex: 1;
    min-width: 0;
    border: none;
    outline: none;
    background: transparent;
    color: var(--app-text-strong);
    font-family: inherit;
    font-size: 16px;
    line-height: 1.4;
    padding: 0;
    resize: none;
    caret-color: var(--app-accent);
  }

  .quick-recall__ask-input::placeholder {
    color: var(--app-text-subtle);
  }

  .quick-recall__answer-area {
    gap: 10px;
  }

  .quick-recall__seeded {
    margin: 0;
    padding: 0 2px;
    font-size: 11px;
    line-height: 1;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--app-text-subtle);
    flex-shrink: 0;
  }

  .quick-recall__state--working {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--app-text-muted);
  }

  .quick-recall__dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--app-accent);
    flex-shrink: 0;
    animation: quick-recall-pulse 1.1s ease-in-out infinite;
  }

  @keyframes quick-recall-pulse {
    0%,
    100% {
      opacity: 0.3;
    }
    50% {
      opacity: 1;
    }
  }

  .quick-recall__answer {
    margin: 0;
    padding: 2px 2px 8px;
    font-size: 14px;
    line-height: 1.6;
    color: var(--app-text);
    white-space: pre-wrap;
    word-break: break-word;
    overflow-wrap: anywhere;
  }

  .quick-recall__caret {
    display: inline-block;
    width: 7px;
    height: 1.05em;
    margin-left: 2px;
    vertical-align: text-bottom;
    background: var(--app-accent);
    animation: quick-recall-blink 1s steps(2, start) infinite;
  }

  @keyframes quick-recall-blink {
    0%,
    100% {
      opacity: 1;
    }
    50% {
      opacity: 0;
    }
  }
</style>
