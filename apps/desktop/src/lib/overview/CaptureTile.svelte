<script lang="ts">
  // Capture (1×1) — the screen's ONE tinted tile (`.tile--live`) and its ONE
  // `--t-display` hero. Both privileges are conditional: the tint is spent only
  // while capture is actually recording, and the hero only when the day holds
  // at least a minute of capture (G8 — no zero standing in for an unknown).
  //
  // Data is all state the shell already holds, so this tile costs no read:
  //   · hours today   — `list_day_coverage`, via the Overview's one load
  //   · state         — `captureSession` + `captureControls` (shell-bootstrapped)
  //   · elapsed       — `sourceSessions.*.startedAtUnixMs`, a real session clock
  // The state ladder is the shared `resolveRecordingPillState`, imported
  // read-only so this tile can never disagree with the titlebar's pill.
  import { captureSession } from "$lib/session.svelte";
  import { captureControls } from "$lib/capture-controls.svelte";
  import { resolveRecordingPillState } from "$lib/components/recording-pill-state";
  import { elapsedClock, heroHours } from "./overview-format";
  import Glyph from "./Glyph.svelte";

  let { coveredMs }: { coveredMs: number | null } = $props();

  const session = $derived(captureSession.value);
  const sources = $derived(captureControls.runtimeSources);

  const pillState = $derived(
    resolveRecordingPillState({
      running: captureControls.running,
      loadingStart: captureControls.loadingStart,
      loadingStop: captureControls.loadingStop,
      loadingSettings: captureControls.loadingSettings,
      userPaused: captureControls.isUserPaused,
      inactivityPaused: Boolean(session?.isInactivityPaused),
      lowDiskSuspended: Boolean(session?.isLowDiskSuspended),
      screenReason: sources?.screen?.reason ?? null,
      hasBlockedSource: false,
      hasLostSource: false,
    }),
  );

  const STATE_LABEL: Record<string, string> = {
    idle: "Not recording",
    starting: "Starting…",
    stopping: "Stopping…",
    recording: "Recording",
    "paused-manual": "Paused",
    "paused-inactive": "Paused — idle",
    "low-disk": "Paused — low disk",
    "screen-asleep": "Screen asleep",
    degraded: "Recording — degraded",
    permission: "Permission needed",
  };

  // Earliest live source session start: the session's real age, not a guess.
  const startedAtMs = $derived.by(() => {
    const s = session?.sourceSessions;
    const starts = [s?.screen, s?.microphone, s?.systemAudio]
      .filter((m) => m != null)
      .map((m) => m.startedAtUnixMs)
      .filter((ms) => Number.isFinite(ms) && ms > 0);
    return starts.length ? Math.min(...starts) : null;
  });

  // Tick only while there is a running clock to tick.
  let nowMs = $state(Date.now());
  $effect(() => {
    if (startedAtMs === null || !captureControls.running) return;
    const id = setInterval(() => (nowMs = Date.now()), 1000);
    return () => clearInterval(id);
  });

  const requested = $derived(session?.requestedSources ?? null);
  const masked = $derived(session?.maskedSources ?? null);
  function live(key: "screen" | "microphone" | "systemAudio"): boolean {
    return Boolean(requested?.[key]) && !masked?.[key];
  }
  const liveNames = $derived(
    [
      live("screen") ? "screen" : null,
      live("microphone") ? "mic" : null,
      live("systemAudio") ? "system" : null,
    ].filter((n) => n !== null),
  );

  const hero = $derived(heroHours(coveredMs));
</script>

<div class="tile tile--static" class:tile--live={pillState === "recording"}>
  <div class="tile__h">
    <span class="t-label">Capture</span>
    {#if liveNames.length === 3}<span class="tile__more">all three sources</span>{/if}
  </div>

  {#if hero}
    <div class="hero">
      <span class="t-display is-num">{hero}</span><span class="t-meta">hours today</span>
    </div>
  {:else}
    <div class="hero"><span class="t-meta">Nothing captured yet today</span></div>
  {/if}

  <div class="trow state">
    <i class="dot" class:dot--live={pillState === "recording"} class:dot--off={pillState === "idle"}></i>
    <span class="t-ui">{STATE_LABEL[pillState] ?? "Not recording"}</span>
    {#if startedAtMs !== null && captureControls.running}
      <span class="t-meta is-mono is-num elapsed">{elapsedClock(startedAtMs, nowMs)}</span>
    {/if}
  </div>

  <div class="trow">
    <span class="glyphs">
      <span class="srcg" class:srcg--screen={live("screen")} class:srcg--off={!live("screen")}>
        <Glyph name="screen" />
      </span>
      <span class="srcg" class:srcg--mic={live("microphone")} class:srcg--off={!live("microphone")}>
        <Glyph name="mic" />
      </span>
      <span class="srcg" class:srcg--sys={live("systemAudio")} class:srcg--off={!live("systemAudio")}>
        <Glyph name="sys" />
      </span>
    </span>
    <span class="t-meta">{liveNames.length ? liveNames.join(" · ") : "no sources on"}</span>
  </div>
</div>

<style>
  .hero {
    display: flex;
    align-items: baseline;
    gap: var(--gap-inline);
  }
  .state {
    margin-top: var(--s-4);
  }
  .elapsed {
    margin-left: auto;
  }
  .glyphs {
    display: inline-flex;
    gap: var(--s-4);
  }
  /* Recording red is a STATE, never an error — the same rule the titlebar pill
     follows. Idle is a hollow ring so the tile never reads "live" when it isn't. */
  .dot {
    flex: 0 0 auto;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--app-text-subtle);
  }
  .dot--live {
    background: var(--app-danger);
  }
  .dot--off {
    background: transparent;
    box-shadow: inset 0 0 0 1.5px var(--app-text-faint);
  }
</style>
