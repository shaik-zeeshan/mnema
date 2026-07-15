<script lang="ts">
  // Jobs & Storage — migrated from the legacy "System" tab's App Infra card and
  // the "Pipeline" tab's Background Jobs + Segment Workspace Cleanup cards.
  // All three describe the same thing: what the app has stored and what it is
  // chewing through.

  import { tip } from "$lib/components/tooltip";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import StatGrid from "./StatGrid.svelte";
  import { anchor } from "../sections";
  import { getDebugController } from "../state/controller.svelte";
  import {
    batchStatusBadgeClass,
    dispositionBadgeClass,
    dispositionLabel,
    formatCount,
    formatJobTs,
    jobStatusBadgeClass,
    ocrStatusBadgeClass,
    severityBadgeClass,
    severityCardClass,
    severityLabel,
    shortenPath,
    type DebugStat,
  } from "../format";

  const { health, pipeline } = getDebugController();

  const severity = $derived(health.severityFor("jobsAndStorage"));
  const infra = $derived(pipeline.infraStatus);

  const stats = $derived.by<DebugStat[]>(() => [
    { key: "workers", label: "Workers", value: infra?.workerThreadCount ?? "—" },
    { key: "total", label: "Jobs total", value: formatCount(infra?.jobCounts.total) },
    {
      key: "queuedRunning",
      label: "Queued / run",
      value: infra ? `${formatCount(infra.jobCounts.queued)} / ${formatCount(infra.jobCounts.running)}` : "—",
      tone: (infra?.jobCounts.running ?? 0) > 0 ? "ok" : undefined,
    },
    {
      key: "failed",
      label: "Failed",
      value: formatCount(infra?.jobCounts.failed),
      tone: (infra?.jobCounts.failed ?? 0) > 0 ? "warn" : undefined,
    },
  ]);
</script>

<SettingGroup
  title="Jobs & Storage"
  hint="app-infra"
  hintInline
  id={anchor("jobsAndStorage")}
  cardClass={severityCardClass(severity)}
>
  {#snippet titleExtra()}
    <span class={severityBadgeClass(severity)} use:tip={health.reasonFor("jobsAndStorage") ?? ""}>
      <span class="b-dot" aria-hidden="true"></span>{severityLabel(severity)}
    </span>
  {/snippet}

  {#snippet actions()}
    <button
      class="btn btn--ghost btn--sm"
      onclick={pipeline.refreshAll}
      disabled={pipeline.loadingInfraStatus || pipeline.loadingJobs}
      aria-label="Refresh app infra"
      use:tip={"Refresh app infra"}
    >
      <span class="refresh-glyph" class:refresh-glyph--spin={pipeline.loadingInfraStatus || pipeline.loadingJobs} aria-hidden="true">↻</span>
    </button>
  {/snippet}

  <!-- ── App infra ─────────────────────────────────────────────────────── -->
  <div class="row">
    <div class="row__main">
      <div class="row__label">Database</div>
      <div class="row__desc row__desc--mono" use:tip={infra?.databasePath ?? ""}>
        {infra ? shortenPath(infra.databasePath, 56) : "not loaded"} · SQLCipher
      </div>
    </div>
    <div class="row__value">
      {#if infra}
        <!-- `migrationsRan` says whether THIS startup applied anything, not
             whether the schema is current — a healthy up-to-date DB reports
             `false`, so neither value is a fault. -->
        <span class={infra.migrationsRan ? "badge badge--ok" : "badge badge--neutral"}>
          {infra.migrationsRan ? "migrations applied" : "up to date"}
        </span>
      {/if}
    </div>
  </div>

  {#if pipeline.infraStatusError}
    <p class="debug-errline" role="alert" aria-live="polite">{pipeline.infraStatusError}</p>
  {/if}

  <StatGrid {stats} />

  <div class="debug-body">
    {#if !infra && !pipeline.infraStatusError}
      <p class="empty">infra status has not loaded yet — press ↻</p>
    {/if}

    <!-- ── Background jobs ─────────────────────────────────────────────── -->
    <div class="debug-block__label">
      Background Jobs
      {#if pipeline.postSubmitPolling}
        <span class="idle-note">polling ({pipeline.postSubmitPollsLeft} left)</span>
      {/if}
      <button
        class="btn btn--ghost btn--sm debug-block__action"
        onclick={pipeline.refreshAll}
        disabled={pipeline.loadingJobs || pipeline.loadingInfraStatus}
        aria-label="Refresh job list"
      >
        {pipeline.loadingJobs ? "…" : "↻ list"}
      </button>
    </div>

    <!-- Submit form -->
    <details class="advanced">
      <summary class="advanced__summary">Submit debug CPU job</summary>
      <form class="job-submit-form" onsubmit={(e) => { e.preventDefault(); pipeline.submitDebugJob(); }}>
        <input class="job-input" type="text" placeholder="document name" bind:value={pipeline.submitDocName} disabled={pipeline.submitting} />
        <input class="job-input" type="text" placeholder="source text" bind:value={pipeline.submitSourceText} disabled={pipeline.submitting} />
        <button
          class="btn btn--primary btn--sm"
          type="submit"
          disabled={pipeline.submitting || pipeline.submitDocName.trim() === "" || pipeline.submitSourceText.trim() === ""}
        >
          {pipeline.submitting ? "…" : "submit"}
        </button>
      </form>
      {#if pipeline.submitError}
        <p class="debug-err" role="alert" aria-live="polite">{pipeline.submitError}</p>
      {/if}
    </details>

    <!-- Job list -->
    <div class="idle-section-label">
      Recent jobs
      {#if pipeline.loadingJobs}<span class="idle-note">loading…</span>{/if}
      {#if pipeline.jobs.length > 0}
        <span class="idle-note">
          {pipeline.jobsPageStart + 1}–{Math.min(pipeline.jobsPageStart + pipeline.jobsPageSize, pipeline.jobs.length)} of {pipeline.jobs.length}
        </span>
      {/if}
    </div>
    {#if pipeline.jobsError}
      <p class="debug-err" role="alert" aria-live="polite">{pipeline.jobsError}</p>
    {:else if pipeline.jobs.length === 0}
      <p class="empty">no jobs yet</p>
    {:else}
      <ul class="job-list">
        {#each pipeline.pagedJobs as job (job.id)}
          <li>
            <button
              class="job-row"
              class:job-row--selected={pipeline.selectedJobId === job.id}
              type="button"
              onclick={() => pipeline.selectJob(job)}
            >
              <span class="job-row__id">#{job.id}</span>
              <span class="job-row__kind">{job.kind}</span>
              <span class={jobStatusBadgeClass(job.status)}>{job.status}</span>
              <span class="job-row__ts">{formatJobTs(job.createdAt)}</span>
            </button>
          </li>
        {/each}
      </ul>
      {#if pipeline.jobsPageCount > 1}
        <div class="job-pager" role="group" aria-label="Recent jobs pagination">
          <button type="button" class="btn btn--ghost btn--sm" onclick={() => (pipeline.jobsPage = Math.max(0, pipeline.jobsPage - 1))} disabled={pipeline.jobsPage === 0} aria-label="Previous page">‹ prev</button>
          <span class="job-pager__info">page {pipeline.jobsPage + 1} / {pipeline.jobsPageCount}</span>
          <button type="button" class="btn btn--ghost btn--sm" onclick={() => (pipeline.jobsPage = Math.min(pipeline.jobsPageCount - 1, pipeline.jobsPage + 1))} disabled={pipeline.jobsPage >= pipeline.jobsPageCount - 1} aria-label="Next page">next ›</button>
        </div>
      {/if}
    {/if}

    <!-- Selected job detail -->
    {#if pipeline.selectedJobId != null}
      <div class="idle-section-label">
        Job #{pipeline.selectedJobId}
        <button class="btn btn--ghost btn--sm debug-inline-btn" onclick={pipeline.refreshSelectedJob} disabled={pipeline.loadingSelectedJob} aria-label="Refresh selected job">
          {pipeline.loadingSelectedJob ? "…" : "↻"}
        </button>
        {#if pipeline.selectedJobOnAnotherPage}
          <button
            type="button"
            class="btn btn--ghost btn--sm debug-inline-btn"
            onclick={pipeline.goToSelectedJobPage}
            aria-label="Jump to the page containing the selected job"
          >
            show in list
          </button>
        {/if}
      </div>
      {#if pipeline.selectedJobError}
        <p class="debug-err" role="alert" aria-live="polite">{pipeline.selectedJobError}</p>
      {/if}
      {#if pipeline.selectedJob}
        {@const job = pipeline.selectedJob}
        <ul class="kv-list">
          <li>
            <span class="kv-key kv-key--wide">status</span>
            <span class={jobStatusBadgeClass(job.status)}>{job.status}</span>
          </li>
          <li><span class="kv-key kv-key--wide">attempts</span><span class="kv-val kv-val--mono">{job.attemptCount}</span></li>
          {#if job.startedAt}
            <li><span class="kv-key kv-key--wide">started</span><span class="kv-val kv-val--mono">{formatJobTs(job.startedAt)}</span></li>
          {/if}
          {#if job.finishedAt}
            <li><span class="kv-key kv-key--wide">finished</span><span class="kv-val kv-val--mono">{formatJobTs(job.finishedAt)}</span></li>
          {/if}
          {#if job.resultText}
            <li class="kv-list-block">
              <span class="kv-key kv-key--wide">result</span>
              <span class="job-detail-text">{job.resultText}</span>
            </li>
          {/if}
          {#if job.lastError}
            <li class="kv-list-block">
              <span class="kv-key kv-key--wide">error</span>
              <span class="job-detail-text job-detail-text--err">{job.lastError}</span>
            </li>
          {/if}
        </ul>
      {/if}
    {/if}

    <!-- ── Segment workspace cleanup ───────────────────────────────────── -->
    <details class="advanced">
      <summary class="advanced__summary">Classify a hidden segment workspace dir</summary>
      <form class="job-submit-form" onsubmit={(e) => { e.preventDefault(); pipeline.classifyWorkspace(); }}>
        <input
          class="job-input"
          type="text"
          placeholder="/…/recordings/YYYY/MM/DD/.session-segment-####"
          bind:value={pipeline.workspaceDirInput}
          disabled={pipeline.loadingWorkspaceClassification}
          aria-invalid={pipeline.workspaceClassificationError ? "true" : undefined}
          spellcheck="false"
          autocomplete="off"
        />
        <button class="btn btn--primary btn--sm" type="submit" disabled={pipeline.loadingWorkspaceClassification || pipeline.workspaceDirInput.trim() === ""}>
          {pipeline.loadingWorkspaceClassification ? "…" : "classify"}
        </button>
      </form>

      {#if pipeline.workspaceClassificationError}
        <p class="debug-err" role="alert" aria-live="polite">{pipeline.workspaceClassificationError}</p>
      {:else if pipeline.workspaceClassificationLoaded && pipeline.workspaceClassification == null}
        <p class="empty">
          not a hidden segment workspace path (expected a directory named
          <code>.&lt;session&gt;-segment-####</code>)
        </p>
      {:else if pipeline.workspaceClassification}
        {@const info = pipeline.workspaceClassification}
        <ul class="kv-list">
          <li>
            <span class="kv-key kv-key--wide">disposition</span>
            <span class={dispositionBadgeClass(info.disposition)}>{dispositionLabel(info.disposition)}</span>
          </li>
          <li>
            <span class="kv-key kv-key--wide">safe to remove</span>
            <span class={info.safeToRemove ? "badge badge--ok badge--sm" : "badge badge--warn badge--sm"}>
              {info.safeToRemove ? "yes" : "no"}
            </span>
          </li>
          <li>
            <span class="kv-key kv-key--wide">visible segment</span>
            <span class={info.visibleSegmentExists ? "badge badge--ok badge--sm" : "badge badge--err badge--sm"}>
              {info.visibleSegmentExists ? "present" : "missing"}
            </span>
            <span class="kv-val kv-val--mono" use:tip={info.paths.visibleSegmentPath}>{shortenPath(info.paths.visibleSegmentPath)}</span>
          </li>
          <li><span class="kv-key kv-key--wide">frame count</span><span class="kv-val kv-val--mono">{info.frameCount}</span></li>
          <li>
            <span class="kv-key kv-key--wide">workspace</span>
            <span class="kv-val kv-val--mono" use:tip={info.paths.workspaceDir}>{shortenPath(info.paths.workspaceDir)}</span>
          </li>
          <li>
            <span class="kv-key kv-key--wide">frames dir</span>
            <span class="kv-val kv-val--mono" use:tip={info.paths.framesDir}>{shortenPath(info.paths.framesDir)}</span>
          </li>
        </ul>

        <div class="idle-section-label">Batch references <span class="idle-note">{info.batchReferences.length}</span></div>
        {#if info.batchReferences.length === 0}
          <p class="empty">none</p>
        {:else}
          <ul class="kv-list">
            {#each info.batchReferences as ref (ref.batchId)}
              <li>
                <span class="kv-key kv-key--wide">batch #{ref.batchId}</span>
                <span class={batchStatusBadgeClass(ref.status)}>{ref.status}</span>
              </li>
            {/each}
          </ul>
        {/if}

        <div class="idle-section-label">Non-terminal OCR references <span class="idle-note">{info.nonterminalOcrReferences.length}</span></div>
        {#if info.nonterminalOcrReferences.length === 0}
          <p class="empty">none</p>
        {:else}
          <ul class="kv-list">
            {#each info.nonterminalOcrReferences as ref (ref.jobId)}
              <li>
                <span class="kv-key kv-key--wide">frame #{ref.frameId} · job #{ref.jobId}</span>
                <span class={ocrStatusBadgeClass(ref.status)}>{ref.status}</span>
              </li>
            {/each}
          </ul>
        {/if}
      {:else}
        <p class="empty">enter a hidden segment workspace path to classify</p>
      {/if}
    </details>
  </div>
</SettingGroup>
