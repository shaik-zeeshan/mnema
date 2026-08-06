<script lang="ts">
  // Subjects — a DESTINATION, not a third main surface (direction 02, README
  // bullet 6). It opens from the Overview's Subjects tile header and comes back
  // through the tool strip's first control; the window still has exactly two
  // surfaces. Inside, the shape is the same as everywhere else: the 30px tool
  // strip navigates, one region scrolls, the 256px inspector carries the
  // selection's record, and the layout's status strip carries live state.
  //
  // Two views, one addressable route:
  //   /subjects                  — the tiered list
  //   /subjects?subject=<name>   — that subject opened, in place
  // The detail is a query parameter rather than a nested route so ⏎ opens it
  // without leaving the destination, and the browser/back history still works.
  import { page } from "$app/stores";
  import { goto } from "$app/navigation";
  import { onMount } from "svelte";
  import IconBack from "~icons/lucide/chevron-left";
  import IconSearch from "~icons/lucide/search";
  import IconRefresh from "~icons/lucide/refresh-cw";
  import IconPanel from "~icons/lucide/panel-right";
  import { convictionTierId } from "$lib/insights/subjectsTiers";
  import { SubjectsData } from "$lib/subjects/subjects-data.svelte";
  import { SubjectDetailData } from "$lib/subjects/subject-detail-data.svelte";
  import SubjectsList from "$lib/subjects/SubjectsList.svelte";
  import SubjectsInspector from "$lib/subjects/SubjectsInspector.svelte";
  import ConclusionCards from "$lib/subjects/ConclusionCards.svelte";
  import ConclusionStory from "$lib/subjects/ConclusionStory.svelte";
  import ConclusionInspector from "$lib/subjects/ConclusionInspector.svelte";

  const list = new SubjectsData();
  onMount(() => list.start());

  const openSubject = $derived($page.url.searchParams.get("subject"));

  // One detail loader per opened subject; the effect's teardown drops its
  // realtime listener when the subject changes or the destination closes.
  let detail = $state<SubjectDetailData | null>(null);
  $effect(() => {
    const subject = openSubject;
    if (!subject) {
      detail = null;
      return;
    }
    const next = new SubjectDetailData(subject);
    detail = next;
    return next.start();
  });

  function open(subject: string): void {
    void goto(`/subjects?subject=${encodeURIComponent(subject)}`);
  }

  function back(): void {
    void goto("/subjects");
  }

  // The inspector collapses below 1000px of pane, measured rather than assumed
  // — the window is resizable and no viewport unit knows about the strips.
  let paneWidth = $state(1100);
  const wide = $derived(paneWidth >= 1000);
  let inspectorPinned = $state(true);
  const inspectorOpen = $derived(wide && inspectorPinned);

  const TIER_NAME: Record<string, string> = {
    strong: "Strongly held",
    forming: "Forming",
    shaping: "Just taking shape",
    fading: "Fading",
  };

  // The opened subject's tier, taken from the list's own row when it is loaded.
  // Absent rather than guessed while the list has not read yet.
  const openTier = $derived.by<string | null>(() => {
    const row = list.displayRows.find((r) => r.subject === openSubject);
    return row ? TIER_NAME[convictionTierId(row)] : null;
  });

  // The detail's count line — each clause only when its count is real.
  const detailCounts = $derived.by<string>(() => {
    const d = detail;
    if (!d || !d.view) return "";
    const parts = [
      `${d.conclusionCount} ${d.conclusionCount === 1 ? "conclusion" : "conclusions"}`,
    ];
    if (d.fadedCount > 0) parts.push(`${d.fadedCount} below floor`);
    if (d.linkedActivityCount > 0) {
      parts.push(
        `${d.linkedActivityCount} linked ${d.linkedActivityCount === 1 ? "activity" : "activities"}`,
      );
    }
    return parts.join(" · ");
  });
</script>

<div class="subjects" bind:clientWidth={paneWidth}>
  {#if detail}
    <!-- Detail tool strip: back to the list, the subject, its tier, its counts. -->
    <div class="ss-tstrip">
      <div class="ss-tstrip__g">
        <button type="button" class="btn btn--sm btn--ghost" onclick={back}>
          <span class="ic" aria-hidden="true"><IconBack /></span>
          Subjects
        </button>
        <div class="ss-tstrip__sep"></div>
        <span class="t-label crumb">{detail.subject}</span>
        {#if openTier}<span class="ss-chip ss-chip--ok">{openTier}</span>{/if}
      </div>
      {#if detailCounts}
        <div class="ss-tstrip__sep"></div>
        <span class="t-meta is-mono">{detailCounts}</span>
      {/if}
      <span class="ss-tstrip__spacer"></span>
      {#if wide}
        <button
          type="button"
          class="btn btn--sm btn--icon"
          class:is-on={inspectorOpen}
          aria-pressed={inspectorOpen}
          aria-label="Toggle inspector"
          onclick={() => (inspectorPinned = !inspectorPinned)}><IconPanel /></button
        >
      {/if}
    </div>

    <div class="ss-body">
      <div class="ss-main ss-main--split">
        {#if detail.loadError && !detail.view}
          <div class="state">
            <p class="state__t">Couldn't load this subject.</p>
            <p class="t-meta">{detail.loadError}</p>
            <button type="button" class="btn btn--sm" onclick={() => void detail?.load()}
              >Try again</button
            >
          </div>
        {:else if detail.loading && !detail.view}
          <p class="quiet t-meta">Reading this subject…</p>
        {:else if detail.conclusionCount === 0}
          <div class="state">
            <p class="state__t">Nothing concluded about {detail.subject} yet.</p>
            <p class="t-meta">
              Conclusions form as evidence accumulates. This subject has no active or
              faded conclusions to chart.
            </p>
          </div>
        {:else}
          <ConclusionCards data={detail} />
          <ConclusionStory data={detail} />
        {/if}
      </div>

      {#if inspectorOpen}
        <ConclusionInspector data={detail} />
      {/if}
    </div>
  {:else}
    <!-- List tool strip: back to Overview, the grouping axis, the search field,
         and the staged-refresh pill (only when the engine really has news). -->
    <div class="ss-tstrip">
      <div class="ss-tstrip__g">
        <button
          type="button"
          class="btn btn--sm btn--ghost"
          onclick={() => void goto("/overview")}
        >
          <span class="ic" aria-hidden="true"><IconBack /></span>
          Overview
        </button>
        <div class="ss-tstrip__sep"></div>
        <span class="t-label crumb">Subjects</span>
      </div>

      <div class="ss-tstrip__sep"></div>
      <div class="ss-seg" role="group" aria-label="Group subjects by">
        <button
          type="button"
          class="ss-seg__i"
          class:is-on={list.axis === "conviction"}
          aria-pressed={list.axis === "conviction"}
          onclick={() => (list.axis = "conviction")}>By conviction</button
        >
        <button
          type="button"
          class="ss-seg__i"
          class:is-on={list.axis === "movement"}
          aria-pressed={list.axis === "movement"}
          onclick={() => (list.axis = "movement")}>By movement</button
        >
      </div>

      <label class="find">
        <span class="ic" aria-hidden="true"><IconSearch /></span>
        <span class="sr-only">Search subjects</span>
        <input
          class="input"
          type="search"
          placeholder="Search subjects…"
          value={list.query}
          oninput={(e) => list.search(e.currentTarget.value)}
        />
      </label>

      <span class="ss-tstrip__spacer"></span>

      {#if list.pendingCount > 0}
        <button type="button" class="ss-chip pill-btn" onclick={() => list.applyStaged()}>
          <span class="ic" aria-hidden="true"><IconRefresh /></span>
          {list.pendingCount} views updated
        </button>
      {/if}

      {#if wide}
        <div class="ss-tstrip__sep"></div>
        <button
          type="button"
          class="btn btn--sm btn--icon"
          class:is-on={inspectorOpen}
          aria-pressed={inspectorOpen}
          aria-label="Toggle inspector"
          onclick={() => (inspectorPinned = !inspectorPinned)}><IconPanel /></button
        >
      {/if}
    </div>

    <div class="ss-body">
      <div class="ss-main">
        <SubjectsList data={list} onopen={open} />
      </div>
      {#if inspectorOpen}
        <SubjectsInspector data={list} onopen={open} />
      {/if}
    </div>
  {/if}
</div>

<style>
  .subjects {
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }

  /* The detail is master–detail INSIDE the surface: the conclusion strip and
     the story column sit side by side in the one scrolling region's place. */
  .ss-main--split {
    flex-direction: row;
  }

  .crumb {
    color: var(--app-text-strong);
  }

  .ic {
    display: flex;
    font-size: 11px;
  }

  .btn.is-on {
    background: var(--app-surface-active);
  }

  /* The tool strip's search field: the kit's `.input`, at chrome height. */
  .find {
    position: relative;
    display: flex;
    align-items: center;
  }

  .find .ic {
    position: absolute;
    left: 6px;
    color: var(--app-text-subtle);
    pointer-events: none;
  }

  .find input {
    width: 180px;
    height: var(--h-sm);
    padding: 0 var(--s-6) 0 22px;
    font-size: var(--t-meta);
  }

  .find input::-webkit-search-cancel-button {
    appearance: none;
  }

  .pill-btn {
    border: var(--hairline) solid var(--app-bezel);
    cursor: pointer;
  }

  .pill-btn:hover {
    color: var(--app-text-strong);
    border-color: var(--app-border-hover);
  }

  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }

  .quiet,
  .state {
    padding: var(--s-16);
  }

  .state {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--s-6);
  }

  .state__t {
    margin: 0;
    font: var(--w-medium) var(--t-ui) / 1.3 var(--app-font-sans);
    color: var(--app-text-strong);
  }

  .state :global(p.t-meta) {
    margin: 0;
    max-width: 56ch;
  }
</style>
