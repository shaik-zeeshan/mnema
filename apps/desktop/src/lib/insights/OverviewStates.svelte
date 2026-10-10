<script lang="ts" module>
  import type { UserContextSummarizingFailure } from "$lib/types/recording";

  export type OverviewState =
    /** 2a: nothing captured in the current range and not recording. */
    | { kind: "idle"; paused: boolean }
    /** 4b: Activity summarizing keeps failing. */
    | { kind: "failing"; failure: UserContextSummarizingFailure; coveredUntilMs: number | null }
    /** 4a: history backfill still running. */
    | { kind: "catching-up" };

  /** "2:40 pm" today, "Oct 3, 2:40 pm" otherwise. */
  export function clockLabel(ms: number): string {
    const d = new Date(ms);
    const time = d
      .toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" })
      .toLowerCase();
    if (d.toDateString() === new Date().toDateString()) return time;
    return `${d.toLocaleDateString(undefined, { month: "short", day: "numeric" })}, ${time}`;
  }
</script>

<script lang="ts">
  // Overview's page-level states (Direction A, frames 2a, 4a, 4b): one plain
  // sentence and at most one action, in place of zero tiles or "still
  // learning…" copy that hides a real cause.
  import { openSettings } from "$lib/surface-windows";
  import { captureControls, resumeCapture, startCapture } from "$lib/capture-controls.svelte";

  let { state }: { state: OverviewState } = $props();
</script>

{#if state.kind === "idle"}
  <div class="state state--empty">
    {#if state.paused}
      <p class="state-title">Recording is paused.</p>
      <p class="state-detail">
        Nothing has been captured in this range, so there's nothing to read or chart yet.
        Resume and the first summary lands after about 10 minutes of work.
      </p>
      <div class="state-actions">
        <button
          type="button"
          class="btn btn--accent"
          disabled={captureControls.loadingPause}
          onclick={() => void resumeCapture()}>Resume recording</button
        >
      </div>
    {:else}
      <p class="state-title">Mnema isn't recording yet.</p>
      <p class="state-detail">
        Nothing has been captured, so there's nothing to read or chart. Start recording and the
        first summary lands after about 10 minutes of work. The read follows once there are two
        activities.
      </p>
      <div class="state-actions">
        <button
          type="button"
          class="btn btn--accent"
          disabled={captureControls.loadingStart}
          onclick={() => void startCapture()}>Start recording</button
        >
      </div>
    {/if}
  </div>
{:else if state.kind === "failing"}
  {@const f = state.failure}
  <div class="state state--error" role="alert">
    <p class="state-title">Summaries are failing.</p>
    <p class="state-detail">
      {f.provider ?? "The AI provider"} turned away the last {f.failures === 1
        ? "try"
        : `${f.failures} tries`}. {f.reason} Recording isn't affected{state.coveredUntilMs
        ? `; everything since ${clockLabel(state.coveredUntilMs)} is on the Timeline`
        : ""}.
    </p>
    <div class="state-actions">
      <button type="button" class="btn btn--accent" onclick={() => void openSettings("intelligence")}
        >Open engine settings</button
      >
    </div>
  </div>
{:else}
  <p class="quiet-line">
    <span class="pulse" aria-hidden="true"></span>
    Catching up on older history — past days fill in as it goes.
  </p>
{/if}

<style>
  .state {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 18px;
    background: var(--app-surface);
    border: 1px solid var(--app-border);
    border-radius: 9px;
  }
  .state--error {
    border-color: var(--app-danger-border);
    background: var(--app-danger-bg);
  }
  .state--empty {
    border-style: dashed;
  }
  .state-title {
    margin: 0;
    font-size: var(--text-md);
    color: var(--app-text-strong);
  }
  .state-detail {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--app-text-muted);
    line-height: 1.6;
  }
  .state-actions {
    margin-top: 6px;
  }
  .btn {
    font: inherit;
    font-size: var(--text-sm);
    line-height: 1;
    display: inline-flex;
    align-items: center;
    padding: 0 12px;
    height: 28px;
    border-radius: 6px;
    cursor: pointer;
    border: 1px solid var(--app-accent-border);
    background: var(--app-accent-bg);
    color: var(--app-accent-strong);
  }
  .btn:hover {
    border-color: var(--app-accent);
    color: var(--app-accent);
  }
  .btn:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
  }
  .btn:disabled {
    opacity: 0.6;
    cursor: default;
  }
  .quiet-line {
    display: flex;
    align-items: center;
    gap: 9px;
    margin: 0;
    font-size: var(--text-sm);
    line-height: 1.55;
    color: var(--app-text-muted);
  }
  .pulse {
    flex: none;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--app-accent);
    animation: pulse 1.6s ease-in-out infinite;
  }
  @keyframes pulse {
    0%,
    100% {
      opacity: 0.35;
    }
    50% {
      opacity: 1;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .pulse {
      animation: none;
    }
  }
</style>
