// Quick Recall SEARCH-mode store — the single frontend owner of the launcher's
// search state (query text, debounce/scheduling, results, thumbnails,
// selection, filter chips, picker/ghost/value-list via `filters`, syntax-help
// open state, loading/error/parse-error states, semantic-hint state).
// Extracted from the quick-recall +page.svelte in the search-mode extraction
// slice — NO behavior change. Mirrors the repo's `.svelte.ts` store idiom
// (a class with `$state`/`$derived` class fields + a module-level singleton),
// like `conversationStore`. The page keeps the window shell, mode cross-fade,
// the whole Ask AI mode, and the page-level listeners/effects; components
// (ResultsList / FilterPicker / SyntaxHelp) render off this singleton.
import { invoke } from "@tauri-apps/api/core";
import { message } from "@tauri-apps/plugin-dialog";
import { framePreviewAssetUrl } from "$lib/frame-preview";
import { openCapturedUrl } from "$lib/open-captured-url";
import { closeCurrentWindow, openSettings } from "$lib/surface-windows";
import { humanizeError } from "$lib/format-error";
import type { SemanticSearchModelStatusResponse } from "$lib/types";
import type {
  SearchCaptureResponse,
  SearchCaptureRefinements,
  SearchParseError,
  FrameSearchResultDto,
  AudioSearchResultDto,
  FrameScrubPreviewsDto,
  SearchableApp,
} from "$lib/types/app-infra";
import {
  deriveActiveFilterChips,
  friendlyParseError,
  stripChipTokens,
  type ActiveFilterChip,
} from "./filter-chips";
import { isTrailingOperatorPartial } from "./query-tokens";
import {
  AUDIO_FETCH_LIMIT,
  AUDIO_VISIBLE_CAP,
  FRAME_FETCH_LIMIT,
  FRAME_VISIBLE_CAP,
  remapSelection,
  visibleCount,
} from "./result-sections";
import { FilterSurfaces } from "./filterSurfaces.svelte";
import { appIcons } from "./app-icons.svelte";

export const MIN_QUERY_LENGTH = 2;
const DEBOUNCE_MS = 250;

export const OPTION_ID_PREFIX = "qr-opt-";

// The selected result surfaced to the detail pane: the kind discriminant plus
// the concrete DTO (frame xor audio).
export type SelectedSearchResult =
  | { kind: "frame"; frame: FrameSearchResultDto }
  | { kind: "audio"; audio: AudioSearchResultDto };

export class SearchStore {
  query = $state("");
  inputEl = $state<HTMLInputElement | null>(null);

  frames = $state<FrameSearchResultDto[]>([]);
  audio = $state<AudioSearchResultDto[]>([]);
  loading = $state(false);
  errorMessage = $state<string | null>(null);
  // The query string that the currently-displayed results belong to.
  resultsQuery = $state("");
  thumbnailCache = $state(new Map<number, string>());

  // Parsed search scope (advanced search syntax): `search_capture` runs the
  // backend operator parser on EVERY raw query and returns three fields we
  // capture here. We do NOT re-parse operators in the frontend — these come
  // straight from the backend's desugared response; the raw query text still
  // carries the operators.
  //   - appliedRefinements: the desugared scope (date range, apps, window
  //     title, audio sources, screen-only flag) the parser extracted.
  //   - residualQuery: the free-text left after operators were stripped.
  //   - parseErrors: malformed-operator diagnostics. A non-empty list means the
  //     backend SUPPRESSED results (paused), so a distinct "results paused"
  //     state is driven from `firstParseError` instead of "no results".
  // All three reset wherever the result state resets (catch branch in
  // runSearch, the below-minimum branch of scheduleSearch, and
  // clearSearchState), so chips/errors never linger past their query.
  appliedRefinements = $state<SearchCaptureRefinements | null>(null);
  residualQuery = $state("");
  parseErrors = $state<SearchParseError[]>([]);

  // Roving selection over the flattened VISIBLE result list (visible frames
  // first, then visible audio — collapsed show-more overflow is NOT in the
  // index space). -1 means nothing highlighted. The search input keeps DOM
  // focus the whole time; selection is surfaced via aria-activedescendant + a
  // `selected` class. Selecting PREVIEWS (the detail pane consumes
  // `selectedResult`); only Enter opens + closes the window.
  selectedIndex = $state(-1);

  // Per-section show-more expansion (mockup `.more-row` toggle). Reset on a
  // new search (query change), never carried across queries.
  framesExpanded = $state(false);
  audioExpanded = $state(false);

  // Syntax-help popover open state + the wrapper element (bound by the
  // SyntaxHelp component; the page-level outside-click effect reads it).
  syntaxHelpOpen = $state(false);
  syntaxHelpEl = $state<HTMLDivElement | null>(null);

  // Lazily-loaded distinct captured apps, cached for the session. A transient
  // failure leaves this null so the next partial retries (ghost/value list just
  // won't complete app values until it loads).
  searchableApps = $state<SearchableApp[] | null>(null);
  searchableAppsLoading = $state(false);

  // Whether the input caret is collapsed at the very end of `query`. The ghost
  // only ever shows (and only ever accepts) at end-of-input, so we track this
  // rather than a full caret position. Updated on every input/keyup/click/select.
  caretAtEnd = $state(true);

  // In-search discoverability hint (issue #125): when no Semantic Search Model
  // is installed, search is keyword-only; a hint points the user to Settings.
  semanticSearchModelInstalled = $state<boolean | null>(null);

  // In-flight latch for the captured-page open: one selected result (and one
  // answer-source chip) opens at a time, so a single boolean is enough to keep
  // the ⌃O key or a chip double-click from stacking opens / feedback dialogs.
  // The latch wraps the actual brokered open, so it covers the keyboard path
  // and the Ask AI answer-source chips alike.
  openingCapturedUrl = $state(false);

  // Generation token so stale (out-of-order) responses are discarded.
  searchGeneration = 0;
  #debounceTimer: ReturnType<typeof setTimeout> | null = null;

  clearDebounce(): void {
    if (this.#debounceTimer !== null) {
      clearTimeout(this.#debounceTimer);
      this.#debounceTimer = null;
    }
  }

  scheduleSearch(raw: string): void {
    this.clearDebounce();

    // While the caret sits in an un-committed field-operator value, the Filter
    // Value List owns the results region; the partial operator must NOT reach
    // the backend (a half-typed value would otherwise read as empty results).
    // Cancel any in-flight search and leave the current results state intact.
    if (isTrailingOperatorPartial(raw)) {
      this.searchGeneration += 1;
      // Clear any in-flight loading state: the invalidated response will be
      // dropped by the generation guard and never reach `loading = false`, so
      // without this the panel stays stuck "running" — keeping the value list
      // owner from rendering cleanly and blocking the idle-clear teardown
      // (operationRunning would never fall back to false).
      this.loading = false;
      return;
    }

    const trimmed = raw.trim();

    if (trimmed.length < MIN_QUERY_LENGTH) {
      // Invalidate any in-flight request and reset to the idle state.
      this.searchGeneration += 1;
      this.frames = [];
      this.audio = [];
      this.loading = false;
      this.errorMessage = null;
      this.resultsQuery = "";
      this.selectedIndex = -1;
      this.framesExpanded = false;
      this.audioExpanded = false;
      // Drop any parsed scope so stale chips/parse errors don't linger once
      // the query falls back below the minimum length.
      this.appliedRefinements = null;
      this.residualQuery = "";
      this.parseErrors = [];
      return;
    }

    this.#debounceTimer = setTimeout(() => {
      void this.runSearch(trimmed);
    }, DEBOUNCE_MS);
  }

  async runSearch(trimmed: string): Promise<void> {
    this.searchGeneration += 1;
    const generation = this.searchGeneration;
    this.loading = true;
    this.errorMessage = null;

    try {
      // Narrow the per-section limits to the active scope so a
      // source-restricted query doesn't waste a slot fetching the other kind.
      // Scope is only known AFTER a response, so `sectionLimits` is a $derived
      // off the PREVIOUS response's appliedRefinements. `appliedRefinements`
      // always belongs to `resultsQuery`, so the narrowing is only trustworthy
      // when the pending query MATCHES that prior query (a same-query re-run /
      // pagination). When the query CHANGED, the cached scope is stale and
      // could zero out the section the new query actually scopes to. Over-
      // fetching is harmless (the backend honors the raw query's operators
      // regardless of these limits), so for a changed query we fetch both
      // sections at full limit and let the scope settle on the next keystroke.
      // `refinements` stays empty: the operators live in the query TEXT.
      const scopeMatchesPendingQuery =
        this.appliedRefinements !== null && this.resultsQuery === trimmed;
      const limits = scopeMatchesPendingQuery
        ? this.sectionLimits
        : { frameLimit: FRAME_FETCH_LIMIT, audioLimit: AUDIO_FETCH_LIMIT };
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

      if (generation !== this.searchGeneration) {
        return;
      }

      // Expansion state resets on a NEW search; a same-query re-run (retry /
      // scope-narrowed refetch) keeps whatever the user expanded.
      if (trimmed !== this.resultsQuery) {
        this.framesExpanded = false;
        this.audioExpanded = false;
      }
      this.frames = response.frames;
      this.audio = response.audio;
      this.resultsQuery = trimmed;
      // Capture the parsed scope so chip/error/limit derivations update.
      this.appliedRefinements = response.appliedRefinements;
      this.residualQuery = response.residualQuery;
      this.parseErrors = response.parseErrors;
      this.loading = false;
      // Auto-highlight the top hit so a hurried Enter opens it (spotlight-style).
      this.selectedIndex =
        response.frames.length + response.audio.length > 0 ? 0 : -1;

      appIcons.ensure(response.frames.map((f) => f.appBundleId ?? f.appName));
      void this.loadThumbnails(response.frames, generation);
    } catch (error) {
      if (generation !== this.searchGeneration) {
        return;
      }
      this.frames = [];
      this.audio = [];
      this.resultsQuery = trimmed;
      this.loading = false;
      this.selectedIndex = -1;
      this.framesExpanded = false;
      this.audioExpanded = false;
      // A transport/backend failure isn't a parse outcome — clear the parsed
      // scope so a prior query's chips/parse errors don't survive an error.
      this.appliedRefinements = null;
      this.residualQuery = "";
      this.parseErrors = [];
      this.errorMessage = humanizeError(error);
    }
  }

  // One batch covers ALL fetched frame rows (up to 24), including show-more
  // overflow that starts collapsed — so expanding a section never needs a
  // round trip and keeps the single-batch behavior.
  async loadThumbnails(
    frameResults: FrameSearchResultDto[],
    generation: number,
  ): Promise<void> {
    const frameIds = frameResults
      .map((result) => result.thumbnailFrameId)
      .filter((id) => !this.thumbnailCache.has(id));

    const uniqueIds = Array.from(new Set(frameIds));
    if (uniqueIds.length === 0) {
      return;
    }

    try {
      const response = await invoke<FrameScrubPreviewsDto>("get_frame_scrub_previews", {
        request: { frameIds: uniqueIds },
      });

      if (generation !== this.searchGeneration) {
        return;
      }

      const next = new Map(this.thumbnailCache);
      for (const entry of response.previews) {
        if (entry.preview) {
          next.set(entry.frameId, framePreviewAssetUrl(entry.preview.filePath));
        }
      }
      this.thumbnailCache = next;
    } catch {
      // Thumbnails are best-effort; the card falls back to its glyph.
    }
  }

  // Surface a hand-off failure for the open-result/open-source paths. The
  // brokered open is this window's core action, so a rejected invoke must not
  // close the window onto nothing — we keep the window open and report instead.
  async surfaceResultHandoffFailure(err: unknown): Promise<void> {
    await message(
      `Couldn't open that result: ${humanizeError(err, "it may no longer be available.")}`,
      { title: "Couldn't open result", kind: "error" },
    );
  }

  // Open a frame result in the main-window timeline and close Quick Recall —
  // Enter's action (selection alone only previews; see openResultAt).
  async openFrameResult(result: FrameSearchResultDto): Promise<void> {
    try {
      await invoke("open_capture_result_in_main_window", {
        kind: "frame",
        frameId: result.representativeFrame.id,
        audioSegmentId: null,
      });
    } catch (err) {
      await this.surfaceResultHandoffFailure(err);
      return;
    }
    await closeCurrentWindow();
  }

  async openAudioResult(result: AudioSearchResultDto): Promise<void> {
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
      await this.surfaceResultHandoffFailure(err);
      return;
    }
    await closeCurrentWindow();
  }

  // Open a captured page via the shared brokered helper. The helper owns the
  // feedback: a no-openable-URL result shows a brief info note and a real
  // opener failure shows an error dialog (mirroring the timeline's "Couldn't
  // open URL: …"). The raw URL stays in Rust; the UI never sees it.
  async openCapturedFrameUrl(frameId: number): Promise<void> {
    if (this.openingCapturedUrl) return;
    this.openingCapturedUrl = true;
    try {
      await openCapturedUrl(frameId);
    } finally {
      this.openingCapturedUrl = false;
    }
  }

  // ── Keyboard navigation (flattened visible results list) ──────────────────
  // Results render as a single flattened list (visible frames first, then
  // visible audio) so the arrow keys can roam across both sections.
  // `selectedIndex` indexes into that flattened VISIBLE order — rows hidden
  // behind a collapsed show-more are not selectable; the helpers below
  // translate it back to a concrete result.

  visibleFrames = $derived(
    this.frames.slice(
      0,
      visibleCount(this.frames.length, FRAME_VISIBLE_CAP, this.framesExpanded),
    ),
  );
  visibleAudio = $derived(
    this.audio.slice(
      0,
      visibleCount(this.audio.length, AUDIO_VISIBLE_CAP, this.audioExpanded),
    ),
  );
  // The selection space (visible rows only). The total fetched count lives in
  // `totalResultCount` (section headers / SR announcement).
  resultCount = $derived(this.visibleFrames.length + this.visibleAudio.length);
  totalResultCount = $derived(this.frames.length + this.audio.length);

  // The selected result (kind + DTO) — select = PREVIEW: this is what the
  // detail pane renders from (Slice 5); nothing opens until Enter.
  selectedResult = $derived.by<SelectedSearchResult | null>(() => {
    const index = this.selectedIndex;
    if (index < 0) {
      return null;
    }
    if (index < this.visibleFrames.length) {
      return { kind: "frame", frame: this.visibleFrames[index] };
    }
    const audio = this.visibleAudio[index - this.visibleFrames.length];
    return audio !== undefined ? { kind: "audio", audio } : null;
  });

  // The currently-selected result is an openable frame carrying a captured
  // page. Gates the ⌘O "open page" footer hint so it tracks the *selected*
  // result — never advertising a no-op for an audio/url-less selection.
  selectedResultIsOpenable = $derived(
    this.selectedResult?.kind === "frame" && this.selectedResult.frame.url != null,
  );
  activeOptionId = $derived(
    this.selectedIndex >= 0 ? `${OPTION_ID_PREFIX}${this.selectedIndex}` : undefined,
  );

  // Jump selection to the visible row at `index` (click on a row, ⌘1–9).
  // Selecting only previews — it never opens anything. Refocus the input so a
  // click keeps keyboard flow (Enter/arrows live on the input's keydown; the
  // click otherwise blurs it and the keyboard goes dead).
  selectResultAt(index: number): void {
    this.inputEl?.focus();
    if (index < 0 || index >= this.resultCount) {
      return;
    }
    this.selectedIndex = index;
  }

  // Timeline-strip click (Slice 6): select a FETCHED result by section +
  // fetched-array index. Mirrors the mockup's select(): a dot can target a row
  // hidden behind a collapsed show-more, so expand that section first, then
  // set the visible-space index (post-expansion the fetched index IS the
  // section-local visible index). Selecting only previews, like selectResultAt.
  selectFetchedResult(kind: "frame" | "audio", index: number): void {
    this.inputEl?.focus();
    if (kind === "frame") {
      if (index < 0 || index >= this.frames.length) {
        return;
      }
      if (index >= this.visibleFrames.length) {
        this.framesExpanded = true;
      }
      this.selectedIndex = index;
      return;
    }
    if (index < 0 || index >= this.audio.length) {
      return;
    }
    if (index >= this.visibleAudio.length) {
      this.audioExpanded = true;
    }
    this.selectedIndex = this.visibleFrames.length + index;
  }

  // Toggle a section's show-more expansion. If the collapse hides the selected
  // row (or shifts the audio block), remap so the same result stays selected
  // where possible, clamped to the nearest visible row otherwise.
  toggleFramesExpanded(): void {
    this.inputEl?.focus();
    const prev = { frames: this.visibleFrames.length, audio: this.visibleAudio.length };
    this.framesExpanded = !this.framesExpanded;
    this.#clampSelection(prev);
  }

  toggleAudioExpanded(): void {
    this.inputEl?.focus();
    const prev = { frames: this.visibleFrames.length, audio: this.visibleAudio.length };
    this.audioExpanded = !this.audioExpanded;
    this.#clampSelection(prev);
  }

  #clampSelection(prev: { frames: number; audio: number }): void {
    this.selectedIndex = remapSelection(this.selectedIndex, prev, {
      frames: this.visibleFrames.length,
      audio: this.visibleAudio.length,
    });
  }

  // Enter's action: open the visible result at `index` in the main-window
  // timeline (frame or audio) and close Quick Recall.
  openResultAt(index: number): void {
    if (index < 0 || index >= this.resultCount) {
      return;
    }
    if (index < this.visibleFrames.length) {
      void this.openFrameResult(this.visibleFrames[index]);
    } else {
      void this.openAudioResult(this.visibleAudio[index - this.visibleFrames.length]);
    }
  }

  // Open the captured page behind the currently-selected frame result in the
  // default browser — the keyboard path (⌘/Ctrl+O) to the per-card open chip,
  // which can't sit in the tab order inside the aria-activedescendant listbox.
  // The footer hint is gated on `selectedResultIsOpenable`, but the ⌘O shortcut
  // still fires while a non-openable result is selected, so surface a benign
  // note (same opener feedback path as a real failure) instead of silence.
  openSelectedResultUrl(): void {
    if (this.selectedResult?.kind === "frame" && this.selectedResult.frame.url != null) {
      void this.openCapturedFrameUrl(this.selectedResult.frame.thumbnailFrameId);
      return;
    }
    void message("No openable page for this result.", {
      title: "Couldn't open page",
      kind: "info",
    });
  }

  moveSelection(delta: number): void {
    if (this.resultCount === 0) {
      return;
    }
    // Wrap around the ends; a first ArrowDown from -1 lands on the top result.
    const base =
      this.selectedIndex < 0 ? (delta > 0 ? -1 : 0) : this.selectedIndex;
    this.selectedIndex = (base + delta + this.resultCount) % this.resultCount;
  }

  // ── Caret / app-catalog helpers ────────────────────────────────────────────

  async ensureSearchableAppsLoaded(): Promise<void> {
    if (this.searchableApps !== null || this.searchableAppsLoading) {
      return;
    }
    this.searchableAppsLoading = true;
    try {
      this.searchableApps = await invoke<SearchableApp[]>("list_searchable_apps");
      // The filter value list is keyed by app NAME (rows dedupe on it), so
      // resolve icons by name too — resolve_app_icons handles display names.
      appIcons.ensure(this.searchableApps.map((app) => app.name));
    } catch {
      // Leave `searchableApps` null (not an empty list) so a transient failure
      // is retried on the next `app:` partial rather than disabling completion.
    } finally {
      this.searchableAppsLoading = false;
    }
  }

  // Recompute whether the caret sits at end-of-input. Cheap; called from the
  // input event handlers so the ghost derivation only fires when relevant.
  updateCaretAtEnd(): void {
    const el = this.inputEl;
    if (el === null) {
      this.caretAtEnd = true;
      return;
    }
    this.caretAtEnd =
      el.selectionStart === el.value.length && el.selectionEnd === el.value.length;
  }

  // ── Filter chips ───────────────────────────────────────────────────────────

  // The normalized active-chip list derived from the backend desugar.
  activeFilterChips = $derived.by<ActiveFilterChip[]>(() =>
    deriveActiveFilterChips(this.appliedRefinements),
  );

  // Drop a chip's operator token(s) from the query and let the reactive effect
  // rerun the search (which re-derives chips and restores sections via
  // sectionLimits). Refocus the input so removal keeps keyboard flow.
  removeChip(chip: ActiveFilterChip): void {
    this.query = stripChipTokens(this.query, chip);
    this.inputEl?.focus();
  }

  // ── Syntax help (static popover; the only state is open/closed) ───────────

  toggleSyntaxHelp(): void {
    this.syntaxHelpOpen = !this.syntaxHelpOpen;
    // Keep typing flow on the search input — the help is an occasional-use
    // affordance, so it's toggled by click but never becomes the focus target.
    if (!this.syntaxHelpOpen) {
      this.inputEl?.focus();
    }
  }

  closeSyntaxHelp(): void {
    if (!this.syntaxHelpOpen) {
      return;
    }
    this.syntaxHelpOpen = false;
    this.inputEl?.focus();
  }

  // ── Result-state derivations ───────────────────────────────────────────────

  // The first parse error (or null). One inline error line renders from this;
  // its presence is also what tells the results region the backend paused
  // results rather than finding none.
  firstParseError = $derived<SearchParseError | null>(this.parseErrors[0] ?? null);

  // Dynamic per-section fetch limits derived from the active source/app scope.
  // Read inside runSearch (optimistically — off the PREVIOUS response's scope;
  // see the note there). Rules:
  //   - any audio source active        → audio only  (frame 0 / audio 12)
  //   - source:screen OR any app chip  → screen only (audio 0 / frame 24)
  //   - otherwise                      → both (24 / 12)
  // Audio sources and screen/app scope are mutually exclusive at the operator
  // level, but if both ever appear we prefer audio-only (the audio branch wins).
  sectionLimits = $derived.by<{ frameLimit: number; audioLimit: number }>(() => {
    const refinements = this.appliedRefinements;
    const hasAudioSource = (refinements?.audioSources?.length ?? 0) > 0;
    if (hasAudioSource) {
      return { frameLimit: 0, audioLimit: AUDIO_FETCH_LIMIT };
    }
    const screenOnly =
      refinements?.screenSource === true || (refinements?.apps?.length ?? 0) > 0;
    if (screenOnly) {
      return { frameLimit: FRAME_FETCH_LIMIT, audioLimit: 0 };
    }
    return { frameLimit: FRAME_FETCH_LIMIT, audioLimit: AUDIO_FETCH_LIMIT };
  });

  trimmedQuery = $derived(this.query.trim());
  belowMinimum = $derived(this.trimmedQuery.length < MIN_QUERY_LENGTH);
  hasResults = $derived(this.frames.length > 0 || this.audio.length > 0);

  // The friendly line for the active parse error (or null when none). Drives
  // both the inline error line under the input and the paused-results branch;
  // gated off `belowMinimum` so a parse error never shows for a query that's
  // too short to have run.
  parseErrorMessage = $derived<string | null>(
    this.firstParseError !== null && !this.belowMinimum
      ? friendlyParseError(this.firstParseError)
      : null,
  );

  // The bare "No matches" empty state must NOT show when the backend paused
  // results for a malformed filter — that reads as "found nothing" rather than
  // "your filter is broken". Gated on `parseErrorMessage === null` so the
  // dedicated paused-results branch owns the parse-error case instead.
  showEmpty = $derived(
    !this.belowMinimum &&
      !this.loading &&
      !this.errorMessage &&
      this.parseErrorMessage === null &&
      !this.hasResults &&
      this.resultsQuery.length > 0,
  );

  // Results are PAUSED (not empty, not errored) when the backend returned a
  // parse error for an at/above-minimum query that isn't mid-flight. The
  // backend suppresses results in this case; this branch renders a calm "fix
  // the filter" state in place of stale cards or the bare empty state.
  resultsPaused = $derived(
    !this.belowMinimum &&
      !this.loading &&
      !this.errorMessage &&
      this.parseErrorMessage !== null,
  );

  // Show the hint once results have run and no model is installed — the hint is
  // most useful exactly when keyword-only search underwhelms.
  showSemanticSearchHint = $derived(
    this.semanticSearchModelInstalled === false &&
      !this.belowMinimum &&
      !this.loading &&
      this.parseErrorMessage === null &&
      this.resultsQuery.length > 0,
  );

  async loadSemanticSearchModelInstalled(): Promise<void> {
    try {
      const status = await invoke<SemanticSearchModelStatusResponse>(
        "get_semantic_search_model_status",
      );
      this.semanticSearchModelInstalled = status.models.some(
        (model) => model.available,
      );
    } catch {
      // Best-effort: a failure just suppresses the hint (never blocks search).
      this.semanticSearchModelInstalled = null;
    }
  }

  async openSemanticSearchSettings(): Promise<void> {
    await openSettings("semanticSearch");
  }

  // Reset the search mode to the just-summoned empty state. The page-level
  // clearState() wraps this with the Ask AI teardown + mode flip + refocus.
  clearSearchState(): void {
    this.clearDebounce();
    // Invalidate any in-flight search so a late response can't repopulate.
    this.searchGeneration += 1;
    this.query = "";
    this.frames = [];
    this.audio = [];
    this.resultsQuery = "";
    this.errorMessage = null;
    this.loading = false;
    this.selectedIndex = -1;
    this.framesExpanded = false;
    this.audioExpanded = false;
    // Reset parsed scope to pristine so a re-summon starts with no chips /
    // parse errors / scope-narrowed limits.
    this.appliedRefinements = null;
    this.residualQuery = "";
    this.parseErrors = [];
    // A fresh summon starts with the Filter Picker closed.
    this.filters.pickerOpen = false;
    this.filters.pickerIndex = 0;
  }

  // The picker / ghost / value-list surfaces, composed last so every store
  // field above is initialized before the sub-surface captures the reference.
  filters = new FilterSurfaces(this);
}

export const quickRecallSearch = new SearchStore();
