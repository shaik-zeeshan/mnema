<!--
  Processor drill-in (developer.html `?state=processor`): breadcrumb, the
  processor's spec, the provider-unreachable notice, the status filter and the
  jobs table. A job opens JobInspector in place (`?state=job`). The meaning
  index has no jobs, so it shows only the semantic-index spec.
-->
<script lang="ts">
  import { untrack } from "svelte";
  import IconBack from "~icons/lucide/arrow-left";
  import IconChevron from "~icons/lucide/chevron-right";
  import IconCloudOff from "~icons/lucide/cloud-off";
  import IconAlert from "~icons/lucide/triangle-alert";
  import IconInbox from "~icons/lucide/inbox";
  import Segmented from "$lib/components/Segmented.svelte";
  import { formatClock } from "$lib/time-format";
  import JobInspector from "./JobInspector.svelte";
  import { formatJobTs, parseJobTs } from "../format";
  import { engineWords } from "../pipeline-view";
  import { DETAIL_SPECS } from "../detail/specs";
  import {
    filterCounts,
    isTransientError,
    jobMessage,
    jobProvider,
    jobState,
    providerName,
    subjectLabel,
    type JobFilter,
    type JobState,
  } from "../detail/jobs";
  import { JOBS_SHOWN } from "../state/detail.svelte";
  import { getDebugController } from "../state/controller.svelte";
  import type { ProcessingJobListing } from "$lib/types";

  const controller = getDebugController();
  const { nav, pipeline, detail, features } = controller;

  const spec = $derived(DETAIL_SPECS[nav.processor ?? "transcription"]);

  // Reset on drill-in, then poll this processor's jobs while mounted.
  $effect(() => {
    const key = nav.processor;
    untrack(() => detail.reset(key));
    if (!key || !DETAIL_SPECS[key].processor) return;
    return controller.poll(detail.fetchJobs, 2000);
  });

  const lane = $derived(spec.processor ? pipeline.lane(spec.processor) : null);
  const counts = $derived(lane ? filterCounts(lane) : null);
  const config = $derived(spec.config(pipeline.settings));
  const cloud = $derived(config?.provider === "deepgram");
  const lang = $derived(language((config as { language?: string } | null)?.language));

  const FILTERS: { value: JobFilter; label: string }[] = [
    { value: "all", label: "All" },
    { value: "queued", label: "Queued" },
    { value: "running", label: "Running" },
    { value: "retrying", label: "Retrying" },
    { value: "failed", label: "Failed" },
  ];

  const rows = $derived(
    detail.visible.slice(0, JOBS_SHOWN).map((job) => {
      const state = jobState(job, detail.now);
      return { job, state, msg: jobMessage(job, state, detail.now, lane?.retrying ?? 0) };
    }),
  );
  const retryingRows = $derived(detail.jobs.filter((job) => jobState(job, detail.now) === "retrying"));

  // "N more · same error": what this filter holds beyond the rows drawn.
  const tail = $derived.by(() => {
    const split = detail.filter === "queued" || detail.filter === "retrying";
    const more = (split ? (counts?.[detail.filter] ?? 0) : detail.total) - rows.length;
    if (more <= 0) return null;
    const word = detail.filter === "all" ? "" : ` ${detail.filter}`;
    // ponytail: "same error" judges the drawn rows only; hidden ones may differ.
    const errors = new Set(rows.map((r) => r.job.lastError));
    const same = detail.filter !== "all" && errors.size === 1 && !errors.has(null) ? " · same error" : "";
    return `${more.toLocaleString()} more${word}${same}`;
  });

  // Provider-unreachable: a cloud lane, retrying jobs, and a transient last error.
  const unreachable = $derived.by(() => {
    if (!lane || lane.retrying === 0 || !cloud) return null;
    const error = retryingRows[0]?.lastError ?? lane.lastError;
    if (!isTransientError(error)) return null;
    const newest = Math.max(...retryingRows.map((job) => parseJobTs(job.updatedAt) ?? 0));
    return { at: newest > 0 ? formatClock(newest) : null, waiting: lane.retrying };
  });
  const test = $derived(cloud ? features.deepgramTestResult : features.aiTestResult);
  const testing = $derived(cloud ? features.testingDeepgram : features.testingAi);
  const runTest = () => (cloud ? features.testDeepgram() : features.testAiConnection());

  const CHIP: Partial<Record<JobState, string>> = {
    retrying: "mx-chip--warn",
    failed: "mx-chip--danger",
    running: "mx-chip--info",
    preparing: "mx-chip--info",
  };

  function avgTime(seconds: number | null): string | null {
    if (seconds == null) return null;
    return seconds < 0.1 ? `${Math.round(seconds * 1000)} ms` : `${seconds < 10 ? seconds.toFixed(1) : Math.round(seconds)} s`;
  }

  function language(code: string | undefined): string | null {
    if (!code || code === "auto") return null;
    try {
      return new Intl.DisplayNames(["en"], { type: "language" }).of(code) ?? code;
    } catch {
      return code;
    }
  }

  function open(job: ProcessingJobListing) {
    detail.inspected = job;
    controller.openJob(job.id);
  }

  const n = (v: number | null | undefined) => (v == null ? "—" : v.toLocaleString());
</script>

{#snippet count(value: string)}
  <span class="num">{counts?.[value as JobFilter]?.toLocaleString() ?? "—"}</span>
{/snippet}

<nav class="dv-crumb" aria-label="Breadcrumb">
  <button class="mx-btn mx-btn--ghost mx-btn--icon mx-btn--sm" aria-label="Back" onclick={() => controller.back()}>
    <IconBack width="14" height="14" aria-hidden="true" />
  </button>
  <a href="#pipeline" onclick={(e) => { e.preventDefault(); controller.go("pipeline"); }}>Pipeline</a>
  <IconChevron width="12" height="12" aria-hidden="true" />
  {#if nav.jobId != null}
    <a href="#processor" onclick={(e) => { e.preventDefault(); controller.back(); }}>{spec.name}</a>
  {:else}
    {spec.name}
  {/if}
</nav>

{#if nav.jobId != null && detail.inspected}
  <JobInspector job={detail.inspected} />
{:else if !spec.processor}
  {@const index = pipeline.semanticIndex}
  <dl class="mx-spec">
    <div><dt>Passages searchable</dt><dd><span class="num">{n(index?.vectorCount)}</span></dd><dd class="mx-spec__note"></dd></div>
    <div><dt>Behind</dt><dd><span class="num">{n(index?.backlogCount)}</span></dd><dd class="mx-spec__note">waiting to be indexed</dd></div>
    <div><dt>Dimension</dt><dd><span class="num">{index ? (index.liveDimension ?? "no index") : "—"}</span></dd><dd class="mx-spec__note"></dd></div>
    <div><dt>Model loaded</dt><dd>{index?.modelLoaded == null ? "—" : index.modelLoaded ? "Yes" : "No"}</dd><dd class="mx-spec__note">unloads when idle</dd></div>
    <div><dt>Load failures</dt><dd><span class="num">{n(index?.consecutiveLoadFailures)}</span></dd><dd class="mx-spec__note">in a row</dd></div>
    <div><dt>Quarantined</dt><dd><span class="num">{n(index?.quarantinedCount)}</span></dd><dd class="mx-spec__note">since app start</dd></div>
  </dl>
  {#if index?.lastLoadError}
    <div class="mx-notice" data-tone="danger">
      <span class="mx-notice__icon"><IconAlert width="15" height="15" aria-hidden="true" /></span>
      <b class="mx-notice__title">The model couldn’t load</b>
      <p class="mx-notice__text"><code class="dv-raw-err">{index.lastLoadError}</code></p>
    </div>
  {/if}
{:else}
  {@const avg = avgTime(lane?.averageCompletedSecondsLast24h ?? null)}
  <dl class="mx-spec">
    <div><dt>Average time</dt><dd>{#if avg}<span class="num">{avg}</span> {spec.per}{:else}—{/if}</dd><dd class="mx-spec__note">last 24 h</dd></div>
    <div><dt>Failed</dt><dd><span class="num">{n(lane?.failedLast24h)}</span> in the last 24 h</dd><dd class="mx-spec__note"></dd></div>
    <div><dt>Provider</dt><dd>{engineWords(spec.id, pipeline.settings, null).replace(", ", " · ") || "—"}</dd><dd class="mx-spec__note">{cloud ? "your key" : ""}</dd></div>
    <div>
      <dt>Model</dt>
      <dd><span class="num">{config?.modelId ?? "default"}</span>{#if lang}{` · ${lang}`}{/if}</dd>
      <dd class="mx-spec__note"></dd>
    </div>
  </dl>

  {#if unreachable}
    <div class="mx-notice" data-tone="warn">
      <span class="mx-notice__icon"><IconCloudOff width="15" height="15" aria-hidden="true" /></span>
      <b class="mx-notice__title">{providerName(config?.provider ?? jobProvider(retryingRows[0]?.payloadJson) ?? "provider")} can’t be reached</b>
      <p class="mx-notice__text">
        {#if unreachable.at}Timed out at <span class="num">{unreachable.at}</span>{". "}{/if}{unreachable.waiting} job{unreachable.waiting === 1 ? "" : "s"} wait and retry on their own; waiting doesn’t count as a failed try.
        {#if test}<span class="dv-test" data-tone={test.ok ? "ok" : "danger"}>{test.ok ? "Reachable now." : test.message}</span>{/if}
      </p>
      <div class="mx-notice__acts">
        <button class="mx-btn mx-btn--sm" disabled={testing} onclick={runTest}>{testing ? "Testing…" : "Test"}</button>
      </div>
    </div>
  {/if}

  <section class="dv-sec">
    <div class="dv-filter">
      <Segmented
        options={FILTERS}
        value={detail.filter}
        ariaLabel="Job status"
        icon={count}
        onValueChange={(v) => (detail.filter = v as JobFilter)}
      />
    </div>
    {#if rows.length === 0}
      <div class="mx-empty mx-empty--compact dv-empty">
        <span class="mx-empty__glyph"><IconInbox width="14" height="14" aria-hidden="true" /></span>
        <b class="mx-empty__title">{detail.error ?? "No jobs here"}</b>
      </div>
    {:else}
      <div class="dv-jobs">
        <div class="dv-jobs__head"><span>Job</span><span>Recording</span><span>Status</span><span>Tries</span><span>Updated</span></div>
        {#each rows as { job, state, msg } (job.id)}
          <button class="dv-job" data-st={state} onclick={() => open(job)}>
            <span class="dv-job__id">#{job.id}</span>
            <span class="dv-job__subj">{subjectLabel(job)}</span>
            <span class="mx-chip {CHIP[state] ?? ''}">{state}</span>
            <span class="dv-job__n">{job.attemptCount}</span>
            <span class="dv-job__t">{formatJobTs(job.updatedAt)}</span>
            {#if msg}<span class="dv-job__msg">{msg}</span>{/if}
          </button>
        {/each}
      </div>
      {#if tail}<p class="dv-tail__end">{tail}</p>{/if}
      {#if detail.error}<p class="dv-err">{detail.error}</p>{/if}
    {/if}
  </section>
{/if}

<style>
  .dv-filter { display: flex; align-items: center; gap: var(--s-2); }
  /* ponytail: Segmented's icon slot carries the count; reordered after the label. */
  .dv-filter :global(.mx-seg .seg__icon) { order: 1; width: auto; height: auto; }
  .dv-filter :global(.mx-seg .num) { font-size: var(--text-xs); color: var(--app-text-subtle); }
  .dv-jobs { display: grid; grid-template-columns: auto minmax(0, 1fr) auto auto auto; column-gap: 10px; border-top: 1px solid var(--mx-hairline); }
  .dv-jobs__head, .dv-job { display: grid; grid-column: 1 / -1; grid-template-columns: subgrid; align-items: center; }
  .dv-jobs__head { padding: 6px 4px; font: 500 var(--text-xs)/1 var(--font-sans); color: var(--app-text-subtle); border-bottom: 1px solid var(--mx-hairline); }
  .dv-job { appearance: none; border: 0; background: none; width: 100%; text-align: left; font: inherit; cursor: pointer;
    row-gap: 3px; padding: 7px 4px; border-bottom: 1px solid var(--mx-hairline); border-radius: 0; }
  .dv-job:hover { background: var(--mx-wash); }
  .dv-job:focus-visible { outline: 2px solid var(--app-accent); outline-offset: -2px; }
  .dv-job__id { font: 450 var(--text-sm)/1 var(--font-mono); color: var(--app-text-subtle); }
  .dv-job__subj { font: 400 var(--text-base)/1.3 var(--font-sans); color: var(--app-text); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .dv-job .mx-chip { justify-self: start; }
  .dv-job__n, .dv-job__t { font: 450 var(--text-sm)/1 var(--font-mono); font-variant-numeric: tabular-nums; color: var(--app-text-muted); text-align: right; }
  .dv-job__t { color: var(--app-text-subtle); }
  .dv-job__msg { grid-column: 2 / -1; font: 400 var(--text-sm)/1.4 var(--font-sans); color: var(--app-text-subtle); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .dv-job[data-st="failed"] .dv-job__msg { color: var(--app-danger-text); }
  .dv-tail__end { margin: 0; padding: 4px 0 0; font: 400 var(--text-sm)/1 var(--font-sans); color: var(--app-text-subtle); }
  .dv-test { display: block; margin-top: 4px; color: var(--tone); }
  .dv-err { margin: 0; font: 400 var(--text-sm)/1.4 var(--font-sans); color: var(--app-danger-text); }
  .dv-raw-err { display: block; margin-top: 6px; padding: 6px 8px; border-radius: var(--r-sm); background: var(--mx-wash); font: 450 var(--text-sm)/1.45 var(--font-mono); color: var(--app-text); overflow-wrap: anywhere; }
  .dv-empty .mx-empty__glyph { grid-row: auto; }
</style>
