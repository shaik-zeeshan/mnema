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

  onMount(() => {
    focusActiveField();
    void loadAskAvailability();

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
          void tick().then(focusActiveField);
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
                    >⇥</kbd
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
    .quick-recall__activity-caret {
      transition: none;
    }
  }
</style>
