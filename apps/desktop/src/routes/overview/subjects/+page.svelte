<script lang="ts">
  // ══ SUBJECTS — the tier list ═══════════════════════════════════════════════
  //
  // A DESTINATION inside Overview (`/overview/subjects`). Rows drill into
  // `/overview/subjects/[subject]`; the titlebar back control walks the trail
  // (`+layout.svelte`), so nothing here draws one.
  //
  // Subjects group into ordered TIERS along an axis — conviction (how firmly
  // held) or movement (which way it is heading). The three live conviction
  // thresholds are the ENGINE's own numbers, printed on each tier header, and
  // they are imported from `subjectsTiers.ts` rather than re-typed here.
  //
  // The direction's third instrument face lives on every row: the confidence
  // TRACE, in a well, with the 0.15 display floor drawn dashed under it. It
  // READS — nothing on this page turns. Its x-axis is real snapshot time, and
  // every row on the page shares ONE domain (`traceDomain`), so a six-week-old
  // trace actually looks six weeks long next to a six-day-old one.
  //
  // G8: every count and percentage below is a real read. A subject whose
  // trajectory fetch fails renders no trace rather than a straight line, and
  // confidence prints as `NN%` here exactly as it does on the drill-in — the
  // old index printed a raw `0.86` for the same number.
  //
  // Two corrections exist in this feature, both per-conclusion and both on the
  // drill-in: Pin and Dismiss. There is no edit — a belief is superseded, never
  // rewritten — and no per-subject forget.
  import { untrack } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import type {
    AiRuntimeStatus,
    ConfidenceSnapshot,
    Conclusion,
    SubjectView,
    UserContextStatus,
  } from "$lib/types/recording";
  import {
    DISPLAY_FLOOR,
    INITIAL_BASE,
    STRONGLY_HELD,
    buildTiers,
    debounce,
    decideRefresh,
    deriveTrend,
    isSparse,
    subjectsDiff,
    summaryCounts,
    type Axis,
    type TierSubject,
  } from "$lib/insights/subjectsTiers";
  import { rankSubjects } from "$lib/insights/subjectSearch";
  import { humanizeError } from "$lib/format-error";
  import { openSettings } from "$lib/surface-windows";
  import Segmented from "$lib/components/Segmented.svelte";
  import SubjectRow from "$lib/overview/subjects/SubjectRow.svelte";

  // A row satisfies TierSubject, so the tiering helpers group it directly.
  interface Row extends TierSubject {
    subject: string;
    conclusions: Conclusion[];
    conclusionCount: number;
    pinned: boolean;
    statement: string;
    lead: ConfidenceSnapshot[];
    others: ConfidenceSnapshot[][];
  }

  let conclusions = $state<Conclusion[] | null>(null);
  let loadError = $state<string | null>(null);
  // Per-subject real confidence history: subject → (conclusionId → snapshots).
  let traces = $state(new Map<string, Map<number, ConfidenceSnapshot[]>>());
  let tracesGen = 0;
  // null until the probe resolves — the empty state must tell "engine off" from
  // "engine on, nothing formed yet", which are different next steps.
  let engineOn = $state<boolean | null>(null);

  let axis = $state<Axis>("conviction");
  const AXIS_OPTIONS = [
    { value: "conviction", label: "Conviction" },
    { value: "movement", label: "Movement" },
  ];

  let searchQuery = $state("");
  let appliedQuery = $state("");
  const applySearch = debounce((q: string) => {
    appliedQuery = q;
  }, 200);

  // Engine `user_context_changed` events never reflow the page while you read:
  // a debounced reload lands here and the chip offers the swap.
  let staged = $state<Conclusion[] | null>(null);
  let pendingCount = $state(0);
  let atTop = $state(true);
  let paneEl = $state<HTMLElement | null>(null);

  const rows = $derived.by<Row[]>(() => {
    if (!conclusions) return [];
    const groups = new Map<string, Conclusion[]>();
    for (const c of conclusions) {
      const bucket = groups.get(c.subject);
      if (bucket) bucket.push(c);
      else groups.set(c.subject, [c]);
    }
    const out: Row[] = [];
    for (const [subject, cs] of groups) {
      const history = traces.get(subject);
      const sorted = [...cs].sort((a, b) => b.confidence - a.confidence);
      const top = sorted[0];
      // deriveTrend reads plain confidence arrays; the traces themselves keep
      // their timestamps because the x-axis is time, not index.
      const asNumbers = new Map<number, number[]>();
      if (history)
        for (const [id, points] of history)
          asNumbers.set(
            id,
            points.map((p) => p.confidence),
          );
      out.push({
        subject,
        conclusions: sorted,
        conclusionCount: cs.length,
        pinned: cs.some((c) => c.pinned),
        faded: cs.every((c) => c.status === "faded"),
        statement: top?.statement ?? subject,
        lastMovedAtMs: cs.reduce(
          (acc, c) => Math.max(acc, c.updatedAtMs, c.lastSupportedAtMs),
          0,
        ),
        trend: deriveTrend(cs, asNumbers),
        topConfidence: top?.confidence ?? 0,
        lead: (top && history?.get(top.id)) || [],
        others: sorted
          .slice(1)
          .map((c) => history?.get(c.id) ?? [])
          .filter((h) => h.length >= 2),
      });
    }
    return out;
  });

  // One ordering feeds both the flat list and the tiers: live before fading,
  // then confidence desc.
  const displayRows = $derived.by<Row[]>(() =>
    [...rows].sort(
      (a, b) =>
        Number(a.faded) - Number(b.faded) ||
        b.topConfidence - a.topConfidence ||
        a.subject.localeCompare(b.subject),
    ),
  );

  // ONE clock for every trace on the page — the whole point of the x-axis fix.
  const traceDomain = $derived.by<[number, number] | undefined>(() => {
    let first = Infinity;
    let last = -Infinity;
    for (const row of displayRows)
      for (const line of [row.lead, ...row.others])
        for (const point of line) {
          if (point.snapshotAtMs < first) first = point.snapshotAtMs;
          if (point.snapshotAtMs > last) last = point.snapshotAtMs;
        }
    return last > first ? [first, last] : undefined;
  });

  const searching = $derived(appliedQuery.trim().length > 0);
  const results = $derived(searching ? rankSubjects(displayRows, appliedQuery) : []);
  const sparse = $derived(isSparse(displayRows.length));
  const tiers = $derived(buildTiers(displayRows, axis).filter((t) => t.items.length > 0));
  const summary = $derived(summaryCounts(displayRows));

  // "12 active views · 4 fading" — each half only when it is really there.
  const countLine = $derived.by(() => {
    const parts: string[] = [];
    if (summary.active > 0)
      parts.push(`${summary.active} active view${summary.active === 1 ? "" : "s"}`);
    if (summary.fading > 0) parts.push(`${summary.fading} fading`);
    return parts.join(" · ");
  });
  // "3 warming ▲ · 7 steady – · 2 cooling ▼", zero buckets omitted.
  const movementLine = $derived.by(() => {
    const parts: string[] = [];
    if (summary.warming > 0) parts.push(`${summary.warming} warming ▲`);
    if (summary.steady > 0) parts.push(`${summary.steady} steady –`);
    if (summary.cooling > 0) parts.push(`${summary.cooling} cooling ▼`);
    return parts.join(" · ");
  });

  // The tier note carries the ENGINE CONSTANT that defines the tier, so the
  // grouping is auditable rather than asserted. Only conviction has thresholds;
  // movement's buckets are the ±0.04 dead-band, which the note already words.
  function tierNote(id: string, note: string): string {
    if (id === "strong") return `${note} · ≥ ${STRONGLY_HELD.toFixed(2)}`;
    if (id === "forming") return `${note} · ≥ ${INITIAL_BASE.toFixed(2)}`;
    if (id === "shaping") return `${note} · under ${INITIAL_BASE.toFixed(2)}`;
    if (id === "fading") return "below the display floor";
    return note;
  }

  // ── reads ────────────────────────────────────────────────────────────────
  async function fetchConclusions(): Promise<Conclusion[]> {
    try {
      const list = await invoke<Conclusion[]>("list_user_context_conclusions", {
        includeFaded: true,
      });
      loadError = null;
      return list;
    } catch (error) {
      // A background refresh that fails keeps the rows on screen and says
      // nothing; only a first load with nothing to preserve shows the error.
      if (!untrack(() => conclusions)?.length) loadError = humanizeError(error);
      return untrack(() => conclusions) ?? [];
    }
  }

  function apply(list: Conclusion[]): void {
    conclusions = list;
    void loadTraces(list);
  }

  async function loadTraces(list: Conclusion[]): Promise<void> {
    const gen = ++tracesGen;
    const subjects = [...new Set(list.map((c) => c.subject))];
    const next = new Map<string, Map<number, ConfidenceSnapshot[]>>();
    const CONCURRENCY = 4;
    let cursor = 0;
    const worker = async (): Promise<void> => {
      while (cursor < subjects.length) {
        const subject = subjects[cursor++];
        try {
          const view = await invoke<SubjectView>("get_user_context_subject", { subject });
          const byId = new Map<number, ConfidenceSnapshot[]>();
          for (const t of view.trajectories) byId.set(t.conclusionId, t.history);
          next.set(subject, byId);
        } catch {
          // Best-effort: no trace beats an invented one.
        }
      }
    };
    await Promise.all(Array.from({ length: Math.min(CONCURRENCY, subjects.length) }, worker));
    if (gen !== tracesGen) return; // a newer load won
    traces = next;
  }

  async function loadEngineStatus(): Promise<void> {
    const [ai, ctx] = await Promise.all([
      invoke<AiRuntimeStatus>("get_ai_runtime_status").catch(() => null),
      invoke<UserContextStatus>("get_user_context_status").catch(() => null),
    ]);
    engineOn = Boolean(ai?.enabled && ai?.available) || Boolean(ctx?.engineAvailable);
  }

  function applyStaged(): void {
    if (!staged) return;
    apply(staged);
    staged = null;
    pendingCount = 0;
  }

  async function onContextChanged(): Promise<void> {
    const next = await fetchConclusions();
    const order = (list: Conclusion[]): string[] => {
      const best = new Map<string, { faded: boolean; top: number }>();
      for (const c of list) {
        const held = best.get(c.subject);
        best.set(c.subject, {
          faded: (held?.faded ?? true) && c.status === "faded",
          top: Math.max(held?.top ?? 0, c.confidence),
        });
      }
      return [...best.entries()]
        .sort(
          (a, b) =>
            Number(a[1].faded) - Number(b[1].faded) ||
            b[1].top - a[1].top ||
            a[0].localeCompare(b[0]),
        )
        .map(([subject]) => subject);
    };
    const diff = subjectsDiff(order(untrack(() => conclusions) ?? []), order(next));
    // No row expands on this surface — the drill-in is a route — so the only
    // reason to hold a swap back is that the reader has scrolled.
    const action = decideRefresh({
      changed: diff.changed,
      expanded: false,
      atTop: untrack(() => atTop),
    });
    if (action === "stage") {
      staged = next;
      pendingCount = diff.count;
      return;
    }
    // "ignore" still refreshes the FIGURES in place: membership and order are
    // unchanged, so nothing moves under the reader — only the numbers catch up.
    apply(next);
    staged = null;
    pendingCount = 0;
  }

  $effect(() => {
    void untrack(async () => {
      apply(await fetchConclusions());
    });
    void untrack(() => loadEngineStatus());

    const debounced = debounce(() => void onContextChanged(), 500);
    let unlisten: UnlistenFn | undefined;
    let disposed = false;
    void listen("user_context_changed", () => debounced()).then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    });
    return () => {
      disposed = true;
      unlisten?.();
      debounced.cancel();
    };
  });

  function onScroll(): void {
    atTop = (paneEl?.scrollTop ?? 0) < 24;
    if (atTop && staged) applyStaged();
  }
</script>

{#snippet subjectRow(row: Row)}
  <SubjectRow
    subject={row.subject}
    statement={row.statement}
    conclusionCount={row.conclusionCount}
    confidence={row.topConfidence}
    trend={row.trend}
    pinned={row.pinned}
    faded={row.faded}
    lastMovedAtMs={row.lastMovedAtMs}
    lead={row.lead}
    others={row.others}
    domain={traceDomain}
  />
{/snippet}

<div class="dest">
  <header class="dest__bar">
    <span class="t-title">Subjects</span>
    <span class="t-meta dest__lede">
      what Mnema has come to believe about you — and how firmly
    </span>
    <span class="dest__sp"></span>
    <Segmented
      options={AXIS_OPTIONS}
      value={axis}
      onValueChange={(v) => (axis = v as Axis)}
      ariaLabel="Group subjects by"
      compact
    />
    <input
      class="input dest__search"
      type="search"
      placeholder="Search subjects"
      aria-label="Search subjects"
      bind:value={searchQuery}
      oninput={() => applySearch(searchQuery)}
    />
  </header>

  <div class="dest__pane" bind:this={paneEl} onscroll={onScroll}>
    <div class="scol">
      {#if loadError && !conclusions?.length}
        <div class="estate">
          <span class="t-ui estate__t">Couldn't load Subjects.</span>
          <span class="t-meta">{loadError}</span>
          <button
            type="button"
            class="btn btn--sm estate__go"
            onclick={() => void (async () => apply(await fetchConclusions()))()}
          >
            Try again
          </button>
        </div>
      {:else if displayRows.length === 0 && conclusions !== null}
        {#if engineOn === false}
          <div class="estate">
            <span class="t-ui estate__t">The Reasoning Engine is off.</span>
            <span class="t-meta">
              Subjects appear as the engine forms views about you, each with its own
              confidence trajectory. Turn it on to begin.
            </span>
            <button
              type="button"
              class="btn btn--sm estate__go"
              onclick={() => void openSettings("intelligence")}
            >
              Open engine settings
            </button>
          </div>
        {:else}
          <div class="estate">
            <span class="t-ui estate__t">No subjects yet.</span>
            <span class="t-meta">
              As the engine forms views about you, each one appears here with its own
              confidence trajectory. Keep working and check back — they build up as
              evidence accumulates.
            </span>
          </div>
        {/if}
      {:else if conclusions !== null}
        <!-- The count line stands down when the no-match card is up: the card
             already names the term, and saying it twice is not honesty. -->
        <div class="sumline" class:sumline--hidden={searching && results.length === 0}>
          {#if searching}
            <span class="t-ui strong is-num">
              {results.length}
              {results.length === 1 ? "subject matches" : "subjects match"}
              “{appliedQuery.trim()}”
            </span>
          {:else}
            {#if countLine}<span class="t-ui strong is-num">{countLine}</span>{/if}
            {#if movementLine}<span class="t-meta is-num">— {movementLine}</span>{/if}
          {/if}
          {#if pendingCount > 0}
            <button type="button" class="ti-chip ti-chip--acc sumline__refresh" onclick={applyStaged}>
              ↻ {pendingCount} view{pendingCount === 1 ? "" : "s"} updated · refresh
            </button>
          {/if}
        </div>

        {#if searching}
          {#if results.length === 0}
            <div class="estate">
              <span class="t-ui estate__t">No subjects match “{appliedQuery.trim()}”.</span>
              <span class="t-meta">
                Search looks at subject names and the wording of each belief. Try a
                shorter term, or clear the search to browse all subjects.
              </span>
            </div>
          {:else}
            <div class="ti-grp">
              {#each results as row (row.subject)}{@render subjectRow(row)}{/each}
            </div>
          {/if}
        {:else if sparse}
          <!-- Under five subjects, sorting three things into four buckets is
               theatre: one flat list, no tier headers. -->
          <div class="ti-grp">
            {#each displayRows as row (row.subject)}{@render subjectRow(row)}{/each}
          </div>
        {:else}
          {#each tiers as tier (tier.id)}
            <div class="tier-h">
              <span class="t-ui tier-h__t">{tier.title}</span>
              <span class="t-meta tier-h__note is-num">{tierNote(tier.id, tier.note)}</span>
            </div>
            <div class="ti-grp">
              {#each tier.items as row (row.subject)}{@render subjectRow(row)}{/each}
            </div>
          {/each}
        {/if}

        {#if !searching || results.length > 0}
          <p class="t-meta foot">
            Confidence is recency-weighted — a view warms with fresh evidence and cools
            on its own unless it is pinned. Anything under {Math.round(DISPLAY_FLOOR * 100)}%
            is kept for history, never deleted.
          </p>
        {/if}
      {/if}
    </div>
  </div>
</div>

<style>
  .dest {
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .dest__bar {
    flex: 0 0 auto;
    height: 40px;
    display: flex;
    align-items: center;
    gap: var(--s-8);
    padding: 0 var(--s-16);
    box-shadow: inset 0 -1px 0 var(--app-border);
  }
  .dest__sp {
    flex: 1 1 auto;
  }
  .dest__lede {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .dest__search {
    width: 168px;
    height: var(--h-sm);
    font-size: var(--t-meta);
  }
  .dest__pane {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    padding: var(--s-12) var(--s-20) var(--s-24);
  }
  .scol {
    max-width: 820px;
    margin: 0 auto;
  }

  .sumline {
    display: flex;
    align-items: baseline;
    gap: var(--s-8);
    padding: var(--s-4) var(--s-2) var(--s-12);
  }
  .sumline--hidden {
    display: none;
  }
  .sumline__refresh {
    margin-left: auto;
    cursor: pointer;
  }
  .strong {
    color: var(--app-text-strong);
  }

  .tier-h {
    display: flex;
    align-items: baseline;
    gap: var(--s-8);
    padding: var(--s-16) var(--s-2) var(--s-6);
  }
  .tier-h__t {
    font-weight: var(--w-semi);
    color: var(--app-text-strong);
  }
  .tier-h__note {
    margin-left: auto;
    color: var(--app-text-faint);
  }

  /* An empty state is a fill on the window, not a bordered card. */
  .estate {
    display: flex;
    flex-direction: column;
    gap: var(--s-6);
    align-items: flex-start;
    max-width: 52ch;
    margin-top: var(--s-16);
    padding: var(--s-16);
    border-radius: var(--r-lg);
    background: var(--ti-grp-fill);
  }
  .estate__t {
    font-weight: var(--w-medium);
    color: var(--app-text-strong);
  }
  .estate__go {
    margin-top: var(--s-4);
  }

  .foot {
    padding: var(--s-16) var(--s-2) 0;
    max-width: 76ch;
  }
</style>
