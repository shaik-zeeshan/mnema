<script lang="ts">
  // Subjects — the Subjects destination's index (page 09, direction 03).
  //
  // This shell owns the data: the load, the per-subject trajectory fetch, the
  // realtime staging buffer, tiering and search. `SubjectRow.svelte` draws a
  // row and `subject-rows.ts` derives one; tier thresholds live in
  // `subjectsTiers.ts`. Tier labels float over the pane; the rows a tier holds
  // sit on ONE opaque plate, so a whole tier reads as a single card.
  //
  // Honesty rules this surface obeys (see 09's audit block):
  //   · There is NO subject-level score in the backend. The 0.72 on a row is
  //     literally the top conclusion's `confidence`, rendered toFixed(2) — which
  //     is why it never gets a progress bar of its own.
  //   · The index shows NO evidence count. It can only say "N conclusions";
  //     linked-activity counts appear on the DETAIL, where refs are resolved.
  //   · The sparkline has NO time axis. Snapshot points are evenly spaced by
  //     index — the backend stores a confidence history, not a time series.
  //   · The dashed line inside every sparkline is the 0.15 display floor, which
  //     is why the fading tier's lines are drawn crossing it, not deleted.
  //
  // Subjects are derived CLIENT-SIDE from `list_user_context_conclusions`,
  // grouped by `subject`. To draw honest per-conclusion trajectories (not flat
  // baselines) we lazily fetch `get_user_context_subject` per subject and use
  // each trajectory's real Confidence History; a failed fetch falls back to a
  // flat baseline at the current confidence.
  //
  // Tiering thresholds live ENTIRELY in `subjectsTiers.ts` — this component
  // never re-derives them. Below SPARSE_LIMIT subjects we skip tiers and render
  // one flat plate so early users don't see mostly-empty headers.
  //
  // Props:
  //   onOpenSubject: (subject: string) => void

  import { untrack } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { openSettings } from "$lib/surface-windows";
  import type {
    Conclusion,
    SubjectView,
    AiRuntimeStatus,
    UserContextStatus,
  } from "$lib/types/recording";
  import Skeleton from "$lib/insights/Skeleton.svelte";
  import SubjectRow from "$lib/insights/SubjectRow.svelte";
  import Segmented from "$lib/components/Segmented.svelte";
  import {
    type Axis,
    buildTiers,
    isSparse,
    summaryCounts,
    subjectsDiff,
    decideRefresh,
    debounce,
  } from "$lib/insights/subjectsTiers";
  import {
    type SubjectRow as Row,
    buildSubjectRows,
    displayedSubjectOrder,
    sortDisplayRows,
  } from "$lib/insights/subject-rows";
  import { rankSubjects } from "$lib/insights/subjectSearch";
  import { subjectSearch } from "$lib/insights/subject-search-state.svelte";
  import { humanizeError } from "$lib/format-error";

  // Number of placeholder rows shown while the conclusions load.
  const SKELETON_COUNT = 6;

  interface Props {
    onOpenSubject: (subject: string) => void;
  }

  let { onOpenSubject }: Props = $props();

  let conclusions = $state<Conclusion[] | null>(null);
  let loadError = $state<string | null>(null);
  let loading = $state(true);
  // Engine on/off — lets the empty state tell "engine is off, turn it on" apart
  // from "engine is on but hasn't formed any views yet" (two very different next
  // steps). null until the first status call resolves.
  let engineOn = $state<boolean | null>(null);

  // Grouping axis for the tier layout. "conviction" = how firmly held (default);
  // "movement" = which way it's heading. Drives `buildTiers`.
  let axis = $state<Axis>("conviction");
  const AXIS_OPTIONS = [
    { value: "conviction", label: "By conviction" },
    { value: "movement", label: "By movement" },
  ];

  // Search. The field itself lives in the TITLE BAR (page 09) and writes into
  // the shared slice store; `appliedQuery` is the debounced value the ranking
  // runs on. A non-empty applied query swaps the tiered layout for one flat
  // relevance-ranked plate.
  let appliedQuery = $state("");
  const applySearch = debounce((q: string) => {
    appliedQuery = q;
  }, 200);
  $effect(() => {
    applySearch(subjectSearch.query);
  });

  // Per-tier paging: each tier shows TIER_PAGE rows at a time. `tierShown` maps a
  // tier id to how many rows it currently reveals (absent → the default page).
  // Reassign the Map on change so the $state reacts (plain Maps aren't
  // deep-proxied in Svelte 5).
  const TIER_PAGE = 10;
  let tierShown = $state<Map<string, number>>(new Map());

  function shownCount(id: string): number {
    return tierShown.get(id) ?? TIER_PAGE;
  }
  function showMoreTier(id: string): void {
    const next = new Map(tierShown);
    next.set(id, shownCount(id) + TIER_PAGE);
    tierShown = next;
  }
  function collapseTier(id: string): void {
    const next = new Map(tierShown);
    next.delete(id);
    tierShown = next;
  }

  // Per-subject real trajectory history, fetched lazily. Maps subject → (map of
  // conclusionId → oldest-first confidence points). Used to draw honest spark
  // lines + derive warming/steady/cooling from the start-vs-end of the arc.
  let trajectories = $state<Map<string, Map<number, number[]>>>(new Map());
  // Monotonic generation token for `loadTrajectories`. Each call bumps it; only
  // the call whose token still matches at completion may write `trajectories`,
  // so a slow earlier load can't clobber a newer one's results.
  let trajectoriesGen = 0;

  // ---- Realtime staging buffer + refresh pill ------------------------------
  // Engine `user_context_changed` events never reflow the page while the user
  // reads. A debounced reload lands in `stagedConclusions`; the pill surfaces
  // "{pendingCount} views updated · refresh" and the swap happens only on the
  // pill click (or silently when the list is back at the top).
  let stagedConclusions = $state<Conclusion[] | null>(null);
  let pendingCount = $state(0);
  // Whether the scroll surface is at the top (best-effort) — gates silent apply.
  let atTop = $state(true);
  // The section root, used to resolve the nearest scroll container on mount.
  let rootEl = $state<HTMLElement | null>(null);

  const rows = $derived(
    conclusions ? buildSubjectRows(conclusions, trajectories) : [],
  );

  // The one display ordering — feeds the flat list, the tiers and the diff.
  const displayRows = $derived(sortDisplayRows(rows));

  // Ordered, non-empty tiers for the current axis (faded tier last).
  const tiers = $derived.by(() =>
    buildTiers(displayRows, axis).filter((t) => t.items.length > 0),
  );

  // Sparse mode: too few subjects to justify tier headers — one flat plate.
  const sparse = $derived(isSparse(displayRows.length));

  // Honest header counts (no rolled-up score) from the real displayed rows.
  const summary = $derived(summaryCounts(displayRows));

  // True when a search is active. Drives the layout swap (flat ranked plate) and
  // the header line (match count instead of the conviction/movement tallies).
  const searching = $derived(appliedQuery.trim().length > 0);

  // Relevance-ranked matches for the active query. Ranks over `displayRows` so
  // ties fall back to the same confidence-desc, faded-last order the tiers use.
  // Matches name + conclusion statements across ALL loaded rows (fading too).
  const searchResults = $derived.by<Row[]>(() =>
    searching ? rankSubjects(displayRows, appliedQuery) : [],
  );

  // The single network read. Returns the fresh list; sets loadError on failure
  // (and returns the current list so callers don't blow away what's displayed).
  async function fetchConclusions(): Promise<Conclusion[]> {
    try {
      const list = await invoke<Conclusion[]>("list_user_context_conclusions", {
        includeFaded: true,
      });
      loadError = null;
      return list;
    } catch (error) {
      // Only surface the full error screen when there's nothing to preserve
      // (initial load — `conclusions` still null). A background realtime refetch
      // failure keeps the intact rendered rows instead of flashing the error
      // state over good content; we still return the current list below.
      if (!conclusions?.length) {
        loadError = humanizeError(error);
      }
      return conclusions ?? [];
    }
  }

  // Swap a list into the DISPLAYED dataset and refresh its trajectories.
  function applyConclusions(list: Conclusion[]): void {
    conclusions = list;
    void loadTrajectories(list);
  }

  // Initial mount load (immediate — first paint is never gated behind a pill).
  async function loadConclusions(): Promise<void> {
    loading = true;
    try {
      applyConclusions(await fetchConclusions());
    } finally {
      loading = false;
    }
  }

  // Probe whether the Reasoning Engine is on so the empty state can disambiguate
  // engine-off from no-data. Best-effort: a failed probe leaves `engineOn` null,
  // and the empty state falls back to neutral both-cases copy.
  async function loadEngineStatus(): Promise<void> {
    const [ai, ctx] = await Promise.all([
      invoke<AiRuntimeStatus>("get_ai_runtime_status").catch(() => null),
      invoke<UserContextStatus>("get_user_context_status").catch(() => null),
    ]);
    engineOn =
      Boolean(ai?.enabled && ai?.available) || Boolean(ctx?.engineAvailable);
  }

  // Apply the staged reload now (the refresh-pill click, or auto-apply on idle).
  function applyStaged(): void {
    if (!stagedConclusions) return;
    applyConclusions(stagedConclusions);
    stagedConclusions = null;
    pendingCount = 0;
  }

  // Lazily fetch real per-subject Confidence History so the sparklines + trend
  // glyphs reflect actual movement rather than flat baselines. Best-effort: a
  // failed fetch just leaves that subject on its baseline. Bounded concurrency
  // keeps a large dossier responsive.
  async function loadTrajectories(list: Conclusion[]): Promise<void> {
    const gen = ++trajectoriesGen;
    const subjects = [...new Set(list.map((c) => c.subject))];
    const next = new Map<string, Map<number, number[]>>();
    const CONCURRENCY = 4;
    let cursor = 0;
    async function worker(): Promise<void> {
      while (cursor < subjects.length) {
        const subject = subjects[cursor++];
        try {
          const view = await invoke<SubjectView>("get_user_context_subject", {
            subject,
          });
          const byId = new Map<number, number[]>();
          for (const t of view.trajectories) {
            byId.set(
              t.conclusionId,
              t.history.map((h) => h.confidence),
            );
          }
          next.set(subject, byId);
        } catch {
          // Best-effort; subject keeps its flat baseline.
        }
      }
    }
    await Promise.all(
      Array.from({ length: Math.min(CONCURRENCY, subjects.length) }, worker),
    );
    // Drop the result if a newer load started while this one was in flight.
    if (gen !== trajectoriesGen) return;
    trajectories = next;
  }

  // The debounced engine-change handler: fetch fresh data, diff it against what
  // is displayed (in the SAME display order), then decide. "apply" swaps it in
  // silently; "stage" holds it behind the pill; "ignore" refreshes figures only.
  async function onContextChanged(): Promise<void> {
    const next = await fetchConclusions();
    const displayedOrder = displayedSubjectOrder(
      untrack(() => conclusions) ?? [],
    );
    const stagedOrder = displayedSubjectOrder(next);
    const diff = subjectsDiff(displayedOrder, stagedOrder);
    // Rows navigate rather than expand in place (page 09), so the only reason to
    // hold a reload back is that the reader has scrolled away from the top.
    const action = decideRefresh({
      changed: diff.changed,
      expanded: false,
      atTop: untrack(() => atTop),
    });
    if (action === "ignore") {
      // Membership + display order unchanged — but the per-row FIGURES may have
      // moved (confidence values, trajectory/sparkline points). Refresh them in
      // place: no row reorders, appears or disappears, only the numbers and
      // sparklines catch up.
      applyConclusions(next);
      return;
    }
    if (action === "apply") {
      applyConclusions(next);
      stagedConclusions = null;
      pendingCount = 0;
      return;
    }
    // "stage": hold behind the pill; do NOT touch `conclusions`.
    stagedConclusions = next;
    pendingCount = diff.count;
  }

  // Auto-apply on idle: once the surface is back at the top, a staged reload
  // swaps in without needing the pill click. The pill remains the primary path.
  $effect(() => {
    if (atTop && stagedConclusions !== null) {
      applyStaged();
    }
  });

  $effect(() => {
    void untrack(() => loadConclusions());
    void untrack(() => loadEngineStatus());

    // Debounce the engine-change reload (store the wrapped fn so cleanup can
    // cancel a pending trailing call on unmount).
    const debounced = debounce(() => void onContextChanged(), 500);

    let unlisten: UnlistenFn | undefined;
    let disposed = false;
    void listen("user_context_changed", () => {
      debounced();
      void loadEngineStatus();
    }).then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    });

    // atTop detection: find the nearest scroll container (the destination pane
    // scrolls) and track its scrollTop. Best-effort — fall back to window
    // scroll if no container resolves. Listener removed on unmount.
    let scrollEl: HTMLElement | null = null;
    let usingWindow = false;
    const updateAtTop = (): void => {
      const top = scrollEl ? scrollEl.scrollTop : window.scrollY;
      atTop = top <= 8;
    };
    if (rootEl) {
      let el: HTMLElement | null = null;
      let p = rootEl.parentElement;
      while (p) {
        const oy = getComputedStyle(p).overflowY;
        if (oy === "auto" || oy === "scroll") {
          el = p;
          break;
        }
        p = p.parentElement;
      }
      scrollEl = el;
    }
    if (scrollEl) {
      scrollEl.addEventListener("scroll", updateAtTop, { passive: true });
    } else {
      usingWindow = true;
      window.addEventListener("scroll", updateAtTop, { passive: true });
    }
    updateAtTop();

    return () => {
      disposed = true;
      unlisten?.();
      debounced.cancel();
      applySearch.cancel();
      if (scrollEl) scrollEl.removeEventListener("scroll", updateAtTop);
      if (usingWindow) window.removeEventListener("scroll", updateAtTop);
    };
  });
</script>

<section class="subjects" aria-label="Subjects" bind:this={rootEl}>
  <div class="sbhead">
    <h1>Subjects</h1>
    <p>
      What Mnema has come to believe about you — and how firmly. Strongest views
      first; fading ones are kept for history.
    </p>
  </div>

  <!-- Realtime refresh pill — appears when an engine update is staged behind a
       scrolled reader. Click swaps it in; the page never reflows underneath. -->
  {#if pendingCount > 0 || stagedConclusions !== null}
    <div class="refresh-bar">
      <button type="button" class="btn btn--accent btn--sm" onclick={applyStaged}>
        ↻ {pendingCount > 0
          ? `${pendingCount} ${pendingCount === 1 ? "view" : "views"} updated`
          : "views updated"} · refresh
      </button>
    </div>
  {/if}

  <div class="sbbar">
    <!-- Honest counts line (no rolled-up score). Held until conclusions load so
         the header doesn't jank; the empty state covers the zero case. -->
    {#if conclusions && displayRows.length > 0}
      {#if searching}
        <span class="counts is-num">
          {searchResults.length}
          {searchResults.length === 1 ? "match" : "matches"} for “{appliedQuery.trim()}”
        </span>
      {:else}
        <span class="counts is-num">
          {summary.active} active views · {summary.fading} fading —
          {summary.warming} warming <span class="up">▲</span> ·
          {summary.steady} steady · {summary.cooling} cooling
          <span class="dn">▼</span>
        </span>
      {/if}
    {:else}
      <span class="counts"></span>
    {/if}
    <Segmented
      options={AXIS_OPTIONS}
      value={axis}
      onValueChange={(v) => (axis = v as Axis)}
      ariaLabel="Organize subjects by"
    />
  </div>
  <!-- Define the active grouping axis — "conviction" and "movement" aren't
       self-evident terms. -->
  <p class="sbhint">
    {#if axis === "conviction"}
      Conviction — how firmly the engine holds each view (its confidence).
    {:else}
      Movement — which way each view is trending: warming, steady, or cooling.
    {/if}
  </p>

  {#if loadError && !conclusions}
    <div class="plate state state--error">
      <p class="state-title">Couldn't load Subjects.</p>
      <p class="state-detail">{loadError}</p>
      <button
        type="button"
        class="btn btn--sm state-retry"
        onclick={() => void loadConclusions()}
        disabled={loading}
      >
        ↻ Try again
      </button>
    </div>
  {:else if loading && !conclusions}
    <!-- Loading skeleton — rows on a plate, matching the loaded row shape so the
         swap to real content causes no layout shift. -->
    <div class="tplate" aria-label="Loading subjects" aria-busy="true">
      {#each Array.from({ length: SKELETON_COUNT }) as _, i (i)}
        <div class="skrow">
          <span class="skrow__t">
            <Skeleton variant="text" width="34%" height="14px" />
            <Skeleton variant="text" width="62%" height="12px" />
            <Skeleton variant="text" width="86px" height="11px" />
          </span>
          <Skeleton width="172px" height="34px" radius="4px" />
          <Skeleton variant="text" width="44px" height="17px" />
        </div>
      {/each}
    </div>
  {:else if displayRows.length === 0}
    <div class="plate state">
      {#if engineOn === false}
        <!-- Engine is off — the actionable case: a direct path to turn it on. -->
        <p class="state-title">The Reasoning Engine is off.</p>
        <p class="state-detail">
          Subjects appear as the engine forms views about you — each with its own
          confidence trajectory. Turn it on to begin.
        </p>
        <button
          type="button"
          class="btn btn--accent state-retry"
          onclick={() => void openSettings("intelligence")}
        >
          Open engine settings
        </button>
      {:else}
        <!-- Engine is on (or status unknown) — nothing concluded yet. -->
        <p class="state-title">No subjects yet.</p>
        <p class="state-detail">
          As the Reasoning Engine forms views about you, each one appears here
          with its own confidence trajectory. Keep working and check back — they
          build up as evidence accumulates.
        </p>
      {/if}
    </div>
  {:else if searching}
    <!-- Search active: one flat plate ranked by relevance, no tier headers. -->
    {#if searchResults.length === 0}
      <div class="plate state">
        <p class="state-title">No subjects match “{appliedQuery.trim()}”.</p>
        <p class="state-detail">
          Search looks at subject names and the wording of each belief. Try a
          shorter or different term, or clear the search to browse all subjects.
        </p>
      </div>
    {:else}
      <div class="tplate">
        {#each searchResults as r (r.subject)}
          <SubjectRow row={r} onOpen={onOpenSubject} />
        {/each}
      </div>
    {/if}
  {:else if sparse}
    <!-- Sparse: one ungrouped plate, no tier headers. -->
    <div class="tplate">
      {#each displayRows as r (r.subject)}
        <SubjectRow row={r} onOpen={onOpenSubject} />
      {/each}
    </div>
    <p class="sfoot">
      Confidence is recency-weighted — views warm with fresh evidence and cool on
      their own. Faded views are kept for history, never deleted.
    </p>
  {:else}
    <!-- Tiered: the label floats over the pane, the rows share ONE plate, so a
         whole tier reads as a single card. -->
    {#each tiers as tier (tier.id)}
      {@const visible = shownCount(tier.id)}
      {@const shown = tier.items.slice(0, visible)}
      {@const hidden = tier.items.length - shown.length}
      {@const nextPage = Math.min(TIER_PAGE, hidden)}
      <section class="tier" class:tier--faded={tier.faded}>
        <div class="tier__h">
          <span class="tier__t">{tier.title}</span>
          <span class="tier__note">{tier.note}</span>
          <span class="tier__n is-num">{tier.items.length}</span>
        </div>
        <div class="tplate">
          {#each shown as r (r.subject)}
            <SubjectRow row={r} onOpen={onOpenSubject} />
          {/each}
        </div>
        {#if hidden > 0}
          <button
            type="button"
            class="btn btn--ghost btn--sm tier__more"
            onclick={() => showMoreTier(tier.id)}
          >
            Show {nextPage} more
          </button>
        {:else if visible > TIER_PAGE}
          <button
            type="button"
            class="btn btn--ghost btn--sm tier__more"
            onclick={() => collapseTier(tier.id)}
          >
            Show less
          </button>
        {/if}
      </section>
    {/each}

    <p class="sfoot">
      Confidence is recency-weighted — views warm with fresh evidence and cool on
      their own. Faded views are kept for history, never deleted.
    </p>
  {/if}
</section>

<style>
  .subjects {
    display: flex;
    flex-direction: column;
  }

  /* ---- Header ---- */
  .sbhead {
    margin-bottom: 10px;
  }
  .sbhead h1 {
    margin: 0 0 var(--s-4);
    font: var(--w-semi) var(--t-display) / var(--lh-display) var(--app-font-sans);
    letter-spacing: var(--ls-display);
    color: var(--app-text-strong);
  }
  .sbhead p {
    margin: 0;
    max-width: 78ch;
    font: var(--w-regular) var(--t-meta) / 1.5 var(--app-font-sans);
    color: var(--app-text-muted);
  }

  .sbbar {
    display: flex;
    align-items: center;
    gap: var(--s-12);
    margin: var(--s-12) 0 var(--s-6);
  }
  .counts {
    flex: 1 1 auto;
    min-width: 0;
    font: var(--w-regular) var(--t-meta) / var(--lh-meta) var(--app-font-sans);
    color: var(--app-text-muted);
  }
  .counts .up {
    color: var(--app-accent);
  }
  /* Cooling is normal decay, not an error — it stays quiet. */
  .counts .dn {
    color: var(--app-text-faint);
  }
  .sbhint {
    margin: 0 0 var(--s-16);
    font: var(--w-regular) var(--t-meta) / 1.4 var(--app-font-sans);
    color: var(--app-text-subtle);
  }

  .refresh-bar {
    position: sticky;
    top: var(--s-6);
    z-index: 3;
    display: flex;
    justify-content: center;
    margin: 0 0 var(--s-8);
  }

  /* ---- Tier: a labelled band. The label floats over the pane; the rows sit on
     ONE opaque plate, so a whole tier reads as a single card. ---- */
  .tier {
    margin-bottom: var(--s-16);
  }
  .tier__h {
    display: flex;
    align-items: baseline;
    gap: 9px;
    padding: 0 var(--s-4) var(--s-6);
  }
  .tier__h:after {
    content: "";
    flex: 1;
    height: 1px;
    background: var(--app-border);
    align-self: center;
  }
  .tier__t {
    font: var(--w-medium) var(--t-label) / var(--lh-label) var(--app-font-mono);
    letter-spacing: var(--ls-label);
    text-transform: uppercase;
    color: var(--app-text-muted);
  }
  .tier--faded .tier__t {
    color: var(--app-text-subtle);
  }
  .tier__note {
    font: var(--w-regular) var(--t-meta) / var(--lh-meta) var(--app-font-sans);
    color: var(--app-text-subtle);
  }
  .tier__n {
    font: var(--w-regular) var(--t-meta) / 1 var(--app-font-mono);
    color: var(--app-text-faint);
  }
  .tier__more {
    margin-top: var(--s-6);
  }

  /* The tier plate — opaque, one per tier. */
  .tplate {
    border-radius: var(--r-lg);
    background: var(--app-surface);
    box-shadow: var(--sh-tile);
    padding: 0 var(--s-12);
  }
  /* Rows share ONE plate, so their only separator is a hairline of the
     material's own rim — never a border. Lives here rather than in
     SubjectRow.svelte: a component can't see its own siblings, so the compiler
     prunes the rule as unused when it is scoped to the row. */
  .tplate :global(.srow + .srow) {
    box-shadow: inset 0 1px 0 var(--glass-line);
  }

  /* Skeleton row — matches the loaded row's rhythm so the swap causes no
     layout shift. The real row's styles live in SubjectRow.svelte. */
  .skrow {
    display: flex;
    align-items: center;
    gap: 14px;
    min-height: 52px;
    padding: 9px 0;
  }
  .skrow + .skrow {
    box-shadow: inset 0 1px 0 var(--glass-line);
  }
  .skrow__t {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: var(--s-4);
  }

  .sfoot {
    margin-top: var(--s-2);
    padding: 0 var(--s-4);
    max-width: 82ch;
    font: var(--w-regular) var(--t-meta) / 1.5 var(--app-font-sans);
    color: var(--app-text-subtle);
  }

  /* ---- States ---- */
  .state {
    display: flex;
    flex-direction: column;
    gap: var(--s-6);
    padding: var(--s-16);
  }
  .state--error {
    background: var(--app-danger-bg);
    box-shadow: var(--sh-tile), inset 0 0 0 var(--hairline) var(--app-danger-border);
  }
  .state-title {
    margin: 0;
    font: var(--w-medium) var(--t-ui) / var(--lh-ui) var(--app-font-sans);
    color: var(--app-text-strong);
  }
  .state-detail {
    margin: 0;
    font: var(--w-regular) var(--t-meta) / 1.6 var(--app-font-sans);
    color: var(--app-text-muted);
    max-width: 70ch;
  }
  .state-retry {
    align-self: flex-start;
    margin-top: var(--s-4);
  }

</style>
