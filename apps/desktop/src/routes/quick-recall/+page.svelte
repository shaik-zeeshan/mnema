<script lang="ts">
  import { onMount, onDestroy, tick } from "svelte";
  import { fade } from "svelte/transition";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import SearchResultCard from "$lib/components/SearchResultCard.svelte";
  import AnswerSourceCard from "$lib/components/AnswerSourceCard.svelte";
  import { framePreviewAssetUrl } from "$lib/frame-preview";
  import { closeCurrentWindow } from "$lib/surface-windows";
  import { renderMarkdown } from "$lib/markdown";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import type {
    SearchCaptureResponse,
    SearchCaptureRefinements,
    SearchParseError,
    SearchAppRefinement,
    SearchDateRangeRefinement,
    AudioSegmentSourceKind,
    FrameSearchResultDto,
    AudioSearchResultDto,
    FrameScrubPreviewsDto,
    SearchableApp,
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

  // ---------------------------------------------------------------------------
  // Slice 1: parsed search scope (advanced search syntax foundation)
  //
  // `search_capture` runs the backend operator parser on EVERY raw query and
  // returns three fields Quick Recall previously ignored. We capture them into
  // state here so later slices (2–6) can render filter chips, an inline parse
  // error line, and narrow the section limits to the active scope. We do NOT
  // re-parse operators in the frontend — these come straight from the backend's
  // desugared response; the raw query text still carries the operators.
  //
  //   - appliedRefinements: the desugared scope (date range, apps, window title,
  //     audio sources, screen-only flag) that the parser extracted.
  //   - residualQuery: the free-text left after operators were stripped.
  //   - parseErrors: malformed-operator diagnostics. A non-empty list means the
  //     backend SUPPRESSED results (paused), so later slices drive a distinct
  //     "results paused" state from `firstParseError` instead of "no results".
  //
  // All three reset wherever the result state resets (catch branch in runSearch,
  // the below-minimum branch of scheduleSearch, and clearState), so chips/errors
  // never linger past the query they belong to.
  let appliedRefinements = $state<SearchCaptureRefinements | null>(null);
  let residualQuery = $state("");
  let parseErrors = $state<SearchParseError[]>([]);

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

  // Whether the user prefers reduced motion. Drives the JS-side Svelte mode-switch
  // transition (CSS animations/transitions are gated separately in the style block). Kept
  // live so a system preference flip mid-session is honored.
  let prefersReducedMotion = $state(false);
  // Duration of the hero mode-switch cross-fade; 0 when reduced motion is on.
  let modeFadeMs = $derived(prefersReducedMotion ? 0 : 140);

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

    // While the caret sits in an un-committed field-operator value, the Filter
    // Value List owns the results region; the partial operator must NOT reach the
    // backend (a half-typed value would otherwise read as empty results). Cancel
    // any in-flight search and leave the current results state intact underneath.
    if (isTrailingOperatorPartial(raw)) {
      searchGeneration += 1;
      return;
    }

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
      // Slice 1: drop any parsed scope so stale chips/parse errors don't linger
      // once the query falls back below the minimum length.
      appliedRefinements = null;
      residualQuery = "";
      parseErrors = [];
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
      // Slice 1: narrow the per-section limits to the active scope so a
      // source-restricted query doesn't waste a slot fetching the other kind.
      // Scope is only known AFTER a response, so we read `sectionLimits` (a
      // $derived off the PREVIOUS response's appliedRefinements) optimistically:
      // the FIRST query after a scope change runs at the prior limits and the
      // section narrows on the next keystroke once appliedRefinements catches up.
      // This is the simplest correct approach — the backend still honors the
      // operators in the raw query regardless of these limits, and over-fetching
      // by one section for a single keystroke is harmless. `refinements` stays
      // empty: the operators live in the query TEXT, not this struct.
      const limits = sectionLimits;
      const response = await invoke<SearchCaptureResponse>("search_capture", {
        request: {
          query: trimmed,
          frameLimit: limits.frameLimit,
          frameOffset: 0,
          audioLimit: limits.audioLimit,
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
      // Slice 1: capture the parsed scope so chip/error/limit derivations update.
      appliedRefinements = response.appliedRefinements;
      residualQuery = response.residualQuery;
      parseErrors = response.parseErrors;
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
      // Slice 1: a transport/backend failure isn't a parse outcome — clear the
      // parsed scope so a prior query's chips/parse errors don't survive an error.
      appliedRefinements = null;
      residualQuery = "";
      parseErrors = [];
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

  // Hand off an Ask AI answer source to the main window, mirroring
  // selectFrame/selectAudio (frame xor audio carried by the source kind).
  async function selectSource(source: AskAiSource): Promise<void> {
    await invoke("open_capture_result_in_main_window", {
      kind: source.kind,
      frameId: source.frameId,
      audioSegmentId: source.audioSegmentId,
    });
    await closeCurrentWindow();
  }

  // Load thumbnails for answer-source frames, mirroring loadThumbnails. Best
  // effort: a card without a cached preview falls back to its glyph. No search
  // generation guard applies here (these come from the ask stream, not search).
  async function loadSourceThumbnails(sources: AskAiSource[]): Promise<void> {
    const frameIds = sources
      .filter((source) => source.kind === "frame" && source.frameId != null)
      .map((source) => source.frameId as number)
      .filter((id) => !thumbnailCache.has(id));

    const uniqueIds = Array.from(new Set(frameIds));
    if (uniqueIds.length === 0) {
      return;
    }

    try {
      const response = await invoke<FrameScrubPreviewsDto>("get_frame_scrub_previews", {
        request: { frameIds: uniqueIds },
      });

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

    // Slice 7: while the syntax-help popover is open, a plain Escape just closes
    // it and is stopped here (preventDefault + stopPropagation) so it never
    // reaches the layout's window-close handler. This runs FIRST so the popover
    // is the innermost Escape target; when it's closed the branch is skipped and
    // Escape falls through to the normal search→close-window behavior. The picker
    // and ask-mode never coexist with this (the trigger only renders in search
    // mode and the popover is a transient overlay), so their Escape paths are
    // unaffected.
    if (
      syntaxHelpOpen &&
      event.key === "Escape" &&
      !event.metaKey &&
      !event.ctrlKey &&
      !event.altKey &&
      !event.shiftKey
    ) {
      event.preventDefault();
      event.stopPropagation();
      closeSyntaxHelp();
      return;
    }

    // Slice 5: while the picker is open it fully owns Arrow / Enter / Escape /
    // Tab. Hand the event to the picker FIRST; if it consumes it, none of the
    // results navigation / Ask AI pivot / ghost-accept below runs (exactly one
    // list owns the arrows at any instant).
    if (pickerOpen) {
      if (handlePickerKeydown(event)) {
        return;
      }
      // An unconsumed key while the picker is open (e.g. plain typing) falls
      // through to normal input handling, but the results-navigation switch at
      // the bottom is gated on !pickerOpen so it never double-drives selection.
    }

    // Slice 5: Ctrl+F / Cmd+F opens the Filter Picker from anywhere (empty or
    // not) — a launcher-native summon that doesn't depend on the input contents.
    if (
      (event.metaKey || event.ctrlKey) &&
      !event.altKey &&
      !event.shiftKey &&
      (event.key === "f" || event.key === "F")
    ) {
      event.preventDefault();
      if (!pickerOpen) {
        openPicker();
      }
      return;
    }

    // Slice 5: `/` on an EMPTY input opens the picker. preventDefault so no `/`
    // is inserted (the `/` was the trigger). Empty-input-only by design: once
    // the input is non-empty, `/` is a literal character (so `/usr/local/bin`
    // stays typeable), which is also why "Escape leaves a literal slash" holds —
    // the literal-slash path is exactly the non-empty case, where `/` never
    // triggers, so the picker can't have eaten a slash the user meant literally.
    if (
      event.key === "/" &&
      !event.metaKey &&
      !event.ctrlKey &&
      !event.altKey &&
      !event.shiftKey &&
      query.trim().length === 0 &&
      !pickerOpen
    ) {
      event.preventDefault();
      openPicker();
      return;
    }

    // While the Filter Value List is up it fully owns ↑/↓/Enter/Escape so exactly
    // one list consumes the arrows. It sits ABOVE the Ask AI pivot so Ctrl+Enter
    // is suppressed while the list is up (Esc out first). Tab and → fall through
    // to ghost-accept below (a value-accept), so they are intentionally NOT here.
    if (valueListActive) {
      switch (event.key) {
        case "ArrowDown":
          event.preventDefault();
          moveValueListSelection(1);
          return;
        case "ArrowUp":
          event.preventDefault();
          moveValueListSelection(-1);
          return;
        case "Enter":
          event.preventDefault();
          // Plain Enter commits the highlighted enabled row; Ctrl/Cmd+Enter is
          // suppressed (no Ask AI pivot while the list is up) and is a no-op.
          if (!event.metaKey && !event.ctrlKey) {
            commitHighlightedValueListRow();
          }
          return;
        case "Escape":
          if (!event.metaKey && !event.ctrlKey && !event.altKey && !event.shiftKey) {
            event.preventDefault();
            event.stopPropagation();
            abandonOperator();
            return;
          }
          break;
      }
    }

    // The Ask AI pivot is Ctrl/Cmd+Enter ONLY (ADR 0025). Tab is reserved for
    // ghost-text accept (handled below), never the pivot. The Filter Value List
    // block above suppresses this pivot while it's up (Enter is consumed there).
    if (
      askAvailable &&
      event.key === "Enter" &&
      (event.metaKey || event.ctrlKey)
    ) {
      event.preventDefault();
      void activateAskAi();
      return;
    }

    // Tab accepts the inline ghost completion at ANY caret position (the `→`
    // accept below additionally works, but only at end-of-input, fish-style).
    // Tab no longer pivots to Ask AI. When no ghost is showing, Tab is a no-op
    // that keeps focus in the launcher rather than escaping via native focus
    // traversal. Shift+Tab is left to native focus traversal.
    if (
      event.key === "Tab" &&
      !event.shiftKey &&
      !event.metaKey &&
      !event.ctrlKey &&
      !event.altKey
    ) {
      event.preventDefault();
      if (hasGhost) {
        acceptGhost();
      }
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

    // Slice 2: Backspace with a collapsed caret at position 0 removes the LAST
    // chip (rightmost, nearest the caret) instead of deleting text — the chip row
    // reads as an extension of the query, so a leading Backspace peels off the
    // most-recently-applied scope. Any non-zero caret or selection falls through
    // to native Backspace. Guarded to plain Backspace so it never collides with
    // the Tab / ⌘Enter Ask AI pivot or the arrow-navigation switch below.
    if (
      event.key === "Backspace" &&
      !event.metaKey &&
      !event.ctrlKey &&
      !event.altKey &&
      activeFilterChips.length > 0 &&
      inputEl !== null &&
      inputEl.selectionStart === 0 &&
      inputEl.selectionEnd === 0
    ) {
      event.preventDefault();
      removeChip(activeFilterChips[activeFilterChips.length - 1]);
      return;
    }

    // Slice 4: ArrowRight (→) also accepts the inline ghost — but ONLY at
    // end-of-input (so → isn't used to move through existing text). Tab accepts
    // at any caret position (above). Only as a plain keypress: Enter/ArrowUp/
    // ArrowDown above keep their meaning, so ghost-text never fights results
    // navigation or the Ask AI pivot. When the gate fails we DON'T
    // preventDefault, letting → do its native cursor move. Handled before the
    // switch so it never preventDefaults unconditionally.
    if (
      event.key === "ArrowRight" &&
      !event.metaKey &&
      !event.ctrlKey &&
      !event.altKey &&
      !event.shiftKey &&
      hasGhost &&
      caretAtEnd
    ) {
      event.preventDefault();
      acceptGhost();
      return;
    }

    // Slice 5: the picker owns navigation while open (handled + returned above),
    // so the roving results-list selection below must NOT also run. The Filter
    // Value List likewise owns the arrows while up, so Home/End and any stray key
    // must not drive the results list underneath it. This guard backstops the
    // early returns in case an unconsumed key reaches here.
    if (pickerOpen || valueListActive) {
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
  //
  // On the very first summon the window is built and made key while its webview
  // is still loading, so the initial focus attempt can fire a frame before the
  // field is interactable. `retriesLeft` re-attempts on the next frames until the
  // active element is actually the target, which makes first-open focus reliable
  // without affecting the already-working reused-window summons.
  function focusActiveField(retriesLeft = 8): void {
    const target: HTMLElement | null | undefined =
      mode === "ask" ? (askSubmitted ? askAreaEl : askInputEl) : inputEl;
    target?.focus();
    // Select any leftover query so typing immediately replaces it.
    if (mode !== "ask") inputEl?.select();

    if (target && document.activeElement !== target && retriesLeft > 0) {
      requestAnimationFrame(() => focusActiveField(retriesLeft - 1));
    }
  }

  // On the first summon the native panel is made key before this webview has
  // loaded, so AppKit never routes keyboard focus into it and `focusActiveField`
  // alone can't recover a caret. Ask the Rust side to make the (now-loaded)
  // webview the panel's first responder, then place the DOM cursor.
  async function focusQuickRecall(): Promise<void> {
    try {
      await invoke("focus_quick_recall_window");
    } catch {
      // Best-effort: non-macOS, or the window/webview isn't ready yet. The
      // retrying focusActiveField below still does its part.
    }
    focusActiveField();
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
  // Slice 7: body-operator syntax help affordance
  //
  // A small `?` trigger in the search field row toggles a static popover that
  // documents the Body Match Operators (`"phrase"`, `-term`, `OR`, `term*`).
  // These operators stay TYPED TEXT — they are not chips and not in the Filter
  // Picker — so this affordance is the place they're discoverable. The field
  // operators (app:/source:/date:) are listed too for completeness since they
  // pair with the picker, but the body operators lead.
  //
  // The content is entirely static: the only state is an open/close boolean.
  // There's no parsing, no derivations, no list — just a documentation panel.
  //
  // Dismissal is threefold and must NOT clobber the surrounding Escape handlers:
  //   - clicking the trigger again toggles it closed;
  //   - an outside pointerdown closes it (the $effect below registers a document
  //     listener only while open, and ignores clicks inside the help wrapper);
  //   - Escape closes it WITHOUT bubbling to the layout's window-close handler.
  //     The Escape branch lives at the very top of handleSearchKeydown (the input
  //     keeps DOM focus), guarded so it ONLY runs while the popover is open — when
  //     closed, Escape falls through to search→close-window exactly as before, and
  //     the picker's Escape (handlePickerKeydown) and Ask-mode Escape
  //     (handleRootKeydown) are untouched.
  // ---------------------------------------------------------------------------

  let syntaxHelpOpen = $state(false);
  // The trigger + popover live in this wrapper; the outside-click listener uses
  // it to distinguish an in-help click (keep open) from an outside click (close).
  let syntaxHelpEl = $state<HTMLDivElement | null>(null);
  const SYNTAX_HELP_POPOVER_ID = "quick-recall-syntax-help";

  function toggleSyntaxHelp(): void {
    syntaxHelpOpen = !syntaxHelpOpen;
    // Keep typing flow on the search input — the help is an occasional-use
    // affordance, so it's toggled by click but never becomes the focus target.
    if (!syntaxHelpOpen) {
      inputEl?.focus();
    }
  }

  function closeSyntaxHelp(): void {
    if (!syntaxHelpOpen) {
      return;
    }
    syntaxHelpOpen = false;
    inputEl?.focus();
  }

  // Outside-pointerdown dismissal: registered only while the popover is open, so
  // there's no global listener cost when it's closed. A pointerdown inside the
  // help wrapper (trigger or popover) is ignored — the trigger's own onclick owns
  // the toggle — so this only fires for genuine outside clicks.
  $effect(() => {
    if (!syntaxHelpOpen) {
      return;
    }
    const onPointerDown = (event: PointerEvent): void => {
      const target = event.target as Node | null;
      if (syntaxHelpEl !== null && target !== null && syntaxHelpEl.contains(target)) {
        return;
      }
      syntaxHelpOpen = false;
    };
    document.addEventListener("pointerdown", onPointerDown, true);
    return () => document.removeEventListener("pointerdown", onPointerDown, true);
  });

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
    // Raw camelCase tool params the agent passed (per the slice-1 backend
    // contract). Shape varies by tool; treated opaquely and narrowed in the
    // formatting helpers below.
    params?: Record<string, unknown>;
  };

  // One recorded brokered tool call: the tool kind plus a humane filter label
  // (e.g. `Searching "invoice" in Safari · Jun 1`). Accumulated as `phase:"tool"`
  // events arrive so the collapsed summary can count kinds and the disclosure
  // can list the individual activities. Fully ephemeral, reset per ask.
  type AskToolKind = "search" | "timeline" | "show_text" | "other";
  type AskToolActivityEntry = {
    kind: AskToolKind;
    label: string;
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

  // One cited answer source — a captured frame or audio segment the agent drew
  // on. Arrives already ordered and capped (6 frame / 4 audio) by the host. The
  // event may fire more than once per conversation; the LAST set replaces prior.
  type AskAiSource = {
    kind: "frame" | "audio";
    frameId: number | null;
    audioSegmentId: number | null;
    appName: string | null;
    windowTitle: string | null;
    startedAt: string;
    endedAt: string;
    sourceKind: "microphone" | "system" | null;
  };

  type AskAiSourceEvent = {
    conversationId: string;
    sources: AskAiSource[];
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
  // Current brokered tool activity label (e.g. `Searching "invoice"`), shown as
  // the live animated working line while the agent gathers context mid-answer.
  let askToolActivity = $state<string | null>(null);
  // Every tool call that has run this ask, in order. Drives the collapsed,
  // expandable summary chip once tokens start streaming. Reset per ask.
  let askToolActivities = $state<AskToolActivityEntry[]>([]);
  // Disclosure toggle for the collapsed summary chip.
  let askSummaryExpanded = $state(false);
  // True between ask_ai_start resolving and a terminal done/error event.
  let askStreaming = $state(false);

  // Cited answer sources for the current ask, buffered as `ask_ai_source` events
  // arrive (the last event replaces the set). Always buffered; the markup gates
  // rendering on askPhase === "done". Split into Screen/Audio for the strip.
  let askSources = $state<AskAiSource[]>([]);
  let askFrameSources = $derived(askSources.filter((s) => s.kind === "frame"));
  let askAudioSources = $derived(askSources.filter((s) => s.kind === "audio"));

  // The seed used for the current/last ask, so an error "Try again" can re-run
  // the exact same question + seed pairing. Set inside startAsk.
  let askLastSeed = $state<string | null>(null);

  // Copy-confirmation flash for the answer copy button (icon swaps to a check).
  let askCopied = $state(false);
  let askCopiedTimer: ReturnType<typeof setTimeout> | null = null;

  // The streamed answer is Markdown; render it to HTML for display. Recomputed
  // on each delta — incomplete Markdown (e.g. an unclosed code fence mid-stream)
  // renders gracefully and resolves once the closing token arrives.
  let askAnswerHtml = $derived(askAnswer.length > 0 ? renderMarkdown(askAnswer) : "");

  // Route link clicks inside the rendered answer through the OS browser instead
  // of navigating the webview. Links are tagged with data-external in markdown.ts.
  function handleAnswerClick(event: MouseEvent): void {
    const anchor = (event.target as HTMLElement | null)?.closest(
      "a[data-external]",
    ) as HTMLAnchorElement | null;
    if (anchor === null) {
      return;
    }
    event.preventDefault();
    const href = anchor.getAttribute("href");
    if (href !== null && href.length > 0) {
      void openUrl(href);
    }
  }

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

  // ---------------------------------------------------------------------------
  // Tool-activity formatting (pure helpers)
  //
  // Turn the raw camelCase tool params from a `phase:"tool"` status event into a
  // single humane line. Dates are short ("Jun 1", "Jun 1–2", "today"/"yesterday")
  // and times only appear for a sub-day (same calendar day) window. These are
  // pure aside from `new Date()` (acceptable for a live UI per the plan).
  // ---------------------------------------------------------------------------

  function readString(params: Record<string, unknown>, key: string): string | null {
    const value = params[key];
    return typeof value === "string" && value.trim().length > 0 ? value.trim() : null;
  }

  // Parse a backend date/datetime string. Tolerates "YYYY-MM-DD HH:MM:SS" (space
  // separator) by normalizing to ISO-ish form, matching SearchResultCard.
  function parseToolDate(value: string): Date | null {
    const normalized = value.includes("T") ? value : value.replace(" ", "T");
    const d = new Date(normalized);
    return isNaN(d.getTime()) ? null : d;
  }

  function isSameCalendarDay(a: Date, b: Date): boolean {
    return (
      a.getFullYear() === b.getFullYear() &&
      a.getMonth() === b.getMonth() &&
      a.getDate() === b.getDate()
    );
  }

  // "today" / "yesterday" relative to `now`, else null.
  function relativeDayWord(d: Date, now: Date): string | null {
    if (isSameCalendarDay(d, now)) return "today";
    const yesterday = new Date(now);
    yesterday.setDate(now.getDate() - 1);
    if (isSameCalendarDay(d, yesterday)) return "yesterday";
    return null;
  }

  function shortDate(d: Date): string {
    return d.toLocaleDateString(undefined, { month: "short", day: "numeric" });
  }

  function shortTime(d: Date): string {
    return d.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
  }

  // A spoken time span on a single day, collapsing a shared meridiem so the
  // AM/PM is written once: "5:50 to 6:30 AM" rather than "5:50 AM to 6:30 AM".
  // Falls back to the full both-sided form when the meridiems differ
  // ("11:50 AM to 1:30 PM") or the locale has no trailing token (24-hour).
  function formatTimeSpan(start: Date, end: Date): string {
    const s = shortTime(start);
    const e = shortTime(end);
    const sParts = s.split(" ");
    const eParts = e.split(" ");
    if (sParts.length === 2 && eParts.length === 2 && sParts[1] === eParts[1]) {
      return `${sParts[0]} to ${e}`;
    }
    return `${s} to ${e}`;
  }

  // Humane label for a from/to window. Handles either bound missing, a single
  // instant, a same-day (sub-day) window with times, or a multi-day range.
  function formatDateRange(
    fromRaw: string | null,
    toRaw: string | null,
    now: Date,
  ): string | null {
    const from = fromRaw ? parseToolDate(fromRaw) : null;
    const to = toRaw ? parseToolDate(toRaw) : null;

    if (from && to) {
      if (isSameCalendarDay(from, to)) {
        // Sub-day window: the day, then a time span (times only when sub-day).
        const day = relativeDayWord(from, now) ?? shortDate(from);
        const start = shortTime(from);
        const end = shortTime(to);
        if (start === end) {
          return `${day} ${start}`;
        }
        return `${day}, ${formatTimeSpan(from, to)}`;
      }
      // Multi-day range: spoken "from X to Y" so mixed relative/absolute tokens
      // ("from today to Jun 3") read as a human range, not a hyphenated word.
      const fromLabel = relativeDayWord(from, now) ?? shortDate(from);
      const toLabel = relativeDayWord(to, now) ?? shortDate(to);
      return `from ${fromLabel} to ${toLabel}`;
    }

    const single = from ?? to;
    if (single) {
      // Relative words read bare ("today"); an absolute day takes "on 3 Jun".
      const word = relativeDayWord(single, now);
      return word ?? `on ${shortDate(single)}`;
    }
    return null;
  }

  // Build the live working-line label for one tool call from its raw params.
  function formatToolActivity(
    tool: string | undefined,
    params: Record<string, unknown> | undefined,
  ): { kind: AskToolKind; label: string } {
    const p = params ?? {};
    const now = new Date();

    if (tool === "search") {
      const queryText = readString(p, "query");
      let label = queryText ? `Searching “${queryText}”` : "Searching your captures";
      const app = readString(p, "app");
      if (app) label += ` in ${app}`;
      const range = formatDateRange(readString(p, "from"), readString(p, "to"), now);
      if (range) label += ` · ${range}`;
      return { kind: "search", label };
    }

    if (tool === "timeline") {
      let label = "Scanning timeline";
      const range = formatDateRange(readString(p, "from"), readString(p, "to"), now);
      if (range) label += ` · ${range}`;
      const app = readString(p, "app");
      if (app) label += ` in ${app}`;
      return { kind: "timeline", label };
    }

    if (tool === "show_text") {
      return { kind: "show_text", label: "Reading a capture" };
    }

    return { kind: "other", label: tool ? `Running ${tool}` : "Working" };
  }

  // Collapsed summary chip text: counts of each tool kind that ran, e.g.
  // `3 searches · timeline · 1 read`. Empty when no tools ran.
  let askActivitySummary = $derived.by(() => {
    if (askToolActivities.length === 0) return null;
    let searches = 0;
    let timelines = 0;
    let reads = 0;
    let others = 0;
    for (const entry of askToolActivities) {
      if (entry.kind === "search") searches += 1;
      else if (entry.kind === "timeline") timelines += 1;
      else if (entry.kind === "show_text") reads += 1;
      else others += 1;
    }
    const parts: string[] = [];
    if (searches > 0) parts.push(`${searches} ${searches === 1 ? "search" : "searches"}`);
    if (timelines > 0)
      parts.push(`${timelines} ${timelines === 1 ? "timeline scan" : "timeline scans"}`);
    if (reads > 0) parts.push(`${reads} ${reads === 1 ? "read" : "reads"}`);
    if (others > 0) parts.push(`${others} ${others === 1 ? "step" : "steps"}`);
    return parts.length > 0 ? parts.join(" · ") : null;
  });

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

    // Normalize and record the seed so an error "Try again" reuses it exactly.
    const normalizedSeed =
      seedQuery && seedQuery.trim().length > 0 ? seedQuery.trim() : null;
    askLastSeed = normalizedSeed;

    const conversationId = crypto.randomUUID();
    askConversationId = conversationId;
    askQuestion = trimmedQuestion;
    askSubmitted = true;
    askPhase = "seeding";
    askAnswer = "";
    askErrorMessage = null;
    askSeededResultCount = null;
    askToolActivity = null;
    askToolActivities = [];
    askSummaryExpanded = false;
    askCopied = false;
    askSources = [];
    askStreaming = true;

    try {
      await invoke<void>("ask_ai_start", {
        request: {
          conversationId,
          question: trimmedQuestion,
          seedQuery: normalizedSeed,
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

    // Slice 5: leaving search mode closes the Filter Picker (search-mode only).
    pickerOpen = false;
    pickerIndex = 0;

    // Slice 6: inherit the active chip scope into the pivot. The SEED is a
    // canonical, parser-exact operator string (re-parsed by the broker search);
    // the QUESTION is the residual free text with a plain-language scope suffix.
    // With no chips these collapse to the raw trimmed query (unchanged behavior):
    // buildScopedSeedQuery → residual === trimmedQuery's free text, and
    // buildScopedQuestion → the residual. We fall back to `trimmedQuery` when the
    // backend hasn't populated `residualQuery` yet (e.g. a query below the parse
    // threshold) so the seed/question are never blank when the user typed text.
    const chips = activeFilterChips;
    const residual =
      chips.length === 0 && residualQuery.trim().length === 0
        ? trimmedQuery
        : residualQuery;
    const seed = buildScopedSeedQuery(chips, residual);
    const question = buildScopedQuestion(residual, chips);
    mode = "ask";

    if (seed.length > 0 || question.length > 0) {
      // Seeded: immediately submit the scoped question, seeded by the scoped
      // operator query. Focus the answer region (no text input renders) so
      // Escape/scroll keys are caught.
      askInput = "";
      void startAsk(question, seed);
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

  // Re-run the failed ask with the exact same question + seed pairing.
  async function retryAsk(): Promise<void> {
    const question = askQuestion;
    if (question.trim().length === 0) {
      return;
    }
    void startAsk(question, askLastSeed);
    await tick();
    askAreaEl?.focus();
  }

  // Copy the raw Markdown answer (not the rendered HTML) to the clipboard, with
  // a brief check-icon confirmation. Only meaningful at askPhase === "done".
  async function copyAnswer(): Promise<void> {
    if (askAnswer.length === 0) {
      return;
    }
    try {
      await navigator.clipboard.writeText(askAnswer);
    } catch {
      // Clipboard write is best-effort; swallow (no toast surface here).
      return;
    }
    askCopied = true;
    if (askCopiedTimer !== null) {
      clearTimeout(askCopiedTimer);
    }
    askCopiedTimer = setTimeout(() => {
      askCopied = false;
      askCopiedTimer = null;
    }, 1500);
  }

  function toggleSummaryExpanded(): void {
    askSummaryExpanded = !askSummaryExpanded;
  }

  // ⌘C / Ctrl+C copies the whole answer when the answer region is focused, the
  // ask is done, and nothing is selected. With a selection, let native copy run.
  function handleAnswerAreaKeydown(event: KeyboardEvent): void {
    if (
      (event.metaKey || event.ctrlKey) &&
      (event.key === "c" || event.key === "C") &&
      askPhase === "done" &&
      askAnswer.length > 0
    ) {
      const selection = window.getSelection()?.toString() ?? "";
      if (selection.length === 0) {
        event.preventDefault();
        void copyAnswer();
      }
    }
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
    askToolActivities = [];
    askSummaryExpanded = false;
    askCopied = false;
    askSources = [];
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

  // Slice 5: keep the highlighted picker option within view as it moves, mirroring
  // the results scroll-into-view effect (the app value list can overflow).
  $effect(() => {
    if (!pickerOpen || pickerItemCount === 0) {
      return;
    }
    document
      .getElementById(`${PICKER_OPT_PREFIX}${pickerIndex}`)
      ?.scrollIntoView({ block: "nearest" });
  });

  // ---------------------------------------------------------------------------
  // Slice 1: active filter chip model + parse error + section-limit derivations
  //
  // These derive purely off `appliedRefinements` / `parseErrors` (the parsed
  // backend scope captured in runSearch). Later slices RENDER these; slice 1
  // only computes them so the wiring is testable. Label formatting is kept in
  // the small pure helpers above each derivation; chips carry enough underlying
  // data (app value/kind, source kind, date start/end) for a later slice to
  // reconstruct the operator syntax for editing/removal.
  // ---------------------------------------------------------------------------

  // One normalized active filter chip. `kind` groups by operator family; `data`
  // is the discriminated source payload a later slice uses to rebuild syntax
  // (e.g. `app:Safari`, `source:microphone`, `after:…`/`before:…`).
  type ActiveFilterChip =
    | {
        id: string;
        kind: "app";
        label: string;
        // The app refinement as the backend desugared it (bundle_id/app_name/any
        // + raw value + human displayName). Slice 2+ rebuilds `app:<value>`.
        data: SearchAppRefinement;
      }
    | {
        id: string;
        kind: "source";
        label: string;
        // "screen" when source:screen, else the audio source kind. Slice 2+
        // rebuilds `source:screen` / `source:microphone` / `source:system`.
        data: { source: "screen" } | { source: AudioSegmentSourceKind };
      }
    | {
        id: string;
        kind: "date";
        label: string;
        // The desugared range (ISO start/end + optional origin). Slice 2+
        // rebuilds the originating `after:`/`before:`/`today`/etc. operator.
        data: SearchDateRangeRefinement;
      };

  // Plain-language label for an audio source kind. Mirrors the spoken phrasing
  // the existing answer-source strip uses ("Microphone audio" / "System audio").
  function audioSourceLabel(kind: AudioSegmentSourceKind): string {
    return kind === "microphone" ? "Microphone audio" : "System audio";
  }

  // Parse a backend date string ("YYYY-MM-DD HH:MM:SS" or ISO) into a Date,
  // reusing the same space→T normalization as the Ask AI tool-activity helpers.
  // We deliberately do NOT re-validate dates here — the backend already parsed
  // the operator; this only formats what it returned. Returns null if unparseable.
  function parseRefinementDate(value: string): Date | null {
    return parseToolDate(value);
  }

  // Plain-language label for a desugared date range, e.g. "May 1 – May 30",
  // "May 1" (single day), or "since May 1" / "until May 30" when one bound is
  // open-ended-ish (start === end is treated as a single day). Falls back to the
  // raw strings if either bound won't parse, so a chip never renders blank.
  function dateRangeLabel(range: SearchDateRangeRefinement): string {
    const start = parseRefinementDate(range.startAt);
    const end = parseRefinementDate(range.endAt);
    if (start && end) {
      if (isSameCalendarDay(start, end)) {
        return shortDate(start);
      }
      return `${shortDate(start)} – ${shortDate(end)}`;
    }
    if (start) {
      return shortDate(start);
    }
    if (end) {
      return shortDate(end);
    }
    return `${range.startAt} – ${range.endAt}`;
  }

  // The normalized active-chip list rendered by later slices. Order is stable:
  // date first (broadest scope), then apps, then sources — so the row reads
  // "when · where · what kind". `screenSource` and `audioSources` are mutually
  // exclusive per the backend contract, so at most one yields source chips.
  let activeFilterChips = $derived.by<ActiveFilterChip[]>(() => {
    const refinements = appliedRefinements;
    if (refinements === null) {
      return [];
    }
    const chips: ActiveFilterChip[] = [];

    if (refinements.dateRange) {
      chips.push({
        id: "date",
        kind: "date",
        label: dateRangeLabel(refinements.dateRange),
        data: refinements.dateRange,
      });
    }

    for (const app of refinements.apps ?? []) {
      chips.push({
        id: `app:${app.kind}:${app.value}`,
        kind: "app",
        label: app.displayName,
        data: app,
      });
    }

    if (refinements.screenSource === true) {
      chips.push({
        id: "source:screen",
        kind: "source",
        label: "Screen",
        data: { source: "screen" },
      });
    }
    for (const source of refinements.audioSources ?? []) {
      chips.push({
        id: `source:${source}`,
        kind: "source",
        label: audioSourceLabel(source),
        data: { source },
      });
    }

    return chips;
  });

  // ---------------------------------------------------------------------------
  // Slice 2: chip removal (strip operator tokens from the raw query)
  //
  // Chips are RENDERED from the backend desugar (appliedRefinements), but the
  // operator TEXT lives in the `query` string. So removing a chip means stripping
  // its operator substring out of `query`; setting `query` re-fires the reactive
  // `$effect(() => scheduleSearch(query))`, the search reruns, and the chips
  // re-derive from the fresh response. We never mutate appliedRefinements by hand.
  //
  // Operator spelling varies (`source:mic` vs `source:microphone`,
  // `date:`/`after:`/`before:`), so we match DEFENSIVELY: drop any whitespace-
  // delimited token whose lowercased text starts with one of the chip's operator
  // prefixes, regardless of the value the user actually typed. A `date` chip
  // clears all of `date:`/`after:`/`before:`; `app`/`source` clear their own
  // prefix. Slices 4/5 reuse `removeChip` as the chip-commit/edit seam.
  // ---------------------------------------------------------------------------

  // The operator prefixes a chip of each kind owns. A token (whitespace-delimited
  // run) is stripped when its lowercased form starts with any of these.
  function operatorPrefixesForChip(chip: ActiveFilterChip): string[] {
    switch (chip.kind) {
      case "date":
        return ["date:", "after:", "before:"];
      case "app":
        return ["app:"];
      case "source":
        return ["source:"];
    }
  }

  // Remove every whitespace-delimited token from `raw` whose lowercased text
  // starts with one of `prefixes`, then collapse the leftover whitespace and
  // trim. Pure: used by removeChip and reusable by later picker-commit slices.
  function stripOperatorTokens(raw: string, prefixes: string[]): string {
    const kept = raw
      .split(/\s+/)
      .filter((token) => token.length > 0)
      .filter((token) => {
        const lower = token.toLowerCase();
        return !prefixes.some((prefix) => lower.startsWith(prefix));
      });
    return kept.join(" ").trim();
  }

  // Drop a chip's operator token(s) from the query and let the reactive effect
  // rerun the search (which re-derives chips and restores sections via
  // sectionLimits). Refocus the input so removal keeps keyboard flow.
  function removeChip(chip: ActiveFilterChip): void {
    query = stripOperatorTokens(query, operatorPrefixesForChip(chip));
    inputEl?.focus();
  }

  // ---------------------------------------------------------------------------
  // Slice 6: Ask AI pivot scope inheritance
  //
  // Pivoting search → ask carries the active chip scope into the ask TWO ways:
  //
  //   1. Structurally, into the SEED. `ask_ai_start`'s `seedQuery` flows to the
  //      Rust broker search, which runs the SAME backend `parse_search_query`
  //      (crates/app-infra `search_capture`, called by `broker_search`). So an
  //      operator-bearing seed is re-parsed and the seed context is scoped to the
  //      chips with no Rust change. We rebuild a CANONICAL operator string from
  //      `activeFilterChips` (the desugared truth) + `residualQuery` rather than
  //      forwarding the raw typed query, so a messy/abbreviated raw query still
  //      yields a clean, parser-exact seed (the single source of truth).
  //
  //   2. In natural language, into the QUESTION. The read-only question header
  //      shows `askQuestion`; appending a spoken scope suffix ("in Safari from
  //      May 1 to May 30") makes the scope legible to both the user and the
  //      agent. The question's free-text base is `residualQuery` (operators
  //      stripped) so it reads cleanly; body operators that survive into the
  //      residual (quoted phrase, `-term`, `OR`, `term*`) stay verbatim.
  //
  // Both builders are pure (chips + residual in, string out) so they're easy to
  // reason about and reuse. Dropping a chip needs no extra wiring here: removeChip
  // rewrites `query` → the search reruns → chips/residual re-derive, so a later
  // pivot naturally reflects the reduced scope from both outputs.
  // ---------------------------------------------------------------------------

  // Quote an operator value when it contains whitespace (or is empty) so the
  // backend tokenizer keeps it as one token, e.g. `app:"Google Chrome"`. Bare
  // single-word values pass through unquoted (`app:Safari`, `app:com.apple.Safari`).
  function quoteOperatorValue(value: string): string {
    return /\s/.test(value) || value.length === 0 ? `"${value}"` : value;
  }

  // Format a Date as a local `YYYY-MM-DD` day, the form the backend
  // `after:`/`before:` parser accepts (resolve_point_date). Uses local calendar
  // fields (not toISOString, which would shift across the UTC boundary).
  function toOperatorDay(d: Date): string {
    const year = d.getFullYear().toString().padStart(4, "0");
    const month = (d.getMonth() + 1).toString().padStart(2, "0");
    const day = d.getDate().toString().padStart(2, "0");
    return `${year}-${month}-${day}`;
  }

  // The canonical operator token(s) one chip contributes to a reconstructed seed.
  // Mirrors the parser spellings: `app:<value>` (quoted as needed),
  // `source:screen`/`source:microphone`/`source:system_audio` (the audio kind is
  // already a parser-accepted word), and `after:<day> before:<day>` for a range
  // (or a single `after:<day>`/`before:<day>` when only one bound parses).
  function operatorTokensForChip(chip: ActiveFilterChip): string {
    switch (chip.kind) {
      case "app":
        return `app:${quoteOperatorValue(chip.data.value)}`;
      case "source":
        return `source:${chip.data.source}`;
      case "date": {
        const start = parseRefinementDate(chip.data.startAt);
        const end = parseRefinementDate(chip.data.endAt);
        const parts: string[] = [];
        if (start) parts.push(`after:${toOperatorDay(start)}`);
        if (end) parts.push(`before:${toOperatorDay(end)}`);
        // If neither bound parses we emit nothing (the chip's structural scope is
        // unrecoverable as an operator); the natural-language suffix still carries it.
        return parts.join(" ");
      }
    }
  }

  // Build a parser-exact seed query from the active chips + residual free text,
  // e.g. chips `{app:Safari, date 5/1–5/30}` + residual `deploy error` →
  // `app:Safari after:2026-05-01 before:2026-05-30 deploy error`. With no chips
  // this is just the residual, so the seed is unchanged from today.
  function buildScopedSeedQuery(
    chips: ActiveFilterChip[],
    residual: string,
  ): string {
    const operatorTokens = chips
      .map((chip) => operatorTokensForChip(chip))
      .filter((token) => token.length > 0);
    const residualText = residual.trim();
    const parts = [...operatorTokens];
    if (residualText.length > 0) {
      parts.push(residualText);
    }
    return parts.join(" ").trim();
  }

  // The plain-language scope suffix for one chip: `in Safari`,
  // `in microphone audio` / `in system audio`, `in screen captures`, or a date
  // window phrased like the existing labels (`from May 1 to May 30`, `on May 1`).
  function scopeSuffixForChip(chip: ActiveFilterChip): string {
    switch (chip.kind) {
      case "app":
        return `in ${chip.data.displayName}`;
      case "source":
        if (chip.data.source === "screen") return "in screen captures";
        return chip.data.source === "microphone"
          ? "in microphone audio"
          : "in system audio";
      case "date": {
        const start = parseRefinementDate(chip.data.startAt);
        const end = parseRefinementDate(chip.data.endAt);
        if (start && end) {
          if (isSameCalendarDay(start, end)) {
            return `on ${shortDate(start)}`;
          }
          return `from ${shortDate(start)} to ${shortDate(end)}`;
        }
        if (start) return `since ${shortDate(start)}`;
        if (end) return `until ${shortDate(end)}`;
        // Unparseable bounds: fall back to the chip's already-formatted label.
        return chip.label;
      }
    }
  }

  // Build the natural-language question from the residual free text + chips, e.g.
  // residual `deploy error` + chips `{app:Safari, date 5/1–5/30}` →
  // `deploy error in Safari from May 1 to May 30`. When the residual is empty
  // (the query was only operators) we lead with a neutral "Show me everything"
  // so the suffix reads as a sentence ("Show me everything in Safari") rather
  // than a bare fragment. With no chips this is just the residual (unchanged).
  function buildScopedQuestion(
    residual: string,
    chips: ActiveFilterChip[],
  ): string {
    const suffixes = chips
      .map((chip) => scopeSuffixForChip(chip))
      .filter((suffix) => suffix.length > 0);
    const residualText = residual.trim();
    if (suffixes.length === 0) {
      return residualText;
    }
    const base = residualText.length > 0 ? residualText : "Show me everything";
    return [base, ...suffixes].join(" ").trim();
  }

  // ---------------------------------------------------------------------------
  // Slice 5: Quick Recall Filter Picker (ADR 0025, path B)
  //
  // A launcher-native overlay that REPLACES the results region and, while open,
  // FULLY OWNS Arrow / Enter / Escape so exactly one list consumes arrows at any
  // instant. It is a CATEGORY DOOR ONLY: it lists App / Source / Date, and
  // selecting one writes that operator's stub (`app:` / `source:` / `date:`) into
  // the raw `query` and immediately hands off to the Filter Value List — there is
  // no in-picker drilling and no second navigable list. The value list owns ALL
  // value selection (the same surface typing the operator directly reaches), so
  // arrow navigation lives in exactly one place per surface, never duplicated in
  // the door. DOM focus stays on the search input (same aria-activedescendant
  // pattern as the results listbox).
  //
  // Summoning:
  //   - `/` on an EMPTY input opens the picker (handled in handleSearchKeydown).
  //     We preventDefault so no `/` is inserted; since the trigger only fires on
  //     an empty input, a slash-leading literal search like `/usr/local/bin`
  //     stays fully typeable (once the input is non-empty, `/` is a literal
  //     character and never triggers). The ADR's "Escape leaves a literal slash"
  //     is therefore satisfied structurally: the literal-slash path is exactly
  //     the non-empty-input case, where `/` is never a trigger, so closing the
  //     picker with Escape can't have eaten a slash the user meant literally.
  //   - Ctrl+F / Cmd+F opens the picker from anywhere (empty or not).
  //
  // Structure: three plain-language categories. Selecting App / Source / Date
  // injects its operator stub and the Filter Value List takes over (apps from the
  // distinct-captured-apps catalog, three fixed source rows, or date presets —
  // custom date ranges are typed as `after:`/`before:`, no calendar/From-To UI).
  // ---------------------------------------------------------------------------

  // Whether the picker overlay is open (replacing the results region).
  //
  // The picker is a thin CATEGORY DOOR only: it lists App / Source / Date and,
  // on selection, writes the operator stub (`app:` / `source:` / `date:`) into
  // the query and hands off to the Filter Value List — which owns ALL value
  // navigation and selection. There is no in-picker drilling, no second arrow
  // surface, and no From/To field pair; the value list is the single surface
  // that consumes ↑/↓/Enter for values (reached identically by typing the
  // operator directly). This keeps exactly one navigable value list at any time.
  let pickerOpen = $state(false);
  // The highlighted category index within the root category list.
  let pickerIndex = $state(0);
  // Bound to the picker overlay so we can move DOM focus there on open (the
  // search input keeps focus via aria-activedescendant; this is the listbox the
  // input's keydown drives, and the date inputs live inside it).
  let pickerEl = $state<HTMLDivElement | null>(null);

  // The three categories, in display order. `level` is the operator family the
  // category commits (its stub is written and the value list takes over).
  const PICKER_CATEGORIES = [
    { id: "app", label: "App", hint: "Narrow to one captured app", level: "app" as const },
    {
      id: "source",
      label: "Source",
      hint: "Microphone, system audio, or screen",
      level: "source" as const,
    },
    {
      id: "date",
      label: "Date range",
      hint: "A day, a preset, or a from/to span",
      level: "date" as const,
    },
  ];

  // The three fixed Source values and the operator each commits.
  const PICKER_SOURCES = [
    { id: "mic", label: "Microphone audio", token: "source:mic" },
    { id: "system", label: "System audio", token: "source:system" },
    { id: "screen", label: "Screen", token: "source:screen" },
  ];

  // Date presets. The backend parser (crates/app-infra/src/search.rs) accepts
  // `date:today` / `date:yesterday` as named-period day spans, and relative
  // point tokens `Nd` for after:/before: — so "Last 7/30 days" commit `after:7d`
  // / `after:30d` (an open-ended "since N days ago" window) rather than computing
  // absolute dates. Verified against resolve_day_or_period / resolve_point_date.
  const PICKER_DATE_PRESETS = [
    { id: "today", label: "Today", token: "date:today" },
    { id: "yesterday", label: "Yesterday", token: "date:yesterday" },
    { id: "last7", label: "Last 7 days", token: "after:7d" },
    { id: "last30", label: "Last 30 days", token: "after:30d" },
  ];

  // The App value list: captured-app names from `searchableApps`, in recency
  // order. The backend collapses by app IDENTITY (bundle id, else name), so two
  // distinct bundle ids that share a display name — common when the same app
  // ships under more than one bundle (e.g. a dev and a release build) — return
  // two rows with the same name. We dedupe by name (case-insensitive, first/most-
  // recent wins) so the value list never renders two identical rows: a duplicate
  // key in the `{#each}` would reconcile to a stale node and swallow the first
  // click/enter. The value commits `app:<name>`, so collapsing same-name rows is
  // also semantically correct (both would scope to the same name regardless).
  let pickerAppNames = $derived.by<string[]>(() => {
    const apps = searchableApps;
    if (apps === null) {
      return [];
    }
    const seen = new Set<string>();
    const names: string[] = [];
    for (const app of apps) {
      const name = (app.name ?? "").trim();
      if (name.length === 0) {
        continue;
      }
      const key = name.toLowerCase();
      if (seen.has(key)) {
        continue;
      }
      seen.add(key);
      names.push(name);
    }
    return names;
  });

  // The number of arrow-navigable categories, used to clamp pickerIndex on
  // ArrowUp/ArrowDown. The picker is category-only now, so this is constant.
  let pickerItemCount = $derived(PICKER_CATEGORIES.length);

  // The active option id for aria-activedescendant on the search input while the
  // picker is open, mirroring the results listbox's activeOptionId pattern.
  const PICKER_OPT_PREFIX = "qr-picker-opt-";
  let pickerActiveOptionId = $derived(
    pickerOpen && pickerItemCount > 0
      ? `${PICKER_OPT_PREFIX}${pickerIndex}`
      : undefined,
  );

  // Quote an app name that contains whitespace so `app:"Google Chrome"` parses as
  // one token; a single-word name is emitted bare (`app:Safari`).
  function appOperatorToken(name: string): string {
    return name.includes(" ") ? `app:"${name}"` : `app:${name}`;
  }

  // Open the picker fresh at the category list, resetting the highlight. DOM
  // focus stays on the search input (its keydown owns picker navigation while
  // open), so we refocus it rather than the overlay. The captured-app catalog is
  // warmed here so the App value list has selectable rows the instant the App
  // category is chosen (otherwise its first row would arrive after the lazy
  // load, swallowing the first Enter/click — the source/date lists are static).
  function openPicker(): void {
    pickerOpen = true;
    pickerIndex = 0;
    void ensureSearchableAppsLoaded();
    void tick().then(() => inputEl?.focus());
  }

  // Close the picker and return DOM focus to the search input so the search (now
  // carrying any just-appended operator) reruns and the chip appears.
  function closePicker(): void {
    pickerOpen = false;
    pickerIndex = 0;
    void tick().then(() => inputEl?.focus());
  }

  // Selecting a category writes its operator STUB (`app:` / `source:` / `date:`)
  // into the query and closes the picker. The caret then sits in an un-committed
  // operator value, so the Filter Value List opens and shows that operator's
  // values (apps / sources / date presets) — the SAME surface the typed path
  // reaches. This converges the category door with the typed path: one value
  // list, reached two ways. No trailing space is added so the empty value keeps
  // the full list up; the user arrows or types to filter, then commits a value.
  function commitOperatorStub(level: "app" | "source" | "date"): void {
    const stub = level === "app" ? "app:" : level === "source" ? "source:" : "date:";
    const base = query.trimEnd();
    query = base.length > 0 ? `${base} ${stub}` : stub;
    // Close the category door and open the value list in the SAME reactive flush:
    // `caretAtEnd` is set synchronously (we know the caret lands at end) so
    // `activeOperatorContext` becomes non-null the instant `pickerOpen` flips
    // false — the surface swaps in one step with no empty flash and no dependence
    // on focus/tick timing (WebKit focus restore is unreliable in this webview).
    pickerOpen = false;
    pickerIndex = 0;
    caretAtEnd = true;
    // Warm the app catalog so the App value list has selectable rows immediately.
    if (level === "app") {
      void ensureSearchableAppsLoaded();
    }
    void tick().then(() => {
      const el = inputEl;
      if (el !== null) {
        el.focus();
        const end = el.value.length;
        el.setSelectionRange(end, end);
        caretAtEnd = true;
      }
    });
  }

  // Commit the highlighted category: write its operator stub (`app:`/`source:`/
  // `date:`) into the query and hand off to the Filter Value List. The picker is
  // a category door only — it never selects values itself, so this is its single
  // commit path.
  function pickerSelectHighlighted(): void {
    const category = PICKER_CATEGORIES[pickerIndex];
    if (category) {
      commitOperatorStub(category.level);
    }
  }

  // Move the category highlight, wrapping at the ends.
  function pickerMove(delta: number): void {
    const count = pickerItemCount;
    if (count === 0) {
      return;
    }
    pickerIndex = (pickerIndex + delta + count) % count;
  }

  // Picker key ownership: while the picker is open, it consumes Arrow/Enter/
  // Escape/Tab before the normal results navigation runs (handleSearchKeydown
  // calls this first and returns early when it handles the event). Returns true
  // when the event was consumed. Tab is ALWAYS suppressed while open so the Ask
  // AI pivot can't fire mid-picker (no ambiguous owner of the keys). The picker
  // is a flat category list, so ↑/↓ move and Enter/→ select a category (which
  // injects its operator stub and closes the picker); Escape closes it.
  function handlePickerKeydown(event: KeyboardEvent): boolean {
    if (!pickerOpen || event.isComposing) {
      return false;
    }

    // Tab never pivots to Ask AI while the picker owns the keys.
    if (event.key === "Tab") {
      event.preventDefault();
      return true;
    }

    switch (event.key) {
      case "Escape":
        event.preventDefault();
        event.stopPropagation();
        closePicker();
        return true;
      case "Enter":
      case "ArrowRight":
        event.preventDefault();
        pickerSelectHighlighted();
        return true;
      case "ArrowDown":
        event.preventDefault();
        pickerMove(1);
        return true;
      case "ArrowUp":
        event.preventDefault();
        pickerMove(-1);
        return true;
    }

    return false;
  }

  // ---------------------------------------------------------------------------
  // Slice 4: inline ghost-text autocomplete (ADR 0025, path A)
  //
  // Ambient dimmed completion of known Field Operator names and their two
  // enumerable value vocabularies, shown trailing the caret. It NEVER consumes
  // navigation keys — the accept is bound to ArrowRight at end-of-input ONLY
  // (handled in handleSearchKeydown), so Enter/Tab/ArrowUp/ArrowDown keep their
  // existing meaning. The ghost is a pure $derived off `query` + caretAtEnd +
  // the lazily-loaded app catalog; it renders as an aria-hidden overlay mirror
  // that aligns the typed text (transparent) with the dimmed suffix.
  //
  // Completes:
  //   - operator NAMES: app: / source: / date: / after: / before:
  //   - source: VALUES: mic / system / screen (canonical short spellings the
  //     parser accepts — see split_field_operator in app-infra/src/search.rs,
  //     where "mic"|"microphone", "system"|"system_audio", "screen" are valid)
  //   - app: VALUES: from list_searchable_apps (case-insensitive on name);
  //     a completion containing a space inserts the quoted form (app:"Name").
  //   - date:/after:/before: VALUES are NOT completed (free-form, no vocab).
  // ---------------------------------------------------------------------------

  // The canonical operator names we complete. Order matters only for first match.
  const GHOST_OPERATORS = ["app:", "source:", "date:", "after:", "before:"] as const;
  // The canonical source: values we complete (short spellings the parser accepts).
  const GHOST_SOURCE_VALUES = ["mic", "system", "screen"] as const;

  // Lazily-loaded distinct captured apps, cached for the session. Mirrors the
  // dashboard's pattern; a transient failure leaves this null so the next
  // partial retries (ghost just won't complete app values until it loads).
  let searchableApps = $state<SearchableApp[] | null>(null);
  let searchableAppsLoading = $state(false);

  // Whether the input caret is collapsed at the very end of `query`. The ghost
  // only ever shows (and only ever accepts) at end-of-input, so we track this
  // rather than a full caret position. Updated on every input/keyup/click/select.
  let caretAtEnd = $state(true);

  async function ensureSearchableAppsLoaded(): Promise<void> {
    if (searchableApps !== null || searchableAppsLoading) {
      return;
    }
    searchableAppsLoading = true;
    try {
      searchableApps = await invoke<SearchableApp[]>("list_searchable_apps");
    } catch {
      // Leave `searchableApps` null (not an empty list) so a transient failure
      // is retried on the next `app:` partial rather than disabling completion.
    } finally {
      searchableAppsLoading = false;
    }
  }

  // Recompute whether the caret sits at end-of-input. Cheap; called from the
  // input event handlers so the ghost derivation only fires when relevant.
  function updateCaretAtEnd(): void {
    const el = inputEl;
    if (el === null) {
      caretAtEnd = true;
      return;
    }
    caretAtEnd =
      el.selectionStart === el.value.length && el.selectionEnd === el.value.length;
  }

  // The trailing whitespace-delimited token of `query` (the partial the user is
  // typing at the caret). Empty when `query` ends in whitespace.
  function trailingToken(value: string): string {
    const match = value.match(/(\S+)$/);
    return match ? match[1] : "";
  }

  // Whether the trailing token of `raw` is an un-committed field-operator value
  // (`app:…`/`source:…`/`date:…`/`after:…`/`before:…`). Pure; used both as the
  // backend gate (a half-typed value never reaches `search_capture`) and as the
  // basis for the Filter Value List context below. Because `trailingToken`
  // returns "" once `raw` ends in whitespace, a committed `app:Safari ` is NOT a
  // partial → false, so committing/abandoning the value re-opens the backend.
  function isTrailingOperatorPartial(raw: string): boolean {
    return /^(app|source|date|after|before):/i.test(trailingToken(raw));
  }

  // Compute the ghost SUFFIX for a trailing operator-value partial, or null. The
  // partial is everything after the first ":" of the trailing token. For app:
  // values we may need quoting, so we return the suffix that, appended to the
  // already-typed partial, yields a valid token — quoting the WHOLE value when
  // the completion contains a space (so we re-emit the value with quotes).
  function ghostForValue(
    operator: string,
    typedValue: string,
  ): string | null {
    if (operator === "source:") {
      const lower = typedValue.toLowerCase();
      if (lower.length === 0) {
        return null;
      }
      for (const value of GHOST_SOURCE_VALUES) {
        if (value.startsWith(lower) && value.length > lower.length) {
          // Source values never contain spaces; plain suffix.
          return value.slice(typedValue.length);
        }
      }
      return null;
    }

    if (operator === "app:") {
      // Don't try to complete an already-quoted partial (the user is steering
      // the quoting themselves); keep it simple per the plan.
      if (typedValue.startsWith('"')) {
        return null;
      }
      const apps = searchableApps;
      if (apps === null) {
        // Kick off the lazy load so the next partial can complete; no ghost yet.
        void ensureSearchableAppsLoaded();
        return null;
      }
      const lower = typedValue.toLowerCase();
      if (lower.length === 0) {
        return null;
      }
      for (const app of apps) {
        const name = (app.name ?? "").trim();
        if (name.length === 0) {
          continue;
        }
        if (name.toLowerCase().startsWith(lower) && name.length > typedValue.length) {
          if (name.includes(" ")) {
            // Completing to a name with a space: re-emit the whole value quoted,
            // so the trailing token becomes app:"Full Name". The suffix replaces
            // the unquoted partial entirely (the accept handler swaps the token).
            return `"${name}" `;
          }
          return name.slice(typedValue.length);
        }
      }
      return null;
    }

    return null;
  }

  // The active ghost completion (the dimmed SUFFIX to append), or null. Only
  // shown when the caret is at end-of-input. Derives the operator/value tier
  // from the trailing token of `query`. A trailing app-value completion that
  // requires quoting is encoded with a sentinel NUL (see ghostForValue) and is
  // resolved separately at accept time; for DISPLAY we strip the sentinel.
  let ghostRaw = $derived.by<string | null>(() => {
    if (!caretAtEnd) {
      return null;
    }
    const token = trailingToken(query);
    if (token.length === 0) {
      return null;
    }

    const colon = token.indexOf(":");
    if (colon === -1) {
      // No colon yet → completing an operator NAME from a partial at the token
      // start. Only complete a bare alphabetic partial (not e.g. "-app").
      if (!/^[a-z]+$/i.test(token)) {
        return null;
      }
      const lower = token.toLowerCase();
      for (const op of GHOST_OPERATORS) {
        if (op.startsWith(lower) && op.length > lower.length) {
          return op.slice(token.length);
        }
      }
      return null;
    }

    // Has a colon → completing an operator VALUE. Only the enumerable vocabs.
    const operator = token.slice(0, colon + 1).toLowerCase();
    const typedValue = token.slice(colon + 1);
    return ghostForValue(operator, typedValue);
  });

  // Whether the active ghost is a quoted app-value replacement (sentinel form).
  let ghostIsQuotedAppValue = $derived(ghostRaw !== null && ghostRaw.endsWith(" "));

  // The display suffix (dimmed text shown after the typed text). For the quoted
  // app-value case we still want the overlay to read sensibly: the typed partial
  // gets visually replaced, so we show the remaining characters of the quoted
  // name. We render the closing-quote-stripped suffix relative to what's typed.
  let ghostCompletion = $derived.by<string | null>(() => {
    if (ghostRaw === null) {
      return null;
    }
    if (ghostIsQuotedAppValue) {
      // Sentinel-encoded: the replacement is `"Full Name"` for the whole value.
      // For display, show the tail beyond the already-typed partial characters.
      const token = trailingToken(query);
      const colon = token.indexOf(":");
      const typedValue = colon === -1 ? "" : token.slice(colon + 1);
      const quoted = ghostRaw.slice(0, -1); // strip sentinel → `"Full Name"`
      // The opening quote sits before the typed partial; the visible ghost is
      // the closing portion after the typed characters plus the closing quote.
      // Simplest faithful render: show the quoted form minus the leading `"` and
      // the already-typed partial, i.e. the remainder of the name + closing `"`.
      const inner = quoted.slice(1, -1); // Full Name
      if (!inner.toLowerCase().startsWith(typedValue.toLowerCase())) {
        return null;
      }
      return `${inner.slice(typedValue.length)}"`;
    }
    return ghostRaw;
  });

  // True when a ghost completion is currently shown (drives the accept gate).
  // Slice 5: suppressed while the Filter Picker is open so the dimmed completion
  // doesn't paint behind the overlay and ArrowRight stays owned by the picker.
  let hasGhost = $derived(
    !pickerOpen && ghostCompletion !== null && ghostCompletion.length > 0,
  );

  // Accept the active ghost: mutate `query` to include the completion, move the
  // caret to the end, and clear the ghost (it re-derives empty). Operator-NAME
  // accepts leave the caret ready for a value (no trailing space). Value accepts
  // complete a full operator token and add a trailing space so typing continues.
  // Returns true when something was accepted.
  function acceptGhost(): boolean {
    if (ghostRaw === null) {
      return false;
    }
    const token = trailingToken(query);
    const colon = token.indexOf(":");

    if (ghostIsQuotedAppValue) {
      // Replace the trailing unquoted `app:partial` token with `app:"Full Name"`.
      const operator = token.slice(0, colon + 1); // preserves typed case of key
      const quoted = ghostRaw.slice(0, -1); // `"Full Name"`
      const replaced = query.slice(0, query.length - token.length) + operator + quoted;
      query = `${replaced} `;
    } else if (colon === -1) {
      // Operator NAME accept: append the suffix; no trailing space (value next).
      query = query + ghostRaw;
    } else {
      // Operator VALUE accept (source: or unquoted app:): append + trailing space.
      query = query + ghostRaw + " ";
    }

    // Move the caret to the very end after the value commits.
    void tick().then(() => {
      const el = inputEl;
      if (el !== null) {
        const end = el.value.length;
        el.setSelectionRange(end, end);
        caretAtEnd = true;
      }
    });
    return true;
  }

  // ---------------------------------------------------------------------------
  // Slice 4 (typed path): the Filter Value List
  //
  // When the caret sits in an UN-COMMITTED field-operator value — the trailing
  // token of `query` is `app:…`/`source:…`/`date:…`/`after:…`/`before:…`, the
  // caret is at end-of-input, and the picker is closed — the results region is
  // REPLACED by a value list for that operator. It fully owns ↑/↓/Enter/Escape
  // while up. This is the keyboard-native sibling of the Filter Picker: the same
  // value vocabularies, the same operator-token commit seam (rewrite `query` →
  // the search reruns → the chip derives from the backend), reusing the picker's
  // CSS so it looks identical to a drilled picker value list.
  //
  // It's mutually exclusive with the picker by construction: activeOperatorContext
  // returns null while `pickerOpen`, so exactly one of the two surfaces is ever up.
  // ---------------------------------------------------------------------------

  // The active operator + the value typed so far, or null when the value list
  // shouldn't be up. Null while the picker is open or the caret isn't at end (the
  // ghost/value affordances only ever act at end-of-input). The operator key is
  // lowercased and colon-suffixed (`app:`, `source:`, `date:`, `after:`, `before:`).
  let activeOperatorContext = $derived.by<
    | {
        operator: "app:" | "source:" | "date:" | "after:" | "before:";
        typedValue: string;
      }
    | null
  >(() => {
    if (pickerOpen || !caretAtEnd) {
      return null;
    }
    const match = trailingToken(query).match(/^(app|source|date|after|before):(.*)$/i);
    if (match === null) {
      return null;
    }
    const operator = `${match[1].toLowerCase()}:` as
      | "app:"
      | "source:"
      | "date:"
      | "after:"
      | "before:";
    return { operator, typedValue: match[2] };
  });

  // Whether the Filter Value List currently owns the results region + the arrows.
  let valueListActive = $derived(activeOperatorContext !== null);

  // The rows for the active operator's value list. App names filter as a SUBSTRING
  // match on the typed value; source values filter on canonical value/label; date
  // operators always show the four presets (date values are free-form, so we
  // never filter them by the typed text). `disabled` marks rows that would create
  // a structural conflict with an already-active chip (app+audio source are
  // mutually exclusive at the operator level), so committing them is blocked.
  let valueListRows = $derived.by<
    Array<{ id: string; label: string; token: string; disabled: boolean }>
  >(() => {
    const context = activeOperatorContext;
    if (context === null) {
      return [];
    }
    const hasAppChip = activeFilterChips.some((c) => c.kind === "app");
    const hasAudioSourceChip = activeFilterChips.some(
      (c) => c.kind === "source" && c.data.source !== "screen",
    );
    const typed = context.typedValue.toLowerCase();

    if (context.operator === "app:") {
      return pickerAppNames
        .filter((name) => typed.length === 0 || name.toLowerCase().includes(typed))
        .map((name) => ({
          id: `app:${name}`,
          label: name,
          token: appOperatorToken(name),
          disabled: hasAudioSourceChip,
        }));
    }

    if (context.operator === "source:") {
      return PICKER_SOURCES.filter((source) => {
        if (typed.length === 0) {
          return true;
        }
        // The canonical value is the token's text after `source:` (e.g. `mic`).
        const value = source.token.slice("source:".length).toLowerCase();
        return value.includes(typed) || source.label.toLowerCase().includes(typed);
      }).map((source) => ({
        id: `source:${source.id}`,
        label: source.label,
        token: source.token,
        // The Screen row is never disabled; mic/system clash with an app chip.
        disabled: source.id !== "screen" && hasAppChip,
      }));
    }

    // date: / after: / before: — always the four presets, never filtered.
    return PICKER_DATE_PRESETS.map((preset) => ({
      id: `date:${preset.id}`,
      label: preset.label,
      token: preset.token,
      disabled: false,
    }));
  });

  // A single one-line note shown below the value list when the active operator
  // structurally conflicts with an already-active chip (mirrors the backend's
  // app_source_conflict). Null when there's no conflict.
  let valueListConflictReason = $derived<string | null>(
    activeOperatorContext?.operator === "source:" &&
      activeFilterChips.some((c) => c.kind === "app")
      ? "Audio has no app — remove the app filter to search audio"
      : activeOperatorContext?.operator === "app:" &&
          activeFilterChips.some(
            (c) => c.kind === "source" && c.data.source !== "screen",
          )
        ? "Audio has no app — remove the audio filter to scope by app"
        : null,
  );

  // The empty-state line for the `app:` operator only (source/date always have
  // rows). Distinguishes "still loading", "nothing captured", and "no match" so
  // the surface never renders a blank list.
  let valueListEmptyMessage = $derived.by<string | null>(() => {
    if (activeOperatorContext?.operator !== "app:") {
      return null;
    }
    if (searchableApps === null && searchableAppsLoading) {
      return "Loading apps…";
    }
    if (pickerAppNames.length === 0) {
      return "No apps captured yet";
    }
    if (valueListRows.length === 0) {
      return "No matching app";
    }
    return null;
  });

  // The highlighted row index within the value list. Kept pointed at an ENABLED
  // row by the effect below; -1 means no enabled row exists (so Enter is a no-op).
  let valueListIndex = $state(0);

  // Keep `valueListIndex` on an enabled row. Re-runs when the operator changes or
  // the row set changes (filtering as the user types): if the current index is
  // out of range or its row is disabled, snap to the first enabled row, or -1 if
  // none are selectable. Keyed on the operator + rows so a fresh operator resets.
  $effect(() => {
    if (!valueListActive) {
      return;
    }
    // Touch the operator so an operator switch re-evaluates the highlight.
    void activeOperatorContext?.operator;
    const rows = valueListRows;
    const current = rows[valueListIndex];
    if (current !== undefined && !current.disabled) {
      return;
    }
    const firstEnabled = rows.findIndex((row) => !row.disabled);
    valueListIndex = firstEnabled;
  });

  // Move the highlight among ENABLED rows only, wrapping at the ends. A no-op when
  // no row is selectable (e.g. every row conflicts with an active chip).
  function moveValueListSelection(delta: number): void {
    const rows = valueListRows;
    const enabled = rows
      .map((row, index) => ({ row, index }))
      .filter((entry) => !entry.row.disabled);
    if (enabled.length === 0) {
      return;
    }
    const currentPos = enabled.findIndex((entry) => entry.index === valueListIndex);
    // From an unset/disabled position a forward move lands on the first enabled
    // row, a backward move on the last — same wrap idiom as moveSelection.
    const base = currentPos < 0 ? (delta > 0 ? -1 : 0) : currentPos;
    const nextPos = (base + delta + enabled.length) % enabled.length;
    valueListIndex = enabled[nextPos].index;
  }

  // Commit a value: REPLACE the trailing partial operator token in `query` with
  // the full operator token + a trailing space. The trailing space empties the
  // trailing token → valueListActive flips false → the reactive search effect
  // runs with the committed operator → the chip derives from the backend response.
  // Mirrors how acceptGhost finalizes the caret at end-of-input.
  async function commitValueListRow(token: string): Promise<void> {
    const t = trailingToken(query);
    const base = query.slice(0, query.length - t.length);
    query = `${base}${token} `;
    await tick();
    const el = inputEl;
    if (el !== null) {
      const end = el.value.length;
      el.setSelectionRange(end, end);
      caretAtEnd = true;
    }
    inputEl?.focus();
  }

  // Commit the highlighted enabled row (Enter). Real-values-only: if nothing is
  // highlighted or the highlighted row is disabled, this is a NO-OP — there's no
  // phantom commit of a half-typed value the parser wouldn't accept.
  function commitHighlightedValueListRow(): void {
    if (valueListIndex < 0) {
      return;
    }
    const row = valueListRows[valueListIndex];
    if (row === undefined || row.disabled) {
      return;
    }
    void commitValueListRow(row.token);
  }

  // Escape out of an un-committed operator: strip the trailing partial operator
  // token from `query` (and any trailing whitespace it leaves), then refocus. The
  // search reruns on the residual, so the prior results return cleanly.
  function abandonOperator(): void {
    const t = trailingToken(query);
    query = query.slice(0, query.length - t.length).replace(/\s+$/, "");
    inputEl?.focus();
  }

  // Load the captured-app catalog as soon as the App value list opens, so its
  // rows populate (mirrors the picker's lazy load on drilling into App).
  $effect(() => {
    if (activeOperatorContext?.operator === "app:") {
      void ensureSearchableAppsLoaded();
    }
  });

  // The active option id for aria-activedescendant on the search input while the
  // value list is up, mirroring the results/picker activeOptionId pattern.
  const VALUE_LIST_OPT_PREFIX = "qr-vl-opt-";
  let valueListActiveOptionId = $derived(
    valueListActive && valueListIndex >= 0
      ? `${VALUE_LIST_OPT_PREFIX}${valueListIndex}`
      : undefined,
  );

  // The first parse error (or null). Slice 3 renders ONE inline error line from
  // this; its presence is also what tells the results region the backend paused
  // results rather than finding none.
  let firstParseError = $derived<SearchParseError | null>(parseErrors[0] ?? null);

  // ---------------------------------------------------------------------------
  // Slice 3: friendly parse-error message (pure helper)
  //
  // The backend parse messages are accurate but terse/technical ("…must be a
  // valid RFC3339 timestamp", "windowTitle must be non-empty", "OR needs a
  // search term on both sides"). This maps the known `kind` values to plain
  // language, interpolating the offending `token` where it sharpens the hint
  // (e.g. `"notadate" isn't a date I understand`). Any unmapped kind falls back
  // to the raw backend `message`, so a new backend error never renders blank.
  // Only the FIRST parse error is ever shown, so this only formats one.
  // The `parseErrorMessage` derivation that consumes this lives below, after
  // `belowMinimum` is declared (it gates on it).
  // ---------------------------------------------------------------------------
  function friendlyParseError(err: SearchParseError): string {
    const token = err.token.trim();
    switch (err.kind) {
      case "bad_date":
        return token.length > 0
          ? `“${token}” isn't a date I understand. Try a day like 2024-05-01, or today / yesterday.`
          : "That date filter isn't one I understand. Try a day like 2024-05-01, or today / yesterday.";
      case "unknown_source":
        return token.length > 0
          ? `“${token}” isn't a source I know. Use source:mic, source:system, or source:screen.`
          : "Use source:mic, source:system, or source:screen.";
      case "unbalanced_quote":
        return "There's an unclosed quote in your search — add the matching closing quote.";
      case "empty_value":
        return "That filter is missing a value — add a name after the colon.";
      case "app_source_conflict":
        return "app: and source: can't be combined — app: narrows the screen, source: narrows audio.";
      case "screen_audio_source_conflict":
        return "source:screen can't be combined with source:mic or source:system.";
      case "dangling_or":
        return "OR needs a search term on both sides.";
      case "pure_negation":
        return "An exclusion like -term needs at least one positive term to match.";
      default:
        // Unknown backend kind: surface its message rather than render blank.
        return err.message;
    }
  }

  // Dynamic per-section fetch limits derived from the active source/app scope,
  // replacing the old hardcoded frameLimit:5 / audioLimit:5. Read inside
  // runSearch (optimistically — off the PREVIOUS response's scope; see the note
  // there). Rules:
  //   - any audio source active        → audio only  (frame 0 / audio 5)
  //   - source:screen OR any app chip   → screen only (audio 0 / frame 5)
  //   - otherwise                       → both (5 / 5, today's behavior)
  // Audio sources and screen/app scope are mutually exclusive at the operator
  // level, but if both ever appear we prefer audio-only (the audio branch wins).
  let sectionLimits = $derived.by<{ frameLimit: number; audioLimit: number }>(() => {
    const refinements = appliedRefinements;
    const hasAudioSource = (refinements?.audioSources?.length ?? 0) > 0;
    if (hasAudioSource) {
      return { frameLimit: 0, audioLimit: 5 };
    }
    const screenOnly =
      refinements?.screenSource === true || (refinements?.apps?.length ?? 0) > 0;
    if (screenOnly) {
      return { frameLimit: 5, audioLimit: 0 };
    }
    return { frameLimit: 5, audioLimit: 5 };
  });

  let trimmedQuery = $derived(query.trim());
  let belowMinimum = $derived(trimmedQuery.length < MIN_QUERY_LENGTH);
  let hasResults = $derived(frames.length > 0 || audio.length > 0);

  // Slice 3: the friendly line for the active parse error (or null when none).
  // Drives both the inline error line under the input and the paused-results
  // branch; kept off `belowMinimum` so a parse error never shows for a query
  // that's too short to have run.
  let parseErrorMessage = $derived<string | null>(
    firstParseError !== null && !belowMinimum ? friendlyParseError(firstParseError) : null,
  );

  // Slice 3: the bare "No matches" empty state must NOT show when the backend
  // paused results for a malformed filter — that reads as "found nothing" rather
  // than "your filter is broken". Gate it on `parseErrorMessage === null` so the
  // dedicated paused-results branch owns the parse-error case instead.
  let showEmpty = $derived(
    !belowMinimum &&
      !loading &&
      !errorMessage &&
      parseErrorMessage === null &&
      !hasResults &&
      resultsQuery.length > 0,
  );

  // Slice 3: results are PAUSED (not empty, not errored) when the backend
  // returned a parse error for an at/above-minimum query that isn't mid-flight.
  // The backend suppresses results in this case; this branch renders a calm
  // "fix the filter" state in place of stale cards or the bare empty state.
  let resultsPaused = $derived(
    !belowMinimum && !loading && !errorMessage && parseErrorMessage !== null,
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
    // Slice 1: reset parsed scope to pristine so a re-summon starts with no
    // chips / parse errors / scope-narrowed limits.
    appliedRefinements = null;
    residualQuery = "";
    parseErrors = [];
    // Slice 5: a fresh summon starts with the Filter Picker closed.
    pickerOpen = false;
    pickerIndex = 0;
    askConversationId = null;
    askQuestion = "";
    askInput = "";
    askSubmitted = false;
    askPhase = "seeding";
    askAnswer = "";
    askErrorMessage = null;
    askSeededResultCount = null;
    askToolActivity = null;
    askToolActivities = [];
    askSummaryExpanded = false;
    askCopied = false;
    askSources = [];
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

  // ---------------------------------------------------------------------------
  // Launcher sub-surface keys via a WINDOW CAPTURE listener (focus-independent)
  //
  // DOM focus is unreliable in this WKWebView (the same reason the app uses
  // window capture-phase keydown listeners elsewhere instead of element
  // onkeydown). So while the Filter Picker or Filter Value List is up we must NOT
  // rely on the search input keeping focus to own Escape/Arrow/Enter. If focus
  // drifts off the input, a plain Escape would otherwise reach the layout's
  // bubble-phase `dismissQuickRecallOnEscape` and close the ENTIRE Quick Recall
  // window rather than just the open sub-surface (the reported bug).
  //
  // This runs in the CAPTURE phase (before the layout's bubble handler and
  // regardless of focus). While a sub-surface is open it owns Escape/Arrow/Enter,
  // calling the same helpers the input-level handlers use, and stops propagation
  // so neither handleSearchKeydown nor the layout window-close runs for those
  // keys. Every other key (typing, Tab/ghost, Ctrl+Enter when no value list) is
  // left untouched so the focused input still handles it normally. When nothing
  // is open this does nothing, so a plain-search Escape still closes the window.
  function handleLauncherCaptureKeydown(event: KeyboardEvent): void {
    if (event.isComposing || mode !== "search") {
      return;
    }
    const plain =
      !event.metaKey && !event.ctrlKey && !event.altKey && !event.shiftKey;

    // Syntax-help popover closes on a plain Escape.
    if (syntaxHelpOpen && event.key === "Escape" && plain) {
      event.preventDefault();
      event.stopPropagation();
      closeSyntaxHelp();
      return;
    }

    if (pickerOpen) {
      switch (event.key) {
        case "Escape":
          if (!plain) return;
          event.preventDefault();
          event.stopPropagation();
          closePicker();
          return;
        case "ArrowDown":
          event.preventDefault();
          event.stopPropagation();
          pickerMove(1);
          return;
        case "ArrowUp":
          event.preventDefault();
          event.stopPropagation();
          pickerMove(-1);
          return;
        case "Enter":
          event.preventDefault();
          event.stopPropagation();
          pickerSelectHighlighted();
          return;
      }
      return;
    }

    if (valueListActive) {
      switch (event.key) {
        case "Escape":
          if (!plain) return;
          event.preventDefault();
          event.stopPropagation();
          abandonOperator();
          return;
        case "ArrowDown":
          event.preventDefault();
          event.stopPropagation();
          moveValueListSelection(1);
          return;
        case "ArrowUp":
          event.preventDefault();
          event.stopPropagation();
          moveValueListSelection(-1);
          return;
        case "Enter":
          // Plain Enter commits the highlighted row; Ctrl/Cmd+Enter is suppressed
          // (no Ask AI pivot while the value list is up).
          event.preventDefault();
          event.stopPropagation();
          if (!event.metaKey && !event.ctrlKey) {
            commitHighlightedValueListRow();
          }
          return;
      }
      return;
    }
  }

  onMount(() => {
    void focusQuickRecall();
    void loadAskAvailability();
    // Warm the captured-app catalog up front so the App value list (whether
    // reached by typing `app:` or via the picker) has selectable rows on first
    // open — the source/date lists are static, so only App needs the head start.
    void ensureSearchableAppsLoaded();

    // Focus-independent key ownership for the picker / value list (see above).
    window.addEventListener("keydown", handleLauncherCaptureKeydown, {
      capture: true,
    });

    // Track the reduced-motion preference for the JS-driven mode-switch fade.
    const motionQuery = window.matchMedia("(prefers-reduced-motion: reduce)");
    prefersReducedMotion = motionQuery.matches;
    const onMotionChange = (e: MediaQueryListEvent) => {
      prefersReducedMotion = e.matches;
    };
    motionQuery.addEventListener("change", onMotionChange);

    let destroyed = false;
    let unlistenStatus: (() => void) | undefined;
    let unlistenDelta: (() => void) | undefined;
    let unlistenDone: (() => void) | undefined;
    let unlistenError: (() => void) | undefined;
    let unlistenSource: (() => void) | undefined;
    let unlistenFocus: (() => void) | undefined;

    // The window is hidden/re-shown rather than recreated across summons, so
    // re-grab focus each time it becomes key — onMount alone fires only once.
    getCurrentWindow()
      .onFocusChanged(({ payload: focused }) => {
        windowFocused = focused;
        if (focused) {
          void tick().then(() => focusQuickRecall());
        }
      })
      .then((fn) => {
        if (destroyed) fn();
        else unlistenFocus = fn;
      });

    listen<AskAiStatusEvent>("ask_ai_status", (event) => {
      if (event.payload.conversationId !== askConversationId) return;
      // A "tool" status is mid-answer activity: surface the real filter label
      // without touching askPhase, so any already-streamed answer text stays
      // visible, and record it for the collapsed summary chip.
      if (event.payload.phase === "tool") {
        const activity = formatToolActivity(event.payload.tool, event.payload.params);
        askToolActivity = activity.label;
        askToolActivities = [...askToolActivities, activity];
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

    listen<AskAiSourceEvent>("ask_ai_source", (event) => {
      if (event.payload.conversationId !== askConversationId) return;
      // Buffer always; the last event replaces the prior set. The markup gates
      // rendering on askPhase === "done", so a mid-stream set still arrives now.
      askSources = event.payload.sources;
      void loadSourceThumbnails(event.payload.sources);
    }).then((fn) => {
      if (destroyed) fn();
      else unlistenSource = fn;
    });

    return () => {
      destroyed = true;
      window.removeEventListener("keydown", handleLauncherCaptureKeydown, {
        capture: true,
      });
      motionQuery.removeEventListener("change", onMotionChange);
      unlistenStatus?.();
      unlistenDelta?.();
      unlistenDone?.();
      unlistenError?.();
      unlistenSource?.();
      unlistenFocus?.();
    };
  });

  onDestroy(() => {
    clearDebounce();
    clearIdleTimer();
    if (askCopiedTimer !== null) {
      clearTimeout(askCopiedTimer);
      askCopiedTimer = null;
    }
    void cancelActiveAsk();
  });
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="quick-recall" onkeydown={handleRootKeydown}>
  <!-- The search↔Ask AI swap is the one hero transition: a brief cross-fade
       between the two mode subtrees, gated to instant when reduced-motion is on
       (modeFadeMs → 0). Each panel fills the mode area so they overlap cleanly
       during the fade without reflowing the fixed frame. -->
  <div class="quick-recall__mode">
    {#if mode === "search"}
      <div
        class="quick-recall__panel"
        in:fade={{ duration: modeFadeMs }}
        out:fade={{ duration: modeFadeMs }}
      >
        <div class="quick-recall__field">
          <span class="quick-recall__glyph" aria-hidden="true">⌕</span>
          <!-- Slice 4: input + ghost overlay. The real <input> stays the focus
               target; the absolutely-positioned mirror behind it renders the
               typed text invisibly then the dimmed ghost suffix, matching the
               input's font metrics/padding so the ghost aligns under the caret.
               The mirror is aria-hidden and pointer-events:none so it never
               steals interaction or screen-reader attention. -->
          <div class="quick-recall__input-wrap">
            {#if hasGhost}
              <div class="quick-recall__ghost" aria-hidden="true">
                <span class="quick-recall__ghost-typed">{query}</span><span
                  class="quick-recall__ghost-suffix">{ghostCompletion}</span
                >
              </div>
            {/if}
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
              aria-expanded={pickerOpen || resultCount > 0}
              aria-controls={pickerOpen
                ? "quick-recall-picker"
                : "quick-recall-results-list"}
              aria-activedescendant={pickerOpen
                ? pickerActiveOptionId
                : valueListActive
                  ? valueListActiveOptionId
                  : activeOptionId}
              onkeydown={handleSearchKeydown}
              oninput={updateCaretAtEnd}
              onkeyup={updateCaretAtEnd}
              onclick={updateCaretAtEnd}
              onselect={updateCaretAtEnd}
              onfocus={() => {
                updateCaretAtEnd();
                void ensureSearchableAppsLoaded();
              }}
            />
          </div>
          <!-- Slice 7: syntax-help affordance. A quiet `?` trigger toggling a
               static popover that documents the typed Body Match Operators (and,
               for completeness, the field operators that pair with the picker).
               The wrapper is the positioning context for the popover and the
               outside-click anchor (a pointerdown inside it keeps the popover
               open). The button is occasional-use and deliberately doesn't keep
               focus — DOM focus stays on the search input. -->
          <div class="quick-recall__syntax" bind:this={syntaxHelpEl}>
            <button
              type="button"
              class="quick-recall__syntax-trigger"
              onclick={toggleSyntaxHelp}
              aria-label="Search syntax help"
              aria-expanded={syntaxHelpOpen}
              aria-controls={SYNTAX_HELP_POPOVER_ID}
            >
              ?
            </button>
            {#if syntaxHelpOpen}
              <div
                id={SYNTAX_HELP_POPOVER_ID}
                class="quick-recall__syntax-popover"
                role="tooltip"
                in:fade={{ duration: modeFadeMs }}
              >
                <p class="quick-recall__syntax-heading">Refine the words you type</p>
                <dl class="quick-recall__syntax-list">
                  <div class="quick-recall__syntax-row">
                    <dt><code>"phrase"</code></dt>
                    <dd>Match an exact phrase</dd>
                  </div>
                  <div class="quick-recall__syntax-row">
                    <dt><code>-term</code></dt>
                    <dd>Exclude a term</dd>
                  </div>
                  <div class="quick-recall__syntax-row">
                    <dt><code>cat OR dog</code></dt>
                    <dd>Match either side</dd>
                  </div>
                  <div class="quick-recall__syntax-row">
                    <dt><code>term*</code></dt>
                    <dd>Prefix / wildcard match</dd>
                  </div>
                </dl>
                <p class="quick-recall__syntax-heading quick-recall__syntax-heading--secondary">
                  Or narrow the scope
                </p>
                <dl class="quick-recall__syntax-list">
                  <div class="quick-recall__syntax-row">
                    <dt><code>app:</code></dt>
                    <dd>One captured app</dd>
                  </div>
                  <div class="quick-recall__syntax-row">
                    <dt><code>source:</code></dt>
                    <dd>Screen, mic, or system audio</dd>
                  </div>
                  <div class="quick-recall__syntax-row">
                    <dt><code>date: · after: · before:</code></dt>
                    <dd>A day or a date range</dd>
                  </div>
                </dl>
              </div>
            {/if}
          </div>
          {#if askAvailable}
            <button
              type="button"
              class="quick-recall__ask-button"
              onclick={() => void activateAskAi()}
              aria-label="Ask AI"
            >
              Ask AI <span class="quick-recall__ask-key" aria-hidden="true">⌃↵</span>
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

        <!-- Slice 2: active filter chip band. A thin row under the search input
             rendering each applied refinement (from the backend desugar) as a
             plain-language pill with an × that strips its operator token(s) from
             the query. This band shares its vertical slot with the inline parse
             error line (added in slice 3); a chip and a live error never apply to
             the same token, so they can coexist here. Only rendered when at least
             one chip is active. -->
        {#if activeFilterChips.length > 0}
          <div class="quick-recall__chips" role="list" aria-label="Active filters">
            {#each activeFilterChips as chip (chip.id)}
              <span class="quick-recall__chip" role="listitem">
                <span class="quick-recall__chip-label">{chip.label}</span>
                <button
                  type="button"
                  class="quick-recall__chip-remove"
                  onclick={() => removeChip(chip)}
                  aria-label={`Remove ${chip.label} filter`}
                  title={`Remove ${chip.label} filter`}
                >
                  ×
                </button>
              </span>
            {/each}
          </div>
        {/if}

        <!-- Slice 3: inline parse-error line. When the backend reports a
             malformed operator it ALSO suppresses results (paused), so we surface
             ONE friendly line here — in the same band slot as the chips above (a
             chip and a live error never apply to the same token, so they coexist
             cleanly). Only the first error is shown. The Ask AI pivot stays
             reachable: a malformed filter never blocks a natural-language ask. -->
        {#if parseErrorMessage !== null}
          <p class="quick-recall__parse-error" role="alert">{parseErrorMessage}</p>
        {/if}

        {#if askUnavailableHint}
          <p class="quick-recall__ask-hint">{askUnavailableHint}</p>
        {/if}

        {#if pickerOpen}
          <!-- Slice 5: Filter Picker overlay. Replaces the results region while
               open. DOM focus stays on the search input (above); this listbox is
               navigated via aria-activedescendant (pickerActiveOptionId), the
               same pattern as the results listbox. The input's keydown routes
               Arrow/Enter/Escape/Tab here through handlePickerKeydown. -->
          <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
          <div
            bind:this={pickerEl}
            id="quick-recall-picker"
            class="quick-recall__results quick-recall__picker"
            role="listbox"
            tabindex="-1"
            aria-label="Filter picker"
            onkeydown={(event) => {
              // The category door owns Arrow/Enter/Escape/Tab while open; route
              // them through the shared handler (also called from the input's
              // keydown and the window capture listener for focus resilience).
              if (handlePickerKeydown(event)) {
                return;
              }
            }}
          >
            <!-- Category door header: the picker only lists categories; selecting
                 one writes its operator stub and hands off to the value list. -->
            <div class="quick-recall__picker-header">
              <span class="quick-recall__picker-title">Filters</span>
              <span class="quick-recall__orient-cue-dot" aria-hidden="true">·</span>
              <span class="quick-recall__picker-crumb-hint">pick a category</span>
            </div>

            <div class="quick-recall__picker-list" role="presentation">
              {#each PICKER_CATEGORIES as category, i (category.id)}
                <!-- svelte-ignore a11y_click_events_have_key_events -->
                <div
                  id={`${PICKER_OPT_PREFIX}${i}`}
                  class="quick-recall__picker-item"
                  class:quick-recall__picker-item--selected={pickerIndex === i}
                  role="option"
                  tabindex="-1"
                  aria-selected={pickerIndex === i}
                  onmousemove={() => (pickerIndex = i)}
                  onclick={() => {
                    pickerIndex = i;
                    pickerSelectHighlighted();
                  }}
                >
                  <span class="quick-recall__picker-item-label">{category.label}</span>
                  <span class="quick-recall__picker-item-hint">{category.hint}</span>
                  <span class="quick-recall__picker-item-chevron" aria-hidden="true">›</span>
                </div>
              {/each}
            </div>

            <p class="quick-recall__picker-cue" aria-hidden="true">
              <kbd>↑</kbd><kbd>↓</kbd> move · <kbd>↵</kbd> select · <kbd>esc</kbd> close
            </p>
          </div>
        {:else if valueListActive}
          <!-- Slice 4 (typed path): the Filter Value List. Replaces the results
               region while the caret sits in an un-committed field-operator value.
               Reuses the picker's CSS so it reads as a drilled picker value list:
               a header naming the operator, the value rows (or an app-only empty
               line), an optional conflict note, a typed-date hint for date
               operators, and the shared move/select/back cue. DOM focus stays on
               the search input; this listbox is driven via aria-activedescendant
               (valueListActiveOptionId), the same pattern as the results/picker
               lists, and the input's keydown owns Arrow/Enter/Escape while up. -->
          <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
          <div
            class="quick-recall__results quick-recall__picker"
            role="listbox"
            aria-label="Filter values"
          >
            <div class="quick-recall__picker-header">
              <span class="quick-recall__picker-title">
                {activeOperatorContext?.operator === "app:"
                  ? "App"
                  : activeOperatorContext?.operator === "source:"
                    ? "Source"
                    : "Date range"}
              </span>
            </div>

            {#if valueListEmptyMessage !== null}
              <p class="quick-recall__state">{valueListEmptyMessage}</p>
            {:else}
              <div class="quick-recall__picker-list" role="presentation">
                {#each valueListRows as row, i (row.id)}
                  <!-- svelte-ignore a11y_click_events_have_key_events -->
                  <div
                    id={`${VALUE_LIST_OPT_PREFIX}${i}`}
                    class="quick-recall__picker-item"
                    class:quick-recall__picker-item--selected={valueListIndex === i &&
                      !row.disabled}
                    class:quick-recall__picker-item--disabled={row.disabled}
                    role="option"
                    tabindex="-1"
                    aria-selected={valueListIndex === i && !row.disabled}
                    aria-disabled={row.disabled}
                    onmousemove={() => {
                      if (!row.disabled) valueListIndex = i;
                    }}
                    onclick={() => {
                      if (!row.disabled) void commitValueListRow(row.token);
                    }}
                  >
                    <span class="quick-recall__picker-item-label">{row.label}</span>
                  </div>
                {/each}
              </div>
            {/if}

            {#if valueListConflictReason !== null}
              <p class="quick-recall__picker-conflict">{valueListConflictReason}</p>
            {/if}

            {#if activeOperatorContext?.operator === "date:" || activeOperatorContext?.operator === "after:" || activeOperatorContext?.operator === "before:"}
              <p class="quick-recall__picker-hint">
                Type a date like <code>after:2026-05-01</code> for a custom range.
              </p>
            {/if}

            <p class="quick-recall__picker-cue" aria-hidden="true">
              <kbd>↑</kbd><kbd>↓</kbd> move · <kbd>↵</kbd> select · <kbd>esc</kbd> back
            </p>
          </div>
        {:else}
          <div
            id="quick-recall-results-list"
            class="quick-recall__results"
            role="listbox"
            aria-label="Search results"
          >
            {#if belowMinimum}
            <!-- Slice 4: feature-teaching orientation view for the pristine /
                 short-query state. No clickable canned queries — calm cues only. -->
            <div class="quick-recall__orient">
              <span class="quick-recall__orient-mark" aria-hidden="true">⌕</span>
              <p class="quick-recall__orient-tagline">
                Search everything you've captured.
              </p>
              <div class="quick-recall__orient-cues">
                <span class="quick-recall__orient-cue">Screen</span>
                <span class="quick-recall__orient-cue-dot" aria-hidden="true">·</span>
                <span class="quick-recall__orient-cue">Audio</span>
                <span class="quick-recall__orient-cue-dot" aria-hidden="true">·</span>
                <span class="quick-recall__orient-cue">Ask AI</span>
              </div>
              <p class="quick-recall__orient-hint">
                Type to find a moment{askAvailable ? ", or press " : "."}{#if askAvailable}<kbd
                    >⌃↵</kbd
                  > to ask AI.{/if}
              </p>
            </div>
          {:else if loading}
            <!-- Slice 6: skeleton rows mirroring SearchResultCard's two-column
                 layout (116px 16/10 thumb + stacked text lines). -->
            <div class="quick-recall__skeletons" aria-hidden="true">
              {#each [0, 1, 2] as row (row)}
                <div class="quick-recall__skeleton-row">
                  <div class="quick-recall__skeleton-thumb"></div>
                  <div class="quick-recall__skeleton-body">
                    <span
                      class="quick-recall__skeleton-line quick-recall__skeleton-line--title"
                    ></span>
                    <span class="quick-recall__skeleton-line"></span>
                    <span
                      class="quick-recall__skeleton-line quick-recall__skeleton-line--short"
                    ></span>
                  </div>
                </div>
              {/each}
            </div>
          {:else if errorMessage}
            <p class="quick-recall__state quick-recall__state--error">{errorMessage}</p>
          {:else if resultsPaused}
            <!-- Slice 3: paused-results state. The backend suppressed results for
                 a malformed filter, so we render neither stale cards nor the bare
                 "No matches" empty state — instead a calm line pointing back at
                 the inline error above. This branch precedes showEmpty / the
                 normal results branch so a parse error always wins here. Ask AI
                 stays reachable (Tab / footer hint), so the question path is open
                 even while search results are paused. -->
            <p class="quick-recall__state">
              Results paused — fix the filter above to search.
            </p>
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
        {/if}
      </div>
    {:else}
      <div
        class="quick-recall__panel"
        in:fade={{ duration: modeFadeMs }}
        out:fade={{ duration: modeFadeMs }}
      >
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
          onkeydown={handleAnswerAreaKeydown}
        >
          {#if !askSubmitted}
            <p class="quick-recall__state">Type a question and press Enter to ask.</p>
          {:else if askPhase === "error"}
            <p class="quick-recall__state quick-recall__state--error">
              {askErrorMessage ?? "Ask AI failed."}
            </p>
            <div class="quick-recall__retry-row">
              <button
                type="button"
                class="quick-recall__retry"
                onclick={() => void retryAsk()}
              >
                Try again
              </button>
            </div>
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
                <!-- Copy button: only on done, never while streaming/seeding/error. -->
                {#if askPhase === "done" && askAnswer.length > 0}
                  <button
                    type="button"
                    class="quick-recall__copy"
                    class:quick-recall__copy--copied={askCopied}
                    onclick={() => void copyAnswer()}
                    aria-label="Copy answer"
                    title="Copy answer"
                  >
                    {#if askCopied}
                      <svg
                        width="14"
                        height="14"
                        viewBox="0 0 14 14"
                        fill="none"
                        stroke="currentColor"
                        stroke-width="1.4"
                        stroke-linecap="round"
                        stroke-linejoin="round"
                        aria-hidden="true"
                      >
                        <path d="M2.5 7.5 6 11l5.5-7" />
                      </svg>
                    {:else}
                      <svg
                        width="14"
                        height="14"
                        viewBox="0 0 14 14"
                        fill="none"
                        stroke="currentColor"
                        stroke-width="1.2"
                        stroke-linecap="round"
                        stroke-linejoin="round"
                        aria-hidden="true"
                      >
                        <rect x="4.5" y="4.5" width="7" height="7" rx="1.4" />
                        <path d="M9.5 4.5V3a1 1 0 0 0-1-1h-5a1 1 0 0 0-1 1v5a1 1 0 0 0 1 1H4" />
                      </svg>
                    {/if}
                  </button>
                {/if}

                <!-- Collapsed, expandable activity summary chip (replaces the
                     verbose live line once tokens stream). -->
                {#if askActivitySummary !== null}
                  <div class="quick-recall__activity">
                    <button
                      type="button"
                      class="quick-recall__activity-chip"
                      aria-expanded={askSummaryExpanded}
                      onclick={toggleSummaryExpanded}
                    >
                      <span
                        class="quick-recall__activity-caret"
                        class:quick-recall__activity-caret--open={askSummaryExpanded}
                        aria-hidden="true">▸</span
                      >
                      <span class="quick-recall__activity-summary">{askActivitySummary}</span>
                    </button>
                    {#if askSummaryExpanded}
                      <ul class="quick-recall__activity-list">
                        {#each askToolActivities as activity, i (i)}
                          <li class="quick-recall__activity-item">{activity.label}</li>
                        {/each}
                      </ul>
                    {/if}
                  </div>
                {/if}

                <!-- Click delegation for rendered links; the <a> elements carry their
                     own keyboard semantics (Enter dispatches a click that bubbles here). -->
                <!-- svelte-ignore a11y_no_static_element_interactions -->
                <!-- svelte-ignore a11y_click_events_have_key_events -->
                <div
                  bind:this={askAnswerEl}
                  class="quick-recall__answer"
                  class:quick-recall__answer--streaming={askStreaming}
                  onclick={handleAnswerClick}
                >{@html askAnswerHtml}</div>

                <!-- Answer sources: the captured frames/audio the agent drew on,
                     surfaced only once the answer is done. Mirrors the search
                     Screen/Audio split but as horizontally-scrolling card rows. -->
                {#if askPhase === "done" && askSources.length > 0}
                  <div class="quick-recall__sources">
                    <span class="quick-recall__sources-heading">Sources</span>
                    {#if askFrameSources.length > 0}
                      <div class="quick-recall__section" role="presentation">
                        <span class="quick-recall__section-label">Screen</span>
                        <div class="quick-recall__source-row" role="presentation">
                          {#each askFrameSources as s, i (`${s.kind}-${s.frameId}-${s.audioSegmentId}-${s.startedAt}-${i}`)}
                            <AnswerSourceCard
                              kind="frame"
                              appName={s.appName}
                              windowTitle={s.windowTitle}
                              startedAt={s.startedAt}
                              endedAt={s.endedAt}
                              thumbnailUrl={s.frameId != null
                                ? (thumbnailCache.get(s.frameId) ?? null)
                                : null}
                              onselect={() => void selectSource(s)}
                            />
                          {/each}
                        </div>
                      </div>
                    {/if}

                    {#if askAudioSources.length > 0}
                      <div class="quick-recall__section" role="presentation">
                        <span class="quick-recall__section-label">Audio</span>
                        <div class="quick-recall__source-row" role="presentation">
                          {#each askAudioSources as s, i (`${s.kind}-${s.frameId}-${s.audioSegmentId}-${s.startedAt}-${i}`)}
                            <AnswerSourceCard
                              kind="audio"
                              appName={s.appName}
                              windowTitle={s.windowTitle}
                              startedAt={s.startedAt}
                              endedAt={s.endedAt}
                              sourceKind={s.sourceKind}
                              onselect={() => void selectSource(s)}
                            />
                          {/each}
                        </div>
                      </div>
                    {/if}
                  </div>
                {/if}
              {/if}
              {#if askToolActivity !== null}
                <!-- Live animated working line: the real tool filter string. -->
                <p class="quick-recall__state quick-recall__state--working">
                  <span class="quick-recall__dot" aria-hidden="true"></span>
                  <span class="quick-recall__working-label">{askToolActivity}</span>
                </p>
              {/if}
            {/if}
          {/if}
        </div>
      </div>
    {/if}
  </div>

  <div class="quick-recall__footer" aria-hidden="true">
    {#if mode === "search"}
      {#if pickerOpen}
        <!-- Slice 5: while the picker owns the keys, the footer reflects its own
             navigation contract (no Ask AI pivot — Tab is suppressed). -->
        <span class="quick-recall__hint-item"><kbd>↑</kbd><kbd>↓</kbd> move</span>
        <span class="quick-recall__hint-item"><kbd>↵</kbd> select</span>
        <span class="quick-recall__hint-item"><kbd>esc</kbd> close</span>
      {:else if resultCount > 0}
        <span class="quick-recall__hint-item"><kbd>↑</kbd><kbd>↓</kbd> navigate</span>
        <span class="quick-recall__hint-item"><kbd>↵</kbd> open</span>
        {#if askAvailable}
          <span class="quick-recall__hint-item"><kbd>⌃↵</kbd> Ask AI</span>
        {/if}
        <span class="quick-recall__hint-item"><kbd>esc</kbd> close</span>
      {:else}
        {#if askAvailable}
          <span class="quick-recall__hint-item"><kbd>⌃↵</kbd> Ask AI</span>
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

  /* Mode area: positioning context for the cross-fading panels. flex-1 so it
     fills the space between the (absent) header and the footer. */
  .quick-recall__mode {
    position: relative;
    flex: 1;
    min-height: 0;
    display: flex;
  }

  /* Each mode subtree fills the mode area absolutely so that, during the hero
     cross-fade, the incoming and outgoing panels overlap perfectly without
     reflowing the fixed frame. The mode area has a concrete height (flex-1 in
     the fixed 420px frame), so absolute fill is well-defined. */
  .quick-recall__panel {
    position: absolute;
    inset: 0;
    min-height: 0;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }

  .quick-recall__field {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 15px;
    flex-shrink: 0;
    border-bottom: 1px solid var(--app-border);
  }

  .quick-recall__glyph {
    font-size: 16px;
    line-height: 1;
    color: var(--app-text-muted);
    flex-shrink: 0;
    transform: rotate(-45deg);
  }

  /* Slice 4: wrapper that establishes the positioning context for the ghost
     overlay. Takes the flex slot the input used to own; the input fills it. */
  .quick-recall__input-wrap {
    position: relative;
    flex: 1;
    min-width: 0;
    display: flex;
  }

  .quick-recall__input {
    flex: 1;
    min-width: 0;
    border: none;
    outline: none;
    background: transparent;
    color: var(--app-text-strong);
    font-family: inherit;
    font-size: 14px;
    line-height: 1.4;
    padding: 0;
    caret-color: var(--app-accent);
    /* Sits above the ghost mirror so the real caret/text are what's interacted
       with; the mirror only paints the dimmed suffix behind it. */
    position: relative;
    z-index: 1;
  }

  .quick-recall__input::placeholder {
    color: var(--app-text-subtle);
  }

  /* Slice 4: ghost-text mirror. Overlays the input exactly, rendering the typed
     text transparently (so the dimmed suffix lines up after it) and the
     completion suffix in a muted color. Static dimmed text — no animation, so
     reduced-motion needs no special-casing. Single-line, clipped, non-wrapping
     to mirror the input's own overflow behavior. */
  .quick-recall__ghost {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    pointer-events: none;
    font-family: inherit;
    font-size: 14px;
    line-height: 1.4;
    white-space: pre;
    overflow: hidden;
    z-index: 0;
  }

  .quick-recall__ghost-typed {
    /* Invisible spacer that reserves the exact width of the typed text so the
       suffix begins right where the caret is. */
    color: transparent;
  }

  .quick-recall__ghost-suffix {
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
    font-size: 12px;
    line-height: 1.5;
    color: var(--app-text-muted);
  }

  .quick-recall__state--error {
    color: var(--app-accent);
  }

  /* Slice 4: feature-teaching orientation view shown pre-query (belowMinimum).
     Centered in the results area so the empty frame reads as deliberate. */
  .quick-recall__orient {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 14px;
    text-align: center;
    padding: 8px 24px 18px;
  }

  .quick-recall__orient-mark {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 40px;
    height: 40px;
    font-size: 20px;
    line-height: 1;
    color: var(--app-text-muted);
    background: var(--app-bg);
    border: 1px solid var(--app-border);
    border-radius: 11px;
    transform: rotate(-45deg);
  }

  .quick-recall__orient-tagline {
    margin: 0;
    font-size: 13.5px;
    line-height: 1.4;
    color: var(--app-text-strong);
  }

  .quick-recall__orient-cues {
    display: inline-flex;
    align-items: center;
    gap: 9px;
  }

  .quick-recall__orient-cue {
    font-size: 11px;
    line-height: 1;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--app-text-subtle);
    background: var(--app-surface-subtle);
    border: 1px solid var(--app-border);
    border-radius: 6px;
    padding: 6px 10px;
  }

  .quick-recall__orient-cue-dot {
    color: var(--app-text-subtle);
    font-size: 11px;
    line-height: 1;
  }

  .quick-recall__orient-hint {
    margin: 0;
    font-size: 11.5px;
    line-height: 1.5;
    color: var(--app-text-subtle);
  }

  .quick-recall__orient-hint kbd {
    font-family: inherit;
    font-size: 10px;
    line-height: 1;
    color: var(--app-text-muted);
    background: var(--app-surface);
    border: 1px solid var(--app-border);
    border-radius: 5px;
    padding: 2px 5px;
    margin: 0 1px;
  }

  /* Slice 6: loading skeleton rows that mirror SearchResultCard's two-column
     layout — a reserved 116px / 16:10 thumb block plus stacked shimmer lines —
     so the transition from skeleton to real cards doesn't jump. */
  .quick-recall__skeletons {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .quick-recall__skeleton-row {
    display: flex;
    gap: 11px;
    align-items: stretch;
    padding: 6px 9px;
    border: 1px solid var(--app-border);
    border-radius: 8px;
    background: var(--app-surface-subtle);
  }

  .quick-recall__skeleton-thumb {
    flex-shrink: 0;
    width: 96px;
    aspect-ratio: 16 / 10;
    border-radius: 6px;
    background: var(--app-bg);
    border: 1px solid var(--app-border);
  }

  .quick-recall__skeleton-body {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: 7px;
    padding: 2px 0;
  }

  .quick-recall__skeleton-line {
    display: block;
    height: 9px;
    border-radius: 5px;
    background: var(--app-surface-raised);
  }

  .quick-recall__skeleton-line--title {
    height: 11px;
    width: 62%;
  }

  .quick-recall__skeleton-line--short {
    width: 38%;
  }

  /* Shimmer sweep over the skeleton blocks (gated off under reduced motion). */
  .quick-recall__skeleton-thumb,
  .quick-recall__skeleton-line {
    position: relative;
    overflow: hidden;
  }

  .quick-recall__skeleton-thumb::after,
  .quick-recall__skeleton-line::after {
    content: "";
    position: absolute;
    inset: 0;
    transform: translateX(-100%);
    background: linear-gradient(
      90deg,
      transparent,
      color-mix(in srgb, var(--app-text-subtle) 14%, transparent),
      transparent
    );
    animation: quick-recall-shimmer 1.4s ease-in-out infinite;
  }

  @keyframes quick-recall-shimmer {
    100% {
      transform: translateX(100%);
    }
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

  /* Slice 7: syntax-help affordance. The wrapper is the positioning context for
     the popover; the trigger borrows the quiet subtle-surface / hairline-border
     idiom of .quick-recall__ask-button and .quick-recall__back (accent reserved
     for hover), sized as a small round `?`. */
  .quick-recall__syntax {
    position: relative;
    flex-shrink: 0;
    display: flex;
  }

  .quick-recall__syntax-trigger {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 22px;
    height: 22px;
    font-family: inherit;
    font-size: 12px;
    line-height: 1;
    color: var(--app-text-muted);
    background: var(--app-surface-subtle);
    border: 1px solid var(--app-border);
    border-radius: 50%;
    cursor: pointer;
    transition: border-color 0.12s ease, color 0.12s ease;
  }

  .quick-recall__syntax-trigger:hover {
    border-color: var(--app-accent);
    color: var(--app-text-strong);
  }

  /* The popover floats below-right of the trigger, anchored to the wrapper. It's
     static documentation — no interactive content — so role="tooltip" suffices.
     The raised surface + hairline border match the rest of the surface chrome. */
  .quick-recall__syntax-popover {
    position: absolute;
    top: calc(100% + 8px);
    right: 0;
    z-index: 10;
    width: 280px;
    max-width: min(280px, calc(100vw - 30px));
    padding: 10px 12px;
    background: var(--app-surface-raised);
    border: 1px solid var(--app-border);
    border-radius: 8px;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.28);
  }

  .quick-recall__syntax-heading {
    margin: 0 0 6px;
    font-size: 11px;
    line-height: 1.3;
    color: var(--app-text-muted);
  }

  .quick-recall__syntax-heading--secondary {
    margin-top: 10px;
    padding-top: 10px;
    border-top: 1px solid var(--app-border);
  }

  .quick-recall__syntax-list {
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: 5px;
  }

  .quick-recall__syntax-row {
    display: flex;
    align-items: baseline;
    gap: 8px;
  }

  .quick-recall__syntax-row dt {
    flex-shrink: 0;
    margin: 0;
  }

  .quick-recall__syntax-row dt code {
    font-family: ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas,
      monospace;
    font-size: 11px;
    line-height: 1;
    padding: 0.15em 0.4em;
    border-radius: 4px;
    background: var(--app-surface-subtle);
    border: 1px solid var(--app-border);
    color: var(--app-text-strong);
    white-space: nowrap;
  }

  .quick-recall__syntax-row dd {
    margin: 0;
    font-size: 11.5px;
    line-height: 1.35;
    color: var(--app-text);
  }

  .quick-recall__ask-hint {
    margin: 0;
    padding: 6px 18px 0;
    font-size: 11px;
    line-height: 1.4;
    color: var(--app-text-subtle);
    flex-shrink: 0;
  }

  /* Slice 2: active filter chip band. A thin wrapping row beneath the input,
     sharing its slot with the slice-3 inline error line. Pills use the same
     subtle-surface / hairline-border idiom as .quick-recall__ask-button and
     .quick-recall__orient-cue, with the accent reserved for the remove hover. */
  .quick-recall__chips {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    padding: 8px 15px 0;
    flex-shrink: 0;
  }

  .quick-recall__chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 11.5px;
    line-height: 1;
    color: var(--app-text);
    background: var(--app-surface-subtle);
    border: 1px solid var(--app-border);
    border-radius: 6px;
    padding: 4px 4px 4px 8px;
  }

  .quick-recall__chip-label {
    white-space: nowrap;
  }

  .quick-recall__chip-remove {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 16px;
    height: 16px;
    font-family: inherit;
    font-size: 13px;
    line-height: 1;
    color: var(--app-text-muted);
    background: transparent;
    border: none;
    border-radius: 4px;
    padding: 0;
    cursor: pointer;
    transition: color 0.12s ease, background-color 0.12s ease;
  }

  .quick-recall__chip-remove:hover {
    color: var(--app-text-strong);
    background: color-mix(in srgb, var(--app-accent) 18%, transparent);
  }

  /* Slice 3: inline parse-error line under the input. Shares the chip band's
     horizontal padding so it lines up with the chips it sits alongside, and uses
     the accent color (same as .quick-recall__state--error) to read as a live
     correction prompt rather than chrome. */
  .quick-recall__parse-error {
    margin: 0;
    padding: 8px 15px 0;
    flex-shrink: 0;
    font-size: 11.5px;
    line-height: 1.4;
    color: var(--app-accent);
  }

  /* Slice 5: Filter Picker overlay. Reuses the results-region box (flex column,
     scroll) so it occupies the same slot the results would, then layers a header,
     a vertical option list, and (for dates) the From/To row + presets. Item
     highlight uses the same accent-tinted surface idiom as a selected result. */
  .quick-recall__picker {
    gap: 10px;
  }

  .quick-recall__picker-header {
    display: flex;
    align-items: center;
    gap: 7px;
    flex-shrink: 0;
    padding: 0 2px 2px;
  }

  .quick-recall__picker-title {
    font-size: 11px;
    line-height: 1;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--app-text-subtle);
  }

  .quick-recall__picker-crumb-hint {
    font-size: 11px;
    line-height: 1;
    color: var(--app-text-subtle);
  }

  .quick-recall__picker-list {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .quick-recall__picker-item {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 10px;
    border: 1px solid transparent;
    border-radius: 8px;
    cursor: pointer;
  }

  .quick-recall__picker-item--selected {
    background: color-mix(in srgb, var(--app-accent) 12%, transparent);
    border-color: var(--app-accent-border);
  }

  /* Slice 4 (typed path): a value-list row that would conflict with an active
     chip (app + audio source are mutually exclusive). Dimmed and non-selectable —
     it never highlights on hover and Enter skips it. */
  .quick-recall__picker-item--disabled {
    opacity: 0.4;
    cursor: default;
  }

  /* Slice 4 (typed path): the one-line conflict note below the value list, and
     the typed-date hint for the date operators. Both read as muted guidance
     rather than chrome; the conflict note borrows the accent the parse-error line
     uses so it lands as a live correction prompt. */
  .quick-recall__picker-conflict {
    margin: 0;
    padding: 2px 2px 0;
    flex-shrink: 0;
    font-size: 11.5px;
    line-height: 1.4;
    color: var(--app-accent);
  }

  .quick-recall__picker-hint {
    margin: 0;
    padding: 2px 2px 0;
    flex-shrink: 0;
    font-size: 11.5px;
    line-height: 1.4;
    color: var(--app-text-subtle);
  }

  .quick-recall__picker-hint code {
    font-family: inherit;
    color: var(--app-text-muted);
  }

  .quick-recall__picker-item-label {
    font-size: 13px;
    line-height: 1.3;
    color: var(--app-text-strong);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .quick-recall__picker-item-hint {
    flex: 1;
    min-width: 0;
    font-size: 11.5px;
    line-height: 1.3;
    color: var(--app-text-subtle);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .quick-recall__picker-item-chevron {
    flex-shrink: 0;
    font-size: 13px;
    color: var(--app-text-muted);
  }

  .quick-recall__picker-cue {
    margin: auto 0 0;
    padding: 8px 2px 0;
    flex-shrink: 0;
    font-size: 10.5px;
    line-height: 1;
    color: var(--app-text-subtle);
    display: flex;
    align-items: center;
    gap: 4px;
  }

  .quick-recall__picker-cue kbd {
    font-family: inherit;
    font-size: 10px;
    line-height: 1;
    text-transform: lowercase;
    color: var(--app-text-muted);
    background: var(--app-surface);
    border: 1px solid var(--app-border);
    border-radius: 5px;
    padding: 2px 5px;
    margin: 0 1px;
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
    font-size: 14px;
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
    font-size: 14px;
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
    position: relative;
  }

  /* Answer sources strip: sectioned Screen/Audio rows beneath the answer prose.
     Each section's cards scroll horizontally (AnswerSourceCard is fixed-width),
     separated from the prose by a hairline rule. */
  .quick-recall__sources {
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    gap: 12px;
    margin-top: 4px;
    padding-top: 14px;
    border-top: 1px solid var(--app-border);
  }

  .quick-recall__sources-heading {
    font-size: 11px;
    line-height: 1;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--app-text-subtle);
    padding: 0 2px;
  }

  /* Horizontally-scrolling card row. The thin scrollbar stays out of the way
     until hover, matching the quiet terminal aesthetic of the surface. */
  .quick-recall__source-row {
    display: flex;
    gap: 8px;
    overflow-x: auto;
    padding-bottom: 4px;
    scrollbar-width: thin;
    scrollbar-color: var(--app-border) transparent;
  }

  .quick-recall__source-row::-webkit-scrollbar {
    height: 6px;
  }

  .quick-recall__source-row::-webkit-scrollbar-track {
    background: transparent;
  }

  .quick-recall__source-row::-webkit-scrollbar-thumb {
    background: var(--app-border);
    border-radius: 3px;
  }

  .quick-recall__source-row::-webkit-scrollbar-thumb:hover {
    background: var(--app-border-hover);
  }

  /* Copy button: pinned to the top-right of the answer region, only visible at
     askPhase === "done" (gated in markup). Quiet by default, accent on hover. */
  .quick-recall__copy {
    position: absolute;
    top: 8px;
    right: 10px;
    z-index: 2;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 26px;
    height: 26px;
    padding: 0;
    color: var(--app-text-subtle);
    background: var(--app-surface);
    border: 1px solid var(--app-border);
    border-radius: 6px;
    cursor: pointer;
  }

  .quick-recall__copy:hover {
    color: var(--app-text-strong);
    border-color: var(--app-accent);
  }

  .quick-recall__copy--copied {
    color: var(--app-accent);
    border-color: var(--app-accent-border);
  }

  /* Error "Try again" affordance. */
  .quick-recall__retry-row {
    padding: 2px;
  }

  .quick-recall__retry {
    font-family: inherit;
    font-size: 12px;
    line-height: 1;
    color: var(--app-text);
    background: var(--app-surface-subtle);
    border: 1px solid var(--app-border);
    border-radius: 6px;
    padding: 7px 11px;
    cursor: pointer;
  }

  .quick-recall__retry:hover {
    border-color: var(--app-accent);
    color: var(--app-text-strong);
  }

  /* Collapsed, expandable activity summary chip. */
  .quick-recall__activity {
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .quick-recall__activity-chip {
    align-self: flex-start;
    max-width: 100%;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-family: inherit;
    font-size: 11px;
    line-height: 1;
    color: var(--app-text-muted);
    background: var(--app-surface-subtle);
    border: 1px solid var(--app-border);
    border-radius: 6px;
    padding: 5px 9px;
    cursor: pointer;
  }

  .quick-recall__activity-chip:hover {
    border-color: var(--app-border-hover);
    color: var(--app-text);
  }

  .quick-recall__activity-caret {
    flex-shrink: 0;
    font-size: 9px;
    color: var(--app-text-subtle);
    transition: transform 0.12s ease;
  }

  .quick-recall__activity-caret--open {
    transform: rotate(90deg);
  }

  .quick-recall__activity-summary {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .quick-recall__activity-list {
    margin: 0;
    padding: 2px 2px 2px 4px;
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }

  /* The expanded disclosure exists to show the full filter detail, so its rows
     wrap rather than truncate (unlike the one-line live working label). */
  .quick-recall__activity-item {
    font-size: 11px;
    line-height: 1.4;
    color: var(--app-text-subtle);
    min-width: 0;
    overflow-wrap: anywhere;
  }

  /* The live working line's filter string wraps to its full text (the answer
     area scrolls) rather than truncating, so the user sees exactly what ran. */
  .quick-recall__working-label {
    flex: 1;
    min-width: 0;
    overflow-wrap: anywhere;
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
    align-items: flex-start;
    gap: 8px;
    color: var(--app-text-muted);
  }

  .quick-recall__dot {
    width: 7px;
    height: 7px;
    /* Nudge down so the dot centers on the first line of a wrapping label. */
    margin-top: 0.4em;
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
    font-size: 13px;
    line-height: 1.55;
    color: var(--app-text);
    word-break: break-word;
    overflow-wrap: anywhere;
  }

  /* Rendered Markdown blocks. The answer is a flow of <p>/<ul>/<pre>/… so we
     tame default browser margins and tie everything to the app palette. The
     `:global()` wrappers are required because this HTML is injected via {@html}
     and would otherwise be stripped by Svelte's scoped-style pruning. */
  .quick-recall__answer :global(> :first-child) {
    margin-top: 0;
  }

  .quick-recall__answer :global(> :last-child) {
    margin-bottom: 0;
  }

  .quick-recall__answer :global(p),
  .quick-recall__answer :global(ul),
  .quick-recall__answer :global(ol),
  .quick-recall__answer :global(blockquote),
  .quick-recall__answer :global(pre),
  .quick-recall__answer :global(table) {
    margin: 0 0 0.7em;
  }

  .quick-recall__answer :global(h1),
  .quick-recall__answer :global(h2),
  .quick-recall__answer :global(h3),
  .quick-recall__answer :global(h4),
  .quick-recall__answer :global(h5),
  .quick-recall__answer :global(h6) {
    margin: 1.1em 0 0.5em;
    line-height: 1.3;
    font-weight: 600;
    color: var(--app-text-strong);
  }

  .quick-recall__answer :global(h1) {
    font-size: 1.3em;
  }
  .quick-recall__answer :global(h2) {
    font-size: 1.18em;
  }
  .quick-recall__answer :global(h3) {
    font-size: 1.06em;
  }
  .quick-recall__answer :global(h4),
  .quick-recall__answer :global(h5),
  .quick-recall__answer :global(h6) {
    font-size: 1em;
  }

  .quick-recall__answer :global(strong) {
    font-weight: 600;
    color: var(--app-text-strong);
  }

  .quick-recall__answer :global(a) {
    color: var(--app-accent);
    text-decoration: underline;
    text-underline-offset: 2px;
    cursor: pointer;
  }

  .quick-recall__answer :global(ul),
  .quick-recall__answer :global(ol) {
    padding-left: 1.4em;
  }

  .quick-recall__answer :global(li) {
    margin: 0.2em 0;
  }

  .quick-recall__answer :global(li::marker) {
    color: var(--app-text-muted);
  }

  .quick-recall__answer :global(li > ul),
  .quick-recall__answer :global(li > ol) {
    margin: 0.2em 0;
  }

  /* Inline code. */
  .quick-recall__answer :global(code) {
    font-family: ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas,
      monospace;
    font-size: 0.88em;
    padding: 0.1em 0.35em;
    border-radius: 4px;
    background: var(--app-surface-raised);
    border: 1px solid var(--app-border);
    color: var(--app-text-strong);
  }

  /* Fenced code blocks: the <pre> owns the chrome, the inner <code> resets. */
  .quick-recall__answer :global(pre) {
    padding: 10px 12px;
    border-radius: 8px;
    background: var(--app-surface-raised);
    border: 1px solid var(--app-border);
    overflow-x: auto;
  }

  .quick-recall__answer :global(pre code) {
    padding: 0;
    border: none;
    background: none;
    color: var(--app-text);
    font-size: 0.86em;
    line-height: 1.5;
  }

  .quick-recall__answer :global(blockquote) {
    padding: 0.1em 0 0.1em 0.9em;
    border-left: 2px solid var(--app-accent-border);
    color: var(--app-text-muted);
  }

  .quick-recall__answer :global(hr) {
    margin: 1em 0;
    border: none;
    border-top: 1px solid var(--app-border);
  }

  .quick-recall__answer :global(table) {
    border-collapse: collapse;
    font-size: 0.92em;
  }

  .quick-recall__answer :global(th),
  .quick-recall__answer :global(td) {
    padding: 0.35em 0.7em;
    border: 1px solid var(--app-border);
    text-align: left;
  }

  .quick-recall__answer :global(th) {
    background: var(--app-surface-subtle);
    color: var(--app-text-strong);
    font-weight: 600;
  }

  /* Streaming cursor: a blinking caret tacked onto the last rendered block so it
     trails the freshest token instead of dropping to its own line. */
  .quick-recall__answer--streaming :global(> :last-child::after) {
    content: "";
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

  /* Slice 7: reduced-motion gating for the whole surface. Every animation and
     transition in this file collapses to an instant/static fallback. The hero
     mode-switch cross-fade is JS-driven (modeFadeMs → 0 in the script) and so is
     handled there; everything else is gated here. */
  @media (prefers-reduced-motion: reduce) {
    .quick-recall__dot {
      animation: none;
      opacity: 1;
    }

    .quick-recall__answer--streaming :global(> :last-child::after) {
      animation: none;
    }

    .quick-recall__skeleton-thumb::after,
    .quick-recall__skeleton-line::after {
      animation: none;
      display: none;
    }

    .quick-recall__ask-button,
    .quick-recall__back,
    .quick-recall__copy,
    .quick-recall__retry,
    .quick-recall__activity-chip,
    .quick-recall__activity-caret,
    .quick-recall__syntax-trigger {
      transition: none;
    }
  }
</style>
