<script lang="ts" module>
  import type { UserContextSummarizingFailure } from "$lib/types/recording";
  import { hhmm } from "$lib/insights/journal-view";

  export type OverviewState =
    /** 2a: nothing captured in the current range and not recording. */
    | { kind: "idle"; paused: boolean }
    /** 4b: Activity summarizing keeps failing. */
    | { kind: "failing"; failure: UserContextSummarizingFailure; coveredUntilMs: number | null }
    /** 4a: history backfill still running. */
    | { kind: "catching-up" };

  /** "14:40" today, "Oct 3, 14:40" otherwise. */
  export function clockLabel(ms: number): string {
    const d = new Date(ms);
    const time = hhmm(ms); // Insights' one clock: 24h, like the Journal and the "written" stamp.
    if (d.toDateString() === new Date().toDateString()) return time;
    return `${d.toLocaleDateString(undefined, { month: "short", day: "numeric" })}, ${time}`;
  }
</script>

<script lang="ts">
  // Overview's page-level states (Direction A, frames 2a, 4a, 4b) in kit
  // pieces: the idle lede's sentence + one action, the failing notice, and the
  // catching-up line — never zero tiles or "still learning…" over a real cause.
  import { openSettings } from "$lib/surface-windows";
  import { captureControls, resumeCapture, startCapture } from "$lib/capture-controls.svelte";
  import IconAlert from "~icons/lucide/triangle-alert";

  let { state }: { state: OverviewState } = $props();
</script>

{#if state.kind === "idle"}
  {#if state.paused}
    <p class="mx-prose">
      Recording is paused, so there's nothing to read or chart yet. Resume and the first summary
      lands after about 10 minutes of work.
    </p>
    <div class="acts">
      <button
        type="button"
        class="mx-btn mx-btn--primary"
        disabled={captureControls.loadingPause}
        onclick={() => void resumeCapture()}>Resume recording</button
      >
    </div>
  {:else}
    <p class="mx-prose">
      Mnema isn't recording yet, so there's nothing to read or chart. Start recording and the first
      summary lands after about 10 minutes of work; the read follows once there are two activities.
    </p>
    <div class="acts">
      <button
        type="button"
        class="mx-btn mx-btn--primary"
        disabled={captureControls.loadingStart}
        onclick={() => void startCapture()}>Start recording</button
      >
    </div>
  {/if}
{:else if state.kind === "failing"}
  {@const f = state.failure}
  <div class="mx-notice" data-tone="danger" role="alert">
    <span class="mx-notice__icon"><IconAlert width="15" height="15" aria-hidden="true" /></span>
    <b class="mx-notice__title">Summaries are failing</b>
    <p class="mx-notice__text">
      {f.provider ?? "The AI provider"} turned away the last {f.failures === 1
        ? "try"
        : `${f.failures} tries`}. {f.reason} Recording isn't affected{state.coveredUntilMs
        ? `; everything since ${clockLabel(state.coveredUntilMs)} is on the Timeline`
        : ""}.
    </p>
    <div class="mx-notice__acts">
      <button type="button" class="mx-btn mx-btn--sm" onclick={() => void openSettings("intelligence")}
        >Open engine settings</button
      >
    </div>
  </div>
{:else}
  <span class="mx-inline"
    ><span class="mx-spin mx-spin--sm"></span>Catching up on older history — past days fill in as it goes.</span
  >
{/if}

<style>
  .acts {
    margin-top: var(--s-5);
  }
  .mx-notice {
    margin-bottom: var(--s-5);
  }
</style>
