<script lang="ts">
  // Subjects — the browsable index (insights.html renderSubjects). A lede
  // templated over the four conviction tiers, the conviction map as the hero,
  // then the tiered subject list beside "Moving this fortnight" and a key to how
  // conviction works. A Subject holds MULTIPLE Conclusions, each with its own
  // confidence — never one rolled-up score.
  //
  // Subjects are grouped client-side from `list_user_context_conclusions`. Each
  // subject's real Confidence History comes lazily from `get_user_context_subject`
  // (best-effort) and drives the spark lines, the trend, and the map's 14-day
  // tail. Tier cuts live ENTIRELY in `subjectsTiers.ts`.
  import { untrack } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import type { Conclusion, ConfidenceSnapshot, UserContextStatus } from "$lib/types/recording";
  import Sparkline from "$lib/insights/charts/Sparkline.svelte";
  import { loadSubjectHistories } from "$lib/insights/subject-histories";
  import Segmented from "$lib/components/Segmented.svelte";
  import { inBar } from "$lib/insights/in-bar";
  import ConvictionMap, { type MapRow } from "$lib/insights/ConvictionMap.svelte";
  import SubjectsLede from "$lib/insights/SubjectsLede.svelte";
  import {
    type Axis,
    type ConvictionTier,
    type TierSubject,
    MAP_PAGE,
    TIER_PAGE,
    buildTiers,
    convictionTierId,
    debounce,
    decideRefresh,
    deriveTrend,
    fortnightAgo,
    isSparse,
    movePts,
    pageOf,
    rankMovers,
    subjectsDiff,
  } from "$lib/insights/subjectsTiers";
  import { rankSubjects } from "$lib/insights/subjectSearch";
  import IconSearch from "~icons/lucide/search";
  import IconInbox from "~icons/lucide/inbox";
  import IconAlert from "~icons/lucide/triangle-alert";
  import IconRetry from "~icons/lucide/rotate-ccw";
  import IconRight from "~icons/lucide/chevron-right";

  interface Props {
    onOpenSubject: (subject: string) => void;
    // "View dismissed": the shell switches to Context with Dismissed open.
    onViewDismissed?: () => void;
    // Typing in the bar's filter (it stays on the bar over a subject's page)
    // returns to the index, as in the mockup.
    onFilter?: () => void;
  }
  let { onOpenSubject, onViewDismissed, onFilter }: Props = $props();

  // Spark lines encode magnitude, not identity: the lead conclusion strong, the
  // rest one neutral grey (insights.html multiSpark).
  const SPARK_LEAD = "--app-text-strong";
  const SPARK_REST = "--chart-grey-2";

  interface SubjectRow extends TierSubject {
    subject: string;
    conclusions: Conclusion[]; // confidence desc
    headline: string; // the lead conclusion's statement
    sinceMs: number; // earliest formation
    then: number | null; // lead's confidence 14 days ago; null = formed this fortnight
    spark: { colorVar: string; faded: boolean; points: number[] }[];
  }

  let conclusions = $state<Conclusion[] | null>(null);
  let loadError = $state(false);
  let loading = $state(true);
  // Lets the empty state name its cause (backfilling, too little evidence,
  // everything dismissed). Engine trouble is the shell's notice, not ours.
  let ctx = $state<UserContextStatus | null>(null);
  const heldBack = $derived.by(() => {
    const d = ctx?.lastDistillation;
    return d ? d.guardrailSuppressed + d.belowFormationBar + d.resurfaceBlocked + d.ungrounded : 0;
  });

  let axis = $state<Axis>("conviction");
  const AXIS_OPTIONS = [
    { value: "conviction", label: "Conviction" },
    { value: "movement", label: "Movement" },
  ];

  // `searchQuery` is the input; `appliedQuery` the debounced value the ranking
  // runs on. A non-empty query swaps the tiers for one relevance-ranked list.
  let searchQuery = $state("");
  let appliedQuery = $state("");
  const applySearch = debounce((q: string) => {
    appliedQuery = q;
    setShown("search", null);
  }, 200);
  function clearSearch(): void {
    applySearch.cancel();
    searchQuery = appliedQuery = "";
  }

  // Per-tier paging: TIER_PAGE rows at a time so a crowded tier never floods
  // the surface (search results page the same way, as tier "search"). Reassign
  // the Map so the $state reacts.
  let tierShown = $state<Map<string, number>>(new Map());
  const shownCount = (id: string) => tierShown.get(id) ?? TIER_PAGE;
  function setShown(id: string, n: number | null): void {
    const next = new Map(tierShown);
    if (n === null) next.delete(id);
    else next.set(id, n);
    tierShown = next;
  }

  // subject → (conclusionId → oldest-first Confidence History). The generation
  // token drops a slow load that a newer one overtook.
  let trajectories = $state<Map<string, Map<number, ConfidenceSnapshot[]>>>(new Map());
  let trajectoriesGen = 0;

  // Engine `user_context_changed` events never reflow the page under the reader:
  // a debounced reload lands in `stagedConclusions` behind the refresh pill, and
  // swaps in silently only while the list is scrolled to the top.
  let stagedConclusions = $state<Conclusion[] | null>(null);
  let pendingCount = $state(0);
  let atTop = $state(true);
  let rootEl = $state<HTMLElement | null>(null);

  // Keyed case-insensitively, as the store is (`subject COLLATE NOCASE`: its
  // count, the subject page's read, the dedup index) — so "Mnema" and "mnema"
  // are one subject here and in the tab badge. Shown under the first spelling
  // seen (the list arrives confidence-desc, so the lead's).
  function groupSubjects(list: Conclusion[]): Map<string, Conclusion[]> {
    const groups = new Map<string, Conclusion[]>();
    for (const c of list) {
      const key = c.subject.toLowerCase();
      const bucket = groups.get(key);
      if (bucket) bucket.push(c);
      else groups.set(key, [c]);
    }
    return groups;
  }

  const rows = $derived.by<SubjectRow[]>(() => {
    if (!conclusions) return [];
    const now = Date.now();
    const out: SubjectRow[] = [];
    for (const cs of groupSubjects(conclusions).values()) {
      const subject = cs[0].subject;
      const history = trajectories.get(subject);
      const points = new Map([...(history ?? [])].map(([id, h]) => [id, h.map((s) => s.confidence)]));
      const sorted = [...cs].sort((a, b) => b.confidence - a.confidence);
      const top = sorted[0];
      out.push({
        subject,
        conclusions: sorted,
        faded: cs.every((c) => c.status === "faded"),
        headline: top.statement,
        sinceMs: Math.min(...cs.map((c) => c.formedAtMs)),
        lastMovedAtMs: Math.max(...cs.map((c) => Math.max(c.updatedAtMs, c.lastSupportedAtMs))),
        trend: deriveTrend(cs, points),
        topConfidence: top.confidence,
        then: fortnightAgo(top, history?.get(top.id) ?? [], now),
        // A polyline needs two points: flatten a missing/one-point history.
        spark: sorted.map((c, i) => {
          const p = points.get(c.id);
          return {
            colorVar: i === 0 ? SPARK_LEAD : SPARK_REST,
            faded: c.status === "faded",
            points: p && p.length >= 2 ? p : [p?.[0] ?? c.confidence, p?.[0] ?? c.confidence],
          };
        }),
      });
    }
    return out;
  });

  // One ordering for everything: non-faded by lead confidence, faded last.
  function byDisplay(a: { faded: boolean; topConfidence: number; subject: string }, b: typeof a): number {
    return Number(a.faded) - Number(b.faded) || b.topConfidence - a.topConfidence || a.subject.localeCompare(b.subject);
  }
  const displayRows = $derived([...rows].sort(byDisplay));
  const tiers = $derived(buildTiers(displayRows, axis).filter((t) => t.items.length > 0));
  const sparse = $derived(isSparse(displayRows.length));
  const searching = $derived(appliedQuery.trim().length > 0);
  const searchResults = $derived(searching ? rankSubjects(displayRows, appliedQuery) : []);

  const names = $derived.by(() => {
    const out: Record<ConvictionTier, string[]> = { strong: [], forming: [], shaping: [], fading: [] };
    for (const r of displayRows) out[convictionTierId(r)].push(r.subject);
    return out;
  });
  const conclusionTotal = $derived(conclusions?.length ?? 0);

  // A set filter scopes the whole page below the lede to its matches.
  const listed = $derived(searching ? searchResults : displayRows);

  // The map shows the top MAP_PAGE subjects until "Show all N".
  let mapAll = $state(false);
  const mapRows = $derived<MapRow[]>(
    listed.slice(0, mapAll ? undefined : MAP_PAGE).map((r) => ({
      subject: r.subject,
      headline: r.headline,
      count: r.conclusions.length,
      now: r.topConfidence,
      then: r.then,
      // One tick per distinct whole percent: a subject can hold 1,000+ conclusions.
      others: [...new Set(r.conclusions.slice(1).map((c) => Math.round(c.confidence * 100)))].map((p) => p / 100),
      faded: r.faded,
    })),
  );

  const moving = $derived(rankMovers(listed));

  const pct = (c: number) => Math.round(Math.max(0, Math.min(1, c)) * 100);
  const day = (ms: number) => new Date(ms).toLocaleDateString(undefined, { month: "short", day: "numeric" });

  // Subject display order for any list — the same grouping + sort the page
  // shows — so a staged reload is diffed in the order the reader sees.
  function displayedSubjectOrder(list: Conclusion[]): string[] {
    return [...groupSubjects(list).values()]
      .map((cs) => ({
        subject: cs[0].subject,
        faded: cs.every((c) => c.status === "faded"),
        topConfidence: Math.max(...cs.map((c) => c.confidence)),
      }))
      .sort(byDisplay)
      .map((s) => s.subject);
  }

  // The single network read: the fresh list, or null on failure so callers keep
  // what's shown. Only a failure before anything loaded shows the error state.
  async function fetchConclusions(): Promise<Conclusion[] | null> {
    try {
      const list = await invoke<Conclusion[]>("list_user_context_conclusions", { includeFaded: true });
      loadError = false;
      return list;
    } catch {
      if (untrack(() => conclusions) === null) loadError = true;
      return null;
    }
  }

  function applyConclusions(list: Conclusion[]): void {
    conclusions = list;
    void loadTrajectories(list);
  }

  async function loadConclusions(): Promise<void> {
    loading = true;
    try {
      const list = await fetchConclusions();
      if (list) applyConclusions(list);
    } finally {
      loading = false;
    }
  }

  async function loadStatus(): Promise<void> {
    const next = await invoke<UserContextStatus>("get_user_context_status").catch(() => null);
    if (next) ctx = next;
  }

  function applyStaged(): void {
    if (!stagedConclusions) return;
    applyConclusions(stagedConclusions);
    stagedConclusions = null;
    pendingCount = 0;
  }

  // A failed fetch leaves that subject on its flat baseline. One read per
  // grouped subject, under the name its row is keyed by.
  async function loadTrajectories(list: Conclusion[]): Promise<void> {
    const gen = ++trajectoriesGen;
    const next = await loadSubjectHistories([...groupSubjects(list).values()].map((cs) => cs[0].subject));
    if (gen === trajectoriesGen) trajectories = next;
  }

  // Debounced engine-change handler: diff the fresh list against what's shown
  // (same order) and apply silently, stage behind the pill, or just refresh the
  // figures in place when membership and order held.
  async function onContextChanged(): Promise<void> {
    const next = await fetchConclusions();
    if (!next) return;
    const diff = subjectsDiff(displayedSubjectOrder(untrack(() => conclusions) ?? []), displayedSubjectOrder(next));
    const action = decideRefresh({ changed: diff.changed, expanded: false, atTop: untrack(() => atTop) });
    if (action === "stage") {
      stagedConclusions = next;
      pendingCount = diff.count;
      return;
    }
    applyConclusions(next);
    if (action === "apply") {
      stagedConclusions = null;
      pendingCount = 0;
    }
  }

  // Back at the top: a staged reload swaps in without the pill click.
  $effect(() => {
    if (atTop && stagedConclusions !== null) applyStaged();
  });

  $effect(() => {
    void untrack(() => loadConclusions());
    void untrack(() => loadStatus());
    const debounced = debounce(() => void onContextChanged(), 500);
    let unlisten: UnlistenFn | undefined;
    let disposed = false;
    void listen("user_context_changed", () => {
      debounced();
      void loadStatus();
    }).then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    });

    // atTop: track the Insights scroll container (`.in-main`).
    const scrollEl = rootEl?.closest<HTMLElement>(".in-main") ?? null;
    const updateAtTop = () => (atTop = (scrollEl?.scrollTop ?? window.scrollY) <= 8);
    (scrollEl ?? window).addEventListener("scroll", updateAtTop, { passive: true });
    updateAtTop();

    return () => {
      disposed = true;
      unlisten?.();
      debounced.cancel();
      applySearch.cancel();
      (scrollEl ?? window).removeEventListener("scroll", updateAtTop);
    };
  });
</script>

{#snippet subjRow(r: SubjectRow)}
  <button type="button" class="in-subj" onclick={() => onOpenSubject(r.subject)}>
    <span class="in-subj__name">{r.subject}<small>{r.conclusions.length} concl. · since {day(r.sinceMs)}</small></span>
    <span class="in-subj__lead">{r.headline}</span>
    <span class="in-subj__spark">
      <Sparkline series={r.spark} label={`${r.subject} — confidence over time across ${r.conclusions.length} conclusions`} />
    </span>
    <span class="num">{pct(r.topConfidence)}%</span>
    <IconRight class="arrow" width="14" height="14" aria-hidden="true" />
  </button>
{/snippet}

{#snippet pagedTier(id: string, title: string, note: string, faded: boolean, items: SubjectRow[])}
  {@const page = pageOf(items, shownCount(id), TIER_PAGE)}
  <div class="in-tier" class:in-tier--faded={faded}>
    <p class="in-group">{title} {#if note}<span class="in-group__note">{note}</span>{/if}</p>
    {#each page.visible as r (r.subject)}{@render subjRow(r)}{/each}
    {#if page.more > 0}
      <button type="button" class="mx-btn mx-btn--ghost mx-btn--sm" onclick={() => setShown(id, shownCount(id) + TIER_PAGE)}>
        Show {page.more} more
      </button>
    {:else if page.less}
      <button type="button" class="mx-btn mx-btn--ghost mx-btn--sm" onclick={() => setShown(id, null)}>Show less</button>
    {/if}
  </div>
{/snippet}

<section class="subjects" aria-label="Subjects" bind:this={rootEl}>
  {#if conclusions && displayRows.length > 0}
    <div class="s-ctl" use:inBar>
      <label class="mx-search">
        <IconSearch width="14" height="14" aria-hidden="true" />
        <input
          placeholder="Filter subjects"
          aria-label="Filter subjects"
          autocomplete="off"
          spellcheck="false"
          bind:value={searchQuery}
          oninput={() => { applySearch(searchQuery); onFilter?.(); }}
        />
      </label>
      <Segmented md options={AXIS_OPTIONS} value={axis} onValueChange={(v) => (axis = v as Axis)} ariaLabel="Group by" />
    </div>
  {/if}

  {#if pendingCount > 0 || stagedConclusions !== null}
    <div class="in-refresh">
      <button type="button" class="mx-btn mx-btn--sm" onclick={applyStaged}>
        <IconRetry width="13" height="13" aria-hidden="true" />
        {pendingCount > 0 ? `${pendingCount} ${pendingCount === 1 ? "view" : "views"} updated` : "Views updated"} · Refresh
      </button>
    </div>
  {/if}

  {#if loadError && !conclusions}
    <div class="mx-empty mx-empty--center" data-tone="danger" role="alert">
      <span class="mx-empty__glyph"><IconAlert width="18" height="18" aria-hidden="true" /></span>
      <h4 class="mx-empty__title">Couldn't load Subjects.</h4>
      <p class="mx-empty__text">Reading your conclusions failed. Nothing is lost.</p>
      <div class="mx-empty__acts">
        <button type="button" class="mx-btn mx-btn--sm" disabled={loading} onclick={() => void loadConclusions()}>
          <IconRetry width="13" height="13" aria-hidden="true" />Try again
        </button>
      </div>
    </div>
  {:else if loading && !conclusions}
    <div aria-busy="true">
      <span class="mx-sr">Loading subjects</span>
      <div class="s-skel-read">
        <div class="mx-skel-lines">
          <div class="mx-skel" style="width:140px;height:9px"></div>
          <div class="mx-skel mx-skel--title"></div>
          <div class="mx-skel mx-skel--title"></div>
          <div class="mx-skel"></div>
          <div class="mx-skel"></div>
        </div>
        <div class="s-skel-stats">
          {#each Array.from({ length: 4 }) as _, i (i)}<div><div class="mx-skel"></div><div class="mx-skel"></div></div>{/each}
        </div>
      </div>
      <div class="mx-panel s-skel-map">
        {#each Array.from({ length: 6 }) as _, i (i)}
          <div class="mx-skel-row"><div class="mx-skel"></div><div class="mx-skel-lines"><div class="mx-skel"></div><div class="mx-skel"></div></div><div class="mx-skel"></div></div>
        {/each}
      </div>
    </div>
  {:else if displayRows.length === 0}
    <div class="mx-empty mx-empty--center">
      <span class="mx-empty__glyph"><IconInbox width="18" height="18" aria-hidden="true" /></span>
      <h4 class="mx-empty__title">{(ctx?.dismissedCount ?? 0) > 0 && !ctx?.backfilling ? "No subjects to show right now." : "No subjects yet."}</h4>
      <p class="mx-empty__text">
        {#if ctx?.backfilling}
          As Mnema forms views about you, each one appears here with its own confidence over time. It's
          backfilling your history now — a view needs at least two supporting activities.
        {:else if (ctx?.dismissedCount ?? 0) > 0}
          You've dismissed {ctx?.dismissedCount}. Restoring one in Context only lifts the block — it forms
          again only if recent activity supports it.
        {:else}
          As Mnema forms views about you, each one appears here with its own confidence over time. A view
          needs at least two supporting activities.{#if (ctx?.activityCount ?? 0) > 0}
            {" "}{ctx?.activityCount} {ctx?.activityCount === 1 ? "activity is" : "activities are"} summarized so far{heldBack > 0
              ? `; the last pass held back ${heldBack} ${heldBack === 1 ? "draft" : "drafts"}`
              : ""}.{/if}
        {/if}
      </p>
      {#if (ctx?.dismissedCount ?? 0) > 0 && !ctx?.backfilling && onViewDismissed}
        <div class="mx-empty__acts">
          <button type="button" class="mx-btn mx-btn--sm" onclick={onViewDismissed}>View dismissed</button>
        </div>
      {/if}
    </div>
  {:else}
    <SubjectsLede subjectCount={displayRows.length} conclusionCount={conclusionTotal} {names} />
    <ConvictionMap rows={mapRows} query={appliedQuery.trim()} onOpen={onOpenSubject} onClear={clearSearch}>
      {#if listed.length > MAP_PAGE}
        <button type="button" class="mx-btn mx-btn--ghost mx-btn--sm" onclick={() => (mapAll = !mapAll)}>
          {mapAll ? `Show top ${MAP_PAGE}` : `Show all ${listed.length}`}
        </button>
      {/if}
    </ConvictionMap>

    <div class="in-feed">
      <div class="mx-reveal" style="--i:4">
        {#if searching}
          <!-- No matches: the map above carries the empty state + Clear. -->
          {#if searchResults.length}
            {@render pagedTier("search", `${searchResults.length} match${searchResults.length === 1 ? "" : "es"}`, "", false, searchResults)}
          {/if}
        {:else if sparse}
          <div class="in-tier">{#each displayRows as r (r.subject)}{@render subjRow(r)}{/each}</div>
          <p class="mx-body-sm">Tiers appear once there are five or more subjects.</p>
        {:else}
          {#each tiers as tier (tier.id)}
            {@render pagedTier(tier.id, `${tier.title} · ${tier.items.length}`, tier.note, tier.faded, tier.items)}
          {/each}
        {/if}
      </div>
      <div class="in-side mx-reveal" style="--i:5">
        {#if moving.length}
          <div>
            <div class="in-sec"><span class="mx-kicker">Moving this fortnight</span></div>
            <dl class="mx-spec in-move">
              {#each moving as r (r.subject)}
                {@const d = movePts(r)}
                <!-- svelte-ignore a11y_click_events_have_key_events -->
                <!-- svelte-ignore a11y_no_static_element_interactions -->
                <div class="in-move__row" onclick={() => onOpenSubject(r.subject)}>
                  <dt>{r.subject}</dt>
                  <dd class={d === null ? "in-new" : d > 0 ? "in-up" : "in-down"}>
                    {d === null ? `formed ${day(r.sinceMs)}` : `${d > 0 ? "+" : "−"}${Math.abs(d)} pts`}
                  </dd>
                </div>
              {/each}
            </dl>
          </div>
        {/if}
        <div class="mx-panel mx-panel--inset in-guard">
          <div class="in-sec"><span class="mx-kicker">How conviction works</span></div>
          <dl class="mx-spec in-key">
            <div><dt>strongly held</dt><dd>Lead conclusion at 68% or more.</dd></div>
            <div><dt>forming</dt><dd>30–68%. Building support.</dd></div>
            <div><dt>taking shape</dt><dd>Under 30% — early, or cooling.</dd></div>
            <div><dt>fading</dt><dd>Every conclusion below the 15% display floor: leaves the dossier, history kept. Never deleted.</dd></div>
            <div><dt>decay</dt><dd>Halves every 30 days without fresh evidence. Pin stops it.</dd></div>
          </dl>
        </div>
      </div>
    </div>
  {/if}
</section>

<style>
  /* insights.html Subjects: bar controls, feed, tier rows, side column. */
  .s-ctl {
    display: flex;
    align-items: center;
    gap: var(--s-3);
  }
  .s-ctl .mx-search {
    width: 220px;
  }
  .in-refresh {
    position: sticky;
    top: 64px;
    z-index: 5;
    display: flex;
    justify-content: center;
    margin: -8px 0 var(--s-4);
  }
  .in-feed {
    display: grid;
    grid-template-columns: minmax(0, 1.45fr) minmax(0, 1fr);
    gap: var(--s-6);
    align-items: start;
  }
  .in-side {
    display: grid;
    gap: var(--s-6);
    align-content: start;
  }
  .in-sec {
    display: flex;
    align-items: baseline;
    gap: var(--s-3);
    margin: 0 0 var(--s-3);
  }
  .in-tier {
    margin-bottom: var(--s-5);
  }
  .in-group {
    margin: var(--s-4) 0 var(--s-1);
    font: 500 var(--text-xs) / 1 var(--font-mono);
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--app-text-subtle);
    display: flex;
    align-items: center;
    gap: var(--s-2);
  }
  .in-group::after {
    content: "";
    flex: 1;
    height: 1px;
    background: var(--mx-hairline);
  }
  .in-group__note {
    text-transform: none;
    letter-spacing: 0;
    color: var(--app-text-faint);
  }
  .in-subj {
    display: grid;
    grid-template-columns: 170px minmax(0, 1fr) 100px 44px 14px;
    gap: var(--s-4);
    align-items: center;
    width: calc(100% + var(--s-3) * 2);
    padding: 12px var(--s-3);
    margin: 0 calc(var(--s-3) * -1);
    border: 0;
    border-bottom: 1px solid var(--mx-hairline);
    border-radius: var(--r-sm);
    background: none;
    font: inherit;
    text-align: left;
    cursor: pointer;
    transition: background-color var(--t-fast) var(--ease-quart);
  }
  .in-subj:last-of-type {
    border-bottom: 0;
  }
  .in-subj:hover {
    background: var(--app-surface-hover);
  }
  .in-subj:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
  }
  .in-subj__name {
    font: 600 var(--text-md) / 1.2 var(--font-sans);
    color: var(--app-text-strong);
  }
  .in-subj__name small {
    display: block;
    margin-top: 5px;
    white-space: nowrap;
    font: 400 var(--text-xs) / 1 var(--font-mono);
    color: var(--app-text-subtle);
  }
  .in-subj__lead {
    color: var(--app-text-muted);
    font-size: var(--text-base);
    line-height: 1.45;
  }
  .in-subj__spark {
    display: block;
    height: 30px;
  }
  .in-subj__spark :global(svg) {
    width: 100%;
    height: 100%;
  }
  .in-subj .num {
    text-align: right;
    color: var(--app-text-strong);
  }
  .in-subj :global(.arrow) {
    color: var(--app-text-faint);
    transition: transform var(--t-med) var(--ease-expo);
  }
  .in-subj:hover :global(.arrow) {
    transform: translateX(3px);
    color: var(--app-text-strong);
  }
  .in-tier--faded .in-subj {
    opacity: 0.62;
  }
  .in-move {
    grid-template-columns: 1fr auto;
  }
  .in-move__row {
    cursor: pointer;
  }
  .in-move dt {
    font-family: var(--font-sans);
    font-size: var(--text-base);
    color: var(--app-text);
  }
  .in-move dd {
    text-align: right;
    font: 500 var(--text-sm) / 1.4 var(--font-mono);
  }
  .in-up {
    color: var(--focus-deep);
  }
  .in-down {
    color: var(--focus-distracted);
  }
  .in-new {
    color: var(--app-accent);
  }
  .in-guard {
    padding: var(--s-4);
  }
  .in-key {
    grid-template-columns: 118px 1fr;
  }
  .in-key dd {
    font-size: var(--text-base);
  }
  .s-skel-read {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    gap: var(--s-7);
    align-items: end;
    padding: var(--s-4) 0 var(--s-6);
    margin-bottom: var(--s-6);
    border-bottom: 1px solid var(--mx-hairline);
  }
  .s-skel-read .mx-skel--title {
    height: 34px;
    width: 78%;
  }
  .s-skel-read .mx-skel--title + .mx-skel--title {
    width: 52%;
  }
  .s-skel-stats {
    display: grid;
    grid-template-columns: repeat(2, 132px);
    gap: var(--s-5) var(--s-6);
  }
  .s-skel-stats div {
    display: grid;
    gap: 8px;
  }
  .s-skel-stats .mx-skel:first-child {
    height: 8px;
    width: 60%;
  }
  .s-skel-stats .mx-skel:last-child {
    height: 24px;
    width: 90%;
  }
  .s-skel-map {
    padding: var(--s-5);
    margin-bottom: var(--s-7);
  }
  @media (max-width: 1180px) {
    .in-feed {
      grid-template-columns: 1fr;
    }
  }
</style>
