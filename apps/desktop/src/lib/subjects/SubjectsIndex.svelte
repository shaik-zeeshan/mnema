<script lang="ts">
  // Subjects — the index of mockup 09, skinned in direction 01 (Bento Native).
  //
  // The ranking is SPATIAL before it is textual: each conviction tier is its own
  // 4×1 tile, top to bottom, and the tier's note ("held firmly", "below display
  // floor") rides in the same meta slot every other tile in the app uses. Inside
  // the top tier the sparkline is the hero — a 260-wide trajectory with the 0.15
  // display floor drawn as a dashed line, next to a --t-display figure. Lower
  // tiers keep the identical row grammar at half the width: one object, quieter.
  //
  // Every figure resolves to a real backend field (G8): the confidence is
  // `Conclusion.confidence`, the trend is the averaged first→last change across
  // the subject's real ConfidenceSnapshot histories with a ±0.04 deadband, and
  // the sparkline's X axis is the POINT INDEX, never time — a "last 30 days"
  // label would be a fiction, since the cadence is the engine's own decay beat.
  //
  // Tier thresholds, trend derivation, the summary counts and the search ranking
  // all come from the shared `$lib/insights` helpers; nothing is re-derived here.
  import { goto } from "$app/navigation";
  import { openSettings } from "$lib/surface-windows";
  import Segmented from "$lib/components/Segmented.svelte";
  import Glyph from "$lib/overview/Glyph.svelte";
  import {
    type Axis,
    type Trend,
    DISPLAY_FLOOR,
    buildTiers,
    debounce,
    isSparse,
    summaryCounts,
  } from "$lib/insights/subjectsTiers";
  import { rankSubjects } from "$lib/insights/subjectSearch";
  import { SubjectsIndexData, type SubjectRow } from "./subjects-index.svelte";
  import { ago, conf2, sparkPoints, sparkY } from "./subjects-format";

  const data = new SubjectsIndexData();
  $effect(() => data.start());

  const AXIS_OPTIONS = [
    { value: "conviction", label: "By conviction" },
    { value: "movement", label: "By movement" },
  ];
  let axisValue = $state("conviction");
  const axis = $derived(axisValue as Axis);

  // The search field filters in memory; the debounce only spares re-ranking on
  // every keystroke.
  let searchQuery = $state("");
  let appliedQuery = $state("");
  const applySearch = debounce((q: string) => {
    appliedQuery = q;
  }, 200);
  $effect(() => () => applySearch.cancel());

  const searching = $derived(appliedQuery.trim().length > 0);
  const matches = $derived(
    searching ? rankSubjects(data.rows, appliedQuery) : [],
  );

  const tiers = $derived(
    buildTiers(data.rows, axis).filter((t) => t.items.length > 0),
  );
  // Under five subjects the tiers collapse to one flat list — tier headers over
  // one row each are noise, not ranking.
  const sparse = $derived(isSparse(data.rows.length));
  const summary = $derived(summaryCounts(data.rows));
  // "8 active views · 1 fading — 3 warming ▲ · 4 steady · 1 cooling ▼". Counted,
  // never rolled up into a single score: `active`/`fading` partition the set,
  // and the three movement tallies count WITHIN the active ones.
  const summaryLine = $derived.by(() => {
    const head = [
      `${summary.active} active ${summary.active === 1 ? "view" : "views"}`,
    ];
    if (summary.fading) head.push(`${summary.fading} fading`);
    return `${head.join(" · ")} — ${summary.warming} warming ▲ · ${summary.steady} steady · ${summary.cooling} cooling ▼`;
  });

  const AXIS_NOTE: Record<Axis, string> = {
    conviction:
      "Conviction — how firmly the engine holds each view (its confidence).",
    movement:
      "Movement — which way each view is trending: warming, steady, or cooling.",
  };

  function trendLabel(t: Trend): string {
    return t === "up" ? "▲ warming" : t === "down" ? "▼ cooling" : "– steady";
  }

  function open(row: SubjectRow): void {
    void goto(`/subjects?s=${encodeURIComponent(row.subject)}`);
  }

  // The sparkline box: hero (top tier / flat list) vs the half-width echo every
  // lower tier uses. Same grammar, same floor, half the width.
  const HERO = { w: 260, h: 52 };
  const MINI = { w: 150, h: 30 };
</script>

{#snippet spark(row: SubjectRow, hero: boolean)}
  {@const box = hero ? HERO : MINI}
  <svg
    class="spk"
    class:spk--faded={row.faded}
    viewBox="0 0 {box.w} {box.h}"
    preserveAspectRatio="none"
    role="img"
    aria-label={`${row.subject} — confidence trajectory, ${trendLabel(row.trend)}, across ${row.conclusionCount} ${row.conclusionCount === 1 ? "conclusion" : "conclusions"}`}
  >
    <line
      class="floor"
      x1="0"
      y1={sparkY(DISPLAY_FLOOR, box.h)}
      x2={box.w}
      y2={sparkY(DISPLAY_FLOOR, box.h)}
    />
    {#each row.tracks as track, i (i)}
      {@const pts = sparkPoints(track.points, box.w, box.h)}
      {#if pts}
        <polyline class={i === 0 ? "lead" : i === 1 ? "b" : "c"} points={pts} />
      {/if}
    {/each}
  </svg>
{/snippet}

{#snippet row(r: SubjectRow, hero: boolean)}
  <button
    type="button"
    class="row srow"
    class:srow--hero={hero}
    class:is-faded={r.faded}
    onclick={() => open(r)}
  >
    <span class="stxt">
      <span class="stop">
        <i class="sdot"></i>
        <span class="sname">{r.subject}</span>
        {#if r.pinned}<span class="spin" title="Pinned">★</span>{/if}
        <span class="strend strend--{r.trend}">{trendLabel(r.trend)}</span>
        <span class="scount">
          · {r.conclusionCount}
          {r.conclusionCount === 1 ? "conclusion" : "conclusions"}
        </span>
      </span>
      <span class="shead">{r.headline}</span>
    </span>
    {@render spark(r, hero)}
    <span class="sconf">{conf2(r.topConfidence)}</span>
    <span class="swhen">{ago(r.lastMovedAtMs)}</span>
    <span class="chev"><Glyph name="chevr" /></span>
  </button>
{/snippet}

<div class="sb">
  <div class="dbar">
    <label class="input search">
      <svg class="search__g" viewBox="0 0 14 14" fill="none" aria-hidden="true">
        <circle cx="6" cy="6" r="4.2" stroke="currentColor" stroke-width="1.4" />
        <path d="M9.2 9.2 12.6 12.6" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" />
      </svg>
      <input
        type="search"
        placeholder="Search subjects…"
        aria-label="Search subjects"
        bind:value={searchQuery}
        oninput={() => applySearch(searchQuery)}
      />
    </label>
    <Segmented options={AXIS_OPTIONS} bind:value={axisValue} compact ariaLabel="Group subjects" />
    <span class="t-meta is-mono is-num dbar__meta">
      {#if searching}
        {matches.length}
        {matches.length === 1 ? "match" : "matches"}
      {:else if data.rows.length}
        {summaryLine}
      {/if}
    </span>
  </div>

  <div class="sbody scroll">
    <p class="axis">{AXIS_NOTE[axis]}</p>

    {#if data.loadError && !data.rows.length}
      <div class="tile tile--w4 tile--static state">
        <div class="tile__h"><span class="t-label">Subjects</span></div>
        <p class="t-ui strong">Couldn't load your subjects.</p>
        <p class="t-meta">{data.loadError}</p>
        <button type="button" class="btn btn--sm" onclick={() => void data.load()}>
          Try again
        </button>
      </div>
    {:else if data.engineOn === false && !data.rows.length}
      <!-- Engine off: the pitch, and the one control that changes it. -->
      <div class="tile tile--w4 tile--static state">
        <div class="tile__h">
          <span class="t-label">Subjects</span>
          <span class="tile__more">engine off</span>
        </div>
        <p class="t-ui strong">The Reasoning Engine is off.</p>
        <p class="t-meta">
          Subjects appear as the engine forms views about you — each with its own
          confidence trajectory. Turn it on to begin.
        </p>
        <button
          type="button"
          class="btn btn--sm"
          onclick={() => void openSettings("intelligence")}
        >
          Open engine settings
        </button>
      </div>
    {:else if searching && matches.length === 0}
      <div class="tile tile--w4 tile--static state">
        <div class="tile__h">
          <span class="t-label">Subjects</span>
          <span class="tile__more">no match</span>
        </div>
        <p class="t-ui strong">No subjects match “{appliedQuery}”.</p>
      </div>
    {:else if !data.rows.length}
      <div class="tile tile--w4 tile--static state">
        <div class="tile__h">
          <span class="t-label">Subjects</span>
          <span class="tile__more">{data.loading ? "reading" : "nothing yet"}</span>
        </div>
        <p class="t-ui strong">{data.loading ? "Reading your dossier…" : "No subjects yet."}</p>
        {#if !data.loading}
          <p class="t-meta">
            Views form as the engine sees repeated work — each one keeps its own
            confidence trajectory.
          </p>
        {/if}
      </div>
    {:else if searching}
      <!-- A ranked search is one flat list: relevance is the only order that
           matters while a query is active, so tier headers would lie. -->
      <div class="bento">
        <div class="tile tile--w4 tile--static">
          <div class="tile__h">
            <span class="t-label">Matches</span>
            <span class="tile__more">by relevance</span>
          </div>
          <div class="pay pay--rows">
            {#each matches as r (r.subject)}{@render row(r, true)}{/each}
          </div>
        </div>
      </div>
    {:else if sparse}
      <div class="bento">
        <div class="tile tile--w4 tile--static">
          <div class="tile__h">
            <span class="t-label">Subjects</span>
            <span class="tile__more">too few to tier</span>
          </div>
          <div class="pay pay--rows">
            {#each data.rows as r (r.subject)}{@render row(r, true)}{/each}
          </div>
        </div>
      </div>
    {:else}
      <div class="bento">
        {#each tiers as tier, i (tier.id)}
          <div class="tile tile--w4 tile--static">
            <div class="tile__h">
              <span class="t-label">{tier.title}</span>
              <span class="tile__more">{tier.note}</span>
            </div>
            <div class="pay pay--rows">
              <!-- The top tier is the hero: full-width trajectory, display-sized
                   figure. Every tier below repeats it at half the width. -->
              {#each tier.items as r (r.subject)}{@render row(r, i === 0)}{/each}
              {#if tier.faded}
                <p class="tierfoot">
                  Confidence is recency-weighted — views warm with fresh evidence
                  and cool on their own. Faded views are kept for history, never
                  deleted.
                </p>
              {/if}
            </div>
          </div>
        {/each}
      </div>
    {/if}
  </div>
</div>

<style>
  .sb {
    flex: 1 1 auto; /* height:100% collapses under WKWebView — always flex */
    min-height: 0;
    display: flex;
    flex-direction: column;
  }

  /* The one bar this destination adds above the bento: search, the grouping
     axis, and the honest counts. Chrome height, chrome material. */
  .dbar {
    flex: 0 0 40px;
    display: flex;
    align-items: center;
    gap: var(--s-8);
    padding: 0 var(--pad-window);
    background: var(--app-surface);
    box-shadow: 0 var(--hairline) 0 var(--app-border);
    position: relative;
    z-index: 2;
  }
  .search {
    width: 210px;
    display: flex;
    align-items: center;
    gap: var(--s-6);
  }
  .search__g {
    flex: 0 0 auto;
    width: 12px;
    height: 12px;
    color: var(--app-text-subtle);
  }
  .search input {
    min-width: 0;
    flex: 1 1 auto;
    border: 0;
    outline: none;
    background: transparent;
    color: inherit;
    font: inherit;
  }
  .search input::placeholder {
    color: var(--app-text-subtle);
  }
  .search input::-webkit-search-cancel-button {
    -webkit-appearance: none;
  }
  .dbar__meta {
    margin-left: auto;
    white-space: nowrap;
  }

  .sbody {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    padding: var(--s-16) var(--pad-window) var(--s-48);
  }
  .axis {
    margin: 0 var(--tile-pad) var(--s-12);
    font: var(--w-regular) var(--t-meta) / 1 var(--app-font-sans);
    color: var(--app-text-subtle);
  }
  .bento {
    grid-auto-rows: min-content;
  }

  .state {
    gap: var(--s-6);
  }
  .state p {
    margin: 0;
    max-width: 70ch;
  }
  .state .btn {
    align-self: flex-start;
    margin-top: var(--s-4);
  }
  .strong {
    color: var(--app-text-strong);
  }

  /* ── the subject row ────────────────────────────────────────────────────
     A grid, not a flex row, so the trajectory, the figure and the stamp line
     up column-for-column down the whole page — that alignment is what lets the
     eye read the tiers as one ranking instead of four lists. */
  .srow {
    display: grid;
    grid-template-columns: 1fr 150px 56px 62px 12px;
    align-items: center;
    gap: 0 var(--s-16);
    width: 100%;
    min-height: 0;
    padding: var(--s-8) var(--tile-pad);
    border: 0;
    background: transparent;
    color: var(--app-accent);
    text-align: left;
    cursor: pointer;
  }
  .srow--hero {
    grid-template-columns: 1fr 260px 78px 66px 12px;
    padding: var(--s-12) var(--tile-pad);
  }
  .srow:focus-visible {
    outline: none;
    box-shadow: inset 0 0 0 2px var(--app-accent);
  }
  .stxt {
    min-width: 0;
  }
  .stop {
    display: flex;
    align-items: center;
    gap: var(--s-6);
    min-width: 0;
  }
  .sdot {
    flex: 0 0 auto;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: currentColor;
  }
  .sname {
    font: var(--w-semi) var(--t-ui) / 1.2 var(--app-font-sans);
    letter-spacing: var(--ls-ui);
    color: var(--app-text-strong);
    white-space: nowrap;
  }
  .srow--hero .sname {
    font-size: 15px;
  }
  .spin {
    color: var(--app-warn);
    font-size: 11px;
  }
  .strend {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    font: var(--w-medium) var(--t-label) / 1 var(--app-font-mono);
    letter-spacing: var(--ls-label);
    text-transform: uppercase;
    white-space: nowrap;
  }
  .strend--up {
    color: var(--app-accent);
  }
  .strend--down {
    color: var(--app-danger);
  }
  .strend--steady {
    color: var(--app-text-faint);
  }
  .scount {
    font: var(--w-regular) var(--t-label) / 1 var(--app-font-mono);
    font-variant-numeric: tabular-nums;
    color: var(--app-text-faint);
    white-space: nowrap;
  }
  .shead {
    display: block;
    margin-top: 4px;
    font: var(--w-regular) var(--t-meta) / 1.4 var(--app-font-sans);
    color: var(--app-text-muted);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  /* the sparkline — one polyline per conclusion, the 0.15 display floor dashed */
  .spk {
    display: block;
    width: 100%;
    height: 30px;
    overflow: visible;
  }
  .srow--hero .spk {
    height: 52px;
  }
  .spk polyline {
    fill: none;
    stroke: var(--app-accent);
    stroke-width: 1.6;
    stroke-linejoin: round;
    stroke-linecap: round;
    vector-effect: non-scaling-stroke;
  }
  .spk polyline.b {
    stroke: var(--app-text-faint);
    opacity: 0.8;
  }
  .spk polyline.c {
    stroke: var(--app-text-faint);
    opacity: 0.5;
  }
  .spk .floor {
    stroke: var(--app-border-hover);
    stroke-width: 1;
    stroke-dasharray: 3 3;
    vector-effect: non-scaling-stroke;
  }
  .spk--faded polyline {
    stroke: var(--app-text-faint);
  }

  .sconf {
    text-align: right;
    font: var(--w-semi) 15px / 1 var(--app-font-mono);
    font-variant-numeric: tabular-nums;
    color: var(--app-text-strong);
  }
  .srow--hero .sconf {
    font-size: var(--t-display);
    letter-spacing: var(--ls-display);
  }
  .swhen {
    text-align: right;
    font: var(--w-regular) var(--t-label) / 1 var(--app-font-mono);
    font-variant-numeric: tabular-nums;
    color: var(--app-text-faint);
  }
  .srow .chev {
    width: 8px;
    height: 12px;
  }

  /* A faded row is kept, not deleted — it reads quiet, never broken. */
  .srow.is-faded {
    color: var(--app-text-faint);
  }
  .srow.is-faded .sname,
  .srow.is-faded .sconf {
    color: var(--app-text-muted);
  }

  .tierfoot {
    max-width: 92ch;
    margin: 0;
    padding: var(--s-12) var(--tile-pad) var(--s-4);
    font: var(--w-regular) var(--t-meta) / var(--lh-meta) var(--app-font-sans);
    color: var(--app-text-subtle);
  }

  /* 800×600: the summary readout goes first — the tiers carry the ranking on
     their own, and a nowrap line here would push past the window. Then the
     trajectory yields; the figure and the stamp are the row's load-bearing
     numbers and never shrink. */
  @media (max-width: 900px) {
    .dbar__meta {
      display: none;
    }
    .srow,
    .srow--hero {
      grid-template-columns: 1fr 110px 56px 56px 12px;
      gap: 0 var(--s-12);
    }
    .srow--hero .spk {
      height: 34px;
    }
    .srow--hero .sconf {
      font-size: 15px;
      letter-spacing: 0;
    }
  }
</style>
