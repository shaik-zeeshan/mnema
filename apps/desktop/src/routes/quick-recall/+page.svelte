<script lang="ts">
  import { onMount, onDestroy, tick, untrack } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import type {
    RecordingSettings,
    SemanticSearchModelDownloadProgress,
    RecordingSettingsDomainUpdateResponse,
  } from "$lib/types";
  import { humanizeError } from "$lib/format-error";
  import type { AskAiAvailability } from "$lib/insights/conversation";
  import type { RankedResult } from "$lib/quick-recall/result-sections";
  import {
    quickRecallSearch as search,
    OPTION_ID_PREFIX,
  } from "$lib/quick-recall/searchStore.svelte";
  import { handleLauncherCaptureKeydown as captureKeydown } from "$lib/quick-recall/search-keys";
  import ResultsList from "$lib/quick-recall/ResultsList.svelte";
  import BarShell from "$lib/quick-recall/BarShell.svelte";
  import QuickLook, { quickLook } from "$lib/quick-recall/QuickLook.svelte";
  import FilterPicker from "$lib/quick-recall/FilterPicker.svelte";
  import Field, { fieldFoot } from "$lib/quick-recall/Field.svelte";
  import AskPane from "$lib/quick-recall/AskPane.svelte";
  import EmptyStates, { noticeLine } from "$lib/quick-recall/EmptyStates.svelte";
  import RecentRows from "$lib/quick-recall/RecentRows.svelte";
  import {
    askEngine,
    autoAnswerAllowed,
    isQuestion,
    nextForced,
    scopedQuestion,
    type Forced,
  } from "$lib/quick-recall/question-detect";
  import IconCalendar from "~icons/lucide/calendar";

  // Search-mode extraction (slice 1): all search-mode state — query text,
  // debounce/scheduling, results, thumbnails cache, roving selection, filter
  // chips, pills + ghost + pill popover, and the
  // loading/error/parse-error states — lives in the shared
  // `quickRecallSearch` store singleton (lib/quick-recall/searchStore.svelte.ts).
  // The ask slot — question → answer in place — is AskPane (QR5). This page
  // keeps the window shell, search ⇄ ask, key routing, focus and the idle-clear.
  const filters = search.filters;

  // The window is reused across summons (hidden, not destroyed), so its state
  // persists while it's closed. Re-summoning within 5s resumes where you left
  // off; once it has been closed for 5s, reset to the empty search state so the
  // next summon is fresh. Clearing only happens while the window is *not* open.
  const IDLE_CLEAR_MS = 5000;
  let windowFocused = $state(true);
  let idleClearTimer: ReturnType<typeof setTimeout> | null = null;

  // Put the cursor in the field (the only text input in both modes). Called on
  // mount and every time the (reused) window regains focus on a fresh summon.
  //
  // On the very first summon the window is built and made key while its webview
  // is still loading, so the initial focus attempt can fire a frame before the
  // field is interactable. `retriesLeft` re-attempts on the next frames until the
  // active element is actually the target, which makes first-open focus reliable
  // without affecting the already-working reused-window summons.
  function focusActiveField(retriesLeft = 8): void {
    const target = search.inputEl;
    target?.focus();
    // Select any leftover query so typing immediately replaces it.
    target?.select();
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

  // ---------------------------------------------------------------------------
  // Search ⇄ ask. A typed question (trailing ? or a leading who/what/when…) is
  // answered in place after a pause when "Answer questions automatically"
  // allows it for the model that would answer; otherwise the hero slot offers
  // "Press ↵ to ask". Tab forces either way until the bar empties — like the
  // titlebar's Tab it only flips the mode; a forced ask waits for ↵.
  // ---------------------------------------------------------------------------

  // Availability is resolved on mount (and re-probed per summon). Until it
  // resolves Ask AI is treated as unavailable.
  let askAvailability = $state<AskAiAvailability | null>(null);
  let settings = $state<RecordingSettings | null>(null);
  let forced = $state<Forced>(null);
  let askPane = $state<ReturnType<typeof AskPane> | null>(null);
  let askStreaming = $state(false);
  let askPending = $state(false);

  const askAvailable = $derived(askAvailability?.available === true);
  const autoMode = $derived(settings?.access?.quickRecallAutoAnswer ?? "localOnly");
  const autoOk = $derived(autoAnswerAllowed(autoMode, askEngine(settings)));
  const empty = $derived(search.operatorQuery.trim().length === 0);
  const question = $derived(isQuestion(search.naturalParse.residual));
  const mode = $derived<"search" | "ask">(
    !empty && (forced ? forced === "ask" : question && autoOk) ? "ask" : "search",
  );
  const askView = $derived<"gate" | "ask" | null>(
    mode === "ask" ? "ask" : !empty && !forced && question && askAvailable ? "gate" : null,
  );
  // Idle = the bare 60px bar: no body until there is a query to search, a
  // filter surface open, Recent, or the ask slot.
  const showBody = $derived(
    askView !== null || !search.belowMinimum || filters.popoverOpen || search.states.showRecent,
  );

  // An emptied bar drops whatever Tab forced.
  $effect(() => {
    if (empty) forced = null;
  });

  // Tab (or the mode chip) flips search ⇄ ask.
  function toggleMode(): void {
    filters.selectPill(null);
    forced = nextForced(forced, question && autoOk);
    if (forced === "ask") void search.states.rememberQuery(true);
  }

  // An explicit ask (the gate, "ask instead") sends now and goes to the top
  // of Recent. AskPane may only mount (or turn into the answer view) on this
  // flip, so it sends after the flush.
  function askNow(): void {
    forced = "ask";
    void search.states.rememberQuery(true);
    void tick().then(() => askPane?.submit());
  }

  async function loadAskAvailability(): Promise<void> {
    try {
      askAvailability = await invoke<AskAiAvailability>("ask_ai_availability");
    } catch (error) {
      // Treat a failed availability probe as unavailable rather than erroring.
      askAvailability = { available: false, reason: humanizeError(error) };
    }
    settings = await invoke<RecordingSettings>("get_recording_settings").catch(() => settings);
  }

  $effect(() => {
    search.scheduleSearch(search.operatorQuery);
  });

  // Under the gate or an ask notice no moment is highlighted (recall.html):
  // ↓ moves into the rows. Each search, and a return to search, highlights
  // the top hit again so a hurried ↵ opens it.
  $effect(() => {
    void search.ranked;
    const top = askView === null && untrack(() => search.resultCount) > 0;
    search.selectedIndex = top ? 0 : -1;
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
      return "Search failed.";
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
  // when content (query, results, error, or an answer) is present.
  let hasClearableState = $derived(
    search.trimmedQuery.length > 0 ||
      search.hasResults ||
      search.errorMessage !== null ||
      search.resultsQuery.length > 0,
  );

  // Reset the window to the just-summoned empty search state and refocus. The
  // emptied bar unmounts AskPane, which cancels its turn.
  async function clearState(): Promise<void> {
    forced = null;
    search.clearSearchState();
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
  // preserves the state for a quick resume. An answer still streaming, finished
  // but not yet seen, or just handed to the main window (AskPane's `pending`)
  // is never wiped: the backend finishes and persists it regardless.
  $effect(() => {
    if (!windowFocused && hasClearableState && !operationRunning && !askPending) {
      scheduleIdleClear();
    } else {
      clearIdleTimer();
    }
  });

  // Launcher keys via a WINDOW CAPTURE listener (focus-independent; while the
  // pill popover is up it owns Escape/Arrow/Enter/⌫ regardless of DOM focus),
  // then the ask slot's (stop, evidence, citations, Continue in Chat) ahead of
  // the field's own handler. An open Quick Look owns its keys over the ask
  // slot; the moments' Space-to-look only applies where moments show.
  function handleLauncherCaptureKeydown(event: KeyboardEvent): void {
    if (event.isComposing) return;
    if (!quickLook.result && !filters.popoverOpen && askPane?.handleKey(event)) {
      event.preventDefault();
      event.stopPropagation();
      return;
    }
    const moments = !askView || askPane?.momentsBelow() || quickLook.result !== null;
    captureKeydown(search, event, moments ? quickLook : undefined);
  }

  // Space / the eye on evidence row `at`: Quick Look on it (a citation's
  // search hit when it is one, else a result built from the citation); ↑/↓
  // walk the evidence rows.
  function lookAtSource(rows: (RankedResult | null)[], at: number, onClose: (at: number) => void): void {
    quickLook.showCited(rows, at, onClose);
  }

  onMount(() => {
    void focusQuickRecall();
    void loadAskAvailability();
    // MCP connectors (Workstream C): warm-on-open discovery — background-connect
    // enabled MCP servers so a turn finds their tools ready. Fire-and-forget.
    void invoke("mcp_warm_connectors").catch(() => {});
    void search.states.loadIndexStatus();
    // Warm the captured-app catalog up front so the App value list (whether
    // reached by typing `app:` or via the picker) has selectable rows on first
    // open — the source/date lists are static, so only App needs the head start.
    void search.ensureSearchableAppsLoaded();

    // Focus-independent key ownership for the pill popover (see above).
    window.addEventListener("keydown", handleLauncherCaptureKeydown, {
      capture: true,
    });

    let destroyed = false;
    let unlistenFocus: (() => void) | undefined;
    let unlistenSummon: (() => void) | undefined;
    let unlistenSettings: (() => void) | undefined;
    let unlistenSemanticSearchDownload: (() => void) | undefined;

    // The window is hidden/re-shown rather than recreated across summons, so
    // re-grab focus each time it becomes key — onMount alone fires only once.
    getCurrentWindow()
      .onFocusChanged(({ payload: focused }) => {
        windowFocused = focused;
        if (focused) {
          // The panel is hidden/re-shown rather than recreated, so re-probe Ask
          // AI availability (and the model/auto-answer settings) each time it
          // becomes key — the user may have fixed the engine since the last summon.
          void loadAskAvailability();
          // Re-read the indexing / meaning / recording state behind the notice line.
          void search.states.loadIndexStatus();
          // The captured-app set grows as new apps are seen after launch, but the
          // session cache pins the launch-time set — so apps first captured this
          // session never show in `app:` completion. Invalidate on focus; the lazy
          // loader (kicked by the `app:` derivation) repopulates on the next use.
          search.searchableApps = null;
          void tick().then(() => focusQuickRecall());
        } else if (!empty) void search.states.rememberQuery(mode === "ask");
      })
      .then((fn) => {
        if (destroyed) fn();
        else unlistenFocus = fn;
      });

    // The titlebar's ↵ hands its text over (windows.rs `PENDING_QUICK_RECALL_QUERY`).
    // The event fires on every such summon — the panel may already be up and
    // focused, so no focus change would carry it. Take once on attach too: a
    // cold-built window was summoned before this listener existed.
    const takeSummonQuery = () =>
      void search.takeSummonQuery().then((taken) => {
        if (!taken) return;
        forced = null;
        // A Quick Look left open would sit over the new query's results.
        quickLook.close();
      });
    listen("quick_recall_query", takeSummonQuery)
      .then((fn) => {
        if (destroyed) fn();
        else unlistenSummon = fn;
      })
      .finally(takeSummonQuery);

    // A meaning-search toggle / model pick saved in Settings while this window
    // stays focused: re-read the status so "keyword only" doesn't go stale.
    // Ask AI's switch, model and "Answer questions automatically" (`access`,
    // `ai_runtime`) re-probe the ask slot the same way.
    listen<RecordingSettingsDomainUpdateResponse>(
      "recording_settings_domain_changed",
      (event) => {
        const domain: string = event.payload.domain;
        if (domain === "semantic_search") void search.states.loadIndexStatus();
        if (domain === "access" || domain === "ai_runtime") void loadAskAvailability();
      },
    ).then((fn) => {
      if (destroyed) fn();
      else unlistenSettings = fn;
    });

    // …and when a model download started there finishes (no settings save).
    listen<SemanticSearchModelDownloadProgress>(
      "semantic_search_model_download_progress",
      (event) => {
        if (
          ["completed", "failed", "cancelled"].includes(event.payload.status)
        ) {
          void search.states.loadIndexStatus();
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
      unlistenFocus?.();
      unlistenSummon?.();
      unlistenSettings?.();
      unlistenSemanticSearchDownload?.();
    };
  });

  onDestroy(() => {
    search.clearDebounce();
    clearIdleTimer();
  });
</script>

<!-- The window is the bar (BarShell): the field row, then — once there is
     something to show — the body and the key row. Each mode fills the same
     three slots, so switching mode resizes the bar instead of cross-fading
     two fixed-height panels. -->
{#snippet barRow()}
  {#if mode === "search"}
    <!-- Polite live region announcing result-count / loading / no-matches /
         error transitions to AT, which the aria-activedescendant results
         listbox cannot convey on its own. Visually hidden. -->
    <span class="quick-recall__sr-status" role="status" aria-live="polite"
      >{searchStatusAnnouncement}</span
    >
  {/if}
  <Field {askAvailable} onToggleMode={toggleMode} {mode} auto={mode === "ask" && !forced} />
{/snippet}

{#snippet bodyRegion()}
  {#if quickLook.result}
    <QuickLook />
  {/if}
  <!-- Hidden, not unmounted, under Quick Look: an answer keeps streaming
       and its selected citation is still there when Quick Look closes. -->
  <div class="quick-recall__slot" hidden={quickLook.result !== null}>
    <!-- The ask slot: the gate, a can't-answer notice, or the answer itself. -->
    {#if askView}
      <AskPane
        bind:this={askPane}
        bind:streaming={askStreaming}
        bind:pending={askPending}
        view={askView}
        question={scopedQuestion(search.naturalParse.residual, search.effectivePills)}
        forced={forced === "ask"}
        availability={askAvailability}
        {settings}
        {autoMode}
        focused={windowFocused}
        onAsk={askNow}
        onLook={lookAtSource}
      />
    {/if}
    {#if !askView || askPane?.momentsBelow()}
      <!-- Results paused: the backend rejected an operator (underlined in the
           field and the footer). -->
      {#if search.resultsPaused && search.firstParseError}
        {@const bad = search.firstParseError.token}
        {@const [why, hint] = (search.parseErrorMessage ?? "").split(/(?<=\.)\s+(.*)/s)}
        <div class="quick-recall__paused mx-notice" data-tone="warn" role="alert">
          <span class="mx-notice__icon"><IconCalendar width="15" height="15" aria-hidden="true" /></span>
          <b class="mx-notice__title">Results paused — {why.replace(/\.$/, "")}</b>
          {#if hint}<p class="mx-notice__text">{hint}</p>{/if}
          {#if bad.includes(":")}
            <div class="mx-notice__acts">
              <button
                type="button"
                class="mx-btn mx-btn--ghost mx-btn--sm"
                onclick={() => (search.query = search.query.replace(bad, bad.slice(bad.indexOf(":") + 1)))}
                >Search it as text</button
              >
            </div>
          {/if}
        </div>
      {/if}

      <!-- The results, with the pressed pill's popover laid over them (same
           grid cell, so the bar grows to fit it, up to BarShell's cap). -->
      {@render noticeLine()}
      <div class="quick-recall__body">
        {#if filters.popoverOpen}
          <FilterPicker />
        {/if}
        {#if search.states.showRecent}
          <RecentRows />
        {:else if search.states.emptyState}
          <EmptyStates {askAvailable} onAskAi={askNow} />
        {:else if !search.resultsPaused}
          <ResultsList rowsOnly={askView !== null}
            onlook={(i) => quickLook.show(search.visibleResults[i])} />
        {/if}
      </div>
    {/if}
  </div>
{/snippet}

{#snippet footKeys()}
  {#if askView && askPane && !quickLook.result}
    {@render fieldFoot(askPane.footKeys())}
  {:else}
    {@render fieldFoot()}
  {/if}
{/snippet}

<div class="quick-recall">
  <BarShell bar={barRow} body={showBody ? bodyRegion : undefined} foot={footKeys} />
</div>

<style>
  /* Takes the bar's height from BarShell — never the viewport's: the
     viewport is the window the shell is sizing. */
  .quick-recall {
    font-family: inherit;
  }

  /* The results list's column; the list itself is the body's scroller. One
     grid cell, so the pill popover overlays the results and the bar still
     grows to whichever is taller. */
  .quick-recall__body {
    flex: 1 1 auto;
    min-height: 0;
    display: grid;
    grid-template: minmax(0, 1fr) / minmax(0, 1fr);
  }

  .quick-recall__body > :global(*) {
    grid-area: 1 / 1;
  }

  .quick-recall__slot {
    display: contents;
  }

  .quick-recall__slot[hidden] {
    display: none;
  }

  .quick-recall__paused {
    margin: 6px 2px 4px;
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
</style>
