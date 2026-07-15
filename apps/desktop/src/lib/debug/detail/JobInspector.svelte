<script lang="ts">
  // The selected job, in full: subject, attempts vs counted failures, backoff,
  // next attempt, payload, actions.
  //
  // ── No attempt history ────────────────────────────────────────────────────
  // The mockup's "Attempt history" list is deliberately absent. Retries UPDATE
  // the job row in place, so only the latest error survives — there is nothing
  // per-attempt to read, and a timeline here could only be invented. `attempts /
  // failures / lastError / nextAttemptAt` is the honest shadow of it (PLAN, Out
  // of Scope). Upgrade path when that stops being enough: a per-attempt ring or
  // table Rust-side (the generic form of the OCR-budget ring buffer), appended
  // on each mark_job_failed / claim, then listed by a new command — at which
  // point it renders in the gap this comment sits in.

  import { tip } from "$lib/components/tooltip";
  import { getDebugController } from "../state/controller.svelte";
  import { formatJobTs } from "../format";
  import { DETAIL_SPECS, type DetailFeatureId } from "./specs";
  import { jobState, jobStateBadgeClass, nextAttemptLabel } from "./jobs";

  interface Props {
    feature: DetailFeatureId;
  }

  let { feature }: Props = $props();

  const { detail } = getDebugController();

  const spec = $derived(DETAIL_SPECS[feature]);
  const job = $derived(detail.selectedJob);
  const state = $derived(job ? jobState(job, detail.now) : null);
  const nextAttempt = $derived(job ? nextAttemptLabel(job.nextAttemptAt, detail.now) : null);

  /** Only offer a requeue the backend can actually take for this subject. */
  const canReprocess = $derived(
    job != null && spec.reprocess != null && job.subjectType === spec.subjectType,
  );

  /** Pretty-print the payload when it is JSON; show it raw when it isn't. */
  const payload = $derived.by(() => {
    if (!job?.payloadJson) return null;
    try {
      return JSON.stringify(JSON.parse(job.payloadJson), null, 2);
    } catch {
      return job.payloadJson;
    }
  });
</script>

{#if job}
  <div class="debug-detail__card">
    <div class="debug-detail__insp-head">
      <span class="debug-detail__insp-title">Job #{job.id} — {job.subjectType} #{job.subjectId}</span>
      {#if state}
        <span class={jobStateBadgeClass(state)}>{state}</span>
      {/if}
      <span class="debug-detail__spacer"></span>
      <button class="btn btn--ghost btn--sm" onclick={() => (detail.selectedJobId = null)}>close</button>
    </div>

    <ul class="kv-list debug-detail__insp-kv">
      <li>
        <span class="kv-key kv-key--wide">subject</span>
        <span class="kv-val kv-val--mono">{job.subjectType} #{job.subjectId}</span>
      </li>
      <li>
        <span class="kv-key kv-key--wide">processor</span>
        <span class="kv-val kv-val--mono">{job.processor}</span>
      </li>
      <li>
        <span
          class="kv-key kv-key--wide"
          use:tip={"An attempt is a claim; a failure is a genuine error. Abandonment (quit/crash) and transient-liveness requeues (ADR 0048) re-attempt without counting a failure — so attempts ≥ failures, and only failures approach the retry cap."}
        >
          attempts / failures
        </span>
        <span class="kv-val kv-val--mono">{job.attemptCount} attempts · {job.failureCount} counted failures</span>
      </li>
      <li>
        <span class="kv-key kv-key--wide">next attempt</span>
        <span class="kv-val kv-val--mono">
          {#if nextAttempt}
            {formatJobTs(job.nextAttemptAt)} · {nextAttempt}
          {:else}
            —
          {/if}
        </span>
        {#if state === "retrying"}
          <span class="badge badge--warn badge--sm" use:tip={"Queued behind a retry backoff — the queue won't re-claim it until then."}>backoff</span>
        {/if}
      </li>
      <li>
        <span class="kv-key kv-key--wide">queued / started</span>
        <span class="kv-val kv-val--mono">{formatJobTs(job.queuedAt)} · {formatJobTs(job.startedAt)}</span>
      </li>
      <li>
        <span class="kv-key kv-key--wide">updated / finished</span>
        <span class="kv-val kv-val--mono">{formatJobTs(job.updatedAt)} · {formatJobTs(job.finishedAt)}</span>
      </li>
    </ul>

    {#if job.lastError}
      <!-- The LATEST error, not the last of several: retries overwrite it. -->
      <p class="debug-errline debug-detail__insp-err" role="status" aria-live="polite">
        {job.lastError}
        <span class="debug-errline__meta">latest error · earlier attempts are not retained</span>
      </p>
    {/if}

    {#if payload}
      <pre class="debug-json debug-detail__payload">{payload}</pre>
    {/if}

    <div class="action-row debug-detail__insp-actions">
      <button
        class="btn btn--primary btn--sm"
        onclick={detail.reprocessSelected}
        disabled={!canReprocess || detail.acting}
        use:tip={canReprocess
          ? `Requeue this ${job.subjectType} through ${spec.reprocess?.command}`
          : "No reprocess command exists for this subject"}
      >
        {detail.acting ? "requeueing…" : "reprocess subject"}
      </button>
      <button
        class="btn btn--ghost btn--sm"
        onclick={() => navigator.clipboard?.writeText(String(job.id))}
        use:tip={"Copy the job id — e.g. to grep it in the log tail"}
      >
        copy job id
      </button>
    </div>

    {#if detail.actionMessage}
      <p class="debug-note debug-detail__insp-note" role="status" aria-live="polite">{detail.actionMessage}</p>
    {/if}
  </div>
{/if}
