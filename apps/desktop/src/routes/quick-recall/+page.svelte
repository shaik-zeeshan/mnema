<script lang="ts">
  import { tip } from "$lib/components/tooltip";
  import { onMount, onDestroy, tick } from "svelte";
  import { fade } from "svelte/transition";
  import { convertFileSrc, invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { appIconFallback } from "$lib/app-privacy-exclusion";
  import AnswerSourceCard from "$lib/components/AnswerSourceCard.svelte";
  import { framePreviewAssetUrl } from "$lib/frame-preview";
  import { closeCurrentWindow, openSettings } from "$lib/surface-windows";
  import type {
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
  import type { FrameScrubPreviewsDto } from "$lib/types/app-infra";
  import {
    quickRecallSearch as search,
    OPTION_ID_PREFIX,
  } from "$lib/quick-recall/searchStore.svelte";
  import { PICKER_OPT_PREFIX } from "$lib/quick-recall/filterSurfaces.svelte";
  import {
    buildScopedSeedQuery,
    buildScopedQuestion,
  } from "$lib/quick-recall/filter-chips";
  import {
    handleSearchKeydown as searchKeydown,
    handleLauncherCaptureKeydown as captureKeydown,
  } from "$lib/quick-recall/search-keys";
  import ResultsList from "$lib/quick-recall/ResultsList.svelte";
  import DetailPane from "$lib/quick-recall/DetailPane.svelte";
  import TimelineStrip from "$lib/quick-recall/TimelineStrip.svelte";
  import FilterPicker from "$lib/quick-recall/FilterPicker.svelte";
  import SyntaxHelp from "$lib/quick-recall/SyntaxHelp.svelte";

  // Search-mode extraction (slice 1): all search-mode state — query text,
  // debounce/scheduling, results, thumbnails cache, roving selection, filter
  // chips, picker/ghost/value-list surfaces, syntax-help open state, and the
  // loading/error/parse-error/semantic-hint states — lives in the shared
  // `quickRecallSearch` store singleton (lib/quick-recall/searchStore.svelte.ts).
  // This page keeps the window shell, the mode cross-fade, the whole Ask AI
  // mode, root/window keydown routing, focus management, and the idle-clear.
  const filters = search.filters;

  // Slice 2 (list + detail split): the two-pane body shows for the skeleton
  // and results branches of the results region; the full-width states
  // (orientation, error, results-paused, no-matches) keep the whole width,
  // mirroring the mockup's states gallery. The branch order below mirrors
  // ResultsList's own state branches exactly.
  const searchSplitVisible = $derived.by(() => {
    if (search.belowMinimum) return false;
    if (search.loading && !search.hasResults) return true; // first-search skeleton
    if (search.errorMessage) return false;
    if (search.resultsPaused) return false;
    if (search.showEmpty) return false;
    return true; // results branch
  });

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
  // openFrameResult/openAudioResult (frame xor audio carried by the source
  // kind). Quick
  // Recall is a transient launcher popup, not a place the user is settled into,
  // so a cited source always hops to the main Timeline window and dismisses the
  // launcher — the in-place FrameDetailModal is for the main-window surfaces
  // (Chat, Subjects), not here.
  async function selectSource(source: AskAiSource): Promise<void> {
    // Carry the Audio Search Result Anchor for audio sources (frame sources
    // leave these null), mirroring openAudioResult so the dashboard lands on the
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
      await search.surfaceResultHandoffFailure(err);
      return;
    }
    // Preserve the thread across the close so re-summon restores the answer
    // instead of dropping to a blank search (see sourceHandoffPending above).
    sourceHandoffPending = true;
    await closeCurrentWindow();
  }

  // Open the captured page behind a frame source in the default browser.
  // Frame sources only (audio has frameId/url null). Routed through the store's
  // shared in-flight latch (one open at a time across the selected-result ⌃O
  // path and these answer-source chips alike).
  async function openSourceUrl(source: AskAiSource): Promise<void> {
    if (source.frameId == null) return;
    await search.openCapturedFrameUrl(source.frameId);
  }

  // Load thumbnails for answer-source frames, mirroring the store's
  // loadThumbnails. Best effort: a card without a cached preview falls back to
  // its glyph. No search generation guard applies here (these come from the ask
  // stream, not search).
  async function loadSourceThumbnails(sources: AskAiSource[]): Promise<void> {
    const frameIds = sources
      .filter((source) => source.kind === "frame" && source.frameId != null)
      .map((source) => source.frameId as number)
      .filter((id) => !search.thumbnailCache.has(id));

    const uniqueIds = Array.from(new Set(frameIds));
    if (uniqueIds.length === 0) {
      return;
    }

    try {
      const response = await invoke<FrameScrubPreviewsDto>("get_frame_scrub_previews", {
        request: { frameIds: uniqueIds },
      });

      const next = new Map(search.thumbnailCache);
      for (const entry of response.previews) {
        if (entry.preview) {
          next.set(entry.frameId, framePreviewAssetUrl(entry.preview.filePath));
        }
      }
      search.thumbnailCache = next;
    } catch {
      // Thumbnails are best-effort; the card falls back to its glyph.
    }
  }

  // Search-input keydown: routed through the extracted search-mode key logic
  // (lib/quick-recall/search-keys.ts), which preserves the exact ownership
  // order (syntax help → picker → ⌘F/slash summon → value list → Ask AI pivot →
  // ghost accept → ⌘1–9 / ⌘O / chip Backspace → roving results navigation).
  // The Ask AI pivot's inputs are passed in since ask mode stays page-owned.
  function handleSearchKeydown(event: KeyboardEvent): void {
    searchKeydown(search, event, askAvailable, () => void activateAskAi());
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
      mode === "ask" ? (askSubmitted ? askAreaEl : askInputEl) : search.inputEl;
    target?.focus();
    // Select any leftover query so typing immediately replaces it.
    if (mode !== "ask") search.inputEl?.select();

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
  // Syntax-help outside-click dismissal. The popover itself lives in the
  // SyntaxHelp component; its open state + wrapper element live on the store.
  // Registered only while the popover is open, so there's no global listener
  // cost when it's closed. A pointerdown inside the help wrapper (trigger or
  // popover) is ignored — the trigger's own onclick owns the toggle — so this
  // only fires for genuine outside clicks. Escape dismissal lives at the top of
  // the search keydown routing (search-keys.ts) so it never clobbers the
  // layout's window-close handler, the picker's Escape, or ask-mode Escape.
  // ---------------------------------------------------------------------------

  // Stable id for the disabled-Ask-AI reason line, referenced by the disabled
  // button's aria-describedby so keyboard/AT users reach the reason (the native
  // `title` tooltip is mouse-only).
  const ASK_UNAVAILABLE_HINT_ID = "quick-recall-ask-unavailable-hint";

  $effect(() => {
    if (!search.syntaxHelpOpen) {
      return;
    }
    const onPointerDown = (event: PointerEvent): void => {
      const target = event.target as Node | null;
      if (
        search.syntaxHelpEl !== null &&
        target !== null &&
        search.syntaxHelpEl.contains(target)
      ) {
        return;
      }
      search.syntaxHelpOpen = false;
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

    // Leaving search mode closes the Filter Picker (search-mode only).
    filters.pickerOpen = false;
    filters.pickerIndex = 0;

    // Inherit the active chip scope into the pivot. The SEED is a canonical,
    // parser-exact operator string (re-parsed by the broker search); the
    // QUESTION is the residual free text with a plain-language scope suffix.
    // With no chips these collapse to the raw trimmed query (unchanged
    // behavior): buildScopedSeedQuery → residual === trimmedQuery's free text,
    // and buildScopedQuestion → the residual. We fall back to `trimmedQuery`
    // when the backend hasn't populated `residualQuery` yet (e.g. a query
    // below the parse threshold) so the seed/question are never blank when the
    // user typed text.
    const chips = search.activeFilterChips;
    const residual =
      chips.length === 0 && search.residualQuery.trim().length === 0
        ? search.trimmedQuery
        : search.residualQuery;
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
    search.inputEl?.focus();
  }

  $effect(() => {
    search.scheduleSearch(search.query);
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
    if (mode !== "search" || search.selectedIndex < 0) {
      return;
    }
    document
      .getElementById(`${OPTION_ID_PREFIX}${search.selectedIndex}`)
      ?.scrollIntoView({ block: "nearest" });
  });

  // Keep the highlighted picker option within view as it moves, mirroring the
  // results scroll-into-view effect (the app value list can overflow).
  $effect(() => {
    if (!filters.pickerOpen || filters.pickerItemCount === 0) {
      return;
    }
    document
      .getElementById(`${PICKER_OPT_PREFIX}${filters.pickerIndex}`)
      ?.scrollIntoView({ block: "nearest" });
  });

  // Keep the value list's `valueListIndex` on an enabled row. Re-runs when the
  // operator changes or the row set changes (filtering as the user types): if
  // the current index is out of range or its row is disabled, snap to the
  // first enabled row, or -1 if none are selectable. Keyed on the operator +
  // rows so a fresh operator resets.
  $effect(() => {
    if (!filters.valueListActive) {
      return;
    }
    // Touch the operator so an operator switch re-evaluates the highlight.
    void filters.activeOperatorContext?.operator;
    const rows = filters.valueListRows;
    const current = rows[filters.valueListIndex];
    if (current !== undefined && !current.disabled) {
      return;
    }
    const firstEnabled = rows.findIndex((row) => !row.disabled);
    filters.valueListIndex = firstEnabled;
  });

  // Load the captured-app catalog as soon as the App value list opens, so its
  // rows populate (mirrors the picker's lazy load on drilling into App).
  $effect(() => {
    if (filters.activeOperatorContext?.operator === "app:") {
      void search.ensureSearchableAppsLoaded();
    }
  });

  // Route the unavailable-Ask-AI hint to the Intelligence pane (providers + Ask
  // AI + Reasoning Engine), so the friendly "do X in Settings" reason becomes
  // actionable from here instead of a dead end.
  async function openAskAiSettings(): Promise<void> {
    await openSettings("intelligence");
  }

  // Screen-reader announcement for the results region. The cards live in an
  // aria-activedescendant listbox whose count/loading/empty/error transitions
  // are otherwise silent to AT; this polite live-region string mirrors the
  // visible state so a result-count change (or a switch to loading/no-matches/
  // error) is spoken. Branch order matches the results markup so the spoken
  // state and the rendered state never disagree.
  let searchStatusAnnouncement = $derived.by((): string => {
    if (mode !== "search" || search.belowMinimum) {
      return "";
    }
    if (search.loading) {
      return "Searching…";
    }
    if (search.errorMessage) {
      return search.errorMessage;
    }
    if (search.resultsPaused) {
      return "Results paused — fix the filter above to search.";
    }
    if (search.showEmpty) {
      return `No matches for ${search.resultsQuery}.`;
    }
    return `${search.totalResultCount} ${search.totalResultCount === 1 ? "result" : "results"} for ${search.resultsQuery}.`;
  });

  // A search or Ask AI operation is in flight — that counts as activity, so the
  // idle countdown is suspended while it runs.
  let operationRunning = $derived(search.loading || askStreaming);

  // There is something to reset: anything other than a pristine, empty search
  // box. Clearing the pristine state would be a no-op, so the timer only arms
  // when content (query, results, error, or an Ask AI view) is present.
  let hasClearableState = $derived(
    mode === "ask" ||
      search.trimmedQuery.length > 0 ||
      search.hasResults ||
      search.errorMessage !== null ||
      search.resultsQuery.length > 0,
  );

  // Reset the window to the just-summoned empty search state and refocus.
  async function clearState(): Promise<void> {
    await cancelActiveAsk();
    mode = "search";
    // All search-mode state (debounce, in-flight generation, query, results,
    // parsed scope, picker) resets in the store.
    search.clearSearchState();
    // Idle-window expiry tears down the whole ask thread (the live session was
    // already killed by cancelActiveAsk above).
    resetAskThreadState();
    await tick();
    search.inputEl?.focus();
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

  // Launcher sub-surface keys via a WINDOW CAPTURE listener (focus-independent;
  // see lib/quick-recall/search-keys.ts for the full rationale — while the
  // Filter Picker / Filter Value List / syntax popover is up it owns
  // Escape/Arrow/Enter regardless of DOM focus). The mode gate stays here: ask
  // mode owns its own Escape via handleRootKeydown.
  function handleLauncherCaptureKeydown(event: KeyboardEvent): void {
    if (event.isComposing || mode !== "search") {
      return;
    }
    captureKeydown(search, event);
  }

  onMount(() => {
    void focusQuickRecall();
    void loadAskAvailability();
    // MCP connectors (Workstream C): warm-on-open discovery — background-connect
    // enabled MCP servers so a turn finds their tools ready. Fire-and-forget.
    void invoke("mcp_warm_connectors").catch(() => {});
    void search.loadSemanticSearchModelInstalled();
    // Warm the captured-app catalog up front so the App value list (whether
    // reached by typing `app:` or via the picker) has selectable rows on first
    // open — the source/date lists are static, so only App needs the head start.
    void search.ensureSearchableAppsLoaded();

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
          void search.loadSemanticSearchModelInstalled();
          // The captured-app set grows as new apps are seen after launch, but the
          // session cache pins the launch-time set — so apps first captured this
          // session never show in `app:` completion. Invalidate on focus; the lazy
          // loader (kicked by the `app:` derivation) repopulates on the next use.
          search.searchableApps = null;
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
        void search.loadSemanticSearchModelInstalled();
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
          void search.loadSemanticSearchModelInstalled();
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
    search.clearDebounce();
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
            {#if filters.hasGhost}
              <div class="quick-recall__ghost" aria-hidden="true">
                <span class="quick-recall__ghost-typed">{search.query}</span><span
                  class="quick-recall__ghost-suffix">{filters.ghostCompletion}</span
                >
              </div>
            {/if}
            <input
              bind:this={search.inputEl}
              bind:value={search.query}
              class="quick-recall__input"
              type="text"
              autocomplete="off"
              autocapitalize="off"
              spellcheck="false"
              placeholder="Search your captures…"
              aria-label="Search your captures"
              role="combobox"
              aria-expanded={filters.pickerOpen ||
                filters.valueListActive ||
                search.resultCount > 0}
              aria-keyshortcuts="ArrowUp ArrowDown Enter Escape Control+Enter Control+O"
              aria-controls={filters.pickerOpen
                ? "quick-recall-picker"
                : filters.valueListActive
                  ? "quick-recall-value-list"
                  : "quick-recall-results-list"}
              aria-activedescendant={filters.pickerOpen
                ? filters.pickerActiveOptionId
                : filters.valueListActive
                  ? filters.valueListActiveOptionId
                  : search.activeOptionId}
              onkeydown={handleSearchKeydown}
              oninput={() => search.updateCaretAtEnd()}
              onkeyup={() => search.updateCaretAtEnd()}
              onclick={() => search.updateCaretAtEnd()}
              onselect={() => search.updateCaretAtEnd()}
              onfocus={() => {
                search.updateCaretAtEnd();
                void search.ensureSearchableAppsLoaded();
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
            class:quick-recall__filter-trigger--active={filters.pickerOpen}
            class:quick-recall__filter-trigger--filtered={search
              .activeFilterChips.length > 0}
            onclick={() =>
              filters.pickerOpen ? filters.closePicker() : filters.openPicker()}
            aria-label="Filter results"
            use:tip={"Filter results (⌘F)"}
            aria-expanded={filters.pickerOpen}
            aria-controls={filters.pickerOpen ? "quick-recall-picker" : undefined}
            aria-keyshortcuts="Control+F"
          >
            <svg
              width="11"
              height="11"
              viewBox="0 0 12 12"
              fill="none"
              stroke="currentColor"
              stroke-width="1.2"
              stroke-linejoin="round"
              aria-hidden="true"
            >
              <path d="M1 2h10L7.5 6.5V10l-3-1.5V6.5L1 2z" />
            </svg>
            Filter <kbd aria-hidden="true">⌘F</kbd>
          </button>
          <!-- Syntax-help affordance (`?` trigger + static popover), extracted
               to a component; its open state lives on the search store so the
               keydown routing can close it. -->
          <SyntaxHelp fadeMs={modeFadeMs} />
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
        {#if search.activeFilterChips.length > 0}
          <div class="quick-recall__chips" role="list" aria-label="Active filters">
            {#each search.activeFilterChips as chip (chip.id)}
              <span class="quick-recall__chip" role="listitem">
                <span class="quick-recall__chip-label">{chip.label}</span>
                <button
                  type="button"
                  class="quick-recall__chip-remove"
                  onclick={() => search.removeChip(chip)}
                  aria-label={`Remove ${chip.label} filter`}
                  use:tip={`Remove ${chip.label} filter`}
                >
                  ×
                </button>
              </span>
            {/each}
            <span class="quick-recall__chip-hint" aria-hidden="true"
              >typed filters become chips · ? for syntax</span
            >
          </div>
        {/if}

        <!-- Slice 3: inline parse-error line. When the backend reports a
             malformed operator it ALSO suppresses results (paused), so we surface
             ONE friendly line here — in the same band slot as the chips above (a
             chip and a live error never apply to the same token, so they coexist
             cleanly). Only the first error is shown. The Ask AI pivot stays
             reachable: a malformed filter never blocks a natural-language ask. -->
        {#if search.parseErrorMessage !== null}
          <p class="quick-recall__parse-error" role="alert">{search.parseErrorMessage}</p>
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

        <!-- Slice 2: two-pane body (mockup `.qr-body`). The results list is
             the fixed-width left column and the detail pane fills the right
             whenever the list shows rows (or the first-search skeleton); the
             full-width states and the Filter Picker span the whole body. -->
        <div class="quick-recall__body">
          {#if filters.pickerOpen || filters.valueListActive}
            <!-- Filter Picker / Filter Value List: replaces the results region
                 while a filter sub-surface owns it. Extracted component; the
                 picker branch wins while open (the two are mutually exclusive
                 by construction), preserving the original branch order. -->
            <FilterPicker />
          {:else}
            <!-- The search results region with all its state branches
                 (orientation / skeleton / error+Retry / results-paused /
                 no-matches recovery / semantic hint / Screen+Audio sections),
                 extracted to a component rendering off the search store. -->
            <ResultsList
              {askAvailable}
              onAskAi={() => void activateAskAi()}
              split={searchSplitVisible}
            />
            {#if searchSplitVisible}
              <DetailPane dim={search.loading && !search.hasResults} />
            {/if}
          {/if}
        </div>

        <!-- Slice 6: thin 8-day timeline strip between the body and the footer
             (mockup `.timeline` + `.tl-preview`): one dot per fetched result at
             its true time, hover previews, click selects (auto-expanding a
             collapsed section). Rendered whenever results exist, staying put
             under the Filter Picker like the mockup's always-present strip. -->
        {#if search.hasResults}
          <TimelineStrip />
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
                                      ? (search.thumbnailCache.get(s.frameId) ?? null)
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
      {#if filters.pickerOpen}
        <!-- While the picker owns the keys, the footer reflects its own
             navigation contract (no Ask AI pivot — Tab is suppressed). -->
        <span class="quick-recall__hint-item"><kbd>↑</kbd><kbd>↓</kbd> move</span>
        <span class="quick-recall__hint-item"><kbd>↵</kbd> select</span>
        <span class="quick-recall__hint-item"><kbd>esc</kbd> close</span>
      {:else if search.resultCount > 0}
        <!-- Mockup footer wording exactly: select=preview, Enter opens in the
             main-window timeline, ⌘1–9 jumps selection (no longer opens). -->
        <span class="quick-recall__hint-item"><kbd>↑</kbd><kbd>↓</kbd> navigate</span>
        <span class="quick-recall__hint-item"><kbd>↵</kbd> open in timeline</span>
        {#if search.selectedResultIsOpenable}
          <span class="quick-recall__hint-item"><kbd>⌘O</kbd> open page</span>
        {/if}
        {#if askAvailable}
          <span class="quick-recall__hint-item"><kbd>⌃↵</kbd> ask AI</span>
        {/if}
        <span class="quick-recall__hint-item"><kbd>⌘F</kbd> filter</span>
        <span class="quick-recall__hint-item"><kbd>⌘1-9</kbd> jump</span>
        <span class="quick-recall__hint-item"><kbd>esc</kbd> close</span>
      {:else}
        <span class="quick-recall__hint-item"><kbd>⌘F</kbd> filter</span>
        {#if askAvailable}
          <span class="quick-recall__hint-item"><kbd>⌃↵</kbd> ask AI</span>
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

  /* Slice 2: the search-mode body row hosting the list column + detail pane
     (mockup `.qr-body`). Fills the panel between the field/chip rows and the
     footer via flex (NOT height:100% — WebKit collapses that against a
     flex-stretched parent). */
  .quick-recall__body {
    flex: 1 1 auto;
    min-height: 0;
    min-width: 0;
    display: flex;
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
    min-height: 56px;
    padding: 8px 16px;
    flex-shrink: 0;
    border-bottom: 1px solid var(--app-border);
    /* Mockup `.searchbar`: the chrome rows (search bar, footer) sit on the
       subtle surface so the detail pane's plain surface reads as the stage. */
    background: var(--app-surface-subtle);
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
    transition: color 0.12s ease;
  }

  /* Mockup `.searchbar:focus-within svg.magnifier`: the magnifier warms to the
     accent while the input has focus. */
  .quick-recall__field:focus-within .quick-recall__glyph {
    color: var(--app-accent);
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
    /* Mockup `.searchbar input`: 16px query text. */
    font-size: 16px;
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
    /* Must mirror .quick-recall__input's font metrics exactly. */
    font-size: 16px;
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

  /* Ask AI door (mockup `.askai-btn`): the one accent-filled affordance in the
     field row — accent text on the accent-tinted surface, glowing on hover. */
  .quick-recall__ask-button {
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-family: inherit;
    font-size: var(--text-sm);
    line-height: 1;
    white-space: nowrap;
    color: var(--app-accent);
    background: var(--app-accent-bg);
    border: 1px solid var(--app-accent-border);
    border-radius: 6px;
    padding: 6px 10px;
    cursor: pointer;
    transition:
      border-color 0.12s ease,
      box-shadow 0.12s ease;
  }

  .quick-recall__ask-button:hover {
    border-color: var(--app-accent-strong);
    box-shadow: 0 0 0 3px var(--app-accent-glow);
  }

  .quick-recall__ask-button:focus-visible {
    outline: none;
    border-color: var(--app-accent-strong);
    box-shadow: var(--app-ring);
  }

  .quick-recall__ask-button:not(:disabled):not(.quick-recall__ask-button--disabled):active {
    background: color-mix(in srgb, var(--app-accent) 14%, var(--app-accent-bg));
  }

  /* Mockup `.askai-btn kbd`: accent key cap on a transparent ground. */
  .quick-recall__ask-key {
    font-size: var(--text-xs);
    line-height: 1;
    color: var(--app-accent);
    background: transparent;
    border: 1px solid var(--app-accent-border);
    border-radius: 4px;
    padding: 2px 4px;
  }

  .quick-recall__ask-button--disabled,
  .quick-recall__ask-button:disabled {
    color: var(--app-text-subtle);
    background: var(--app-surface-subtle);
    border-color: var(--app-border);
    cursor: not-allowed;
  }

  .quick-recall__ask-button--disabled:hover {
    border-color: var(--app-border);
    color: var(--app-text-subtle);
    box-shadow: none;
  }

  /* Funnel Filter Picker trigger (mockup `.appfilter > button`): a quiet
     labeled "Filter ⌘F" button on the raised surface. The active variant marks
     it pressed while the picker overlay is open; the filtered variant turns
     accent while any chip narrows the query (mockup `.appfilter.filtered`). */
  .quick-recall__filter-trigger {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    flex-shrink: 0;
    font-family: inherit;
    font-size: var(--text-sm);
    line-height: 1;
    white-space: nowrap;
    color: var(--app-text-muted);
    background: var(--app-surface-raised);
    border: 1px solid var(--app-border-strong);
    border-radius: 6px;
    padding: 5px 10px;
    cursor: pointer;
    transition:
      border-color 0.12s ease,
      color 0.12s ease,
      background 0.12s ease;
  }

  .quick-recall__filter-trigger kbd {
    font-family: inherit;
    font-size: var(--text-xs);
    line-height: 1;
    color: var(--app-text-muted);
    background: var(--app-surface);
    border: 1px solid var(--app-border);
    border-radius: 4px;
    padding: 2px 4px;
  }

  .quick-recall__filter-trigger:hover {
    border-color: var(--app-border-hover);
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

  .quick-recall__filter-trigger--filtered {
    color: var(--app-accent);
    background: var(--app-accent-bg);
    border-color: var(--app-accent-border);
  }

  .quick-recall__filter-trigger--filtered kbd {
    color: var(--app-accent);
    background: transparent;
    border-color: var(--app-accent-border);
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

  /* Active filter chip band (mockup `.chipband` + `.fchip`): a chrome band on
     the subtle surface under the input; every chip is an accent-tinted pill
     with its × remover, trailed by the quiet hint line. Shares its slot with
     the inline parse-error line. */
  .quick-recall__chips {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    padding: 8px 16px;
    flex-shrink: 0;
    border-bottom: 1px solid var(--app-border);
    background: var(--app-surface-subtle);
  }

  .quick-recall__chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: var(--text-sm);
    line-height: 1;
    color: var(--app-accent);
    background: var(--app-accent-bg);
    border: 1px solid var(--app-accent-border);
    border-radius: 999px;
    padding: 4px 5px 4px 10px;
    white-space: nowrap;
  }

  .quick-recall__chip-label {
    white-space: nowrap;
  }

  .quick-recall__chip-hint {
    font-size: var(--text-xs);
    line-height: 1;
    color: var(--app-text-subtle);
  }

  /* Mockup `.fchip .x`: a round remover inheriting the chip's accent, resting
     at 0.7 opacity and filling with an accent wash on hover/press. */
  .quick-recall__chip-remove {
    position: relative;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 16px;
    height: 16px;
    font-family: inherit;
    font-size: var(--text-base);
    line-height: 1;
    color: inherit;
    background: transparent;
    border: none;
    border-radius: 50%;
    padding: 0;
    cursor: pointer;
    opacity: 0.7;
    transition:
      opacity 0.12s ease,
      background-color 0.12s ease;
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
    opacity: 1;
    background: color-mix(in srgb, var(--app-accent) 18%, transparent);
  }

  .quick-recall__chip-remove:focus-visible {
    outline: none;
    opacity: 1;
    box-shadow: var(--app-ring);
  }

  .quick-recall__chip-remove:active {
    opacity: 1;
    background: color-mix(in srgb, var(--app-accent) 26%, transparent);
  }

  /* Slice 3: inline parse-error line under the input. Styled as a band like
     the chip row (they can coexist — a chip and a live error never apply to
     the same token) and uses the danger ramp (same as
     .quick-recall__state--error) — a malformed filter is a genuine failure, so
     it reads as a correction prompt, not success chrome. */
  .quick-recall__parse-error {
    margin: 0;
    padding: 8px 16px;
    flex-shrink: 0;
    font-size: var(--text-sm);
    line-height: 1.4;
    color: var(--app-danger-text);
    border-bottom: 1px solid var(--app-border);
    background: var(--app-surface-subtle);
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

    .quick-recall__ask-button,
    .quick-recall__back,
    .quick-recall__copy,
    .quick-recall__retry,
    .quick-recall__activity-chip,
    .quick-recall__activity-caret,
    .quick-recall__filter-trigger,
    .quick-recall__glyph,
    .quick-recall__chip-remove,
    .quick-recall__stop {
      transition: none;
    }
  }
</style>
