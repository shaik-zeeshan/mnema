// Quick Recall's empty, partial and recent states (recall.html: recent, first
// run, no results, indexing behind, meaning off / building, audio pending, not
// recording) plus the pill popover's counts. Composed INTO the search store
// (`search.states`) like `filters`, because all of it reads the store's query
// and pills. The rules live in `recall-states.ts`; this makes the calls.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type {
  QuickRecallIndexStatus,
  RecentQuery,
  SearchCaptureResponse,
  SearchFacetCounts,
} from "$lib/types/app-infra";
import type { CaptureSession, GetPermissionsResponse } from "$lib/types/session";
import {
  popoverCalls,
  popoverCounts,
  recentText,
  withRecent,
  type FacetCall,
} from "./recall-states";
import { pillOptions } from "./filterSurfaces.svelte";
import { pillOperators } from "./filter-chips";
import type { SearchStore } from "./searchStore.svelte";

export class RecallStates {
  #store: SearchStore;

  constructor(store: SearchStore) {
    this.#store = store;
  }

  // Why results may be partial (QR-B2), from the last search response…
  indexEmpty = $state(false);
  semanticDegraded = $state(false);
  untranscribed = $state(0);
  // …and from the window's own reads: the OCR / meaning backlog and whether
  // Mnema is recording (null = not read yet).
  indexStatus = $state<QuickRecallIndexStatus | null>(null);
  recording = $state<boolean | null>(null);
  #sessionListening = false;

  // A search answered (or was reset: null).
  take(response: SearchCaptureResponse | null): void {
    this.indexEmpty = response?.indexEmpty ?? false;
    this.semanticDegraded = response?.semanticDegraded ?? false;
    this.untranscribed = response?.untranscribedSegmentsInRange ?? 0;
  }

  // Which empty state replaces the results list, once a search has answered.
  // The raw error text is never shown ("failed to search captured content: …").
  emptyState = $derived.by<"first" | "error" | "none" | null>(() => {
    const s = this.#store;
    if (s.belowMinimum || s.loading || s.resultsPaused) return null;
    if (s.errorMessage) return "error";
    if (this.indexEmpty && !s.hasResults) return "first";
    return s.showEmpty ? "none" : null;
  });

  // The OCR / meaning backlog and whether Mnema is recording: read with every
  // search, on mount / focus, and after a meaning-search setting changes.
  async loadIndexStatus(): Promise<void> {
    if (!this.#sessionListening) {
      this.#sessionListening = true;
      // ponytail: never unlistened — the store lives as long as the window.
      void listen<CaptureSession | null>("native_capture_session_changed", (event) => {
        this.recording = event.payload?.isRunning ?? false;
      }).catch(() => {});
    }
    // Best-effort: a failed read just drops its line.
    const [status, permissions] = await Promise.all([
      invoke<QuickRecallIndexStatus>("get_quick_recall_index_status").catch(() => null),
      invoke<GetPermissionsResponse>("get_capture_permissions").catch(() => null),
    ]);
    this.indexStatus = status;
    if (permissions) this.recording = permissions.session?.isRunning ?? false;
  }

  // ── Recent queries ─────────────────────────────────────────────────────────
  // ↓ on the empty bar shows Recent + Try; typing hides them. `recentIndex`
  // walks Recent, then Try.

  recent = $state<RecentQuery[]>([]);
  recentOpen = $state(false);
  recentIndex = $state(0);
  showRecent = $derived.by(() => this.recentOpen && this.#store.operatorQuery === "");

  async openRecent(): Promise<void> {
    this.recentIndex = 0;
    this.recentOpen = true;
    this.recent = await invoke<RecentQuery[]>("get_quick_recall_recent_queries").catch(
      () => this.recent,
    );
  }

  // Every search the user acts on (and every ask) goes to the top of Recent.
  async rememberQuery(ask = false): Promise<void> {
    const s = this.#store;
    const text = recentText(s.naturalParse.residual, s.effectivePills);
    if (text.length < 2) return;
    // Re-read first: `recent` is only loaded once ↓ opened it, and this write
    // replaces the whole stored list.
    const stored = await invoke<RecentQuery[]>("get_quick_recall_recent_queries").catch(() => null);
    if (stored === null) return;
    const queries = withRecent(stored, { text, ask, at: Date.now() });
    this.recent = queries;
    this.recent = await invoke<RecentQuery[]>("set_quick_recall_recent_queries", {
      queries,
    }).catch(() => queries);
  }

  async forgetRecent(index: number): Promise<void> {
    const queries = this.recent.filter((_, i) => i !== index);
    this.recent = queries;
    await invoke("set_quick_recall_recent_queries", { queries }).catch(() => {});
  }

  // Type a Recent / Try row in: its phrases tint, then hop into pills.
  useRecent(text: string): void {
    const s = this.#store;
    this.recentOpen = false;
    s.pills = [];
    s.query = text;
    s.caretAtEnd = true;
    s.inputEl?.focus();
    s.scheduleCommit();
  }

  // ── Counts ─────────────────────────────────────────────────────────────────

  countFacet(call: FacetCall): Promise<SearchFacetCounts | null> {
    return invoke<SearchFacetCounts>("count_search_facet", { request: call }).catch(() => null);
  }

  #facetGeneration = 0;

  // The pressed pill's option counts (`search.facetCounts`), fetched only when
  // its popover opens.
  async loadFacetCounts(index: number): Promise<void> {
    const s = this.#store;
    const generation = ++this.#facetGeneration;
    s.facetCounts = null;
    const pill = s.pills[index];
    if (!pill) return;
    const calls = popoverCalls(s.naturalParse.residual, s.effectivePills, pill);
    const results = await Promise.all(calls.map((call) => this.countFacet(call)));
    if (generation !== this.#facetGeneration) return;
    s.facetCounts = popoverCounts(
      pill,
      pillOptions(pill, s.appNames),
      calls.flatMap((call, i) => {
        const counts = results[i];
        return counts ? [{ ...call, counts }] : [];
      }),
      new Date(),
    );
    // Zero-count rows just dropped out; keep the highlight on the current one.
    const current = pillOperators(pill);
    s.filters.popoverIndex = Math.max(0, s.filters.popoverOptions.findIndex((o) => o.op === current));
  }
}
