<script lang="ts">
  // The Jobs sub-tab: status filter chips, a page of rows, a row selection that
  // drives the inspector below, and a pager.
  //
  // The chips are the FOUR real wire statuses plus "all" — filtering is
  // server-side (that is what pagination requires), and "retrying" is a derived
  // row state, not something the backend can be asked for. So it is a badge in
  // the Status column, never a chip: a client-side "retrying" chip could only
  // filter the page in hand and would silently lie about the rest.

  import { tip } from "$lib/components/tooltip";
  import { getDebugController } from "../state/controller.svelte";
  import { JOBS_PAGE_SIZE } from "../state/detail.svelte";
  import { formatJobTs, truncateDebugText } from "../format";
  import { hasNextPage, jobProvider, jobState, jobStateBadgeClass, nextAttemptLabel, pageCount, pageTotalsLabel } from "./jobs";
  import type { ProcessingJobStatus } from "$lib/types";

  const { detail } = getDebugController();

  const CHIPS: { value: ProcessingJobStatus | null; label: string }[] = [
    { value: null, label: "all" },
    { value: "queued", label: "queued" },
    { value: "running", label: "running" },
    { value: "completed", label: "completed" },
    { value: "failed", label: "failed" },
  ];

  const rows = $derived(detail.jobs);

  /** Clicking the selected row again closes the inspector. */
  function toggle(id: number) {
    detail.selectedJobId = detail.selectedJobId === id ? null : id;
  }
</script>

<div class="debug-detail__card">
  <div class="log-chips debug-detail__filters" role="group" aria-label="Filter jobs by status">
    {#each CHIPS as chip (chip.label)}
      <button
        type="button"
        class="log-chip"
        class:log-chip--active={detail.statusFilter === chip.value}
        aria-pressed={detail.statusFilter === chip.value}
        onclick={() => (detail.statusFilter = chip.value)}
      >
        {chip.label}
      </button>
    {/each}
    <!-- Mockup's "segment id…" search — a server-side subject-id filter (the
         store keeps it digits-only), so it filters the whole lane, not the page
         in hand. Chips + search both repage to page 1. -->
    <div class="debug-detail__search">
      <input
        type="search"
        inputmode="numeric"
        placeholder="segment id…"
        aria-label="Filter jobs by segment id"
        value={detail.search}
        oninput={(event) => (detail.search = event.currentTarget.value)}
      />
    </div>
  </div>

  {#if rows.length === 0}
    <p class="empty debug-detail__empty">
      {detail.search
        ? `no jobs for segment #${detail.search}`
        : detail.statusFilter
          ? `no ${detail.statusFilter} jobs on this page`
          : "no jobs for this processor"}
    </p>
  {:else}
    <div class="debug-table-wrap">
      <table class="debug-table">
        <thead>
          <tr>
            <th class="cell-num" use:tip={"job id"}>job</th>
            <th>subject</th>
            <th>status</th>
            <th class="cell-num" use:tip={"attempts / genuine failures — an abandoned or transient attempt does not count as a failure"}>att / fail</th>
            <th use:tip={"the provider stamped into the job's payload when it was enqueued — “—” when the payload carries none"}>provider</th>
            <th>updated</th>
            <th>last error</th>
          </tr>
        </thead>
        <tbody>
          {#each rows as job (job.id)}
            {@const state = jobState(job, detail.now)}
            <!-- The whole row is the hit target (mockup A). `tabindex` + Enter
                 keep it reachable without keyboard: a <tr> can't be a <button>
                 without losing the table semantics the columns rely on. -->
            <!-- svelte-ignore a11y_click_events_have_key_events -->
            <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
            <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
            <tr
              class="debug-detail__row"
              class:debug-detail__row--selected={detail.selectedJobId === job.id}
              tabindex="0"
              onclick={() => toggle(job.id)}
              onkeydown={(event) => {
                if (event.key !== "Enter" && event.key !== " ") return;
                event.preventDefault();
                toggle(job.id);
              }}
            >
              <td class="cell-num">#{job.id}</td>
              <td class="mono-cell">{job.subjectType} #{job.subjectId}</td>
              <td>
                <span class={jobStateBadgeClass(state)}>{state}</span>
              </td>
              <td class="cell-num">{job.attemptCount} / {job.failureCount}</td>
              <td class="mono-cell">{jobProvider(job.payloadJson) ?? "—"}</td>
              <td>{formatJobTs(job.updatedAt)}</td>
              <td class="debug-detail__err-cell" use:tip={job.lastError ?? ""}>
                {#if state === "retrying"}
                  <span class="debug-detail__retry">{nextAttemptLabel(job.nextAttemptAt, detail.now)}</span>
                {/if}
                {truncateDebugText(job.lastError, 48)}
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {/if}

  <!-- `list_processing_jobs_by_processor` returns the filter's total alongside
       the page, so both mockup readouts are sourced, not guessed: "6 of 12
       jobs" and "page 1/2". -->
  <div class="job-pager">
    <span class="job-pager__info">{pageTotalsLabel(rows.length, detail.total)}</span>
    <span class="debug-detail__spacer"></span>
    <button class="btn btn--ghost btn--sm" onclick={() => (detail.page -= 1)} disabled={detail.page === 0}>‹ prev</button>
    <span class="job-pager__info">page {detail.page + 1}/{pageCount(detail.total, JOBS_PAGE_SIZE)}</span>
    <button
      class="btn btn--ghost btn--sm"
      onclick={() => (detail.page += 1)}
      disabled={!hasNextPage(detail.page, detail.total, JOBS_PAGE_SIZE)}
      use:tip={hasNextPage(detail.page, detail.total, JOBS_PAGE_SIZE) ? "" : "no more results"}
    >
      next ›
    </button>
  </div>
</div>
