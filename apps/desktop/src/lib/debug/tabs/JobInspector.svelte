<!--
  Job inspector (developer.html `?state=job`): replaces the jobs list inside
  ProcessorView; Back returns. Open in Timeline drives the MAIN window through
  `open_capture_result_in_main_window` — this window never navigates.
-->
<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import IconRefresh from "~icons/lucide/refresh-cw";
  import IconRetry from "~icons/lucide/rotate-ccw";
  import IconLog from "~icons/lucide/file-text";
  import IconCopy from "~icons/lucide/copy";
  import IconChevron from "~icons/lucide/chevron-right";
  import IconAlert from "~icons/lucide/triangle-alert";
  import IconCloudOff from "~icons/lucide/cloud-off";
  import { tip } from "$lib/components/tooltip";
  import { humanizeError } from "$lib/format-error";
  import { formatJobTs, parseJobTs } from "../format";
  import { DETAIL_SPECS } from "../detail/specs";
  import {
    errorTitle,
    isTransientError,
    jobModel,
    jobProvider,
    jobState,
    providerName,
    subjectLabel,
    timelineTarget,
  } from "../detail/jobs";
  import { getDebugController } from "../state/controller.svelte";
  import type { ProcessingJobListing } from "$lib/types";

  let { job }: { job: ProcessingJobListing } = $props();

  const controller = getDebugController();
  const { nav, detail } = controller;

  const spec = $derived(DETAIL_SPECS[nav.processor ?? "transcription"]);
  const st = $derived(jobState(job, detail.now));
  const provider = $derived(jobProvider(job.payloadJson));
  const model = $derived(jobModel(job.payloadJson));
  const target = $derived(timelineTarget(job));
  const canReprocess = $derived(
    (st === "failed" || st === "completed") && spec.reprocess != null && job.subjectType === spec.subjectType,
  );

  const took = $derived.by(() => {
    const start = parseJobTs(job.startedAt);
    const end = parseJobTs(job.finishedAt);
    return start != null && end != null && end >= start ? `took ${((end - start) / 1000).toFixed(1)} s` : "";
  });

  // JSON only here: the row with its payload parsed in place.
  const raw = $derived.by(() => {
    let payload: unknown = job.payloadJson;
    try {
      payload = job.payloadJson ? JSON.parse(job.payloadJson) : null;
    } catch {
      /* keep the raw string */
    }
    return JSON.stringify({ ...job, payloadJson: payload }, null, 2);
  });
  const rawBytes = $derived(new TextEncoder().encode(raw).length);

  let timelineError = $state<string | null>(null);
  async function openInTimeline() {
    if (!target) return;
    timelineError = null;
    try {
      // No goto: the main window moves, this one stays on /debug.
      await invoke("open_capture_result_in_main_window", {
        ...target,
        spanStartMs: null,
        alignedFrameId: null,
        query: null,
        byMeaning: null,
      });
    } catch (err) {
      timelineError = humanizeError(err);
    }
  }

  let copied = $state(false);
  let copiedTimer: ReturnType<typeof setTimeout> | undefined;
  async function copyId() {
    try {
      await navigator.clipboard.writeText(String(job.id));
      copied = true;
      clearTimeout(copiedTimer);
      copiedTimer = setTimeout(() => (copied = false), 1400);
    } catch {
      // A rejected clipboard write leaves the tooltip as it was.
    }
  }
  $effect(() => () => clearTimeout(copiedTimer));

  const CHIP: Record<string, string> = { retrying: "mx-chip--warn", failed: "mx-chip--danger", running: "mx-chip--info", preparing: "mx-chip--info" };
  const message = $derived(timelineError ?? detail.actionMessage);
</script>

<header class="dv-insp__head">
  <div class="dv-insp__title"><h2 class="mx-h3">#{job.id}</h2><span class="mx-chip {CHIP[st] ?? ''}">{st}</span></div>
  <p class="dv-insp__sub">{spec.name} of {subjectLabel(job).toLowerCase()}</p>
</header>

{#if job.lastError}
  {@const transient = isTransientError(job.lastError)}
  <div class="mx-notice" data-tone={st === "failed" ? "danger" : "warn"}>
    <span class="mx-notice__icon">
      {#if transient}<IconCloudOff width="15" height="15" aria-hidden="true" />{:else}<IconAlert width="15" height="15" aria-hidden="true" />{/if}
    </span>
    <b class="mx-notice__title">{st === "failed" || transient ? errorTitle(job.lastError, provider) : "The last try failed"}</b>
    <p class="mx-notice__text"><code class="dv-raw-err">{job.lastError}</code></p>
  </div>
{/if}

<dl class="mx-spec">
  <div><dt>Created</dt><dd><span class="num">{formatJobTs(job.createdAt)}</span></dd><dd class="mx-spec__note"></dd></div>
  <div><dt>Last try</dt><dd><span class="num">{formatJobTs(job.startedAt ?? (job.attemptCount > 0 ? job.updatedAt : null))}</span></dd><dd class="mx-spec__note">{took}</dd></div>
  <div>
    <dt>Tries</dt>
    <dd><span class="num">{job.attemptCount}</span> {job.attemptCount === 1 ? "try" : "tries"}, <span class="num">{job.failureCount}</span> failed</dd>
    <dd class="mx-spec__note"></dd>
  </div>
  <div>
    <dt>Provider</dt>
    <dd>{provider ? providerName(provider) : "—"}{#if model}{" · "}<span class="num">{model}</span>{/if}</dd>
    <dd class="mx-spec__note"></dd>
  </div>
</dl>

<div class="dv-acts">
  <button class="mx-btn mx-btn--primary mx-btn--sm" disabled={!target} onclick={openInTimeline}>
    <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" aria-hidden="true"><path d="M3 12h18" /><path d="M7 8v8" /><path d="M12 5v14" /><path d="M17 9v6" /></svg>
    Open in Timeline
  </button>
  {#if canReprocess}
    <button class="mx-btn mx-btn--sm" disabled={detail.acting} onclick={() => detail.reprocess(job)}>
      <IconRefresh width="13" height="13" aria-hidden="true" />Reprocess
    </button>
  {/if}
  {#if st === "retrying"}
    <button class="mx-btn mx-btn--sm" disabled={detail.acting} onclick={() => detail.retryNow(job.id)}>
      <IconRetry width="13" height="13" aria-hidden="true" />Retry now
    </button>
  {/if}
  <span class="mx-spacer"></span>
  <button class="mx-btn mx-btn--ghost mx-btn--icon mx-btn--sm" aria-label="Show in log" use:tip={"Show in log"} onclick={() => controller.showInLog(`job ${job.id}`)}>
    <IconLog width="14" height="14" aria-hidden="true" />
  </button>
  <button class="mx-btn mx-btn--ghost mx-btn--icon mx-btn--sm" aria-label="Copy job id" use:tip={copied ? "Copied" : "Copy id"} onclick={copyId}>
    <IconCopy width="14" height="14" aria-hidden="true" />
  </button>
</div>
{#if message}<p class="dv-msg">{message}</p>{/if}

<details class="dv-raw">
  <summary><IconChevron width="12" height="12" aria-hidden="true" />Raw payload<span class="num">processing_jobs · {rawBytes} B</span></summary>
  <pre>{raw}</pre>
</details>

<style>
  .dv-insp__head { display: grid; gap: 6px; }
  .dv-insp__title { display: flex; align-items: center; gap: var(--s-2); }
  .dv-insp__title .mx-h3 { font-family: var(--font-mono); font-weight: 500; letter-spacing: -0.02em; }
  .dv-insp__sub { margin: 0; font: 400 var(--text-base)/1.4 var(--font-sans); color: var(--app-text-muted); }
  .dv-acts { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }
  .dv-acts .mx-spacer { flex: 1; }
  .dv-msg { margin: -10px 0 0; font: 400 var(--text-sm)/1.4 var(--font-sans); color: var(--app-text-muted); }
  .dv-raw { border: 1px solid var(--app-border); border-radius: var(--r-md); }
  .dv-raw summary { display: flex; align-items: center; gap: 6px; padding: 8px 10px; cursor: pointer; list-style: none; font: 500 var(--text-base)/1 var(--font-sans); color: var(--app-text-muted); }
  .dv-raw summary::-webkit-details-marker { display: none; }
  .dv-raw summary :global(svg) { transition: rotate var(--t-med) var(--ease-expo); color: var(--app-text-subtle); }
  .dv-raw[open] summary :global(svg) { rotate: 90deg; }
  .dv-raw summary .num { margin-left: auto; font-size: var(--text-xs); color: var(--app-text-subtle); font-weight: 400; }
  .dv-raw pre { margin: 0; padding: 8px 10px 10px; border-top: 1px solid var(--mx-hairline); font: 450 var(--text-sm)/1.5 var(--font-mono); color: var(--app-text-muted); white-space: pre-wrap; overflow-wrap: anywhere; }
</style>
