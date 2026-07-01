<script lang="ts">
  import { tip } from "$lib/components/tooltip";
  import { onMount, onDestroy, tick } from "svelte";
  import { fade } from "svelte/transition";
  import { convertFileSrc, invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { appIconFallback } from "$lib/app-privacy-exclusion";
  import SearchResultCard from "$lib/components/SearchResultCard.svelte";
  import AnswerSourceCard from "$lib/components/AnswerSourceCard.svelte";
  import { framePreviewAssetUrl } from "$lib/frame-preview";
  import { openCapturedUrl } from "$lib/open-captured-url";
  import { closeCurrentWindow, openSettings } from "$lib/surface-windows";
  import type {
    SemanticSearchModelStatusResponse,
    SemanticSearchModelDownloadProgress,
    RecordingSettingsDomainUpdateResponse,
  } from "$lib/types";
  import { askAiClock } from "$lib/askAiClock";
  import AnswerProse from "$lib/AnswerProse.svelte";
  import MiniBars from "$lib/insights/charts/MiniBars.svelte";
  import Timeline from "$lib/insights/charts/Timeline.svelte";
  import ConfidenceBar from "$lib/insights/charts/ConfidenceBar.svelte";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { message } from "@tauri-apps/plugin-dialog";
  import { humanizeError } from "$lib/format-error";
  import type {
    Conversation,
    ConversationTurn,
    AskAiSource,
    AnswerBlock,
    ToolActivityEntry,
    TurnView,
    TurnSnapshot,
    TurnUpdate,
    AskAiUpdateEvent,
  } from "$lib/insights/conversation";
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
      // Clear any in-flight loading state: the invalidated response will be
      // dropped by the generation guard and never reach `loading = false`, so
      // without this the panel stays stuck "running" — keeping the value list
      // owner from rendering cleanly and blocking the idle-clear teardown
      // (operationRunning would never fall back to false).
      loading = false;
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
      // Scope is only known AFTER a response, so `sectionLimits` is a $derived
      // off the PREVIOUS response's appliedRefinements. `appliedRefinements`
      // always belongs to `resultsQuery`, so the narrowing is only trustworthy
      // when the pending query MATCHES that prior query (a same-query re-run /
      // pagination). When the query CHANGED, the cached scope is stale and could
      // zero out the section the new query actually scopes to (e.g. switching
      // from `source:screen` to `source:mic …` would send audioLimit:0, the
      // backend would return no audio rows, and nothing reruns since only
      // appliedRefinements changed — a valid search looks empty). Over-fetching
      // is harmless (the backend honors the raw query's operators regardless of
      // these limits), so for a changed query we fetch both sections at full
      // limit and let the scope settle on the next keystroke. `refinements`
      // stays empty: the operators live in the query TEXT, not this struct.
      const scopeMatchesPendingQuery =
        appliedRefinements !== null && resultsQuery === trimmed;
      const limits = scopeMatchesPendingQuery
        ? sectionLimits
        : { frameLimit: 5, audioLimit: 5 };
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
      errorMessage = humanizeError(error);
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

  // Surface a hand-off failure for the open-result/open-source paths. The
  // brokered open is this window's core action, so a rejected invoke must not
  // close the window onto nothing — we keep the window open and report instead.
  async function surfaceResultHandoffFailure(err: unknown): Promise<void> {
    await message(
      `Couldn't open that result: ${humanizeError(err, "it may no longer be available.")}`,
      { title: "Couldn't open result", kind: "error" },
    );
  }

  async function selectFrame(result: FrameSearchResultDto): Promise<void> {
    try {
      await invoke("open_capture_result_in_main_window", {
        kind: "frame",
        frameId: result.representativeFrame.id,
        audioSegmentId: null,
      });
    } catch (err) {
      await surfaceResultHandoffFailure(err);
      return;
    }
    await closeCurrentWindow();
  }

  async function selectAudio(result: AudioSearchResultDto): Promise<void> {
    // Carry the Audio Search Result Anchor (match span start + aligned frame)
    // so the dashboard lands on the selected transcript match rather than the
    // segment start, mirroring the in-dashboard selectAudioSearchResult path.
    try {
      await invoke("open_capture_result_in_main_window", {
        kind: "audio",
        frameId: null,
        audioSegmentId: result.audioSegment.id,
        spanStartMs: result.spanStartMs,
        alignedFrameId: result.alignedFrame?.id ?? null,
      });
    } catch (err) {
      await surfaceResultHandoffFailure(err);
      return;
    }
    await closeCurrentWindow();
  }

  // Opening a cited source closes the Quick Recall window. By the time the user
  // clicks a source they have almost always SEEN the answer (focused + terminal →
  // askOutcomeSeen), so without this flag the dismiss/idle-clear would treat the
  // close as ordinary teardown and re-summon would land on an empty search — the
  // answer gone with no breadcrumb. This one-shot flag keeps the thread alive
  // across the source hand-off so re-summoning restores the same answer (the
  // thread is also persisted server-side and reachable via "Continue in Chat").
  // Reset on the next focus (re-summon) so an ordinary later dismiss is normal.
  let sourceHandoffPending = $state(false);

  // Hand off an Ask AI answer source to the main window, mirroring
  // selectFrame/selectAudio (frame xor audio carried by the source kind).
  async function selectSource(source: AskAiSource): Promise<void> {
    // Carry the Audio Search Result Anchor for audio sources (frame sources
    // leave these null), mirroring selectAudio so the dashboard lands on the
    // cited transcript match rather than the segment start.
    try {
      await invoke("open_capture_result_in_main_window", {
        kind: source.kind,
        frameId: source.frameId,
        audioSegmentId: source.audioSegmentId,
        spanStartMs: source.spanStartMs ?? null,
        alignedFrameId: source.alignedFrameId ?? null,
      });
    } catch (err) {
      await surfaceResultHandoffFailure(err);
      return;
    }
    // Preserve the thread across the close so re-summon restores the answer
    // instead of dropping to a blank search (see sourceHandoffPending above).
    sourceHandoffPending = true;
    await closeCurrentWindow();
  }

  // In-flight latch for the captured-page open: one selected result (and one
  // answer-source chip) opens at a time, so a single boolean is enough to keep the
  // ⌃O key or a chip double-click from stacking opens / feedback dialogs. The
  // latch wraps the actual brokered open, so it covers the keyboard path
  // (openSelectedResultUrl → here) and the answer-source chips (openSourceUrl →
  // here) alike. (Search result chips have their own per-instance latch inside
  // SearchResultCard.)
  let openingCapturedUrl = $state(false);

  // Open a captured page via the shared brokered helper. The helper owns the
  // feedback: a no-openable-URL result shows a brief info note and a real opener
  // failure shows an error dialog (mirroring the timeline's "Couldn't open
  // URL: …"). The raw URL stays in Rust; the UI never sees it.
  async function openCapturedFrameUrl(frameId: number): Promise<void> {
    if (openingCapturedUrl) return;
    openingCapturedUrl = true;
    try {
      await openCapturedUrl(frameId);
    } finally {
      openingCapturedUrl = false;
    }
  }

  // Open the captured page behind a frame source in the default browser.
  // Frame sources only (audio has frameId/url null).
  async function openSourceUrl(source: AskAiSource): Promise<void> {
    if (source.frameId == null) return;
    await openCapturedFrameUrl(source.frameId);
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
  // The currently-selected result is an openable frame: selection is within the
  // frame section (audio selections sit past frames.length) AND that frame
  // carries a captured page. Gates the ⌃/⌘+O "open page" footer hint so it
  // tracks the *selected* result — the hint only shows when the action the
  // keypress fires (openSelectedResultUrl on the selection) actually has a
  // target, never advertising a no-op for an audio/url-less selection.
  let selectedResultIsOpenable = $derived(
    selectedIndex >= 0 &&
      selectedIndex < frames.length &&
      frames[selectedIndex]?.url != null,
  );
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

  // Open the captured page behind the currently-selected frame result in the
  // default browser. The result cards live in an aria-activedescendant listbox
  // (DOM focus stays on the search input), so the per-card open chip can't sit in
  // the tab order without breaking the roving model. This is the keyboard path to
  // that chip's action (⌘/Ctrl+O, wired in handleSearchKeydown): it opens the
  // selected frame's page through the same brokered helper the chip uses. The
  // footer hint is gated on `selectedResultIsOpenable`, so the keypress should
  // only land here on an openable frame — but the ⌃O shortcut still fires while a
  // non-openable result is selected, so surface a benign note (same opener
  // feedback path as a real failure) instead of silently doing nothing.
  function openSelectedResultUrl(): void {
    if (selectedResultIsOpenable) {
      void openCapturedFrameUrl(frames[selectedIndex].thumbnailFrameId);
      return;
    }
    void message("No openable page for this result.", {
      title: "Couldn't open page",
      kind: "info",
    });
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

    // ⌘/Ctrl+O opens the selected frame result's captured page in the browser —
    // the keyboard path to each card's hover-only "open in browser" chip, which
    // can't be a tab stop inside this aria-activedescendant listbox. A no-op when
    // nothing is selected or the selection has no openable URL (audio / no link).
    if (
      (event.metaKey || event.ctrlKey) &&
      !event.altKey &&
      !event.shiftKey &&
      (event.key === "o" || event.key === "O")
    ) {
      event.preventDefault();
      openSelectedResultUrl();
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
  // Stable id for the disabled-Ask-AI reason line, referenced by the disabled
  // button's aria-describedby so keyboard/AT users reach the reason (the native
  // `title` tooltip is mouse-only).
  const ASK_UNAVAILABLE_HINT_ID = "quick-recall-ask-unavailable-hint";

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
  // seeded with redacted broker results for that same query. The in-memory
  // thread is still ephemeral for the live launcher experience — a fresh window
  // summon recreates the component, and returning to search mode resets the
  // in-memory state below.
  //
  // Conversations are now ALSO persisted to the shared conversation store in the
  // Encrypted Capture Index (issue #111, ADR 0031): every real turn is written
  // under the same conversationId with origin "quick_recall", so the same thread
  // can be opened/continued in the Insights Chat workspace ("Continue in Chat").
  // This persistence is purely additive to the ephemeral runtime model above;
  // the stored conversation is governed by Retention Policy and cleared by Wipe
  // User Context. ADR 0031 supersedes ADR 0027's disk-ephemerality for the thread.
  // ---------------------------------------------------------------------------

  type AskAiAvailability = {
    available: boolean;
    reason: string | null;
  };

  // The backend now OWNS the render model and streams it via the single
  // `ask_ai_update` transport (issue #110, ADR 0031), exactly as Chat.svelte
  // consumes. The legacy per-token `ask_ai_*` event shapes
  // (status/delta/reasoning/done/error/source) and their local tool-label /
  // icon-cache machinery are gone — the frontend ONLY applies versioned
  // `TurnUpdate` ops, snapshots on attach, and re-snapshots on a version gap.
  // The shared render types (`AnswerBlock`, `ToolActivityEntry`, `TurnView`,
  // `TurnSnapshot`, `TurnUpdate`, `AskAiUpdateEvent`, `AskAiSource`) are imported
  // from `$lib/insights/conversation`.

  type AskAiPhase = "seeding" | "thinking" | "streaming" | "done" | "error";

  let mode = $state<"search" | "ask">("search");

  // Availability is resolved on mount. Until it resolves the affordance is
  // treated as unavailable so we never render a dead button that errors.
  let askAvailability = $state<AskAiAvailability | null>(null);

  // The editable input used when Ask AI is opened with no seed query (turn 1).
  // `askSubmitted` flips true once the FIRST turn exists (seeded or typed), at
  // which point the transcript renders and this input is replaced by per-turn
  // question headers.
  let askInput = $state("");
  let askInputEl = $state<HTMLTextAreaElement | null>(null);
  let askSubmitted = $state(false);

  // ---------------------------------------------------------------------------
  // Multi-turn thread (ADR 0026)
  //
  // The single one-shot Q&A is now an ephemeral transcript: a vertical list of
  // self-contained turns. `askConversationId` is the THREAD id (one per thread,
  // one live PI session server-side), NOT a per-turn id — every stream event
  // carries it and we ignore any whose id doesn't match (stale-thread guard).
  // Streaming events route to the LAST turn in `askTurns` (the live one). Only
  // turn 1 is seeded (via ask_ai_start); follow-ups go raw to ask_ai_followup.
  // ---------------------------------------------------------------------------

  // One assistant turn in the transcript. This is the backend-owned `TurnView`
  // render model (issue #110, ADR 0031) plus QR's UI-only flags. The frontend
  // ONLY renders: it applies versioned `TurnUpdate` ops from `ask_ai_update`,
  // snapshots on attach, and re-snapshots on a version gap. It does NO fence
  // parsing, NO tool-label formatting, NO icon-cache batching, NO local phase
  // machine — those all moved server-side.
  type AskTurn = {
    // The 0-based turn index within the thread — the key the `ask_ai_update`
    // event addresses (event.turnIndex), so updates route to the right turn.
    turnIndex: number;
    question: string;
    // Render-ready answer blocks (prose markdown stays rendered on the frontend
    // via AnswerProse; the graphical variants carry already-parsed data).
    blocks: AnswerBlock[];
    // The model's reasoning ("thinking") text, or null when none — the Thinking
    // disclosure renders only when this is non-empty.
    reasoning: string | null;
    // Per-turn disclosure toggle for the collapsed "Thought process" chip (the
    // settled reasoning panel); the live reasoning panel ignores this and is
    // always shown expanded while it streams.
    reasoningExpanded: boolean;
    // Render-ready tool-activity log (label + optional app + resolved icon path,
    // all computed server-side).
    toolActivities: ToolActivityEntry[];
    // The live working-line entry for the in-flight tool call (cleared on done),
    // shown beneath the answer for the active turn.
    liveActivity: ToolActivityEntry | null;
    sources: AskAiSource[];
    phase: AskAiPhase;
    errorMessage: string | null;
    // Only turn 1 ever has a seeded-result count (follow-ups aren't seeded).
    seededResultCount: number | null;
    // Per-turn disclosure toggle for the collapsed tool-activity summary chip.
    summaryExpanded: boolean;
    // Per-turn copy-confirmation flash (icon swaps to a check briefly).
    copied: boolean;
    // Per-turn copy-FAILURE flash (button flashes red briefly when the
    // clipboard write rejects, so a failed copy isn't silent).
    copyFailed: boolean;
    // Last applied `ask_ai_update` version for this turn (0 if none — e.g. a
    // hydrated past turn that isn't live).
    version: number;
    // The user STOPPED this turn mid-stream. Frontend-only marker (the backend
    // persists the partial as an ordinary `done` turn) driving a "Stopped early"
    // tag in Quick Recall so the cut-off is acknowledged where it happened.
    stoppedEarly?: boolean;
  };

  // The thread id of the live, streamable thread. null when no thread is open,
  // OR after a Stop (the partial is persisted; its id moves to
  // `stoppedConversationId` so late buffered updates stop matching the guard).
  let askConversationId = $state<string | null>(null);
  // The persisted id of a STOPPED thread. Kept apart from `askConversationId`
  // (so the late-update guard in cancelActiveAsk stays intact) purely to keep
  // "Continue in Chat" + the follow-up composer pointed at the real, already-
  // persisted `done` thread. Cleared on reset or when a follow-up re-adopts it.
  let stoppedConversationId = $state<string | null>(null);
  // The id of whichever thread is continuable right now — live or just-stopped.
  let askContinuableConversationId = $derived(
    askConversationId ?? stoppedConversationId,
  );
  // The transcript. The last entry is the live turn that stream events feed.
  let askTurns = $state<AskTurn[]>([]);
  // True between a turn starting and that turn's terminal done/error event.
  // Thread-level: gates the composer (disabled while any turn streams).
  let askStreaming = $state(false);

  // True once the user has Stopped a streaming answer in place. Stopping drops
  // the live session id (cancelActiveAsk), so the composer and "Continue in
  // Chat" hide via their askConversationId guards — which would otherwise strand
  // the partial answer with no way forward. This flag drives an explicit
  // "Stopped — start a new question" affordance so the surface isn't a dead end.
  // Cleared whenever a fresh thread starts or the thread state is reset.
  let askStopped = $state(false);

  // The seed used for the current thread's FIRST turn, so an error "Retry"
  // can re-run the exact same question + seed pairing as a fresh thread.
  let askLastSeed = $state<string | null>(null);

  // The first-turn question, kept so retryAsk can rebuild a fresh thread with
  // the same seeded question after a turn-1 error.
  let askFirstQuestion = $state("");

  // Per-turn copy-confirmation timers, keyed by the turn's array index. Cleared
  // on teardown. Stored outside the turn objects so the flash never leaks into
  // copied Markdown or re-renders the whole transcript array.
  let askCopiedTimers = new Map<number, ReturnType<typeof setTimeout>>();

  // The live (last) turn, or null when the transcript is empty. Convenience for
  // markup that needs the streaming turn's phase (e.g. composer focus).
  let askLiveTurn = $derived<AskTurn | null>(
    askTurns.length > 0 ? askTurns[askTurns.length - 1] : null,
  );

  // AT-only phase announcement for the Ask AI transcript. The transcript itself
  // is NOT an aria-live region — streaming the answer token-by-token through one
  // polite region floods a screen reader with a re-read of the whole growing
  // answer on every delta. Instead this small string announces ONLY the live
  // turn's phase transitions (searching → thinking → writing → settled / error /
  // stopped), mirroring the search door's `searchStatusAnnouncement` + its
  // visually-hidden `role="status"` region. The visible transcript carries
  // `aria-busy` while streaming so AT knows it's updating without narrating each
  // token.
  let askPhaseAnnouncement = $derived.by((): string => {
    if (mode !== "ask" || !askSubmitted) {
      return "";
    }
    if (askStopped) {
      return "Stopped.";
    }
    const live = askLiveTurn;
    if (live === null) {
      return "";
    }
    switch (live.phase) {
      case "seeding":
        return "Searching your captures.";
      case "thinking":
        return "Thinking.";
      case "streaming":
        return "Writing the answer.";
      case "error":
        return live.errorMessage ?? "Ask AI failed.";
      case "done":
        return live.sources.length > 0
          ? "Answer ready, with sources."
          : "Answer ready.";
      default:
        return "";
    }
  });

  // The live answer is settled and has copyable text — gates the footer's ⌃C
  // copy hint (the ⌘C shortcut existed but was never surfaced anywhere).
  let askCopyAvailable = $derived(
    askLiveTurn !== null &&
      askLiveTurn.phase === "done" &&
      turnAnswerText(askLiveTurn).length > 0,
  );

  // The composer is available once the FIRST answer has completed and no turn is
  // currently streaming-or-pending. It stays VISIBLE (but disabled) while a
  // follow-up streams. Hidden entirely while turn 1 is still seeding / streaming
  // / errored-with-no-completed-answer.
  let askHasCompletedTurn = $derived(askTurns.some((t) => t.phase === "done"));
  // Visible once the first answer completes. A STOPPED thread stays continuable
  // (its partial is persisted as a `done` turn under `stoppedConversationId`),
  // so the composer keeps showing — a follow-up re-adopts that thread and runs
  // a fresh turn that builds on the partial.
  let askComposerVisible = $derived(
    askSubmitted && askHasCompletedTurn && askContinuableConversationId !== null,
  );

  // ---------------------------------------------------------------------------
  // "Seen" state (background completion — see PLAN.md slice 1)
  //
  // A conversation is SEEN once its last turn reached a terminal phase
  // (done | error) AND that terminal turn was rendered while the window was
  // focused — even momentarily. Watching it finish while focused counts; a
  // still-streaming turn is never seen. Thread-level, ephemeral: reset whenever
  // the thread is torn down (resetAskThreadState) and re-armed per new thread.
  //
  // This gates whether dismiss/idle-clear may discard the conversation. An
  // unseen or in-flight conversation survives both; a seen one returns to
  // today's ephemeral rules (cleared on dismiss, 5s idle-clear on blur).
  let askOutcomeSeen = $state(false);

  // The live turn reached a terminal phase (its outcome — answer or error — is
  // fully rendered). Distinct from `askStreaming`: a turn can be terminal while
  // a never-started thread has no live turn at all.
  let askTerminalPhase = $derived(
    askLiveTurn !== null &&
      (askLiveTurn.phase === "done" || askLiveTurn.phase === "error"),
  );

  // There is a conversation worth preserving across a dismiss/blur: a thread is
  // open (id present) and either still in flight or finished-but-unseen. Once
  // seen, this goes false and the conversation becomes ordinarily ephemeral.
  // Background completion is now SERVER-side: a dismissed-but-streaming question
  // finishes and persists regardless, so we never cancel an in-flight or unseen
  // thread on dismiss/blur — it simply stays preservable until seen.
  let askConversationPending = $derived(
    askConversationId !== null && (askStreaming || !askOutcomeSeen),
  );

  // Mark the conversation seen the moment a terminal outcome is rendered while
  // the window is focused. Re-summoning to glance at a finished answer (focus →
  // terminal turn already present) satisfies this on the next focus, which is
  // the accepted consequence that a glance marks it seen.
  $effect(() => {
    if (windowFocused && askTerminalPhase) {
      askOutcomeSeen = true;
    }
  });

  // Split a turn's cited sources into the Screen/Audio strip sections.
  function turnFrameSources(turn: AskTurn): AskAiSource[] {
    return turn.sources.filter((s) => s.kind === "frame");
  }
  function turnAudioSources(turn: AskTurn): AskAiSource[] {
    return turn.sources.filter((s) => s.kind === "audio");
  }

  // Open an external answer link through the OS browser. Passed to AnswerProse as
  // `onOpenLink`; the component already prevents default and extracts the href, so
  // this only owns the launcher-specific blur handling and the actual open.
  async function openAnswerLink(href: string): Promise<void> {
    // Opening a link activates the OS browser, which blurs this non-activating
    // panel and fires `Focused(false)`. Suppress the very next blur-dismiss FIRST
    // so the launcher (and the in-flight Ask AI session being read) survives the
    // hand-off; the flag is one-shot, so ordinary click-away still dismisses.
    // AWAIT the suppression before opening: both are separate IPC round-trips,
    // and if `openUrl` activated the browser before Rust set the one-shot flag,
    // the resulting `Focused(false)` would reach the blur handler unsuppressed
    // and dismiss the panel out from under the user. Awaiting orders the flag
    // strictly before the activation. A failed suppress still opens the link.
    try {
      await invoke("quick_recall_suppress_blur_dismiss");
    } catch {
      // Best-effort: proceed to open even if the suppression call failed.
    }
    // Await the open so a launcher failure surfaces instead of vanishing; match
    // the openCapturedUrl "Couldn't open URL: …" feedback so every open-link
    // affordance reports the same way.
    try {
      await openUrl(href);
    } catch (err) {
      await message(
        `Couldn't open URL: ${humanizeError(err, "the link could not be opened")}`,
        { title: "Couldn't open link", kind: "error" },
      );
    }
  }

  // The scrollable transcript region; focused on entry so Escape (back-to-search)
  // and scroll keys are captured even when the seeded path renders no text input.
  // The scroll-to-bottom effect keeps the live turn / composer in view via this.
  let askAreaEl = $state<HTMLDivElement | null>(null);

  // The follow-up composer (bottom-pinned, present once the first answer is
  // done). Mirrors the unseeded askInput textarea: Enter submits, Shift+Enter
  // inserts a newline. Disabled while any turn streams (askStreaming === true).
  let followupInput = $state("");
  let followupInputEl = $state<HTMLTextAreaElement | null>(null);

  function friendlyAskReason(reason: string | null): string {
    switch (reason) {
      case "ask_ai_disabled":
        return "Enable Ask AI in Settings";
      case "ai_runtime_disabled":
        return "Turn on the Reasoning Engine in Settings";
      case "no_cloud_key":
        return "Add a provider API key in Settings";
      case "no_model":
        return "Choose a Reasoning Engine model in Settings";
      case "no_base_url":
        return "Add the provider base URL in Settings";
      case "local_no_model":
        return "Choose a local model in Settings";
      case "local_endpoint_unreachable":
        return "Local engine unreachable — check it's running";
      default:
        return "Set up the Reasoning Engine to use Ask AI";
    }
  }

  // ---------------------------------------------------------------------------
  // Date formatting (pure helpers, shared by search-mode scope chips)
  //
  // Tool-activity formatting moved server-side: the backend now supplies the
  // render-ready label + app icon path on each `ToolActivityEntry`, so the
  // Quick Recall frontend no longer parses raw tool params into humane lines.
  // The date helpers below remain because search-mode (filter chips / scope
  // summary) still calls them.
  // ---------------------------------------------------------------------------

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

  function shortDate(d: Date): string {
    return d.toLocaleDateString(undefined, { month: "short", day: "numeric" });
  }

  // Collapsed summary chip text for ONE turn: counts of each tool kind that ran,
  // e.g. `3 searches · timeline · 1 read`. Null when no tools ran. Pure helper
  // (per-turn) so each transcript turn renders its own activity summary.
  function activitySummaryFor(toolActivities: ToolActivityEntry[]): string | null {
    if (toolActivities.length === 0) return null;
    let searches = 0;
    let timelines = 0;
    let reads = 0;
    let recalls = 0;
    let others = 0;
    for (const entry of toolActivities) {
      if (entry.kind === "search") searches += 1;
      else if (entry.kind === "timeline") timelines += 1;
      else if (entry.kind === "show_text") reads += 1;
      else if (entry.kind === "recall_context") recalls += 1;
      else others += 1;
    }
    const parts: string[] = [];
    if (searches > 0) parts.push(`${searches} ${searches === 1 ? "search" : "searches"}`);
    if (timelines > 0)
      parts.push(`${timelines} ${timelines === 1 ? "timeline scan" : "timeline scans"}`);
    if (reads > 0) parts.push(`${reads} ${reads === 1 ? "read" : "reads"}`);
    if (recalls > 0) parts.push(`${recalls} ${recalls === 1 ? "recall" : "recalls"}`);
    if (others > 0) parts.push(`${others} ${others === 1 ? "step" : "steps"}`);
    return parts.length > 0 ? parts.join(" · ") : null;
  }

  // Tool-activity app icons are now resolved server-side: each
  // `ToolActivityEntry` carries an `appIconPath`, so the chip is a pure
  // path→URL conversion (no client resolve/batch). The icon-cache machinery and
  // its `resolve_app_icons` round-trip were removed in this migration.

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
        reason: humanizeError(error),
      };
    }
  }

  // Build a fresh, empty turn for a new question. `seeding` for turn 1 (the seed
  // broker search runs first); `thinking` for follow-ups (no seed phase). The
  // backend drives the render fields via versioned `ask_ai_update` ops starting
  // at version 0.
  function makeAskTurn(
    turnIndex: number,
    question: string,
    phase: AskAiPhase,
  ): AskTurn {
    return {
      turnIndex,
      question,
      blocks: [],
      reasoning: null,
      reasoningExpanded: false,
      toolActivities: [],
      liveActivity: null,
      sources: [],
      phase,
      errorMessage: null,
      seededResultCount: null,
      summaryExpanded: false,
      copied: false,
      copyFailed: false,
      version: 0,
    };
  }

  // Clear any pending per-turn copy-flash timers (teardown / fresh thread).
  function clearAskCopiedTimers(): void {
    for (const timer of askCopiedTimers.values()) {
      clearTimeout(timer);
    }
    askCopiedTimers.clear();
  }

  // Narrow an AskTurn phase from a persisted/streamed (string) phase. The full
  // lifecycle (seeding | thinking | streaming | done | error) round-trips now
  // that the backend owns the render model, so all five are accepted.
  function normalizeAskPhase(phase: string): AskAiPhase {
    return phase === "done" ||
      phase === "error" ||
      phase === "streaming" ||
      phase === "thinking" ||
      phase === "seeding"
      ? phase
      : "done";
  }

  // Hydrate a persisted ConversationTurn into an AskTurn. The backend's
  // get_conversation populates `turn.blocks` for EVERY turn (new + legacy
  // parsed-on-read), so blocks are taken DIRECTLY with no frontend parsing.
  //
  // Tool activities: the persisted `tool_activities` JSON is still the raw
  // `{tool, params}` shape on cold history. We map each raw `{tool}` to a
  // minimal render-ready `ToolActivityEntry` (kind + generic label) JUST for the
  // collapsed activity summary on a reloaded thread's past turns — the live
  // reattach replaces them via the snapshot's render-ready toolActivities.
  function hydrateAskTurn(turn: ConversationTurn): AskTurn {
    const t = makeAskTurn(
      turn.turnIndex,
      turn.question,
      normalizeAskPhase(turn.phase),
    );
    t.blocks = turn.blocks ?? [];
    t.reasoning = turn.reasoning;
    t.toolActivities = coerceToolActivities(turn.toolActivities);
    t.sources = coerceSources(turn.sources);
    t.errorMessage = turn.errorMessage;
    t.seededResultCount = turn.seededResultCount;
    return t;
  }

  // Map the persisted raw `{tool, params}` per-turn log to minimal render-ready
  // entries (kind + generic label) so a reloaded thread's collapsed activity
  // summary still counts correctly (activitySummaryFor reads `kind`). Streaming
  // and snapshot views deliver fully render-ready entries instead.
  function coerceToolActivities(value: unknown): ToolActivityEntry[] {
    if (!Array.isArray(value)) return [];
    return value
      .map((e): ToolActivityEntry | null => {
        if (typeof e !== "object" || e === null) return null;
        const rec = e as { tool?: unknown; kind?: unknown; label?: unknown };
        // Already render-ready (a snapshot/live entry round-tripped to the DB).
        if (typeof rec.label === "string" && typeof rec.kind === "string") {
          // Validate the persisted kind against the known set so a stale or
          // unexpected value still buckets as "other" in activitySummaryFor.
          const knownKinds = ["search", "timeline", "show_text", "recall_context", "other"];
          const kind = knownKinds.includes(rec.kind) ? rec.kind : "other";
          return { kind, label: rec.label };
        }
        const tool = typeof rec.tool === "string" ? rec.tool : null;
        if (tool === "search")
          return { kind: "search", label: "Searched your captures" };
        if (tool === "timeline")
          return { kind: "timeline", label: "Scanned timeline" };
        if (tool === "show_text")
          return { kind: "show_text", label: "Read a capture" };
        if (tool === "recall_context")
          return { kind: "recall_context", label: "Recalled what I know about you" };
        return { kind: "other", label: tool ? `Ran ${tool}` : "Working" };
      })
      .filter((x): x is ToolActivityEntry => x !== null);
  }

  function coerceSources(value: unknown): AskAiSource[] {
    if (!Array.isArray(value)) return [];
    return value.filter((s): s is AskAiSource => {
      return (
        typeof s === "object" &&
        s !== null &&
        ((s as AskAiSource).kind === "frame" || (s as AskAiSource).kind === "audio")
      );
    });
  }

  // Re-summon hydration (background completion is now server-side): when the
  // panel regains focus on an armed ask thread whose in-memory transcript is
  // empty (e.g. the window was cleared while a question finished in the
  // background), reload the persisted turns from the shared store, then adopt
  // any in-flight live snapshot so a reattach to a streaming turn is race-free.
  async function hydrateAskFromStore(conversationId: string): Promise<void> {
    try {
      const convo = await invoke<Conversation | null>("get_conversation", {
        conversationId,
      });
      // Drop a stale hydrate if the thread moved on or already has a transcript.
      if (convo === null || askConversationId !== conversationId || askTurns.length > 0) {
        return;
      }
      const hydrated = convo.turns.map(hydrateAskTurn);
      if (hydrated.length === 0) return;
      askTurns = hydrated;
      askSubmitted = true;
      // Restore the first-turn question so the turn-1 "Retry" button (which
      // re-runs the whole thread via retryAsk) isn't silently dead after a
      // re-summon. The seed isn't persisted per turn, so retry re-runs without
      // one (askLastSeed stays null, which startAsk handles).
      askFirstQuestion = hydrated[0].question;
      const last = hydrated[hydrated.length - 1];
      // A persisted "streaming" last turn is still in flight server-side; the
      // snapshot below replaces it with the authoritative live view + version.
      askStreaming = last.phase === "streaming";
      for (const turn of hydrated) void loadSourceThumbnails(turn.sources);
      await adoptLiveSnapshot(conversationId);
    } catch {
      // Best-effort: leave the (empty) thread as-is on a hydrate failure.
    }
  }

  // Snapshot-on-attach: fetch the conversation's in-flight LiveTurn (if any) and
  // adopt it onto the matching turn, bootstrapping a reattach to a streaming turn
  // race-free. `ask_ai_update` events at/below the adopted version are then
  // ignored, and a version gap re-snapshots. A null snapshot means no live turn
  // (all from DB) — nothing to do.
  async function adoptLiveSnapshot(conversationId: string): Promise<void> {
    let snapshot: TurnSnapshot | null;
    try {
      snapshot = await invoke<TurnSnapshot | null>("ask_ai_snapshot", {
        request: { conversationId },
      });
    } catch {
      return;
    }
    if (askConversationId !== conversationId) return;
    if (snapshot === null) {
      // No live turn: the conversation is fully finalized. A persisted
      // "streaming" last turn may have left `askStreaming = true` during
      // hydrate (finalize-race); clear it so the follow-up composer isn't
      // left permanently disabled.
      askStreaming = false;
      return;
    }
    const turn = askTurns.find((t) => t.turnIndex === snapshot.view.turnIndex);
    if (!turn) return;
    adoptView(turn, snapshot.view, snapshot.version);
    askStreaming = turn.phase !== "done" && turn.phase !== "error";
  }

  // Replace a turn's render fields from a backend `TurnView` and stamp its
  // version. Used by snapshot-on-attach and version-gap re-snapshot.
  function adoptView(turn: AskTurn, view: TurnView, version: number): void {
    turn.phase = normalizeAskPhase(view.phase);
    turn.blocks = view.blocks;
    turn.reasoning = view.reasoning;
    turn.toolActivities = view.toolActivities;
    turn.liveActivity = view.liveActivity;
    turn.sources = coerceSources(view.sources);
    turn.errorMessage = view.errorMessage;
    turn.seededResultCount = view.seededResultCount;
    turn.version = version;
    void loadSourceThumbnails(turn.sources);
  }

  // ── Versioned update transport (the SOLE Ask AI stream listener) ─────────
  // The backend owns the render model and streams versioned `TurnUpdate` ops via
  // `ask_ai_update`. The frontend applies each op in order; a version gap (we
  // were detached, e.g. backgrounded) self-heals from `ask_ai_snapshot`.
  //
  // `applyUpdate` mirrors the Rust `apply_update_to_view` reducer EXACTLY so a
  // live stream and a snapshot/reload can never diverge. Mutating the $state turn
  // object here is safe (it runs in an event callback, not during render — unlike
  // the Svelte 5 render-memo gotcha that bans writes-from-template).
  function applyUpdate(turn: AskTurn, update: TurnUpdate): void {
    switch (update.op) {
      case "phase":
        turn.phase = normalizeAskPhase(update.phase);
        break;
      case "appendProse": {
        const last = turn.blocks[turn.blocks.length - 1];
        if (last && last.kind === "prose") {
          // Replace the last block so the $state array notices the change.
          turn.blocks = [
            ...turn.blocks.slice(0, -1),
            { kind: "prose", markdown: last.markdown + update.text },
          ];
        } else {
          turn.blocks = [...turn.blocks, { kind: "prose", markdown: update.text }];
        }
        break;
      }
      case "openBlock":
        turn.blocks = [...turn.blocks, update.block];
        break;
      case "reasoning":
        turn.reasoning = (turn.reasoning ?? "") + update.text;
        // Reasoning streamed LIVE shows as an always-expanded panel; once the
        // answer starts it settles into the collapsed "Thought process" chip.
        // Mark it expanded so a reader mid-thought isn't collapsed out from under
        // them on settle — they can still collapse it manually. Only set on the
        // live stream (this op), never on snapshot/hydrate (adoptView), so a
        // cold-reloaded turn stays collapsed.
        turn.reasoningExpanded = true;
        break;
      case "toolActivity":
        turn.toolActivities = [...turn.toolActivities, update.entry];
        break;
      case "liveActivity":
        turn.liveActivity = update.entry;
        break;
      case "sources":
        turn.sources = coerceSources(update.sources);
        void loadSourceThumbnails(turn.sources);
        break;
      case "error":
        turn.errorMessage = update.message;
        turn.phase = "error";
        break;
      case "done":
        turn.phase = "done";
        turn.liveActivity = null;
        break;
    }
  }

  // Apply one `ask_ai_update` event to the active thread, honouring the version
  // contract: exactly-next applies the op; a gap re-snapshots (or re-hydrates if
  // the turn already finalized); stale/duplicate is ignored. The thread id is the
  // stale-thread guard, and `event.turnIndex` keys the live turn.
  async function handleAskUpdateEvent(event: AskAiUpdateEvent): Promise<void> {
    if (event.conversationId !== askConversationId) return;
    const turn = askTurns.find((t) => t.turnIndex === event.turnIndex);
    if (!turn) return; // start/followup appends the in-flight turn locally.

    if (event.version === turn.version + 1) {
      applyUpdate(turn, event.update);
      turn.version = event.version;
      reconcileAskStreaming(turn);
      return;
    }
    if (event.version <= turn.version) return; // already applied / stale.

    // Gap: we missed updates (detached). Self-heal from the live snapshot.
    const conversationId = event.conversationId;
    let snapshot: TurnSnapshot | null;
    try {
      snapshot = await invoke<TurnSnapshot | null>("ask_ai_snapshot", {
        request: { conversationId },
      });
    } catch {
      return;
    }
    if (askConversationId !== conversationId) return;
    if (snapshot !== null && snapshot.view.turnIndex === turn.turnIndex) {
      adoptView(turn, snapshot.view, snapshot.version);
      reconcileAskStreaming(turn);
      return;
    }
    // Snapshot is null (turn already finalized/removed server-side) — fall back
    // to get_conversation and re-hydrate the matching turn.
    try {
      const convo = await invoke<Conversation | null>("get_conversation", {
        conversationId,
      });
      if (convo === null || askConversationId !== conversationId) return;
      const fresh = convo.turns.find((t) => t.turnIndex === turn.turnIndex);
      if (fresh) {
        const hydrated = hydrateAskTurn(fresh);
        askTurns = askTurns.map((t) =>
          t.turnIndex === turn.turnIndex ? hydrated : t,
        );
        reconcileAskStreaming(hydrated);
      }
    } catch {
      // Best-effort: leave the turn as-is.
    }
  }

  // Keep the thread-level streaming flag in sync with the active (last) turn
  // after an update settles it. Only the live (last) turn drives the composer.
  function reconcileAskStreaming(turn: AskTurn): void {
    if (turn.turnIndex !== askTurns.length - 1) return;
    askStreaming = turn.phase !== "done" && turn.phase !== "error";
  }

  // Begin a FRESH Ask AI thread with its first (seeded) turn. `question` is what
  // gets answered; `seedQuery` seeds the broker search (the prior Quick Search
  // query, or null). Cancels any in-flight thread and resets the transcript.
  async function startAsk(question: string, seedQuery: string | null): Promise<void> {
    const trimmedQuestion = question.trim();
    if (trimmedQuestion.length === 0) {
      return;
    }

    // Tear down any existing thread before starting a new one. This must fire
    // even when the prior thread is NOT streaming: after a turn completes the
    // backend session stays resident (waiting for a follow-up), so replacing
    // `askConversationId` without cancelling would orphan that PI process.
    if (askConversationId !== null) {
      await cancelActiveAsk();
    }

    // Normalize and record the seed so an error "Retry" reuses it exactly.
    const normalizedSeed =
      seedQuery && seedQuery.trim().length > 0 ? seedQuery.trim() : null;
    askLastSeed = normalizedSeed;
    askFirstQuestion = trimmedQuestion;

    const conversationId = crypto.randomUUID();
    askConversationId = conversationId;
    askSubmitted = true;
    // Fresh thread: reset the transcript to a single seeding turn. A new ask's
    // outcome has not been seen yet — re-arm so it survives dismiss/blur until
    // the user lays eyes on its terminal turn (one conversation, newest wins).
    askOutcomeSeen = false;
    askStopped = false;
    clearAskCopiedTimers();
    askTurns = [makeAskTurn(0, trimmedQuestion, "seeding")];
    askStreaming = true;
    // The backend OWNS persistence: ask_ai_start upserts the conversation row
    // (from title/origin) and run_ask_ai_turn persists each turn.
    const title = conversationTitle();

    try {
      await invoke<void>("ask_ai_start", {
        request: {
          conversationId,
          question: trimmedQuestion,
          seedQuery: normalizedSeed,
          origin: "quick_recall",
          title,
          ...askAiClock(),
        },
      });
    } catch (error) {
      // A start that never streamed: ignore stale (superseded) failures.
      if (askConversationId !== conversationId) {
        return;
      }
      askStreaming = false;
      const last = askTurns[askTurns.length - 1];
      if (last) {
        last.phase = "error";
        last.errorMessage = humanizeError(error);
      }
    }
  }

  const CONVERSATION_TITLE_MAX = 80;

  // Trim/truncate a question into a conversation title (matches Chat.svelte).
  function titleFromQuestion(question: string): string {
    const t = question.trim().replace(/\s+/g, " ");
    return t.length > CONVERSATION_TITLE_MAX
      ? `${t.slice(0, CONVERSATION_TITLE_MAX - 1)}…`
      : t;
  }

  // The thread title is the first question (stable across follow-ups), so both
  // doors show the same row label. Passed to ask_ai_start so the backend upserts
  // the conversation row with it. Falls back to the turn-1 question when
  // askFirstQuestion hasn't been recorded yet.
  function conversationTitle(): string {
    const first =
      askFirstQuestion.trim().length > 0
        ? askFirstQuestion
        : (askTurns[0]?.question ?? "");
    return titleFromQuestion(first);
  }

  // Whether the current thread has at least one completed (done) turn, gating the
  // "Open in Chat" affordance — the handoff promotes a real, answered thread.
  let askCanOpenInChat = $derived(
    askContinuableConversationId !== null && askHasCompletedTurn,
  );

  // Promote the current Quick Recall thread into the Insights → Chat workspace.
  // The thread is already persisted under askConversationId (origin
  // "quick_recall", written backend-side), so this just shows/navigates the main
  // window to Insights → Chat and selects this conversation; Chat hydrates it via
  // get_conversation and continues it seamlessly. Mirrors the Answer Sources
  // hand-off (open_capture_result_in_main_window), which also dismisses the Quick
  // Recall window — so we do the same here for consistency.
  async function openInChat(): Promise<void> {
    const conversationId = askContinuableConversationId;
    if (conversationId === null || !askHasCompletedTurn) {
      return;
    }
    try {
      await invoke("open_conversation_in_chat", { conversationId });
    } catch (err) {
      // Leave the Quick Recall thread open AND tell the user, so the action
      // doesn't appear to do nothing when the hand-off rejects.
      await message(
        `Couldn't open this in Chat: ${humanizeError(err, "please try again.")}`,
        { title: "Couldn't continue in Chat", kind: "error" },
      );
      return;
    }
    await closeCurrentWindow();
  }

  // Submit a follow-up question. The backend reloads conversation history from
  // the store server-side, so a follow-up ALWAYS works — even on a thread whose
  // last turn errored. There is no client-side resurrect.
  async function submitFollowup(): Promise<void> {
    const trimmed = followupInput.trim();
    if (trimmed.length === 0) {
      return;
    }
    const conversationId = askContinuableConversationId;
    if (conversationId === null || askStreaming) {
      return;
    }
    // Resuming a STOPPED thread: re-adopt its persisted id as the live thread so
    // the new turn's streaming `ask_ai_update` events match the id guard again.
    if (askConversationId === null) {
      askConversationId = conversationId;
      stoppedConversationId = null;
      askStopped = false;
    }

    followupInput = "";
    // The new follow-up turn's outcome hasn't been seen yet — re-arm so it
    // survives dismiss/blur until the user lays eyes on its terminal turn.
    askOutcomeSeen = false;
    const turnIndex = askTurns.length;
    askTurns = [...askTurns, makeAskTurn(turnIndex, trimmed, "thinking")];
    askStreaming = true;
    // The composer is about to disable (dimmed) — move focus to the transcript
    // region so Escape (back-to-search) and scroll keys keep working while the
    // follow-up streams. The composer refocuses on done via the effect below.
    await tick();
    askAreaEl?.focus();

    try {
      await invoke<void>("ask_ai_followup", {
        request: { conversationId, question: trimmed, ...askAiClock() },
      });
    } catch (error) {
      // The thread moved on (Escape / fresh ask) — drop a stale failure.
      if (askConversationId !== conversationId) {
        return;
      }
      askStreaming = false;
      const turn = askTurns[turnIndex];
      if (turn) {
        turn.phase = "error";
        turn.errorMessage = humanizeError(error);
      }
    }
  }

  async function cancelActiveAsk(): Promise<void> {
    const conversationId = askConversationId;
    if (conversationId === null) {
      return;
    }
    askStreaming = false;
    // Drop the thread id so any buffered ask_ai_update events that arrive AFTER
    // cancel (updates the backend already queued) no longer match the id guard in
    // handleAskUpdateEvent and stop applying to the cancelled turn.
    askConversationId = null;
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
      // operator query. Focus the transcript region (no text input renders) so
      // Escape/scroll keys are caught.
      askInput = "";
      void startAsk(question, seed);
      await tick();
      askAreaEl?.focus();
    } else {
      // Unseeded: show an empty ask input for the user to type a question.
      askInput = "";
      askSubmitted = false;
      clearAskCopiedTimers();
      askTurns = [];
      await tick();
      askInputEl?.focus();
    }
  }

  // Submit the typed question from the unseeded ask input (Enter). This starts
  // the thread's first (unseeded) turn; focus moves to the transcript region.
  async function submitAskInput(): Promise<void> {
    const typed = askInput.trim();
    if (typed.length === 0) {
      return;
    }
    void startAsk(typed, null);
    await tick();
    askAreaEl?.focus();
  }

  // Re-run a failed first turn as a FRESH thread with the same question + seed.
  async function retryAsk(): Promise<void> {
    const question = askFirstQuestion;
    if (question.trim().length === 0) {
      return;
    }
    void startAsk(question, askLastSeed);
    await tick();
    askAreaEl?.focus();
  }

  // A turn's copyable raw Markdown: the concatenation of its prose blocks (the
  // graphical blocks have no useful clipboard form), joined with blank lines.
  // The backend's render model carries answer text as `prose` AnswerBlocks now,
  // so there is no single `answer` string to copy.
  function turnAnswerText(turn: AskTurn): string {
    return turn.blocks
      .filter((b): b is { kind: "prose"; markdown: string } => b.kind === "prose")
      .map((b) => b.markdown)
      .join("\n\n")
      .trim();
  }

  // Copy ONE turn's raw Markdown answer (not rendered HTML) to the clipboard,
  // flashing that turn's copy button. Keyed by the turn's array index so the
  // flash stays scoped to the copied turn.
  async function copyTurnAnswer(turnIndex: number): Promise<void> {
    const turn = askTurns[turnIndex];
    const text = turn ? turnAnswerText(turn) : "";
    if (!turn || text.length === 0) {
      return;
    }
    // Arm a single per-turn flash timer that resets BOTH flash flags. The
    // success and failure flashes share one timer keyed by turnIndex, so a
    // fail-then-success (or vice versa) within the flash window must clear the
    // other flag too — otherwise the surviving flag leaves the button stuck on
    // the wrong state.
    const armFlashReset = () => {
      const existing = askCopiedTimers.get(turnIndex);
      if (existing !== undefined) {
        clearTimeout(existing);
      }
      askCopiedTimers.set(
        turnIndex,
        setTimeout(() => {
          const t = askTurns[turnIndex];
          if (t) {
            t.copied = false;
            t.copyFailed = false;
          }
          askCopiedTimers.delete(turnIndex);
        }, 1500),
      );
    };
    try {
      await navigator.clipboard.writeText(text);
    } catch {
      // Clipboard write rejected: flash the button red so the failure isn't
      // silent, reusing the same per-turn flash-timer machinery as success.
      turn.copyFailed = true;
      turn.copied = false;
      armFlashReset();
      return;
    }
    turn.copied = true;
    turn.copyFailed = false;
    armFlashReset();
  }

  function toggleTurnSummary(turn: AskTurn): void {
    turn.summaryExpanded = !turn.summaryExpanded;
  }

  function toggleTurnReasoning(turn: AskTurn): void {
    turn.reasoningExpanded = !turn.reasoningExpanded;
  }

  // The "Thinking" disclosure renders only once reasoning text has arrived. It
  // is LIVE (an always-expanded streaming panel with a pulsing "Thinking…"
  // header) while reasoning has streamed but the answer hasn't started and the
  // turn isn't terminal; otherwise it SETTLES into the collapsed "Thought
  // process" chip below.
  function hasReasoning(turn: AskTurn): boolean {
    return (turn.reasoning ?? "").trim().length > 0;
  }
  function reasoningIsLive(turn: AskTurn): boolean {
    return (
      (turn.reasoning ?? "").trim().length > 0 &&
      turn.blocks.length === 0 &&
      turn.phase !== "done" &&
      turn.phase !== "error"
    );
  }

  // ⌘C / Ctrl+C copies the LIVE (last) turn's answer when the transcript region
  // is focused, that turn is done, and nothing is selected. With a selection,
  // let native copy run.
  function handleAnswerAreaKeydown(event: KeyboardEvent): void {
    const last = askTurns[askTurns.length - 1];
    if (
      (event.metaKey || event.ctrlKey) &&
      (event.key === "c" || event.key === "C") &&
      last &&
      last.phase === "done" &&
      turnAnswerText(last).length > 0
    ) {
      const selection = window.getSelection()?.toString() ?? "";
      if (selection.length === 0) {
        event.preventDefault();
        void copyTurnAnswer(askTurns.length - 1);
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

  // Follow-up composer: Enter submits, Shift+Enter inserts a newline (mirrors
  // the unseeded ask input). A guard keeps Enter inert while a turn streams (the
  // composer is disabled then anyway, but a stray keydown shouldn't submit).
  function handleFollowupKeydown(event: KeyboardEvent): void {
    // Keep the footer-advertised ⌃C copy working even though focus parks on the
    // composer (a sibling of the answer area that owns handleAnswerAreaKeydown)
    // after an answer settles. Only act when the textarea has no active text
    // selection, so a normal "select-then-copy" inside the composer still runs
    // native copy.
    if (
      (event.metaKey || event.ctrlKey) &&
      (event.key === "c" || event.key === "C")
    ) {
      const el = event.currentTarget as HTMLTextAreaElement;
      const hasComposerSelection = el.selectionStart !== el.selectionEnd;
      const last = askTurns[askTurns.length - 1];
      if (
        !hasComposerSelection &&
        last &&
        last.phase === "done" &&
        turnAnswerText(last).length > 0
      ) {
        event.preventDefault();
        void copyTurnAnswer(askTurns.length - 1);
      }
      return;
    }
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      if (!askStreaming) {
        void submitFollowup();
      }
    }
  }

  // Auto-grow a composer textarea to fit its content (the standard chat-composer
  // affordance): reset to one row, then expand to scrollHeight capped at ~5 rows,
  // after which it scrolls. Driven reactively off the bound value so the field
  // reflects multi-line questions as they're typed (and collapses on clear).
  const ASK_TEXTAREA_MAX_PX = 112;
  function autosizeAskTextarea(el: HTMLTextAreaElement | null): void {
    if (el === null) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, ASK_TEXTAREA_MAX_PX)}px`;
    el.style.overflowY =
      el.scrollHeight > ASK_TEXTAREA_MAX_PX ? "auto" : "hidden";
  }

  // Re-send a failed FOLLOW-UP turn's question through ask_ai_followup. The PI
  // session stays resident after a follow-up error, so this appends a fresh
  // attempt (mirroring a normal follow-up) rather than rebuilding the thread the
  // way retryAsk does for a turn-1 failure.
  async function retryFollowupTurn(turn: AskTurn): Promise<void> {
    // Guard on the *continuable* id, not askConversationId: a Stop nulls
    // askConversationId and parks the thread under stoppedConversationId, but the
    // empty-answer "Try asking again" Retry is still offered. submitFollowup()
    // re-adopts the stopped thread, so keying off askConversationId here makes
    // Retry a dead click on a stopped-empty follow-up turn.
    if (askStreaming || askContinuableConversationId === null) {
      return;
    }
    followupInput = turn.question;
    await submitFollowup();
  }

  // Stop a streaming answer in place: cooperatively cancel the turn and settle it
  // to `done` so its partial answer stays rendered and copyable, rather than
  // abandoning the whole surface the way Escape does. `cancelActiveAsk` nulls the
  // live id (so late buffered updates stop applying), but the backend has already
  // persisted the partial as an ordinary `done` turn — so we stash that id in
  // `stoppedConversationId` to keep the thread continuable ("Continue in Chat" +
  // the follow-up composer both re-point at it; a follow-up re-adopts it live).
  async function stopActiveAsk(): Promise<void> {
    const live = askLiveTurn;
    const conversationId = askConversationId;
    await cancelActiveAsk();
    stoppedConversationId = conversationId;
    if (live && live.phase !== "done" && live.phase !== "error") {
      live.phase = "done";
      live.liveActivity = null;
      // Tag this turn as cut off, so Quick Recall shows "Stopped early" on it.
      live.stoppedEarly = true;
    }
    askStopped = true;
  }

  // Tear down all ephemeral thread state (transcript, ids, timers, inputs). Does
  // NOT cancel the live session — callers route through cancelActiveAsk first.
  function resetAskThreadState(): void {
    clearAskCopiedTimers();
    askConversationId = null;
    stoppedConversationId = null;
    askTurns = [];
    askSubmitted = false;
    askInput = "";
    followupInput = "";
    askLastSeed = null;
    askFirstQuestion = "";
    askStreaming = false;
    askOutcomeSeen = false;
    askStopped = false;
    sourceHandoffPending = false;
  }

  // Return to search mode, abandoning the whole thread (a single Escape drops
  // the user back to search from anywhere, even with half-typed composer text).
  async function backToSearch(): Promise<void> {
    await cancelActiveAsk();
    mode = "search";
    resetAskThreadState();
    await tick();
    inputEl?.focus();
  }

  $effect(() => {
    scheduleSearch(query);
  });

  // Keep both Ask AI composer textareas grown to their content. Reading the bound
  // value + the element ref tracks typing AND programmatic clears / first mount.
  $effect(() => {
    void askInput;
    autosizeAskTextarea(askInputEl);
  });
  $effect(() => {
    void followupInput;
    autosizeAskTextarea(followupInputEl);
  });

  // Keep the live (last) turn and the composer in view as the transcript grows:
  // pin the scroll region to the bottom on each delta and whenever a new turn is
  // appended. Reads the live turn's answer length + the turn count so the effect
  // re-runs on both streaming growth and new follow-up turns.
  //
  // FIX #6: the bottom-pin is coalesced into a single requestAnimationFrame so a
  // burst of streamed deltas writes `scrollTop` at most once per frame instead of
  // reading `scrollHeight` + writing `scrollTop` on every token (which forced a
  // synchronous reflow per delta). A pending frame is reused/cancelled so we never
  // queue more than one outstanding scroll.
  let pendingScrollFrame: number | null = null;
  $effect(() => {
    const live = askLiveTurn;
    // Touch reactive deps so the effect tracks streaming + turn-append growth.
    // appendProse replaces the blocks array (and its last prose block) on every
    // delta, so tracking the block count + the last prose block's length pins the
    // panel as the answer streams. Reasoning length is tracked too so the live
    // "Thinking…" panel pins as it streams (it streams before any answer text).
    const _blockCount = live?.blocks.length ?? 0;
    const _lastBlock = live?.blocks[_blockCount - 1];
    const _len =
      _lastBlock && _lastBlock.kind === "prose" ? _lastBlock.markdown.length : 0;
    const _reasoningLen = live?.reasoning?.length ?? 0;
    const _count = askTurns.length;
    if (mode !== "ask" || !askAreaEl || _count === 0) {
      return;
    }
    // Collapse a burst of deltas into one outstanding frame: while a scroll frame
    // is already queued, further effect runs just ride it (the frame reads the
    // freshest scrollHeight when it fires), so scrollTop is written at most once
    // per animation frame regardless of how many tokens arrived.
    if (pendingScrollFrame !== null) {
      return;
    }
    pendingScrollFrame = requestAnimationFrame(() => {
      pendingScrollFrame = null;
      if (mode === "ask" && askAreaEl) {
        askAreaEl.scrollTop = askAreaEl.scrollHeight;
      }
    });
  });

  // After a follow-up finishes streaming, return focus to the (now re-enabled)
  // composer so the user can immediately type the next follow-up. Only does so
  // when focus is parked on the transcript region (where submitFollowup left it)
  // so it never yanks focus away from a manual selection or another element.
  $effect(() => {
    if (
      mode === "ask" &&
      askComposerVisible &&
      !askStreaming &&
      followupInputEl &&
      document.activeElement === askAreaEl
    ) {
      followupInputEl.focus();
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

  // Split `raw` into whitespace-delimited tokens, but keep a double-quoted run
  // as ONE token so a quoted operator value containing spaces (e.g.
  // `app:"Google Chrome"`) is never shredded mid-value. An unterminated quote
  // still flushes its trailing run rather than dropping it.
  function tokenizeQuery(raw: string): string[] {
    const tokens: string[] = [];
    let current = "";
    let inQuotes = false;
    for (const ch of raw) {
      if (ch === '"') {
        inQuotes = !inQuotes;
        current += ch;
      } else if (!inQuotes && /\s/.test(ch)) {
        if (current.length > 0) {
          tokens.push(current);
          current = "";
        }
      } else {
        current += ch;
      }
    }
    if (current.length > 0) {
      tokens.push(current);
    }
    return tokens;
  }

  // The operator values (lowercased, unquoted) that identify THIS chip's token,
  // or null for a date chip — which owns the whole `date:`/`after:`/`before:`
  // range and so removes its family wholesale. Source chips accept their short
  // and long spellings so a `source:mic` token still matches a microphone chip.
  function chipTokenValues(chip: ActiveFilterChip): string[] | null {
    switch (chip.kind) {
      case "app":
        return [chip.data.value.toLowerCase()];
      case "source":
        if (chip.data.source === "screen") return ["screen"];
        return chip.data.source === "microphone"
          ? ["microphone", "mic"]
          : ["system", "system_audio"];
      case "date":
        return null;
    }
  }

  // Whether `token` is an operator token of one of `prefixes` whose unquoted,
  // lowercased value is one of `values`.
  function tokenMatchesChipValue(
    token: string,
    prefixes: string[],
    values: string[],
  ): boolean {
    const lower = token.toLowerCase();
    const prefix = prefixes.find((p) => lower.startsWith(p));
    if (prefix === undefined) {
      return false;
    }
    const unquoted = token.slice(prefix.length).replace(/^"(.*)"$/, "$1").toLowerCase();
    return values.includes(unquoted);
  }

  // Remove a single chip's operator token(s) from `raw`, quote-aware. Prefers a
  // TARGETED removal — drop only the token whose value matches this chip, so
  // removing one `app:`/`source:` chip leaves any sibling chips of the same kind
  // intact. The query may carry the user's own spelling, which can differ from
  // the backend's desugared value, so when no token matches we fall back to
  // dropping every token of that operator family (the original defensive
  // behavior) rather than leaving the chip un-removable. Either path is
  // quote-aware, so `app:"Google Chrome"` is removed cleanly instead of leaving
  // a dangling `Chrome"`. Pure: used by removeChip.
  function stripChipTokens(raw: string, chip: ActiveFilterChip): string {
    const tokens = tokenizeQuery(raw);
    const prefixes = operatorPrefixesForChip(chip);

    const values = chipTokenValues(chip);
    if (values !== null) {
      const index = tokens.findIndex((token) =>
        tokenMatchesChipValue(token, prefixes, values),
      );
      if (index >= 0) {
        tokens.splice(index, 1);
        return tokens.join(" ").trim();
      }
    }

    // Fallback: drop every token of this operator family (spelling-tolerant).
    const kept = tokens.filter((token) => {
      const lower = token.toLowerCase();
      return !prefixes.some((prefix) => lower.startsWith(prefix));
    });
    return kept.join(" ").trim();
  }

  // Drop a chip's operator token(s) from the query and let the reactive effect
  // rerun the search (which re-derives chips and restores sections via
  // sectionLimits). Refocus the input so removal keeps keyboard flow.
  function removeChip(chip: ActiveFilterChip): void {
    query = stripChipTokens(query, chip);
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
  // input's keydown drives).
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
      hint: "A day, a preset, or a typed range",
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
            return `"${name}"\0`;
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
  let ghostIsQuotedAppValue = $derived(ghostRaw !== null && ghostRaw.endsWith("\0"));

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

  // In-search discoverability hint (issue #125): when no Semantic Search Model is
  // installed, search is keyword-only. Surface a one-time hint pointing the user
  // to Settings so they can turn on meaning-based search. We load the model
  // status lazily and treat "no model installed" as "no model available".
  let semanticSearchModelInstalled = $state<boolean | null>(null);
  async function loadSemanticSearchModelInstalled(): Promise<void> {
    try {
      const status = await invoke<SemanticSearchModelStatusResponse>(
        "get_semantic_search_model_status",
      );
      semanticSearchModelInstalled = status.models.some((model) => model.available);
    } catch {
      // Best-effort: a failure just suppresses the hint (never blocks search).
      semanticSearchModelInstalled = null;
    }
  }
  async function openSemanticSearchSettings(): Promise<void> {
    await openSettings("semanticSearch");
  }
  // Route the unavailable-Ask-AI hint to the Intelligence pane (providers + Ask
  // AI + Reasoning Engine), so the friendly "do X in Settings" reason becomes
  // actionable from here instead of a dead end.
  async function openAskAiSettings(): Promise<void> {
    await openSettings("intelligence");
  }
  // Show the hint once results have run and no model is installed — the hint is
  // most useful exactly when keyword-only search underwhelms.
  let showSemanticSearchHint = $derived(
    semanticSearchModelInstalled === false &&
      !belowMinimum &&
      !loading &&
      parseErrorMessage === null &&
      resultsQuery.length > 0,
  );

  // Slice 3: results are PAUSED (not empty, not errored) when the backend
  // returned a parse error for an at/above-minimum query that isn't mid-flight.
  // The backend suppresses results in this case; this branch renders a calm
  // "fix the filter" state in place of stale cards or the bare empty state.
  let resultsPaused = $derived(
    !belowMinimum && !loading && !errorMessage && parseErrorMessage !== null,
  );

  // Screen-reader announcement for the results region. The cards live in an
  // aria-activedescendant listbox whose count/loading/empty/error transitions
  // are otherwise silent to AT; this polite live-region string mirrors the
  // visible state so a result-count change (or a switch to loading/no-matches/
  // error) is spoken. Branch order matches the results markup so the spoken
  // state and the rendered state never disagree.
  let searchStatusAnnouncement = $derived.by((): string => {
    if (mode !== "search" || belowMinimum) {
      return "";
    }
    if (loading) {
      return "Searching…";
    }
    if (errorMessage) {
      return errorMessage;
    }
    if (resultsPaused) {
      return "Results paused — fix the filter above to search.";
    }
    if (showEmpty) {
      return `No matches for ${resultsQuery}.`;
    }
    return `${resultCount} ${resultCount === 1 ? "result" : "results"} for ${resultsQuery}.`;
  });

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
    // Idle-window expiry tears down the whole ask thread (the live session was
    // already killed by cancelActiveAsk above).
    resetAskThreadState();
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
  //
  // Background completion (PLAN.md slice 1): a pending Ask AI conversation
  // (in-flight, or finished-but-unseen) must NEVER be destroyed by the 5s blur
  // idle-clear. `operationRunning` already suspends the timer while a turn
  // streams; `askConversationPending` extends that to a finished-unseen answer
  // so it waits for the user instead of being wiped after 5s of blur. Once the
  // conversation is seen, the ordinary idle-clear applies unchanged, and
  // search-mode ephemerality is untouched.
  $effect(() => {
    if (
      !windowFocused &&
      hasClearableState &&
      !operationRunning &&
      !askConversationPending &&
      !sourceHandoffPending
    ) {
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
    void loadSemanticSearchModelInstalled();
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
    let unlistenUpdate: (() => void) | undefined;
    let unlistenFocus: (() => void) | undefined;
    let unlistenDismiss: (() => void) | undefined;
    let unlistenSettings: (() => void) | undefined;
    let unlistenSemanticSearchDownload: (() => void) | undefined;

    // The window is hidden/re-shown rather than recreated across summons, so
    // re-grab focus each time it becomes key — onMount alone fires only once.
    getCurrentWindow()
      .onFocusChanged(({ payload: focused }) => {
        windowFocused = focused;
        if (focused) {
          // A source hand-off kept the thread alive across the close; now that the
          // user re-summoned, return to ordinary ephemeral rules (the in-memory
          // thread is still present, so it renders immediately — no re-hydrate).
          sourceHandoffPending = false;
          // The panel is hidden/re-shown rather than recreated, so re-probe Ask
          // AI availability each time it becomes key — onMount fires only once,
          // and the user may have enabled Ask AI or fixed PI/auth since the last
          // summon. Without this the stale disabled hint would persist forever.
          void loadAskAvailability();
          // Same staleness applies to the "turn on meaning search" hint: a model
          // can be installed/removed in Settings while this (reused) window stays
          // hidden, so re-probe model status on focus too — otherwise the hint
          // keeps showing keyword-only long after a model is installed.
          void loadSemanticSearchModelInstalled();
          // The captured-app set grows as new apps are seen after launch, but the
          // session cache pins the launch-time set — so apps first captured this
          // session never show in `app:` completion. Invalidate on focus; the lazy
          // loader (kicked by the `app:` derivation) repopulates on the next use.
          searchableApps = null;
          // Re-summon hydration: if an ask thread is armed but its in-memory
          // transcript was cleared, reload it from the store so a finished (or
          // still-streaming) answer reappears. Background completion is
          // server-side, so the persisted turn is authoritative.
          if (mode === "ask" && askConversationId !== null && askTurns.length === 0) {
            void hydrateAskFromStore(askConversationId);
          }
          void tick().then(() => focusQuickRecall());
        }
      })
      .then((fn) => {
        if (destroyed) fn();
        else unlistenFocus = fn;
      });

    // The SOLE Ask AI stream listener: versioned render-model updates for the
    // active thread (issue #110, ADR 0031). Stale-thread + version guards live in
    // handleAskUpdateEvent — exactly-next applies the op, a gap self-heals from
    // ask_ai_snapshot, stale/duplicate is dropped. This replaces the six legacy
    // per-token listeners (status/delta/reasoning/done/error/source).
    listen<AskAiUpdateEvent>("ask_ai_update", (event) => {
      void handleAskUpdateEvent(event.payload);
    }).then((fn) => {
      if (destroyed) fn();
      else unlistenUpdate = fn;
    });

    // The Rust dismiss chokepoint (`dismiss_quick_recall_window`) emits this when
    // the panel is ordered out / hidden. The webview is NOT destroyed on dismiss,
    // so the component `onDestroy` does not run.
    //
    // Background completion is server-side now: an unseen or in-flight Ask AI
    // conversation must SURVIVE dismiss so re-summon lands back on it. The backend
    // finishes and PERSISTS the turn regardless of the window, so we never cancel
    // an in-flight/unseen thread on dismiss — re-summon hydrates it from the store
    // if needed. We only tear the thread down (reset to search) once the
    // conversation is seen, or when there is no conversation at all — restoring
    // today's ephemeral search behavior. App exit / onDestroy still tears down.
    listen("quick_recall_dismissed", () => {
      if (askConversationPending || sourceHandoffPending) {
        return;
      }
      void cancelActiveAsk().then(() => {
        resetAskThreadState();
        mode = "search";
      });
    }).then((fn) => {
      if (destroyed) fn();
      else unlistenDismiss = fn;
    });

    // Settings saved in the Settings window broadcast
    // `recording_settings_domain_changed` ({ domain, settings }). The Semantic
    // Search toggle + model selection live in the `semantic_search` domain, so a
    // model selection/toggle made while this (reused) window stays focused must
    // re-probe model status — otherwise the "turn on meaning search" hint goes
    // stale exactly as the onFocusChanged re-probe of Ask AI availability would
    // (focus never changes in this in-place case). The focus re-probe above
    // covers the download-then-refocus case; this covers the focus-stays case.
    listen<RecordingSettingsDomainUpdateResponse>(
      "recording_settings_domain_changed",
      (event) => {
        if (event.payload.domain !== "semantic_search") return;
        void loadSemanticSearchModelInstalled();
      },
    ).then((fn) => {
      if (destroyed) fn();
      else unlistenSettings = fn;
    });

    // A model download started in Settings emits progress here as it runs. The
    // settings-domain listener above only fires on a settings save, not when a
    // background download FINISHES — so if a download completes while this
    // (reused) window stays focused, the "turn on meaning search" hint would keep
    // showing "download"/keyword-only until the next focus change. Re-probe model
    // status when the download reaches a terminal state (completed/failed/
    // cancelled) so the hint reflects the now-installed model live. Mirrors the
    // settings page handler (`handleSemanticSearchDownloadProgress`).
    listen<SemanticSearchModelDownloadProgress>(
      "semantic_search_model_download_progress",
      (event) => {
        if (
          ["completed", "failed", "cancelled"].includes(event.payload.status)
        ) {
          void loadSemanticSearchModelInstalled();
        }
      },
    ).then((fn) => {
      if (destroyed) fn();
      else unlistenSemanticSearchDownload = fn;
    });

    return () => {
      destroyed = true;
      window.removeEventListener("keydown", handleLauncherCaptureKeydown, {
        capture: true,
      });
      motionQuery.removeEventListener("change", onMotionChange);
      unlistenUpdate?.();
      unlistenFocus?.();
      unlistenDismiss?.();
      unlistenSettings?.();
      unlistenSemanticSearchDownload?.();
    };
  });

  onDestroy(() => {
    clearDebounce();
    clearIdleTimer();
    clearAskCopiedTimers();
    // FIX #6: drop any queued bottom-pin scroll frame so it can't fire post-teardown.
    if (pendingScrollFrame !== null) {
      cancelAnimationFrame(pendingScrollFrame);
      pendingScrollFrame = null;
    }
    // Teardown safety: never leave a resident PI session outliving the panel.
    void cancelActiveAsk();
  });
</script>

<!-- Inline app chip for tool-activity lines: the backend-resolved icon (or a
     letter fallback) + the app name, matching the app-icon look used elsewhere.
     The icon path is resolved server-side (entry.appIconPath); here it is a pure
     path→URL conversion — no client resolve/batch. -->
{#snippet toolAppChip(entry: ToolActivityEntry)}
  {@const app = entry.app ?? ""}
  <span class="quick-recall__tool-app">
    <span class="quick-recall__tool-app-icon" aria-hidden="true">
      {#if entry.appIconPath}
        <img src={convertFileSrc(entry.appIconPath)} alt="" />
      {:else}
        {appIconFallback(app, app)}
      {/if}
    </span>
    <span class="quick-recall__tool-app-name">{app}</span>
  </span>
{/snippet}

<!-- The keyword-only hint, guarded by showSemanticSearchHint, shared by the
     empty and results branches so there is one source of truth. -->
{#snippet semanticHint()}
  {#if showSemanticSearchHint}
    <button
      type="button"
      class="quick-recall__semantic-hint"
      onclick={() => void openSemanticSearchSettings()}
    >
      Searching keywords only. Turn on meaning-based search in Settings →
      Processing to also find results by meaning.
    </button>
  {/if}
{/snippet}

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
        <!-- Polite live region announcing result-count / loading / no-matches /
             error transitions to AT, which the aria-activedescendant results
             listbox cannot convey on its own. Visually hidden. -->
        <span class="quick-recall__sr-status" role="status" aria-live="polite"
          >{searchStatusAnnouncement}</span
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
              aria-expanded={pickerOpen || valueListActive || resultCount > 0}
              aria-keyshortcuts="ArrowUp ArrowDown Enter Escape Control+Enter Control+O"
              aria-controls={pickerOpen
                ? "quick-recall-picker"
                : valueListActive
                  ? "quick-recall-value-list"
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
          <!-- Visible Filter Picker trigger: the picker was previously reachable
               only via ⌘F / `/`, advertised nowhere — a funnel in the field row
               gives it a mouse-discoverable door (and surfaces the ⌘F shortcut via
               its title). Click opens the same category picker as the key path; DOM
               focus stays on the search input (openPicker refocuses it), so the
               picker's aria-activedescendant navigation keeps working. -->
          <button
            type="button"
            class="quick-recall__filter-trigger"
            class:quick-recall__filter-trigger--active={pickerOpen}
            onclick={() => (pickerOpen ? closePicker() : openPicker())}
            aria-label="Filter results"
            use:tip={"Filter results (⌘F)"}
            aria-expanded={pickerOpen}
            aria-controls={pickerOpen ? "quick-recall-picker" : undefined}
            aria-keyshortcuts="Control+F"
          >
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
              <path d="M2 3h10l-3.8 4.5v3.2l-2.4 1.2V7.5z" />
            </svg>
          </button>
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
              aria-keyshortcuts="Control+Enter"
            >
              Ask AI <span class="quick-recall__ask-key" aria-hidden="true">⌃↵</span>
            </button>
          {:else}
            <button
              type="button"
              class="quick-recall__ask-button quick-recall__ask-button--disabled"
              disabled
              aria-label={askUnavailableHint ?? "Ask AI unavailable"}
              aria-describedby={ASK_UNAVAILABLE_HINT_ID}
              use:tip={askUnavailableHint ?? "Ask AI unavailable"}
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
                  use:tip={`Remove ${chip.label} filter`}
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

        <!-- Always-present describedby target for the disabled Ask AI button, so
             keyboard/AT users get the reason the native `title` tooltip can't
             surface. Rendered whenever Ask AI is unavailable (mirroring the
             disabled button's condition) with a fallback so the reference never
             dangles before the availability probe resolves. -->
        {#if !askAvailable}
          <button
            type="button"
            id={ASK_UNAVAILABLE_HINT_ID}
            class="quick-recall__ask-hint"
            onclick={() => void openAskAiSettings()}
          >
            {askUnavailableHint ?? "Ask AI unavailable"} — open Settings →
          </button>
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
            id="quick-recall-value-list"
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
            class:quick-recall__results--refetching={loading && hasResults}
            role="listbox"
            aria-label="Search results"
            aria-busy={loading}
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
          {:else if loading && !hasResults}
            <!-- Slice 6: skeleton rows mirroring SearchResultCard's two-column
                 layout (116px 16/10 thumb + stacked text lines). Only the FIRST
                 search (no prior results) shows the full skeleton; a refetch on a
                 subsequent keystroke keeps the prior results visible-but-dimmed
                 (the `--refetching` class on the list) so the surface doesn't flash
                 empty between every keystroke. -->
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
            <!-- A backend search failure offers an explicit recovery (re-issue the
                 same query), mirroring the Ask AI "Retry" so the path isn't a
                 soft dead end the user has to guess at by editing the query. -->
            <p class="quick-recall__state quick-recall__state--error">{errorMessage}</p>
            <div class="quick-recall__retry-row">
              <button
                type="button"
                class="quick-recall__retry"
                onclick={() => void runSearch(resultsQuery)}
              >
                Retry
              </button>
            </div>
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
            <!-- Recovery for a constrained search: when filter chips narrow the
                 query, the most likely fix is loosening a filter, so name that
                 path; offer the Ask AI pivot too so the empty result isn't a dead
                 end. Both are quiet inline actions, not a heavy empty-state block. -->
            {#if activeFilterChips.length > 0 || askAvailable}
              <p class="quick-recall__empty-recovery">
                {#if activeFilterChips.length > 0}
                  Try removing a filter{askAvailable ? ", or " : "."}
                {/if}
                {#if askAvailable}
                  <button
                    type="button"
                    class="quick-recall__empty-action"
                    onclick={() => void activateAskAi()}
                  >
                    ask AI instead <kbd>⌃↵</kbd>
                  </button>
                {/if}
              </p>
            {/if}
            {@render semanticHint()}
          {:else}
            {@render semanticHint()}
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
            aria-keyshortcuts="Escape"
          >
            ← Back
          </button>
          {#if askSubmitted}
            <!-- Once the thread exists the header is just the Back affordance —
                 each turn's question renders as its own header in the transcript. -->
            <span class="quick-recall__ask-thread-label" aria-hidden="true">Ask AI</span>
            {#if askStreaming}
              <!-- Interrupt a streaming answer in place (keeps the partial answer
                   visible) instead of forcing Escape, which abandons the surface. -->
              <button
                type="button"
                class="quick-recall__stop"
                onclick={() => void stopActiveAsk()}
                aria-label="Stop generating"
                use:tip={"Stop generating"}
              >
                <span class="quick-recall__stop-glyph" aria-hidden="true"></span>
                Stop
              </button>
            {/if}
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
              aria-keyshortcuts="Enter Escape"
              onkeydown={handleAskInputKeydown}
            ></textarea>
          {/if}
        </div>

        <!-- Visually-hidden polite live region announcing ONLY the Ask AI phase
             transitions to AT. The transcript below is intentionally NOT a live
             region (token-by-token deltas flooded screen readers); this mirrors
             the search door's `searchStatusAnnouncement` pattern. -->
        <span class="quick-recall__sr-status" role="status" aria-live="polite"
          >{askPhaseAnnouncement}</span
        >

        <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
        <div
          bind:this={askAreaEl}
          class="quick-recall__results quick-recall__answer-area"
          aria-busy={askStreaming}
          tabindex="-1"
          onkeydown={handleAnswerAreaKeydown}
        >
          {#if !askSubmitted}
            <p class="quick-recall__state">Type a question and press Enter to ask.</p>
          {:else}
            <!-- The transcript: a vertical list of self-contained turns. Each
                 renders its own question header, answer, sources, tool-activity
                 summary, and copy/error affordances. Stream events feed the LAST
                 turn (the live one). Keyed by index — turns are append-only. -->
            {#each askTurns as turn, ti (ti)}
              <div class="quick-recall__turn">
                <p class="quick-recall__turn-question" use:tip={turn.question}>
                  {turn.question}
                </p>

                {#if turn.phase === "error"}
                  <p class="quick-recall__state quick-recall__state--error">
                    {turn.errorMessage ?? "Ask AI failed."}
                  </p>
                  <!-- Every errored turn gets a retry. Turn 1 rebuilds the whole
                       thread (same question + seed); a follow-up error re-sends its
                       question through ask_ai_followup on the still-resident session. -->
                  <div class="quick-recall__retry-row">
                    <button
                      type="button"
                      class="quick-recall__retry"
                      disabled={askStreaming}
                      onclick={() => {
                        if (ti === 0) {
                          void retryAsk();
                        } else {
                          void retryFollowupTurn(turn);
                        }
                      }}
                    >
                      Retry
                    </button>
                  </div>
                {:else}
                  <!-- Thinking disclosure: the model's reasoning, ABOVE the
                       answer body. Rendered only when reasoning text arrived.
                       While reasoning streams but the answer hasn't started
                       (and the turn isn't terminal) it's a LIVE expanded panel;
                       otherwise it settles into a collapsed "Thought process"
                       chip. Reasoning is PLAIN TEXT (Svelte-escaped), never
                       routed through AnswerProse, so it reads as distinct. -->
                  {#if hasReasoning(turn)}
                    {#if reasoningIsLive(turn)}
                      <div class="quick-recall__thinking quick-recall__thinking--live">
                        <p class="quick-recall__state quick-recall__state--working">
                          <span class="quick-recall__dot" aria-hidden="true"></span>
                          Thinking…
                        </p>
                        <div class="quick-recall__thinking-text">{turn.reasoning}</div>
                      </div>
                    {:else}
                      <div class="quick-recall__thinking">
                        <button
                          type="button"
                          class="quick-recall__activity-chip"
                          aria-expanded={turn.reasoningExpanded}
                          onclick={() => toggleTurnReasoning(turn)}
                        >
                          <span
                            class="quick-recall__activity-caret"
                            class:quick-recall__activity-caret--open={turn.reasoningExpanded}
                            aria-hidden="true">▸</span
                          >
                          <span class="quick-recall__activity-summary">Thought process</span>
                        </button>
                        {#if turn.reasoningExpanded}
                          <div class="quick-recall__thinking-text">{turn.reasoning}</div>
                        {/if}
                      </div>
                    {/if}
                  {/if}

                  {#if turn.seededResultCount !== null && turn.seededResultCount > 0}
                    <p class="quick-recall__seeded">
                      Seeded with {turn.seededResultCount}
                      {turn.seededResultCount === 1 ? "result" : "results"}
                    </p>
                  {/if}

                  {#if turn.phase === "seeding"}
                    <p class="quick-recall__state quick-recall__state--working">
                      <span class="quick-recall__dot" aria-hidden="true"></span>
                      Searching your captures…
                    </p>
                  {:else if turn.phase === "thinking" && turn.liveActivity === null && !reasoningIsLive(turn)}
                    <p class="quick-recall__state quick-recall__state--working">
                      <span class="quick-recall__dot" aria-hidden="true"></span>
                      Thinking…
                    </p>
                  {:else}
                    {#if turn.phase === "streaming" || turn.phase === "done"}
                      <!-- Per-turn copy: only on done, never while streaming. -->
                      {#if turn.phase === "done" && turnAnswerText(turn).length > 0}
                        <button
                          type="button"
                          class="quick-recall__copy"
                          class:quick-recall__copy--copied={turn.copied}
                          class:quick-recall__copy--failed={turn.copyFailed}
                          onclick={() => void copyTurnAnswer(ti)}
                          aria-label={turn.copyFailed ? "Couldn't copy answer" : "Copy answer"}
                          use:tip={turn.copyFailed ? "Couldn't copy answer" : "Copy answer"}
                        >
                          {#if turn.copyFailed}
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
                              <path d="M3.5 3.5l7 7M10.5 3.5l-7 7" />
                            </svg>
                          {:else if turn.copied}
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
                              <path
                                d="M9.5 4.5V3a1 1 0 0 0-1-1h-5a1 1 0 0 0-1 1v5a1 1 0 0 0 1 1H4"
                              />
                            </svg>
                          {/if}
                        </button>
                      {/if}

                      <!-- Per-turn collapsed, expandable activity summary chip. -->
                      {#if activitySummaryFor(turn.toolActivities) !== null}
                        <div class="quick-recall__activity">
                          <button
                            type="button"
                            class="quick-recall__activity-chip"
                            aria-expanded={turn.summaryExpanded}
                            onclick={() => toggleTurnSummary(turn)}
                          >
                            <span
                              class="quick-recall__activity-caret"
                              class:quick-recall__activity-caret--open={turn.summaryExpanded}
                              aria-hidden="true">▸</span
                            >
                            <span class="quick-recall__activity-summary"
                              >{activitySummaryFor(turn.toolActivities)}</span
                            >
                          </button>
                          {#if turn.summaryExpanded}
                            <ul class="quick-recall__activity-list">
                              {#each turn.toolActivities as activity, ai (ai)}
                                <li class="quick-recall__activity-item">
                                  {activity.label}
                                  {#if activity.app}
                                    in {@render toolAppChip(activity)}
                                  {/if}
                                </li>
                              {/each}
                            </ul>
                          {/if}
                        </div>
                      {/if}

                      <!-- The answer body: render-ready blocks streamed from the
                           backend, switched on `kind`. Prose carries raw markdown
                           (AnswerProse renders + hardens it, owns the code-block
                           copy buttons, external-link delegation through
                           openAnswerLink, and the streaming caret); the graphical
                           blocks carry already-parsed data. The streaming caret
                           rides only the LAST prose block, and only until the turn
                           settles. -->
                      <div class="quick-recall__answer">
                        {#each turn.blocks as block, bi (bi)}
                          {#if block.kind === "prose"}
                            <AnswerProse
                              source={block.markdown}
                              isStreaming={turn.phase !== "done" &&
                                bi === turn.blocks.length - 1}
                              onOpenLink={openAnswerLink}
                            />
                          {:else if block.kind === "bars"}
                            <figure class="quick-recall__graphic">
                              {#if block.title}
                                <figcaption class="quick-recall__graphic-title">
                                  {block.title}
                                </figcaption>
                              {/if}
                              <MiniBars items={block.items} />
                            </figure>
                          {:else if block.kind === "dossier"}
                            <div
                              class="quick-recall__graphic quick-recall__graphic--dossier"
                            >
                              {#each block.items as item, di (di)}
                                <div class="quick-recall__dossier-card">
                                  <p class="quick-recall__dossier-statement">
                                    {item.statement}
                                  </p>
                                  <div class="quick-recall__dossier-foot">
                                    {#if item.subject}
                                      <span class="quick-recall__subject-chip">
                                        {item.subject}
                                      </span>
                                    {/if}
                                    <span class="quick-recall__conf-wrap">
                                      <ConfidenceBar confidence={item.confidence} />
                                    </span>
                                  </div>
                                </div>
                              {/each}
                            </div>
                          {:else if block.kind === "timeline"}
                            <!-- Timeline owns its own caption, so the .graphic
                                 wrapper here doesn't repeat it as a figcaption. -->
                            <figure class="quick-recall__graphic">
                              <Timeline title={block.title} intervals={block.items} />
                            </figure>
                          {/if}
                        {/each}
                      </div>

                      <!-- Per-turn answer sources: the captures this turn drew on,
                           surfaced only once the turn is done. -->
                      {#if turn.phase === "done" && turn.sources.length > 0}
                        <div class="quick-recall__sources">
                          <span class="quick-recall__sources-heading">Sources</span>
                          {#if turnFrameSources(turn).length > 0}
                            <div class="quick-recall__section" role="presentation">
                              <span class="quick-recall__section-label">Screen</span>
                              <div class="quick-recall__source-row" role="presentation">
                                {#each turnFrameSources(turn) as s, si (`${s.kind}-${s.frameId}-${s.audioSegmentId}-${s.startedAt}-${si}`)}
                                  <AnswerSourceCard
                                    kind="frame"
                                    appName={s.appName}
                                    windowTitle={s.windowTitle}
                                    startedAt={s.startedAt}
                                    endedAt={s.endedAt}
                                    thumbnailUrl={s.frameId != null
                                      ? (thumbnailCache.get(s.frameId) ?? null)
                                      : null}
                                    url={s.url}
                                    onselect={() => void selectSource(s)}
                                    onopenurl={() => openSourceUrl(s)}
                                  />
                                {/each}
                              </div>
                            </div>
                          {/if}

                          {#if turnAudioSources(turn).length > 0}
                            <div class="quick-recall__section" role="presentation">
                              <span class="quick-recall__section-label">Audio</span>
                              <div class="quick-recall__source-row" role="presentation">
                                {#each turnAudioSources(turn) as s, si (`${s.kind}-${s.frameId}-${s.audioSegmentId}-${s.startedAt}-${si}`)}
                                  <AnswerSourceCard
                                    kind="audio"
                                    appName={s.appName}
                                    windowTitle={s.windowTitle}
                                    startedAt={s.startedAt}
                                    endedAt={s.endedAt}
                                    sourceKind={s.sourceKind}
                                    url={s.url}
                                    onselect={() => void selectSource(s)}
                                  />
                                {/each}
                              </div>
                            </div>
                          {/if}
                        </div>
                      {/if}

                      <!-- Empty-answer fallback: a settled turn that produced no
                           answer blocks and no sources (e.g. the model returned
                           nothing) would otherwise render a blank turn with no
                           explanation or way forward. Surface a calm line plus the
                           same retry the error branch offers. -->
                      {#if turn.phase === "done" && turn.blocks.length === 0 && turn.sources.length === 0}
                        <p class="quick-recall__state">
                          No answer came back. Try asking again.
                        </p>
                        <div class="quick-recall__retry-row">
                          <button
                            type="button"
                            class="quick-recall__retry"
                            disabled={askStreaming}
                            onclick={() => {
                              if (ti === 0) {
                                void retryAsk();
                              } else {
                                void retryFollowupTurn(turn);
                              }
                            }}
                          >
                            Retry
                          </button>
                        </div>
                      {/if}
                    {/if}
                    {#if turn.liveActivity !== null}
                      <!-- Live animated working line: the real tool filter string. -->
                      <p class="quick-recall__state quick-recall__state--working">
                        <span class="quick-recall__dot" aria-hidden="true"></span>
                        <span class="quick-recall__working-label"
                          >{turn.liveActivity.label}</span
                        >
                        {#if turn.liveActivity.app}
                          in
                          {@render toolAppChip(turn.liveActivity)}
                        {/if}
                      </p>
                    {:else if turn.phase === "streaming"}
                      <!-- Answer text is arriving: label the phase so the caret
                           in AnswerProse reads as the insertion point. -->
                      <p class="quick-recall__state quick-recall__state--working">
                        <span class="quick-recall__dot" aria-hidden="true"></span>
                        Writing…
                      </p>
                    {/if}
                  {/if}
                {/if}
                {#if turn.stoppedEarly}
                  <!-- This turn was cut off by Stop; the partial below is kept
                       and is still continuable (composer / "Continue in Chat"). -->
                  <p class="quick-recall__stopped-tag" role="status">
                    Stopped early
                  </p>
                {/if}
              </div>
            {/each}
          {/if}
        </div>

        <!-- "Open in Chat" / Go deep (issue #111, ADR 0031): promote this thread
             into the full Insights → Chat workspace. The thread is already
             persisted under the same conversation id, so Chat continues it
             seamlessly. Shown once at least one turn has completed. -->
        {#if askCanOpenInChat}
          <div class="quick-recall__handoff-row">
            <button
              type="button"
              class="quick-recall__handoff"
              onclick={() => void openInChat()}
              use:tip={"Continue this thread in the Insights Chat workspace"}
            >
              Continue in Chat
              <span class="quick-recall__handoff-arrow" aria-hidden="true">↗</span>
            </button>
          </div>
        {/if}

        <!-- Follow-up composer: pinned beneath the transcript, present once the
             first answer completes. Disabled while any turn streams. Enter sends,
             Shift+Enter inserts a newline. Submitting appends a new turn and
             routes the raw question through ask_ai_followup; the backend reloads
             history server-side, so a follow-up always works. -->
        {#if askComposerVisible}
          <div class="quick-recall__composer">
            <textarea
              bind:this={followupInputEl}
              bind:value={followupInput}
              class="quick-recall__composer-input"
              rows="1"
              autocomplete="off"
              autocapitalize="off"
              spellcheck="false"
              placeholder={askStreaming
                ? "Answering…"
                : "Ask a follow-up…"}
              aria-label="Ask a follow-up question"
              disabled={askStreaming}
              onkeydown={handleFollowupKeydown}
            ></textarea>
          </div>
        {/if}
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
        {#if selectedResultIsOpenable}
          <span class="quick-recall__hint-item"><kbd>⌃O</kbd> open page</span>
        {/if}
        <span class="quick-recall__hint-item"><kbd>⌃F</kbd> filters</span>
        {#if askAvailable}
          <span class="quick-recall__hint-item"><kbd>⌃↵</kbd> Ask AI</span>
        {/if}
        <span class="quick-recall__hint-item"><kbd>esc</kbd> close</span>
      {:else}
        <span class="quick-recall__hint-item"><kbd>⌃F</kbd> filters</span>
        {#if askAvailable}
          <span class="quick-recall__hint-item"><kbd>⌃↵</kbd> Ask AI</span>
        {/if}
        <span class="quick-recall__hint-item"><kbd>esc</kbd> close</span>
      {/if}
    {:else if !askSubmitted}
      <span class="quick-recall__hint-item"><kbd>↵</kbd> ask</span>
      <span class="quick-recall__hint-item"><kbd>esc</kbd> back</span>
    {:else}
      {#if askCopyAvailable}
        <span class="quick-recall__hint-item"><kbd>⌃C</kbd> copy</span>
      {/if}
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

  /* Visually-hidden polite live region for AT-only result-status announcements. */
  .quick-recall__sr-status {
    position: absolute;
    width: 1px;
    height: 1px;
    padding: 0;
    margin: -1px;
    overflow: hidden;
    clip: rect(0, 0, 0, 0);
    white-space: nowrap;
    border: 0;
  }

  .quick-recall__field {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 16px;
    flex-shrink: 0;
    border-bottom: 1px solid var(--app-border);
  }

  /* The search row keeps a constant neutral hairline divider (Spotlight/Raycast
     style). We intentionally do NOT recolor it to the accent on :focus-within:
     the field only has a `border-bottom`, so an accent border-color paints a
     hard green line on that one edge rather than a ring, and the accompanying
     box-shadow ring is clipped on three sides by the parent's `overflow: hidden`
     rounded frame — leaving an asymmetric halo only below the field. The launcher
     auto-focuses this input on open, so that focus chrome was effectively always
     on. The blinking accent caret already signals focus. */

  .quick-recall__glyph {
    font-size: var(--text-lg);
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
    color: var(--app-text-muted);
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
    color: var(--app-text-muted);
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
    font-size: var(--text-xs);
    line-height: 1;
    color: var(--app-text-subtle);
    white-space: nowrap;
  }

  .quick-recall__footer kbd {
    font-family: inherit;
    font-size: var(--text-xs);
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

  /* Refetch-in-flight: prior results stay on screen but dim slightly so the
     keystroke-driven refresh reads as "updating" without the surface flashing
     empty between every keystroke. */
  .quick-recall__results--refetching {
    opacity: 0.55;
    transition: opacity 0.12s ease;
  }

  .quick-recall__section {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .quick-recall__section-label {
    font-size: var(--text-sm);
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
    font-size: var(--text-base);
    line-height: 1.5;
    color: var(--app-text-muted);
  }

  .quick-recall__state--error {
    color: var(--app-danger-text);
  }

  /* Quiet recovery line under the "No matches" empty state: muted prose with an
     inline link-style action, so it reads as a gentle next step rather than a
     loud CTA block (the dedicated empty-state styling is reserved for the
     semantic-search hint card). */
  .quick-recall__empty-recovery {
    margin: 0;
    padding: 0 2px 8px;
    font-size: var(--text-sm);
    line-height: 1.5;
    color: var(--app-text-subtle);
  }

  .quick-recall__empty-action {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 0;
    font: inherit;
    font-size: var(--text-sm);
    color: var(--app-text-muted);
    background: none;
    border: none;
    cursor: pointer;
    text-decoration: underline;
    text-underline-offset: 2px;
  }

  .quick-recall__empty-action:hover {
    color: var(--app-accent);
  }

  .quick-recall__empty-action:focus-visible {
    outline: none;
    color: var(--app-accent);
    border-radius: 4px;
    box-shadow: var(--app-ring);
  }

  .quick-recall__empty-action kbd {
    font-family: inherit;
    font-size: var(--text-xs);
    line-height: 1;
    color: var(--app-text-muted);
    background: var(--app-surface);
    border: 1px solid var(--app-border);
    border-radius: 5px;
    padding: 2px 5px;
  }

  /* In-search discoverability hint (issue #125): keyword-only search → Settings. */
  .quick-recall__semantic-hint {
    display: block;
    width: 100%;
    margin: 4px 0 8px;
    padding: 8px 10px;
    text-align: left;
    font-size: var(--text-base);
    line-height: 1.5;
    color: var(--app-text-muted);
    background: var(--app-surface-raised);
    border: 1px solid var(--app-border);
    border-radius: 6px;
    cursor: pointer;
  }

  .quick-recall__semantic-hint:hover {
    color: var(--app-text);
    border-color: var(--app-accent);
  }

  .quick-recall__semantic-hint:focus-visible {
    outline: none;
    border-color: var(--app-accent);
    box-shadow: var(--app-ring);
  }

  .quick-recall__semantic-hint:active {
    background: var(--app-surface-active);
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
    font-size: var(--text-xl);
    line-height: 1;
    color: var(--app-text-muted);
    background: var(--app-bg);
    border: 1px solid var(--app-border);
    border-radius: 11px;
    transform: rotate(-45deg);
  }

  .quick-recall__orient-tagline {
    margin: 0;
    font-size: var(--text-md);
    line-height: 1.4;
    color: var(--app-text-strong);
  }

  .quick-recall__orient-cues {
    display: inline-flex;
    align-items: center;
    gap: 9px;
  }

  /* Plain muted labels, NOT pills: the bordered/padded chip styling previously
     read as clickable buttons but these cues are inert (they only name what's
     searchable). Stripping the border/background/padding removes that false
     affordance while keeping the quiet uppercase label tone. */
  .quick-recall__orient-cue {
    font-size: var(--text-sm);
    line-height: 1;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--app-text-subtle);
  }

  .quick-recall__orient-cue-dot {
    color: var(--app-text-subtle);
    font-size: var(--text-sm);
    line-height: 1;
  }

  .quick-recall__orient-hint {
    margin: 0;
    font-size: var(--text-sm);
    line-height: 1.5;
    color: var(--app-text-subtle);
  }

  .quick-recall__orient-hint kbd {
    font-family: inherit;
    font-size: var(--text-xs);
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

  /* Match the resting SearchResultCard chrome (transparent border + fill, 9px
     radius) so a skeleton doesn't visually flip into a frameless card when the
     real results arrive — the shimmer blocks inside carry the loading signal. */
  .quick-recall__skeleton-row {
    display: flex;
    gap: 12px;
    align-items: stretch;
    padding: 6px 8px;
    border: 1px solid transparent;
    border-radius: 9px;
    background: transparent;
  }

  .quick-recall__skeleton-thumb {
    flex-shrink: 0;
    width: 96px;
    aspect-ratio: 16 / 10;
    border-radius: 7px;
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
    font-size: var(--text-base);
    line-height: 1;
    color: var(--app-text);
    background: var(--app-surface-subtle);
    border: 1px solid var(--app-border);
    border-radius: 6px;
    padding: 6px 8px;
    cursor: pointer;
    transition: border-color 0.12s ease, color 0.12s ease;
  }

  .quick-recall__ask-button:hover {
    border-color: var(--app-accent);
    color: var(--app-text-strong);
  }

  .quick-recall__ask-button:focus-visible {
    outline: none;
    border-color: var(--app-accent);
    box-shadow: var(--app-ring);
  }

  .quick-recall__ask-button:not(:disabled):not(.quick-recall__ask-button--disabled):active {
    background: var(--app-surface-active);
  }

  .quick-recall__ask-key {
    font-size: var(--text-sm);
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
    width: 24px;
    height: 24px;
    font-family: inherit;
    font-size: var(--text-base);
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

  .quick-recall__syntax-trigger:focus-visible {
    outline: none;
    border-color: var(--app-accent);
    box-shadow: var(--app-ring);
  }

  .quick-recall__syntax-trigger:active {
    background: var(--app-surface-active);
  }

  /* Funnel Filter Picker trigger: shares the quiet round-chip idiom of the
     syntax `?` trigger (accent reserved for hover/active), so the two field-row
     affordances read as a matched pair. The active variant marks it pressed while
     the picker overlay is open. */
  .quick-recall__filter-trigger {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    width: 24px;
    height: 24px;
    color: var(--app-text-muted);
    background: var(--app-surface-subtle);
    border: 1px solid var(--app-border);
    border-radius: 50%;
    cursor: pointer;
    transition: border-color 0.12s ease, color 0.12s ease, background 0.12s ease;
  }

  .quick-recall__filter-trigger:hover {
    border-color: var(--app-accent);
    color: var(--app-text-strong);
  }

  .quick-recall__filter-trigger:focus-visible {
    outline: none;
    border-color: var(--app-accent);
    box-shadow: var(--app-ring);
  }

  .quick-recall__filter-trigger:active,
  .quick-recall__filter-trigger--active {
    background: var(--app-surface-active);
    border-color: var(--app-accent);
    color: var(--app-accent);
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
    font-size: var(--text-sm);
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
    font-family: var(--app-font-mono);
    font-size: var(--text-sm);
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
    font-size: var(--text-sm);
    line-height: 1.35;
    color: var(--app-text);
  }

  .quick-recall__ask-hint {
    display: block;
    width: 100%;
    margin: 0;
    padding: 6px 18px 0;
    text-align: left;
    font-size: var(--text-sm);
    line-height: 1.4;
    color: var(--app-text-subtle);
    background: none;
    border: none;
    cursor: pointer;
    flex-shrink: 0;
  }

  .quick-recall__ask-hint:hover {
    color: var(--app-text);
    text-decoration: underline;
  }

  .quick-recall__ask-hint:focus-visible {
    outline: none;
    color: var(--app-text);
    border-radius: 4px;
    box-shadow: var(--app-ring);
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
    padding: 8px 16px 0;
    flex-shrink: 0;
  }

  .quick-recall__chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: var(--text-sm);
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
    position: relative;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 16px;
    height: 16px;
    font-family: inherit;
    font-size: var(--text-md);
    line-height: 1;
    color: var(--app-text-muted);
    background: transparent;
    border: none;
    border-radius: 4px;
    padding: 0;
    cursor: pointer;
    transition: color 0.12s ease, background-color 0.12s ease;
  }

  /* Invisible hit area expanding the 16px glyph to the 24px comfortable minimum
     without enlarging the chip itself. */
  .quick-recall__chip-remove::before {
    content: "";
    position: absolute;
    top: 50%;
    left: 50%;
    width: 24px;
    height: 24px;
    transform: translate(-50%, -50%);
  }

  .quick-recall__chip-remove:hover {
    color: var(--app-text-strong);
    background: color-mix(in srgb, var(--app-accent) 18%, transparent);
  }

  .quick-recall__chip-remove:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
  }

  .quick-recall__chip-remove:active {
    background: color-mix(in srgb, var(--app-accent) 28%, transparent);
  }

  /* Slice 3: inline parse-error line under the input. Shares the chip band's
     horizontal padding so it lines up with the chips it sits alongside, and uses
     the danger ramp (same as .quick-recall__state--error) — a malformed filter is
     a genuine failure, so it reads as a correction prompt, not success chrome. */
  .quick-recall__parse-error {
    margin: 0;
    padding: 8px 16px 0;
    flex-shrink: 0;
    font-size: var(--text-sm);
    line-height: 1.4;
    color: var(--app-danger-text);
  }

  /* Slice 5: Filter Picker overlay. Reuses the results-region box (flex column,
     scroll) so it occupies the same slot the results would, then layers a header,
     a vertical option list, and (for dates) the preset rows plus a typed-range
     hint (custom ranges are typed as after:/before: — there is no From/To field
     pair). Item highlight uses the same accent-tinted surface idiom as a selected
     result. */
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
    font-size: var(--text-sm);
    line-height: 1;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--app-text-subtle);
  }

  .quick-recall__picker-crumb-hint {
    font-size: var(--text-sm);
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

  /* Mouse-hover feedback for the clickable rows: a quiet neutral surface that
     stays clearly below the accent-tinted keyboard --selected highlight, so a
     pointer user gets feedback without it reading as the roving selection.
     Excludes disabled rows (they never highlight) and the already-selected row. */
  .quick-recall__picker-item:not(.quick-recall__picker-item--disabled):not(
      .quick-recall__picker-item--selected
    ):hover {
    background: var(--app-surface-raised);
    border-color: var(--app-border);
  }

  /* Pressed feedback for a clickable row (excludes disabled/selected, which carry
     their own treatment), so a pointer click reads as responsive. */
  .quick-recall__picker-item:not(.quick-recall__picker-item--disabled):not(
      .quick-recall__picker-item--selected
    ):active {
    background: var(--app-surface-active);
  }

  .quick-recall__picker-item--selected {
    background: color-mix(in srgb, var(--app-accent) 12%, transparent);
    border-color: var(--app-accent-border);
  }

  /* Slice 4 (typed path): a value-list row that would conflict with an active
     chip (app + audio source are mutually exclusive). Dimmed and non-selectable —
     it never highlights on hover and Enter skips it. */
  .quick-recall__picker-item--disabled {
    opacity: var(--app-disabled-opacity);
    cursor: default;
  }

  /* Slice 4 (typed path): the one-line conflict note below the value list, and
     the typed-date hint for the date operators. The conflict note is a correction
     prompt ("these filters can't combine"), so it shares the danger ramp with the
     sibling .quick-recall__parse-error line rather than reading as success-green. */
  .quick-recall__picker-conflict {
    margin: 0;
    padding: 2px 2px 0;
    flex-shrink: 0;
    font-size: var(--text-sm);
    line-height: 1.4;
    color: var(--app-danger-text);
  }

  .quick-recall__picker-hint {
    margin: 0;
    padding: 2px 2px 0;
    flex-shrink: 0;
    font-size: var(--text-sm);
    line-height: 1.4;
    color: var(--app-text-muted);
  }

  .quick-recall__picker-hint code {
    font-family: inherit;
    color: var(--app-text-muted);
  }

  .quick-recall__picker-item-label {
    font-size: var(--text-md);
    line-height: 1.3;
    color: var(--app-text-strong);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .quick-recall__picker-item-hint {
    flex: 1;
    min-width: 0;
    font-size: var(--text-sm);
    line-height: 1.3;
    color: var(--app-text-muted);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .quick-recall__picker-item-chevron {
    flex-shrink: 0;
    font-size: var(--text-md);
    color: var(--app-text-muted);
  }

  .quick-recall__picker-cue {
    margin: auto 0 0;
    padding: 8px 2px 0;
    flex-shrink: 0;
    font-size: var(--text-xs);
    line-height: 1;
    color: var(--app-text-subtle);
    display: flex;
    align-items: center;
    gap: 4px;
  }

  .quick-recall__picker-cue kbd {
    font-family: inherit;
    font-size: var(--text-xs);
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

  /* WKWebView focus idiom: the borderless ask input delegates its focus ring to
     the bar it sits in, mirroring the follow-up composer. Scoped to the input
     (not bare :focus-within) so the ring doesn't fire when the sibling Back/Stop
     buttons — which carry their own focus chrome — take focus. */
  .quick-recall__field--ask:has(.quick-recall__ask-input:focus) {
    border-color: var(--app-accent);
    box-shadow: var(--app-ring);
  }

  .quick-recall__back {
    flex-shrink: 0;
    font-family: inherit;
    font-size: var(--text-base);
    line-height: 1;
    color: var(--app-text-muted);
    background: var(--app-surface-subtle);
    border: 1px solid var(--app-border);
    border-radius: 6px;
    padding: 6px 8px;
    cursor: pointer;
    transition: border-color 0.12s ease, color 0.12s ease;
  }

  .quick-recall__back:hover {
    border-color: var(--app-accent);
    color: var(--app-text-strong);
  }

  .quick-recall__back:focus-visible {
    outline: none;
    border-color: var(--app-accent);
    box-shadow: var(--app-ring);
  }

  .quick-recall__back:not(:disabled):active {
    background: var(--app-surface-active);
  }

  /* The thread header label once a thread is open (the per-turn question
     headers live in the transcript, so this is just a quiet section marker). */
  .quick-recall__ask-thread-label {
    flex: 1;
    min-width: 0;
    font-size: var(--text-sm);
    line-height: 1;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--app-text-subtle);
  }

  /* Stop the streaming answer in place — quiet by default, shifting to the danger
     register on hover since it interrupts an in-flight action. */
  .quick-recall__stop {
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-family: inherit;
    font-size: var(--text-base);
    line-height: 1;
    color: var(--app-text-muted);
    background: var(--app-surface-subtle);
    border: 1px solid var(--app-border);
    border-radius: 6px;
    padding: 6px 9px;
    cursor: pointer;
    transition: border-color 0.12s ease, color 0.12s ease;
  }

  .quick-recall__stop-glyph {
    width: 8px;
    height: 8px;
    border-radius: 2px;
    background: currentColor;
    flex-shrink: 0;
  }

  .quick-recall__stop:hover {
    border-color: var(--app-danger);
    color: var(--app-danger-text);
  }

  .quick-recall__stop:focus-visible {
    outline: none;
    border-color: var(--app-danger);
    box-shadow: var(--app-ring-danger);
  }

  .quick-recall__stop:active {
    background: var(--app-surface-active);
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
    /* Auto-grown to content in JS (autosizeAskTextarea); the cap + hidden
       overflow keep it bounded to ~5 rows before scrolling. */
    box-sizing: border-box;
    max-height: 112px;
    overflow-y: hidden;
    caret-color: var(--app-accent);
  }

  .quick-recall__ask-input::placeholder {
    color: var(--app-text-muted);
  }

  .quick-recall__answer-area {
    gap: 18px;
    position: relative;
  }

  /* One transcript turn: question header + answer + sources/activity. The turn
     is the positioning context for its own copy button (top-right). Turns are
     separated by a hairline rule so the back-and-forth reads as a transcript. */
  .quick-recall__turn {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .quick-recall__turn:not(:first-child) {
    padding-top: 18px;
    border-top: 1px solid var(--app-border);
  }

  /* Read-only question header above each turn's answer. */
  .quick-recall__turn-question {
    margin: 0;
    padding-right: 34px;
    font-size: 14px;
    line-height: 1.4;
    color: var(--app-text-strong);
    font-weight: 600;
    overflow-wrap: anywhere;
  }

  /* "Stopped early" tag on a turn the user cut off mid-stream. The partial
     answer below stays kept and continuable; this just acknowledges the cut. */
  .quick-recall__stopped-tag {
    margin: 8px 0 0;
    font-size: 12px;
    color: var(--app-text-muted);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  /* Follow-up composer: pinned beneath the scrolling transcript. Mirrors the
     unseeded ask input but framed as its own bottom bar. Disabled (dimmed) while
     a turn streams. */
  .quick-recall__composer {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    padding: 10px 15px;
    border-top: 1px solid var(--app-border);
  }

  /* WKWebView focus idiom: the borderless composer input delegates its focus ring
     to the bar it sits in. */
  .quick-recall__composer:focus-within {
    border-color: var(--app-accent);
    box-shadow: var(--app-ring);
  }

  .quick-recall__composer-input {
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
    /* Auto-grown to content in JS (autosizeAskTextarea); the cap + hidden
       overflow keep it bounded to ~5 rows before scrolling. */
    box-sizing: border-box;
    max-height: 112px;
    overflow-y: hidden;
    caret-color: var(--app-accent);
  }

  .quick-recall__composer-input::placeholder {
    color: var(--app-text-muted);
  }

  .quick-recall__composer-input:disabled {
    color: var(--app-text-muted);
    cursor: default;
  }

  /* The answer body: a column of render-ready blocks (prose + inline graphics).
     The prose blocks render via AnswerProse; the graphical blocks (bars /
     timeline / dossier) sit in a quiet bordered card, mirroring the Insights
     Chat surface so the two doors render answers identically. */
  .quick-recall__answer {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .quick-recall__graphic {
    margin: 0;
    padding: 12px 13px;
    border: 1px solid var(--app-border);
    border-radius: 9px;
    background: var(--app-surface-subtle);
  }
  .quick-recall__graphic-title {
    font-size: var(--text-xs);
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--app-text-muted);
    margin: 0 0 10px;
  }
  .quick-recall__graphic--dossier {
    display: flex;
    flex-direction: column;
    gap: 9px;
  }
  .quick-recall__dossier-card {
    padding: 11px 12px;
    border: 1px solid var(--app-border);
    border-radius: 8px;
    background: var(--app-surface);
    display: flex;
    flex-direction: column;
    gap: 9px;
  }
  .quick-recall__dossier-statement {
    font-size: var(--text-base);
    color: var(--app-text-strong);
    line-height: 1.5;
    margin: 0;
  }
  .quick-recall__dossier-foot {
    display: flex;
    align-items: center;
    gap: 12px;
    flex-wrap: wrap;
  }
  .quick-recall__subject-chip {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: var(--text-xs);
    letter-spacing: 0.02em;
    padding: 2px 8px;
    border-radius: 4px;
    background: var(--app-accent-bg);
    border: 1px solid var(--app-accent-border);
    color: var(--app-accent-strong);
  }
  .quick-recall__conf-wrap {
    flex: 0 0 auto;
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
    font-size: var(--text-sm);
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
    transition: color 0.15s ease, border-color 0.15s ease;
  }

  .quick-recall__copy:hover {
    color: var(--app-text-strong);
    border-color: var(--app-accent);
  }

  .quick-recall__copy:focus-visible {
    outline: none;
    border-color: var(--app-accent);
    box-shadow: var(--app-ring);
  }

  .quick-recall__copy:not(:disabled):active {
    transform: translateY(1px);
  }

  .quick-recall__copy--copied {
    color: var(--app-accent);
    border-color: var(--app-accent-border);
  }

  /* Copy-failed flash: a brief red cue so a rejected clipboard write isn't
     silent (the icon also swaps to an error glyph). */
  .quick-recall__copy--failed {
    color: var(--app-danger-text);
    border-color: var(--app-danger);
  }

  /* "Continue in Chat" hand-off affordance (#111). A subtle, terminal-style
     button pinned above the composer; quiet by default, accent on hover —
     consistent with the other Quick Recall answer-action surfaces. */
  .quick-recall__handoff-row {
    display: flex;
    justify-content: flex-end;
    flex-shrink: 0;
    padding: 4px 2px 0;
  }

  .quick-recall__handoff {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-family: inherit;
    font-size: var(--text-sm);
    line-height: 1;
    letter-spacing: 0.02em;
    color: var(--app-text-muted);
    background: var(--app-surface-subtle);
    border: 1px solid var(--app-border);
    border-radius: 6px;
    padding: 6px 11px;
    cursor: pointer;
    transition: border-color 0.12s ease, color 0.12s ease, background 0.12s ease;
  }

  .quick-recall__handoff:hover {
    color: var(--app-accent-strong);
    border-color: var(--app-accent-border);
    background: var(--app-accent-bg);
  }

  .quick-recall__handoff:focus-visible {
    outline: none;
    border-color: var(--app-accent);
    box-shadow: var(--app-ring);
  }

  .quick-recall__handoff:not(:disabled):active {
    transform: translateY(1px);
  }

  .quick-recall__handoff-arrow {
    font-size: var(--text-base);
    line-height: 1;
  }

  /* Error "Retry" affordance. */
  .quick-recall__retry-row {
    padding: 2px;
  }

  .quick-recall__retry {
    font-family: inherit;
    font-size: var(--text-base);
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

  .quick-recall__retry:focus-visible {
    outline: none;
    border-color: var(--app-accent);
    box-shadow: var(--app-ring);
  }

  .quick-recall__retry:not(:disabled):active {
    background: var(--app-surface-active);
  }

  .quick-recall__retry:disabled {
    opacity: var(--app-disabled-opacity);
    cursor: not-allowed;
  }

  /* Collapsed, expandable activity summary chip. */
  .quick-recall__activity {
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  /* Thinking disclosure: the model's reasoning. Quiet/secondary to the answer —
     reuses the activity-chip styling for its collapsed "Thought process" chip,
     and a muted inset panel for the streamed reasoning text (live or expanded). */
  .quick-recall__thinking {
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .quick-recall__thinking-text {
    max-height: 180px;
    overflow: auto;
    padding: 8px 10px;
    border: 1px solid var(--app-border);
    border-radius: 6px;
    background: var(--app-surface-subtle);
    color: var(--app-text-muted);
    font-size: var(--text-sm);
    line-height: 1.5;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .quick-recall__activity-chip {
    align-self: flex-start;
    max-width: 100%;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-family: inherit;
    font-size: var(--text-sm);
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

  .quick-recall__activity-chip:focus-visible {
    outline: none;
    border-color: var(--app-accent);
    box-shadow: var(--app-ring);
  }

  .quick-recall__activity-chip:active {
    background: var(--app-surface-active);
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
    font-size: var(--text-sm);
    line-height: 1.4;
    color: var(--app-text-subtle);
    min-width: 0;
    overflow-wrap: anywhere;
  }

  /* The live working line's filter string wraps to its full text (the answer
     area scrolls) rather than truncating, so the user sees exactly what ran.
     (No flex-grow: the optional "in <app>" chip sits right after the label.) */
  .quick-recall__working-label {
    flex: 0 1 auto;
    min-width: 0;
    overflow-wrap: anywhere;
  }

  /* Inline app chip in tool-activity lines: the app-icon look at 16px. */
  .quick-recall__tool-app {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    min-width: 0;
    vertical-align: middle;
  }

  .quick-recall__tool-app-icon {
    display: grid;
    width: 16px;
    height: 16px;
    flex: 0 0 16px;
    place-items: center;
    overflow: hidden;
    border: 1px solid var(--app-border);
    border-radius: 4px;
    background: var(--app-surface);
    color: var(--app-text-muted);
    font-size: 9px;
    font-weight: 800;
    line-height: 1;
  }

  .quick-recall__tool-app-icon img {
    width: 13px;
    height: 13px;
    object-fit: contain;
  }

  .quick-recall__tool-app-name {
    color: var(--app-text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .quick-recall__seeded {
    margin: 0;
    padding: 0 2px;
    font-size: var(--text-sm);
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

  /* Slice 7: reduced-motion gating for the whole surface. Every animation and
     transition in this file collapses to an instant/static fallback. The hero
     mode-switch cross-fade is JS-driven (modeFadeMs → 0 in the script) and so is
     handled there; everything else is gated here. */
  @media (prefers-reduced-motion: reduce) {
    .quick-recall__dot {
      animation: none;
      opacity: 1;
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
    .quick-recall__syntax-trigger,
    .quick-recall__filter-trigger,
    .quick-recall__results--refetching,
    .quick-recall__stop {
      transition: none;
    }
  }
</style>
