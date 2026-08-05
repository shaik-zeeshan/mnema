<script lang="ts">
  // The drawer's identity strip: source, index, clock, duration, model label —
  // then the live status pill and the four chrome actions (rerun, timestamps,
  // expand/collapse, close). The status pill is a `role="status"` live region
  // because every frame-6 state changes without the user doing anything.
  import { tip } from "$lib/components/tooltip";
  import type { AudioSegmentRecord } from "./audio-drawer-view";

  export type StatusTone = "ok" | "work" | "warn" | "bad" | "idle";

  interface Props {
    segment: AudioSegmentRecord;
    sourceLabel: string;
    timeRangeLabel: string;
    timeRangeTip: string;
    durationLabel: string;
    modelLabel: string | null;
    status: { tone: StatusTone; label: string; busy: boolean };
    actionLabel: string;
    actionDisabled: boolean;
    actionTitle: string;
    rerunLoading: boolean;
    onRerun: () => void;
    onClose: () => void;
    showTimestamps?: boolean;
    expanded?: boolean;
    closeEl?: HTMLButtonElement | null;
  }

  let {
    segment,
    sourceLabel,
    timeRangeLabel,
    timeRangeTip,
    durationLabel,
    modelLabel,
    status,
    actionLabel,
    actionDisabled,
    actionTitle,
    rerunLoading,
    onRerun,
    onClose,
    showTimestamps = $bindable(false),
    expanded = $bindable(false),
    closeEl = $bindable(null),
  }: Props = $props();
</script>

<header class="rhead">
  <span class="tag tag--{segment.source}">
    <span class="tag__swatch" aria-hidden="true"></span>{sourceLabel}
  </span>
  <span class="tnum rhead__index">#{segment.segmentIndex}</span>
  <span class="rhead__sep" aria-hidden="true">·</span>
  <span class="tnum" use:tip={timeRangeTip}>{timeRangeLabel}</span>
  <span class="rhead__sep" aria-hidden="true">·</span>
  <span class="tnum">{durationLabel}</span>
  {#if modelLabel}
    <span class="rhead__sep" aria-hidden="true">·</span>
    <span class="rhead__model" use:tip={modelLabel}>{modelLabel}</span>
  {/if}
  <span class="rhead__file" use:tip={segment.filePath}>{segment.fileName}</span>
  <span class="rhead__grow"></span>
  <span
    class="statuspill statuspill--{status.tone}"
    role="status"
    aria-live="polite"
    aria-busy={status.busy}
  >
    {#if status.busy}
      <span class="spinner" aria-hidden="true"></span>
    {:else}
      <span class="statuspill__dot" aria-hidden="true"></span>
    {/if}
    {status.label}
  </span>
  <button
    type="button"
    class="ghost"
    onclick={onRerun}
    disabled={actionDisabled}
    use:tip={actionTitle}
  >
    {rerunLoading ? "starting…" : actionLabel}
  </button>
  <button
    type="button"
    class="ghost"
    aria-pressed={showTimestamps}
    onclick={() => (showTimestamps = !showTimestamps)}
  >
    timestamps
  </button>
  <button type="button" class="ghost" aria-pressed={expanded} onclick={() => (expanded = !expanded)}>
    {expanded ? "collapse" : "expand"}
  </button>
  <button
    type="button"
    class="rhead__close"
    bind:this={closeEl}
    onclick={onClose}
    aria-label="Close audio player"
  >
    <svg
      width="11"
      height="11"
      viewBox="0 0 14 14"
      fill="none"
      stroke="currentColor"
      stroke-width="1.4"
      stroke-linecap="round"
      aria-hidden="true"
    >
      <path d="M3.5 3.5l7 7M10.5 3.5l-7 7" />
    </svg>
  </button>
</header>

<style>
  .rhead {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    padding: 8px 12px;
    border-bottom: 1px solid var(--app-border);
    background: var(--app-surface-subtle, var(--app-surface));
    font-size: 11px;
    color: var(--app-text-muted);
  }

  .tnum {
    font-variant-numeric: tabular-nums;
  }

  .tag {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 2px 8px;
    border: 1px solid var(--app-neutral-border, var(--app-border-strong));
    border-radius: 999px;
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.08em;
  }

  .tag__swatch {
    width: 7px;
    height: 7px;
    border-radius: 999px;
    background: var(--app-text-subtle);
  }

  .tag--microphone .tag__swatch {
    background: var(--app-source-mic);
  }

  .tag--systemAudio .tag__swatch {
    background: var(--app-source-sysaudio);
  }

  .rhead__index {
    color: var(--app-text-strong);
    font-weight: 700;
  }

  .rhead__sep {
    color: var(--app-text-subtle);
  }

  .rhead__model {
    font-size: 10px;
    letter-spacing: 0.04em;
    color: var(--app-text-subtle);
  }

  .rhead__file {
    max-width: 14ch;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: var(--app-font-mono);
    font-size: 10px;
    color: var(--app-text-subtle);
  }

  .rhead__grow {
    flex: 1 1 auto;
  }

  .statuspill {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 2px 8px;
    border: 1px solid var(--app-neutral-border, var(--app-border-strong));
    border-radius: 999px;
    background: var(--app-neutral-bg, transparent);
    color: var(--app-text-muted);
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.08em;
  }

  .statuspill__dot {
    width: 5px;
    height: 5px;
    flex: none;
    border-radius: 999px;
    background: currentColor;
  }

  .statuspill--ok {
    border-color: var(--app-accent-border);
    background: var(--app-accent-bg);
    color: var(--app-accent);
  }

  .statuspill--work {
    border-color: var(--app-info-border, var(--app-border-strong));
    background: color-mix(in srgb, var(--app-info) 10%, transparent);
    color: var(--app-info);
  }

  .statuspill--warn {
    border-color: var(--app-warn-border);
    background: color-mix(in srgb, var(--app-warn) 10%, transparent);
    color: var(--app-warn);
  }

  .statuspill--bad {
    border-color: var(--app-danger-border);
    background: color-mix(in srgb, var(--app-danger) 10%, transparent);
    color: var(--app-danger-text, var(--app-danger));
  }

  .ghost {
    padding: 3px 9px;
    border: 1px solid transparent;
    border-radius: 5px;
    background: transparent;
    color: var(--app-text-muted);
    font: inherit;
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    cursor: pointer;
  }

  .ghost:hover:not(:disabled),
  .ghost:focus-visible:not(:disabled) {
    background: var(--app-surface-hover);
    border-color: var(--app-border-strong);
    color: var(--app-text-strong);
    outline: none;
  }

  .ghost:disabled {
    opacity: var(--app-disabled-opacity);
    cursor: not-allowed;
  }

  .ghost[aria-pressed="true"] {
    color: var(--app-accent);
    border-color: var(--app-accent-border);
    background: var(--app-accent-bg);
  }

  .rhead__close {
    width: 24px;
    height: 24px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border: 1px solid var(--app-border-strong);
    border-radius: 4px;
    background: transparent;
    color: var(--app-text-muted);
    cursor: pointer;
  }

  .rhead__close:hover,
  .rhead__close:focus-visible {
    color: var(--app-danger);
    border-color: var(--app-danger-strong);
    background: color-mix(in srgb, var(--app-danger-strong) 8%, transparent);
    outline: none;
  }

  .rhead__close:focus-visible {
    box-shadow: var(--app-ring);
  }

  .spinner {
    width: 10px;
    height: 10px;
    flex: none;
    display: inline-block;
    border-radius: 999px;
    border: 1.5px solid color-mix(in srgb, var(--app-text-muted) 30%, transparent);
    border-top-color: currentColor;
    animation: drawer-head-spin 700ms linear infinite;
    vertical-align: -1px;
  }

  @keyframes drawer-head-spin {
    to {
      transform: rotate(1turn);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .spinner {
      animation: none;
    }
  }
</style>
