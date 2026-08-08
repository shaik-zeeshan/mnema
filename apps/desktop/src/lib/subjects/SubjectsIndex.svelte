<script lang="ts">
  // Subjects — the tiered index (page 09, viewport A).
  //
  // Bands by conviction tier with sticky headers, the same banded idiom the
  // journal and settings use. The row's hero is the 168px time-spaced
  // sparkline; the number beside it is a WHOLE PERCENT, everywhere.
  //
  // Tier thresholds, grouping, summary counts and search ranking all come from
  // the shipping pure modules — this surface re-skins them, it never re-derives
  // them.
  import { untrack } from "svelte";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { toast } from "$lib/toast.svelte";
  import { humanizeError } from "$lib/format-error";
  import { openSettings } from "$lib/surface-windows";
  import { resetDeck, setDeck } from "$lib/deck.svelte";
  import Segmented from "$lib/components/Segmented.svelte";
  import FrameDetailModal from "$lib/components/FrameDetailModal.svelte";
  import {
    DISPLAY_FLOOR,
    INITIAL_BASE,
    STRONGLY_HELD,
    buildTiers,
    debounce,
    isSparse,
    summaryCounts,
    type Axis,
  } from "$lib/insights/subjectsTiers";
  import { rankSubjects } from "$lib/insights/subjectSearch";
  import type { Activity, Conclusion } from "$lib/types/recording";
  import ExpandedRow from "./ExpandedRow.svelte";
  import Spark from "./Spark.svelte";
  import {
    buildRows,
    chipFor,
    dismissConclusion,
    evidenceIds,
    fetchConclusions,
    fetchEngineOn,
    fetchHistory,
    loadFramePreviews,
    openRefInTimeline,
    pct,
    relativeTime,
    resolveActivities,
    setPinned,
    trendClass,
    trendLabel,
    type SubjectRow,
  } from "./data";

  interface Props {
    onOpen: (subject: string) => void;
    /** esc with no search to clear — leave the destination. */
    onExit: () => void;
  }

  let { onOpen, onExit }: Props = $props();

  let conclusions = $state<Conclusion[] | null>(null);
  let rows = $state<SubjectRow[]>([]);
  let engineOn = $state<boolean | null>(null);
  let loadError = $state<string | null>(null);
  let loading = $state(true);

  let axis = $state<Axis>("conviction");
  let query = $state("");
  let applied = $state("");
  let searchEl = $state<HTMLInputElement | null>(null);
  const applyQuery = debounce((q: string) => (applied = q), 160);

  let selected = $state(0);
  let expanded = $state<string | null>(null);
  let acting = $state<number | null>(null);

  // Evidence resolved for the expanded row: activity join + frame previews.
  let activities = $state<Map<number, Activity>>(new Map());
  let previews = $state<Map<number, string>>(new Map());

  let frameOpen = $state(false);
  let frameId = $state<number | null>(null);
  let frameFallback = $state<(() => void) | null>(null);

  const searching = $derived(applied.trim().length > 0);
  const summary = $derived(summaryCounts(rows));
  const results = $derived(searching ? rankSubjects(rows, applied) : rows);
  const sparse = $derived(isSparse(rows.length));
  const bands = $derived(
    searching || sparse ? [] : buildTiers(rows, axis).filter((t) => t.items.length > 0),
  );
  /** Display order, flattened — what ↑↓ walks. */
  const flat = $derived(
    searching || sparse ? results : bands.flatMap((b) => b.items),
  );
  const current = $derived(flat[Math.min(selected, flat.length - 1)] ?? null);

  // Only the four conviction bands can name a real threshold; the movement
  // bands keep their own note (there is no engine constant for "warming").
  const BAND_NOTE: Record<string, string> = {
    strong: `held firmly · at or above ${pct(STRONGLY_HELD)}%`,
    forming: `building support · ${pct(INITIAL_BASE)}% and up`,
    shaping: `early · above the display floor, under ${pct(INITIAL_BASE)}%`,
    fading: `below the ${pct(DISPLAY_FLOOR)}% display floor · never deleted`,
  };

  // ---- loading -------------------------------------------------------------

  async function load(): Promise<void> {
    try {
      const list = await fetchConclusions();
      loadError = null;
      conclusions = list;
      rows = buildRows(list, new Map());
      // Real trajectories arrive second: the list read carries no history, and
      // the sparkline is only honest once it has one.
      const history = await fetchHistory([...new Set(list.map((c) => c.subject))]);
      rows = buildRows(list, history);
    } catch (error) {
      if (!conclusions?.length) loadError = humanizeError(error);
    } finally {
      loading = false;
    }
  }

  async function loadEvidence(subject: string): Promise<void> {
    const row = rows.find((r) => r.subject === subject);
    if (!row) return;
    const wanted = new Set(evidenceIds(row.conclusions));
    const resolved = await resolveActivities(wanted);
    activities = new Map([...activities, ...resolved]);
    const frameIds = [...resolved.values()]
      .map((a) => chipFor(a).frameId)
      .filter((id): id is number => id !== null && !previews.has(id));
    if (frameIds.length === 0) return;
    previews = new Map([...previews, ...(await loadFramePreviews(frameIds))]);
  }

  $effect(() => {
    untrack(() => {
      void load();
      void fetchEngineOn().then((on) => (engineOn = on));
    });
    // The engine reforms views while you read; coalesce the burst.
    const reload = debounce(() => void load(), 500);
    let unlisten: UnlistenFn | undefined;
    let disposed = false;
    void listen("user_context_changed", () => reload()).then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    });
    return () => {
      disposed = true;
      unlisten?.();
      reload.cancel();
      applyQuery.cancel();
    };
  });

  // The deck reads out what the surface actually holds.
  $effect(() => {
    setDeck({
      context: `Subjects · ${summary.active} active · ${summary.fading} fading · by ${axis}`,
      hints: [
        { keys: "↑↓", label: "Move" },
        { keys: "→", label: "Expand" },
        { keys: "⏎", label: "Open subject" },
        { keys: "⌘P", label: "Pin" },
        { keys: "esc", label: "Back to Overview", separator: true },
      ],
    });
    return resetDeck;
  });

  // ---- actions -------------------------------------------------------------

  function toggleExpand(subject: string | null, open?: boolean): void {
    if (!subject) return;
    const next = open ?? expanded !== subject;
    expanded = next ? subject : null;
    if (next) void loadEvidence(subject);
  }

  async function pin(c: Conclusion): Promise<void> {
    if (acting !== null) return;
    acting = c.id;
    try {
      await setPinned(c.id, !c.pinned);
      await load();
    } catch (error) {
      toast({
        tone: "error",
        title: c.pinned ? "Couldn't unpin conclusion" : "Couldn't pin conclusion",
        message: humanizeError(error),
      });
    } finally {
      acting = null;
    }
  }

  async function dismiss(c: Conclusion): Promise<void> {
    if (acting !== null) return;
    acting = c.id;
    try {
      await dismissConclusion(c.id);
      await load();
    } catch (error) {
      toast({
        tone: "error",
        title: "Couldn't dismiss conclusion",
        message: humanizeError(error),
      });
    } finally {
      acting = null;
    }
  }

  /** "View frame ›" — peek the first cited frame in place; audio (or nothing
   *  resolvable) keeps the raw-Timeline hand-off. */
  function viewFrame(row: SubjectRow): void {
    const first = evidenceIds(row.conclusions)
      .map((id) => activities.get(id))
      .find((a): a is Activity => a !== undefined);
    const ref = first?.evidence?.[0];
    if (ref?.subjectType === "frame") {
      frameId = ref.subjectId;
      frameFallback = () => void openRefInTimeline(ref);
      frameOpen = true;
      return;
    }
    void openRefInTimeline(ref);
  }

  // ---- keyboard ------------------------------------------------------------

  function onKeydown(event: KeyboardEvent): void {
    const inSearch = event.target === searchEl;

    if (event.key === "Escape") {
      event.preventDefault();
      if (query) {
        query = "";
        applied = "";
        applyQuery.cancel();
        searchEl?.blur();
        return;
      }
      onExit();
      return;
    }
    if (event.metaKey && event.key.toLowerCase() === "f") {
      event.preventDefault();
      searchEl?.focus();
      searchEl?.select();
      return;
    }
    if (event.altKey && (event.key === "1" || event.key === "2")) {
      event.preventDefault();
      axis = event.key === "1" ? "conviction" : "movement";
      return;
    }
    if (event.metaKey && event.key.toLowerCase() === "p") {
      event.preventDefault();
      const top = current?.conclusions[0];
      if (top) void pin(top);
      return;
    }
    if (event.metaKey && (event.key === "Backspace" || event.key === "Delete")) {
      event.preventDefault();
      const top = current?.conclusions[0];
      if (top) void dismiss(top);
      return;
    }
    if (inSearch) return; // typing beats navigating

    if (event.key === "ArrowDown") {
      event.preventDefault();
      selected = Math.min(selected + 1, flat.length - 1);
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      selected = Math.max(selected - 1, 0);
    } else if (event.key === "ArrowRight") {
      event.preventDefault();
      toggleExpand(current?.subject ?? null, true);
    } else if (event.key === "ArrowLeft") {
      event.preventDefault();
      if (expanded === current?.subject) expanded = null;
    } else if (event.key === "Enter" && current) {
      event.preventDefault();
      onOpen(current.subject);
    }
  }

  // Re-ordering or filtering makes the old index meaningless — start at the top.
  $effect(() => {
    axis;
    applied;
    selected = 0;
  });

  // Keep the selected row in view as ↑↓ walks past the fold.
  $effect(() => {
    const subject = current?.subject;
    if (!subject) return;
    document
      .querySelector(`[data-subject="${CSS.escape(subject)}"]`)
      ?.scrollIntoView({ block: "nearest" });
  });
</script>

<svelte:window onkeydown={onKeydown} />

{#snippet chev()}
  <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.6"
    stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <path d="M6 3.5 10.5 8 6 12.5" />
  </svg>
{/snippet}

{#snippet subjectRow(r: SubjectRow, index: number)}
  {@const open = expanded === r.subject}
  <div
    class="srowv"
    class:is-key={index === selected}
    class:srowv--fade={r.faded}
    data-subject={r.subject}
  >
    <button type="button" class="srowv__hit" onclick={() => onOpen(r.subject)}>
      <span class="srowv__t">
        <span class="srowv__l1">
          <span class="srowv__nm">{r.subject}</span>
          {#if r.pinned}<span class="pinstar" aria-label="pinned">★</span>{/if}
          <span class="trend {trendClass(r.trend)}">{trendLabel(r.trend)}</span>
          <span class="srowv__cc t-meta">
            · {r.conclusionCount}
            {r.conclusionCount === 1 ? "conclusion" : "conclusions"}
          </span>
        </span>
        <span class="srowv__hl">{r.headline}</span>
      </span>
      <Spark
        series={r.series}
        label={`${r.subject} — ${trendLabel(r.trend)}, top confidence ${pct(r.topConfidence)}%`}
      />
      <span class="conf is-num">{pct(r.topConfidence)}%</span>
      <span class="rel is-num">{relativeTime(r.lastMovedAtMs)}</span>
      {#if index === selected}<span class="kbd">⏎</span>{/if}
    </button>
    <button
      type="button"
      class="srowv__caret"
      class:is-open={open}
      aria-expanded={open}
      aria-label={open ? `Collapse ${r.subject}` : `Expand ${r.subject}`}
      onclick={() => toggleExpand(r.subject)}
    >
      {@render chev()}
    </button>
  </div>

  {#if open}
    <ExpandedRow
      row={r}
      {activities}
      {previews}
      {acting}
      onPin={(c) => void pin(c)}
      onDismiss={(c) => void dismiss(c)}
      onViewFrame={() => viewFrame(r)}
    />
  {/if}
{/snippet}

<div class="subj">
  <div class="subj__hd">
    <p class="t-title subj__ttl">Subjects</p>
    {#if rows.length > 0}
      <p class="t-meta is-mono is-num subj__sum">
        {#if searching}
          {results.length}
          {results.length === 1 ? "match" : "matches"} for “{applied.trim()}”
        {:else}
          {summary.active} active · {summary.fading} fading — {summary.warming} warming ▲
          · {summary.steady} steady · {summary.cooling} cooling ▼
        {/if}
      </p>
    {/if}
    <div class="subj__seg">
      <Segmented
        options={[
          { value: "conviction", label: "Conviction" },
          { value: "movement", label: "Movement" },
        ]}
        keys={["⌥1", "⌥2"]}
        value={axis}
        onValueChange={(v) => (axis = v as Axis)}
        ariaLabel="Order subjects by"
      />
    </div>
    <span class="subj__search">
      <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5"
        stroke-linecap="round" aria-hidden="true">
        <circle cx="7" cy="7" r="4.5" /><path d="M10.5 10.5 14 14" />
      </svg>
      <input
        bind:this={searchEl}
        class="subj__input"
        type="text"
        placeholder="Search subjects…"
        aria-label="Search subjects"
        autocomplete="off"
        spellcheck="false"
        bind:value={query}
        oninput={() => applyQuery(query)}
      />
      <span class="kbd">⌘F</span>
    </span>
  </div>

  <p class="subj__sub t-meta">
    {#if axis === "conviction"}
      Conviction — how firmly the engine holds each view (its confidence). Strongest
      first; fading ones are kept for history.
    {:else}
      Movement — which way each view is heading: warming with fresh evidence, steady,
      or cooling on its own.
    {/if}
  </p>

  <!-- `--empty` centers the one card in the region. Left at the top it was a
       full-width panel pinned under the header with ~440px of nothing beneath
       it — a floating panel with an unexplained gap, which is the exact defect
       the founder called out. -->
  <div class="subj__list" class:subj__list--empty={!loading && (rows.length === 0 || (loadError && !conclusions))}>
    {#if loadError && !conclusions}
      <div class="emptybox">
        <span class="gl">◇</span>
        <span class="t-ui strong">Couldn't load Subjects.</span>
        <span class="t-meta">{loadError}</span>
        <button type="button" class="btn btn--sm" onclick={() => void load()}>Try again</button>
      </div>
    {:else if loading}
      <p class="subj__loading t-meta">Loading subjects…</p>
    {:else if rows.length === 0}
      <div class="emptybox">
        <span class="gl">◇</span>
        {#if engineOn === false}
          <span class="t-ui strong">The Reasoning Engine is off.</span>
          <span class="t-meta">
            Subjects appear as the engine forms views about you — each with its own
            confidence trajectory. Turn it on to begin.
          </span>
          <button
            type="button"
            class="btn btn--sm"
            onclick={() => void openSettings("intelligence")}
          >
            Open engine settings <span class="kbd">⌃3</span>
          </button>
        {:else}
          <span class="t-ui strong">No subjects yet.</span>
          <span class="t-meta">
            As the Reasoning Engine forms views about you, each one appears here with
            its own confidence trajectory. Keep working and check back — they build up
            as evidence accumulates.
          </span>
        {/if}
      </div>
    {:else if searching && results.length === 0}
      <div class="emptybox">
        <span class="gl">◇</span>
        <span class="t-ui strong">No subjects match “{applied.trim()}”.</span>
        <span class="t-meta">
          Search looks at subject names and the wording of each belief. Try a shorter or
          different term, or clear the search to browse all subjects.
        </span>
        <span class="hint"><span class="kbd">esc</span><span>clear</span></span>
      </div>
    {:else if searching || sparse}
      {#each results as r, i (r.subject)}
        {@render subjectRow(r, i)}
      {/each}
    {:else}
      {#each bands as band (band.id)}
        {@const offset = flat.indexOf(band.items[0])}
        <div class="tier-h">
          <span class="t-label">{band.title}</span>
          <span class="tier-h__note t-meta">{BAND_NOTE[band.id] ?? band.note}</span>
        </div>
        {#each band.items as r, i (r.subject)}
          {@render subjectRow(r, offset + i)}
        {/each}
      {/each}
    {/if}
  </div>
</div>

<FrameDetailModal
  open={frameOpen}
  {frameId}
  onClose={() => (frameOpen = false)}
  onOpenInTimeline={frameFallback ?? undefined}
/>

<style>
  .subj {
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .subj__hd {
    flex: 0 0 auto;
    display: flex;
    align-items: center;
    gap: var(--s-12);
    padding: var(--s-12) var(--s-16) var(--s-8);
  }
  .subj__ttl,
  .subj__sum {
    margin: 0;
  }
  .subj__seg {
    margin-left: auto;
  }
  .subj__search {
    display: flex;
    align-items: center;
    gap: var(--gap-inline);
    width: 210px;
    height: var(--h-md);
    padding: 0 var(--pad-control);
    border: var(--hairline) solid var(--app-border-strong);
    border-radius: var(--r-md);
    background: var(--app-surface-subtle);
  }
  .subj__search:focus-within {
    border-color: var(--app-accent-border);
    box-shadow: var(--ring);
  }
  .subj__search svg {
    width: 13px;
    height: 13px;
    color: var(--app-text-subtle);
    flex: 0 0 auto;
  }
  .subj__input {
    flex: 1 1 auto;
    min-width: 0;
    border: 0;
    background: none;
    outline: none;
    color: var(--app-text-strong);
    font: var(--w-regular) var(--t-ui) / 1 var(--app-font-sans);
    letter-spacing: var(--ls-ui);
  }
  .subj__input::placeholder {
    color: var(--app-text-subtle);
  }
  .subj__sub {
    flex: 0 0 auto;
    padding: 0 var(--s-16) var(--s-6);
  }
  .subj__list {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    padding: 0 var(--s-16) var(--s-12);
  }
  /* Empty / error: one card, centered in the region rather than pinned to its
     top edge. Still scrolls if the window is shorter than the card. */
  .subj__list--empty {
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .subj__loading {
    padding: var(--s-12) var(--s-8);
  }

  /* Sticky band header — the journal's / settings' idiom. */
  .tier-h {
    position: sticky;
    top: 0;
    z-index: 3;
    display: flex;
    align-items: baseline;
    gap: var(--s-8);
    padding: var(--s-8) var(--s-4) var(--s-6);
    background: linear-gradient(var(--app-bg) 74%, transparent);
  }
  .tier-h__note {
    color: var(--app-text-subtle);
  }

  /* ── the row ──────────────────────────────────────────────────────────── */
  .srowv {
    display: flex;
    align-items: center;
    min-height: 36px;
    border-radius: var(--r-md);
  }
  .srowv + .srowv {
    box-shadow: inset 0 var(--hairline) 0 var(--app-border);
  }
  .srowv.is-key {
    background: var(--app-accent);
    box-shadow: none;
  }
  .srowv.is-key + .srowv {
    box-shadow: none;
  }
  .srowv__hit {
    flex: 1 1 auto;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: var(--s-12);
    padding: 5px var(--s-8);
    border: 0;
    border-radius: var(--r-md);
    background: none;
    color: inherit;
    font: inherit;
    text-align: left;
    cursor: pointer;
  }
  .srowv__hit:focus-visible {
    outline: none;
    box-shadow: var(--ring);
  }
  .srowv:not(.is-key) .srowv__hit:hover {
    background: var(--app-surface-hover);
  }
  .srowv__t {
    flex: 1 1 auto;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .srowv__l1 {
    display: flex;
    align-items: center;
    gap: var(--gap-inline);
  }
  .srowv__nm {
    font: var(--w-medium) var(--t-ui) / 1.25 var(--app-font-sans);
    letter-spacing: var(--ls-ui);
    color: var(--app-text-strong);
  }
  .srowv__cc {
    color: var(--app-text-subtle);
  }
  .srowv__hl {
    font: var(--w-regular) var(--t-meta) / 1.4 var(--app-font-sans);
    color: var(--app-text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 62ch;
  }
  .srowv--fade .srowv__nm,
  .srowv--fade .conf {
    color: var(--app-text-muted);
  }
  .srowv--fade .srowv__hl {
    color: var(--app-text-subtle);
  }

  .conf {
    flex: 0 0 52px;
    text-align: right;
    font: var(--w-semi) var(--t-ui) / 1 var(--app-font-mono);
    color: var(--app-text-strong);
  }
  .rel {
    flex: 0 0 58px;
    text-align: right;
    font: var(--w-regular) var(--t-meta) / 1 var(--app-font-mono);
    color: var(--app-text-subtle);
  }
  .pinstar {
    color: var(--app-warn);
    font-size: 11px;
    line-height: 1;
  }

  .trend {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    height: 16px;
    padding: 0 5px;
    border-radius: var(--r-sm);
    background: var(--app-surface-hover);
    color: var(--app-text-muted);
    font: var(--w-medium) var(--t-label) / 1 var(--app-font-mono);
    letter-spacing: 0.04em;
    white-space: nowrap;
  }
  .trend--warm {
    background: var(--app-accent-bg);
    color: var(--app-accent);
  }
  .trend--cool {
    background: var(--app-info-bg);
    color: var(--app-info);
  }

  .srowv__caret {
    flex: 0 0 auto;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    margin-right: var(--s-4);
    border: 0;
    border-radius: var(--r-sm);
    background: none;
    color: var(--app-text-faint);
    cursor: pointer;
    transition: transform var(--dur-quick) var(--ease);
  }
  .srowv__caret svg {
    width: 11px;
    height: 11px;
  }
  .srowv__caret.is-open {
    transform: rotate(90deg);
  }
  .srowv__caret:focus-visible {
    outline: none;
    box-shadow: var(--ring);
  }

  /* Selected row: everything inside it flips to accent-contrast ink. */
  .srowv.is-key .srowv__nm,
  .srowv.is-key .srowv__hl,
  .srowv.is-key .srowv__cc,
  .srowv.is-key .conf,
  .srowv.is-key .rel,
  .srowv.is-key .srowv__caret,
  .srowv.is-key .pinstar {
    color: var(--app-accent-contrast);
  }
  .srowv.is-key .srowv__hl,
  .srowv.is-key .rel {
    opacity: 0.78;
  }
  .srowv.is-key .trend {
    background: color-mix(in srgb, var(--app-accent-contrast) 18%, transparent);
    color: var(--app-accent-contrast);
  }
  .srowv.is-key :global(.kbd) {
    background: color-mix(in srgb, var(--app-accent-contrast) 20%, transparent);
    color: var(--app-accent-contrast);
    box-shadow: none;
  }
  .srowv.is-key :global(.spk__lead) {
    stroke: var(--app-accent-contrast);
  }
  .srowv.is-key :global(.spk__rest),
  .srowv.is-key :global(.spk__fade) {
    stroke: var(--app-accent-contrast);
    opacity: 0.5;
  }
  .srowv.is-key :global(.spk__floor) {
    stroke: var(--app-accent-contrast);
    opacity: 0.45;
  }

  .emptybox {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--s-6);
    /* A reading-width card, not a full-bleed band — at 1100px the stretched
       version read as a mystery panel rather than a message. */
    width: 100%;
    max-width: 520px;
    margin-top: var(--s-16);
    padding: var(--s-20) var(--s-12);
    border-radius: var(--r-lg);
    text-align: center;
    background: var(--app-surface);
  }
  .emptybox .gl {
    font-size: 20px;
    line-height: 1;
    color: var(--app-text-faint);
  }
  .emptybox .t-meta {
    max-width: 52ch;
  }
  .strong {
    color: var(--app-text-strong);
  }
</style>
