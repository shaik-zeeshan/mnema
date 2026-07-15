<script lang="ts">
  // OCR — engine + admission/execution pacing (mockup A).
  //
  // The card title pushes the feature detail (the `ocr` job lane's table +
  // inspector); this stays the summary card it always was. The two event logs
  // keep their own paged tables inside a body below the stats: they are 10-12
  // columns of forensic detail, not the mockup's 4-column sample, so they keep
  // `.debug-table`'s dense framed treatment rather than the flush `.jobs` look.

  import { tip } from "$lib/components/tooltip";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import StatGrid from "./StatGrid.svelte";
  import { anchor } from "../sections";
  import { getDebugController } from "../state/controller.svelte";
  import {
    activeSignalBadges,
    formatDebugTime,
    formatOptionalMs,
    severityBadgeClass,
    severityCardClass,
    severityLabel,
    truncateDebugText,
    type DebugStat,
  } from "../format";

  const { capture, detail, features, health, pipeline } = getDebugController();

  const severity = $derived(health.severityFor("ocr"));
  const settings = $derived(capture.recordingSettings?.ocr ?? null);
  const lane = $derived(features.lane("ocr"));
  const summary = $derived(pipeline.ocrBudgetDebug?.summary ?? null);

  const engineDesc = $derived.by(() => {
    if (!settings) return null;
    const parts: string[] = [settings.provider];
    if (settings.modelId) parts.push(settings.modelId);
    return parts.join(" · ");
  });

  const stats = $derived.by<DebugStat[]>(() => [
    { key: "queued", label: "Queued", value: lane.queued },
    { key: "running", label: "Running", value: lane.running, tone: lane.running > 0 ? "ok" : undefined },
    {
      key: "lastrun",
      label: "Last run",
      value: summary?.lastRunDurationMs ?? "—",
      unit: summary?.lastRunDurationMs != null ? " ms" : undefined,
      sub: summary?.lastRunStatus ?? null,
    },
    {
      key: "pacing",
      label: "Pacing",
      value: summary?.lastPacingMode ?? "—",
      sub: summary ? `cooldown ${formatOptionalMs(summary.cooldownRemainingMs)}` : null,
    },
  ]);
</script>

<SettingGroup
  title="OCR"
  hint="processor: ocr"
  hintInline
  id={anchor("ocr")}
  onTitleClick={() => detail.open("ocr")}
  cardClass={severityCardClass(severity)}
>
  {#snippet titleExtra()}
    <span class={severityBadgeClass(severity)} use:tip={health.reasonFor("ocr") ?? ""}>
      <span class="b-dot" aria-hidden="true"></span>{severityLabel(severity)}
    </span>
  {/snippet}

  {#snippet actions()}
    <button
      class="btn btn--ghost btn--sm"
      onclick={pipeline.refreshOcrBudget}
      disabled={pipeline.loadingOcrBudget}
      aria-label="Refresh OCR budget"
      use:tip={"Refresh OCR budget"}
    >
      <span class="refresh-glyph" class:refresh-glyph--spin={pipeline.loadingOcrBudget} aria-hidden="true">↻</span>
    </button>
  {/snippet}

  <div class="row">
    <div class="row__main">
      <div class="row__label">Engine</div>
      <div class="row__desc row__desc--mono">{engineDesc ?? "—"}</div>
    </div>
    <div class="row__value">
      {#if settings && !settings.enabled}
        <span class="badge badge--neutral">disabled</span>
      {:else if settings}
        <span class="badge badge--ok">enabled</span>
      {/if}
    </div>
  </div>

  <div class="row">
    <div class="row__main">
      <div class="row__label">Execution</div>
      <div class="row__desc">
        {summary ? `${summary.queuedOrRunningCount} frame${summary.queuedOrRunningCount === 1 ? "" : "s"} queued or running at the last sample` : "budget state has not loaded yet"}
      </div>
    </div>
    <div class="row__value">
      <span class="badge badge--neutral">{summary?.executionState ?? "unknown"}</span>
    </div>
  </div>

  {#if pipeline.ocrBudgetDebugError}
    <p class="debug-errline" role="alert" aria-live="polite">{pipeline.ocrBudgetDebugError}</p>
  {/if}

  <StatGrid {stats} />

  <div class="debug-body">
    {#if pipeline.ocrBudgetDebug}
      {@const budget = pipeline.ocrBudgetDebug}
      <details class="advanced">
        <summary class="advanced__summary">Admission events <span class="idle-note">{budget.admissionEvents.length}</span></summary>
        {#if budget.admissionEvents.length === 0}
          <p class="empty">no admission events in this run</p>
        {:else}
          <div class="debug-table-wrap">
            <table class="debug-table">
              <thead>
                <tr>
                  <th>time</th><th>session</th><th>workspace</th><th class="cell-num" use:tip={"frame id"}>frame</th><th>outcome</th><th>reason</th><th class="cell-num" use:tip={"queue pressure — OCR jobs queued or running at admission"}>queue</th><th class="cell-num" use:tip={"job id"}>job</th><th class="cell-num" use:tip={"related frame id (near-duplicate source)"}>related</th><th use:tip={"active admission signal badges"}>signals</th>
                </tr>
              </thead>
              <tbody>
                {#each pipeline.pagedAdmissionEvents as event (`admission-${event.occurredAt}-${event.frameId}`)}
                  <tr>
                    <td>{formatDebugTime(event.occurredAt)}</td>
                    <td class="mono-cell">{event.sessionId}</td>
                    <td class="mono-cell">{event.workspaceScope}</td>
                    <td class="cell-num">#{event.frameId}</td>
                    <td>
                      <span class={event.outcome === "admitted" ? "badge badge--ok badge--sm" : "badge badge--neutral badge--sm"}>{event.outcome}</span>
                    </td>
                    <td>{event.reason}</td>
                    <td class="cell-num">{event.queuePressureCount}</td>
                    <td class="cell-num">{event.jobId == null ? "—" : `#${event.jobId}`}</td>
                    <td class="cell-num">{event.relatedFrameId == null ? "—" : `#${event.relatedFrameId}`}</td>
                    <td>
                      {#each activeSignalBadges(event.signals) as signal}
                        <span class="badge badge--neutral badge--sm">{signal}</span>
                      {/each}
                    </td>
                  </tr>
                {/each}
              </tbody>
            </table>
          </div>
          {#if pipeline.admissionPageCount > 1}
            <div class="job-pager">
              <button class="btn btn--ghost btn--sm" onclick={() => (pipeline.admissionPage = Math.max(0, pipeline.admissionPage - 1))} disabled={pipeline.admissionPage === 0}>‹ prev</button>
              <span class="job-pager__info">page {pipeline.admissionPage + 1} / {pipeline.admissionPageCount}</span>
              <button class="btn btn--ghost btn--sm" onclick={() => (pipeline.admissionPage = Math.min(pipeline.admissionPageCount - 1, pipeline.admissionPage + 1))} disabled={pipeline.admissionPage >= pipeline.admissionPageCount - 1}>next ›</button>
            </div>
          {/if}
        {/if}
      </details>

      <details class="advanced">
        <summary class="advanced__summary">Execution events <span class="idle-note">{budget.executionEvents.length}</span></summary>
        {#if budget.executionEvents.length === 0}
          <p class="empty">no execution events in this run</p>
        {:else}
          <div class="debug-table-wrap">
            <table class="debug-table">
              <thead>
                <tr>
                  <th>time</th><th class="cell-num" use:tip={"job id"}>job</th><th class="cell-num" use:tip={"frame id"}>frame</th><th>provider</th><th>model</th><th>mode</th><th>status</th><th class="cell-num" use:tip={"run duration (ms)"}>run</th><th class="cell-num" use:tip={"queue wait before execution (ms)"}>wait</th><th class="cell-num" use:tip={"result text length (chars)"}>text</th><th class="cell-num" use:tip={"observation count extracted"}>obs</th><th>error</th>
                </tr>
              </thead>
              <tbody>
                {#each pipeline.pagedExecutionEvents as event (`execution-${event.occurredAt}-${event.jobId}`)}
                  <tr>
                    <td>{formatDebugTime(event.occurredAt)}</td>
                    <td class="cell-num">#{event.jobId}</td>
                    <td class="cell-num">{event.frameId == null ? "—" : `#${event.frameId}`}</td>
                    <td>{event.provider}</td>
                    <td>{event.modelId ?? "—"}</td>
                    <td>{event.recognitionMode ?? "—"}</td>
                    <td>{event.status}</td>
                    <td class="cell-num">{formatOptionalMs(event.runDurationMs)}</td>
                    <td class="cell-num">{formatOptionalMs(event.queueWaitMs)}</td>
                    <td class="cell-num">{event.resultTextLength ?? "—"}</td>
                    <td class="cell-num">{event.observationCount ?? "—"}</td>
                    <td use:tip={event.lastError ?? ""}>{truncateDebugText(event.lastError)}</td>
                  </tr>
                {/each}
              </tbody>
            </table>
          </div>
          {#if pipeline.executionPageCount > 1}
            <div class="job-pager">
              <button class="btn btn--ghost btn--sm" onclick={() => (pipeline.executionPage = Math.max(0, pipeline.executionPage - 1))} disabled={pipeline.executionPage === 0}>‹ prev</button>
              <span class="job-pager__info">page {pipeline.executionPage + 1} / {pipeline.executionPageCount}</span>
              <button class="btn btn--ghost btn--sm" onclick={() => (pipeline.executionPage = Math.min(pipeline.executionPageCount - 1, pipeline.executionPage + 1))} disabled={pipeline.executionPage >= pipeline.executionPageCount - 1}>next ›</button>
            </div>
          {/if}
        {/if}
      </details>
    {:else if pipeline.ocrBudgetFetching || pipeline.loadingOcrBudget}
      <!-- First load in flight: skeleton rows distinguish "fetching" from
           "never loaded" / "no data", and aria-busy announces the wait. -->
      <div class="ocr-skeleton" aria-busy="true" aria-live="polite" aria-label="Loading OCR budget state">
        {#each Array.from({ length: 5 }) as _, i (i)}
          <div class="skeleton-row"></div>
        {/each}
      </div>
    {:else if !pipeline.ocrBudgetDebugError}
      <p class="empty">OCR budget state has not loaded yet</p>
    {/if}
  </div>
</SettingGroup>
