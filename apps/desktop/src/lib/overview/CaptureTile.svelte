<script lang="ts">
  // The grid's one `--t-display`: hours captured on the day being shown.
  //
  // The number comes from `list_day_coverage` — the same cached GROUP BY the
  // jump menu and This Week read (G6/G11), never a second aggregation. A day
  // absent from that response genuinely holds no capture, so the hero renders
  // nothing at all rather than "0:00" (**G8**).
  import IconScreen from "~icons/lucide/monitor";
  import IconMic from "~icons/lucide/mic";
  import IconSys from "~icons/lucide/volume-2";
  import { captureControls, sourceSelection } from "$lib/capture-controls.svelte";
  import type { DayCoverage } from "$lib/types/app-infra";
  import type { LoadState } from "./overview-data.svelte";
  import { formatHeroHours } from "./overview-format";
  import TileShell from "./TileShell.svelte";

  interface Props {
    coverage: LoadState<DayCoverage[]>;
    dayKey: string;
    /** True when `dayKey` is today — only then is the live session about it. */
    isToday: boolean;
    /** The 800px floor: the hero and the session line only. */
    compact?: boolean;
  }

  let { coverage, dayKey, isToday, compact = false }: Props = $props();

  const coveredMs = $derived(
    coverage.status === "ok"
      ? (coverage.value.find((d) => d.day === dayKey)?.coveredMs ?? 0)
      : null,
  );
  const hero = $derived(formatHeroHours(coveredMs));

  // One clock, ticking only while a session runs AND the document is being
  // shown — the same shape the title bar's recording pill uses, visibility gate
  // included. That gate is the load-bearing half: a 1 Hz DOM write that starts
  // on the Record click and never stops is a permanent repaint driver, and a
  // repaint the compositor never shows is what strands WebKit's non-purgeable
  // backing stores. A recorder runs for hours with its window hidden.
  let now = $state(Date.now());
  $effect(() => {
    if (!captureControls.isRunning) return;
    let handle: ReturnType<typeof setInterval> | null = null;
    const stop = () => {
      if (handle !== null) clearInterval(handle);
      handle = null;
    };
    const sync = () => {
      stop();
      if (document.visibilityState !== "visible") return;
      now = Date.now();
      handle = setInterval(() => {
        now = Date.now();
      }, 1000);
    };
    sync();
    document.addEventListener("visibilitychange", sync);
    return () => {
      stop();
      document.removeEventListener("visibilitychange", sync);
    };
  });

  const startedAtMs = $derived.by<number | null>(() => {
    const sessions = captureControls.sourceSessions;
    if (!sessions) return null;
    const stamps = [sessions.screen, sessions.microphone, sessions.systemAudio]
      .filter((meta) => meta !== null)
      .map((meta) => meta.startedAtUnixMs);
    return stamps.length > 0 ? Math.min(...stamps) : null;
  });

  const elapsed = $derived.by<string | null>(() => {
    if (startedAtMs === null) return null;
    const total = Math.floor(Math.max(0, now - startedAtMs) / 1000);
    const pad = (n: number) => String(n).padStart(2, "0");
    return `${Math.floor(total / 3600)}:${pad(Math.floor((total % 3600) / 60))}:${pad(total % 60)}`;
  });

  const sources = $derived(
    [
      { key: "screen", label: "Screen", icon: IconScreen, on: sourceSelection.screen },
      { key: "microphone", label: "Microphone", icon: IconMic, on: sourceSelection.microphone },
      { key: "systemAudio", label: "System audio", icon: IconSys, on: sourceSelection.systemAudio },
    ].filter((s) => s.on),
  );

  const sourcePhrase = $derived(
    sources.length === 3
      ? "all three sources"
      : sources.length === 0
        ? "no sources selected"
        : sources.map((s) => s.label.toLowerCase()).join(" + "),
  );

  const captureState = $derived(
    captureControls.isLowDiskSuspended
      ? { label: "Low disk", tone: "warn" }
      : captureControls.paused
        ? { label: "Paused", tone: "warn" }
        : captureControls.isCapturing
          ? { label: "Recording", tone: "live" }
          : { label: "Not recording", tone: "off" },
  );

  const quiet = $derived(
    coverage.status === "failed" ? "Couldn't read capture coverage." : null,
  );
</script>

<TileShell label="Capture" {quiet}>
  {#if hero}
    <div class="hero">
      <span class="t-display is-num">{hero}</span>
      <span class="t-meta">hours {isToday ? "today" : "captured"}</span>
    </div>
  {:else if coverage.status === "ok"}
    <p class="none">Nothing captured on this day.</p>
  {/if}

  {#if isToday}
    <div class="ss-trow" style="margin-top:auto">
      <span class="dot dot--{captureState.tone}" aria-hidden="true"></span>
      <span class="t-ui">{captureState.label}</span>
      {#if elapsed}<span class="t-meta is-mono is-num ss-r">{elapsed}</span>{/if}
    </div>
  {/if}

  {#if !compact}
  <div class="ss-trow">
    {#each sources as s (s.key)}
      <span class="ic ic--{s.key}" aria-hidden="true"><s.icon /></span>
    {/each}
    <span class="t-meta">{sourcePhrase}</span>
  </div>
  {/if}
</TileShell>

<style>
  .hero {
    display: flex;
    align-items: baseline;
    gap: var(--gap-inline);
    flex: 0 0 auto;
  }

  /* The ramp's `--lh-display` is 1.2; the hero is a single line with its own
     row, so the leading is space this tile does not have at the 800px floor. */
  .hero :global(.t-display) {
    line-height: 1;
  }

  .none {
    margin: 0;
    font: var(--w-regular) var(--t-meta) / 1.45 var(--app-font-sans);
    color: var(--app-text-subtle);
  }

  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--app-text-faint);
    flex: 0 0 auto;
  }
  .dot--live {
    background: var(--app-record);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--app-record) 22%, transparent);
  }
  .dot--warn {
    background: var(--app-warn);
  }

  .ic {
    display: flex;
    flex: 0 0 auto;
    font-size: 13px;
  }
  /* The app's own source palette (`--app-source-*`), not the mockup's shorter
     `--app-src-*` names — those tokens exist only inside the mockup file. */
  .ic--screen {
    color: var(--app-source-screen);
  }
  .ic--microphone {
    color: var(--app-source-mic);
  }
  .ic--systemAudio {
    color: var(--app-source-sysaudio);
  }
</style>
