<script lang="ts">
  // Hours captured today (the screen's ONE display-size number), the live
  // session elapsed, and which sources are actually running.
  //
  // Hours today = today's `covered_ms` from `list_day_coverage` — the same
  // aggregate the timeline jump menu reads (G6/G11: no second aggregation).
  // Elapsed is measured from the earliest live `sourceSessions` start.
  import Tile from "./Tile.svelte";
  import { formatElapsed, formatHoursColon } from "./format";
  import type { CaptureSession } from "$lib/types/session";

  interface Props {
    coveredTodayMs: number | null;
    coverageError: string | null;
    session: CaptureSession | null;
    nowMs: number;
    open: () => void;
  }

  let { coveredTodayMs, coverageError, session, nowMs, open }: Props = $props();

  const live = $derived(Boolean(session?.isRunning));
  const paused = $derived(
    Boolean(session?.isUserPaused || session?.isInactivityPaused || session?.isLowDiskSuspended),
  );

  const startedAtMs = $derived.by(() => {
    const s = session?.sourceSessions;
    if (!s) return null;
    const starts = [s.screen, s.microphone, s.systemAudio]
      .filter((m) => m !== null)
      .map((m) => m.startedAtUnixMs);
    return starts.length > 0 ? Math.min(...starts) : null;
  });

  const sources = $derived.by(() => {
    const requested = session?.requestedSources;
    const masked = session?.maskedSources;
    if (!requested) return [];
    return (
      [
        { key: "screen", label: "screen", on: requested.screen && !masked?.screen },
        { key: "mic", label: "microphone", on: requested.microphone && !masked?.microphone },
        { key: "sys", label: "system audio", on: requested.systemAudio && !masked?.systemAudio },
      ] as const
    ).filter((s) => s.on);
  });

  const sourceWord = $derived(
    sources.length === 3
      ? "all three sources"
      : sources.length === 0
        ? "no sources live"
        : sources.map((s) => s.label).join(" + "),
  );
</script>

<Tile id="capture" title="Capture" kbd="⌃R" {open} openLabel="Open capture settings">
  {#if coverageError}
    <p class="tile-empty t-meta">Coverage unavailable — {coverageError}</p>
  {:else}
    <div class="capture__hero">
      <span class="t-display is-num">{formatHoursColon(coveredTodayMs ?? 0)}</span>
      <span class="t-meta">hours today</span>
    </div>
  {/if}

  <div class="tile-row capture__state">
    {#if live && !paused}
      <i class="rdot" aria-hidden="true"></i><span class="t-ui">Recording</span>
    {:else if live}
      <i class="rdot rdot--paused" aria-hidden="true"></i><span class="t-ui">Paused</span>
    {:else}
      <i class="rdot rdot--off" aria-hidden="true"></i><span class="t-ui">Not recording</span>
    {/if}
    {#if live && startedAtMs !== null}
      <span class="t-meta is-mono is-num capture__elapsed">
        {formatElapsed(nowMs - startedAtMs)}
      </span>
    {/if}
  </div>

  {#if live}
    <div class="tile-row capture__sources">
      <span class="srcs" aria-hidden="true">
        {#each sources as source (source.key)}
          <svg class="src src--{source.key}" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round">
            {#if source.key === "screen"}
              <rect x="1.6" y="2.6" width="12.8" height="8.8" rx="1.6" /><path d="M6 13.6h4" />
            {:else if source.key === "mic"}
              <rect x="6" y="1.8" width="4" height="7.6" rx="2" /><path d="M3.6 7.6a4.4 4.4 0 0 0 8.8 0M8 12v2.2" />
            {:else}
              <path d="M8.6 2.6 5 5.6H2.4v4.8H5l3.6 3z" /><path d="M11.2 5.8a3.4 3.4 0 0 1 0 4.4" />
            {/if}
          </svg>
        {/each}
      </span>
      <span class="t-meta">{sourceWord}</span>
    </div>
  {/if}
</Tile>
