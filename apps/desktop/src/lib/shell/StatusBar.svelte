<script lang="ts">
  // The 30px status bar (SHELL.md › Status bar): recording state, its controls,
  // the three source lanes (toggles while stopped, display while recording), at
  // most one flag, the engine pill only when unhealthy, today's tracked time,
  // "local only" and the shortcuts ?. What to show is `status-bar.ts`; the
  // actions are the existing capture-controls seam, moved out of the titlebar.
  import { invoke } from "@tauri-apps/api/core";
  import { tip } from "$lib/components/tooltip";
  import {
    captureControls,
    pauseCapture,
    resumeCapture,
    resyncCaptureSession,
    sourceSelection,
    startCapture,
    stopCapture,
    subscribeRuntimeSources,
    toggleSourceSelected,
  } from "$lib/capture-controls.svelte";
  import { captureSession } from "$lib/session.svelte";
  import { appNotifications } from "$lib/notifications.svelte";
  import { licenseStatus } from "$lib/licensing-store.svelte";
  import { renderIdle } from "$lib/render-idle.svelte";
  import { openSettings } from "$lib/surface-windows";
  import { getEffectiveGlobalShortcut, type GlobalShortcutId } from "$lib/global-shortcuts";
  import { formatShortcut, type KeyboardPlatform } from "$lib/keyboard";
  import type { AiRuntimeStatus } from "$lib/types/recording";
  import ShortcutsPop from "./ShortcutsPop.svelte";
  import {
    LANES,
    engineFlag,
    formatClock,
    formatTracked,
    isLocalOnly,
    liveLanes,
    recView,
    screenPermissionMissing,
    statusFlag,
    stoppedReasonFrom,
    type LaneKey,
    type StatusInput,
  } from "./status-bar";

  interface Props {
    platform: KeyboardPlatform;
    devEnabled: boolean;
    canToggleRecording: boolean;
    canToggleSources: boolean;
  }
  let { platform, devEnabled, canToggleRecording, canToggleSources }: Props = $props();

  const keyOf = (id: GlobalShortcutId) => {
    const binding = getEffectiveGlobalShortcut(id).bindings[0];
    return binding ? formatShortcut(binding, platform).join("") : "";
  };

  // ── Session clocks ────────────────────────────────────────────────────
  const sessionStartMs = $derived.by<number | null>(() => {
    const s = captureSession.value?.sourceSessions;
    const starts = [s?.screen, s?.microphone, s?.systemAudio].flatMap((m) => (m ? [m.startedAtUnixMs] : []));
    return starts.length ? Math.min(...starts) : null;
  });
  // Kept after the session ends: a "Recording stopped — …" notification older
  // than the last session we saw is history, not the current state.
  let lastSessionStartMs = $state<number | null>(null);
  $effect(() => {
    if (sessionStartMs !== null) lastSessionStartMs = sessionStartMs;
  });
  // The session carries no pause instant, so "PAUSED 00:12" counts from when
  // this window saw the pause begin (no clock when it was already paused).
  let pausedSinceMs = $state<number | null>(null);
  let sawUnpaused = false;
  $effect(() => {
    if (!captureControls.isUserPaused) {
      sawUnpaused = true;
      pausedSinceMs = null;
    } else if (pausedSinceMs === null && sawUnpaused) {
      pausedSinceMs = Date.now();
    }
  });

  const readOnly = $derived(licenseStatus.value?.kind === "readOnly" || licenseStatus.value?.kind === "revoked");
  const input = $derived<StatusInput>({
    running: captureControls.running,
    loadingStart: captureControls.loadingStart,
    userPaused: captureControls.isUserPaused,
    inactivityPaused: captureControls.isInactivityPaused,
    lowDisk: captureControls.isLowDiskSuspended,
    runtime: captureControls.runtimeSources,
    selected: {
      screen: sourceSelection.screen,
      microphone: sourceSelection.microphone,
      systemAudio: sourceSelection.systemAudio,
    },
    permissions: captureControls.permissions,
    startError: captureControls.startError,
    readOnly,
    stoppedReason: stoppedReasonFrom(appNotifications.items, lastSessionStartMs),
  });
  const rec = $derived(recView(input));
  const lanes = $derived(liveLanes(input, rec));
  const flag = $derived(statusFlag(input));
  const screenBlocked = $derived(screenPermissionMissing(input));

  let now = $state(Date.now());
  $effect(() => {
    if (!rec.clock) return;
    now = Date.now();
    const handle = setInterval(() => {
      if (!renderIdle()) now = Date.now();
    }, 1000);
    return () => clearInterval(handle);
  });
  const clockText = $derived(
    rec.clock === "elapsed" && sessionStartMs !== null
      ? formatClock(now - sessionStartMs, true)
      : rec.clock === "paused" && pausedSinceMs !== null
        ? formatClock(now - pausedSinceMs, false)
        : null,
  );

  // Live per-source status while recording (2 s poll in the shared seam).
  $effect(() => {
    if (!captureControls.running) return;
    return subscribeRuntimeSources();
  });

  // ── Engine pill + today's tracked time + permissions, refreshed on focus ─
  let aiStatus = $state<AiRuntimeStatus | null>(null);
  let trackedMs = $state<number | null>(null);
  const engine = $derived(engineFlag(aiStatus, captureControls.recordingSettings?.aiRuntime ?? null));
  const localOnly = $derived(isLocalOnly(captureControls.recordingSettings));

  async function refresh(): Promise<void> {
    const start = new Date();
    start.setHours(0, 0, 0, 0);
    const [status, usage] = await Promise.all([
      invoke<AiRuntimeStatus>("get_ai_runtime_status").catch(() => null),
      invoke<{ timePerApp: { activeMs: number }[] }>("get_usage_charts", {
        startMs: start.getTime(),
        endMs: Date.now(),
      }).catch(() => null),
    ]);
    aiStatus = status;
    trackedMs = usage ? usage.timePerApp.reduce((sum, a) => sum + a.activeMs, 0) : null;
  }
  $effect(() => {
    void refresh();
    const onFocus = () => {
      void refresh();
      void resyncCaptureSession();
    };
    // ponytail: 5-min tick for "today"; a usage-changed event is the upgrade path.
    const handle = setInterval(() => !renderIdle() && void refresh(), 300_000);
    window.addEventListener("focus", onFocus);
    return () => {
      clearInterval(handle);
      window.removeEventListener("focus", onFocus);
    };
  });

  // ── Controls (moved from the titlebar) ────────────────────────────────
  // Pause is a whole-session control and stays enabled through an inactivity
  // pause (it locks the session). Resume shows for a user pause or a low-disk
  // suspension; the latter is the one case it's disabled — it resumes itself.
  const showResume = $derived(captureControls.isUserPaused || captureControls.isLowDiskSuspended);
  const pauseDisabled = $derived(
    captureControls.loadingPause || (captureControls.isLowDiskSuspended && !captureControls.isUserPaused),
  );
  const pauseTitle = $derived(
    captureControls.isUserPaused
      ? "Resume recording"
      : captureControls.isLowDiskSuspended
        ? "Resumes on its own once space frees up"
        : captureControls.isInactivityPaused
          ? "Pause the whole session"
          : "Pause recording",
  );

  let restarting = $state(false);
  async function restartForPrivacy(): Promise<void> {
    if (captureControls.loadingStart || captureControls.loadingStop || restarting || !captureControls.running) return;
    restarting = true;
    try {
      // stop/start report failures through the shared capture-error dialog.
      await stopCapture();
      if (!captureControls.isRunning) await startCapture();
    } finally {
      restarting = false;
    }
  }

  let openingPrivacy = $state(false);
  async function openScreenPrivacy(): Promise<void> {
    openingPrivacy = true;
    try {
      // macOS won't re-prompt once denied: today's path deep-links the pane.
      await invoke("open_capture_privacy_settings", { kind: "screen" });
    } catch {
      // Best-effort; the flag stays until the permission changes.
    } finally {
      openingPrivacy = false;
    }
  }

  function runFlag(): void {
    switch (flag?.kind) {
      case "privacy-restart":
        void restartForPrivacy();
        break;
      case "start-failed":
        void startCapture();
        break;
      case "low-disk":
      case "stopped":
        void openSettings("storage");
        break;
      case "read-only":
        void openSettings("license");
        break;
      case "permission":
        void openScreenPrivacy();
        break;
    }
  }

  const LIVE_TIP: Record<string, string> = {
    on: "recording",
    paused: "paused",
    starting: "starting…",
    off: "off",
    error: "stopped — privacy filter",
  };
  const NAMES: Record<LaneKey, string> = { screen: "Screen", microphone: "Microphone", systemAudio: "System audio" };
  const SHORTCUT: Record<LaneKey, GlobalShortcutId> = {
    screen: "toggleSourceScreen",
    microphone: "toggleSourceMicrophone",
    systemAudio: "toggleSourceSystemAudio",
  };
  const selectTip = (key: LaneKey) =>
    `${NAMES[key]}: ${
      sourceSelection.isSelected(key) ? "on — click to skip on next recording" : "off — click to include in next recording"
    } (${keyOf(SHORTCUT[key])})`;

  const startingOrRunning = $derived(captureControls.running || captureControls.loadingStart);
</script>

<footer class="mx-statusbar">
  <span class="mx-rec{rec.mod ? ` mx-rec--${rec.mod}` : ''}" aria-live="polite">
    {#if rec.mod === "starting"}<span class="mx-spin mx-spin--sm"></span>{:else}<span class="mx-rec__dot"></span>{/if}
    {rec.label}{#if clockText} <span class="num">{clockText}</span>{/if}
  </span>

  {#if captureControls.running}
    <button
      type="button"
      class="mx-btn mx-btn--ghost mx-btn--icon"
      style="--_h:22px"
      aria-label={pauseTitle}
      aria-busy={captureControls.loadingPause}
      disabled={pauseDisabled}
      use:tip={pauseTitle}
      onclick={showResume ? resumeCapture : pauseCapture}
    >
      {#if showResume}
        <svg width="11" height="11" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="M7 5.5v13a1 1 0 0 0 1.5.9l10.4-6.5a1 1 0 0 0 0-1.8L8.5 4.6A1 1 0 0 0 7 5.5z" /></svg>
      {:else}
        <svg width="11" height="11" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><rect x="6" y="5" width="4" height="14" rx="1" /><rect x="14" y="5" width="4" height="14" rx="1" /></svg>
      {/if}
    </button>
  {/if}
  {#if startingOrRunning}
    <button
      type="button"
      class="mx-btn mx-btn--ghost mx-btn--icon"
      style="--_h:22px"
      aria-label="Stop recording"
      aria-busy={captureControls.loadingStop}
      disabled={!captureControls.running || captureControls.loadingStop}
      use:tip={`Stop recording  ${keyOf("toggleRecording")}`}
      onclick={stopCapture}
    >
      <svg width="11" height="11" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><rect x="6" y="6" width="12" height="12" rx="2" /></svg>
    </button>
  {:else}
    {@const recordOff = captureControls.loadingSettings || readOnly}
    <button
      type="button"
      class="mx-btn mx-btn--sm"
      style="--_h:22px"
      aria-disabled={recordOff}
      aria-busy={captureControls.loadingSettings}
      use:tip={captureControls.loadingSettings ? "Preparing recording controls…" : `Start recording  ${keyOf("toggleRecording")}`}
      onclick={() => !recordOff && void startCapture()}
    >
      <svg width="10" height="10" viewBox="0 0 24 24" fill="currentColor" style:color={recordOff ? undefined : "var(--app-danger)"} aria-hidden="true"><circle cx="12" cy="12" r="6" /></svg>Record
    </button>
  {/if}
  <span class="mx-sep"></span>

  {#if startingOrRunning}
    {#each lanes as lane (lane.key)}
      {@const meta = LANES.find((l) => l.key === lane.key)!}
      <span
        class="mx-lane mx-lane--{meta.mod} mx-lane--{lane.mod}"
        role="status"
        aria-label={`${NAMES[lane.key]}: ${LIVE_TIP[lane.mod]}`}
        use:tip={`${NAMES[lane.key]}: ${LIVE_TIP[lane.mod]}`}
      >{meta.label}{#if lane.small} <small>{lane.small}</small>{/if}</span>
    {/each}
  {:else}
    {#each LANES as meta (meta.key)}
      {@const blocked = meta.key === "screen" && screenBlocked}
      <button
        type="button"
        class="mx-lane mx-lane--{meta.mod}"
        class:mx-lane--blocked={blocked}
        aria-pressed={sourceSelection.isSelected(meta.key)}
        disabled={sourceSelection.isSaving(meta.key) || captureControls.loadingSettings || readOnly}
        use:tip={blocked ? "Screen Recording is off for Mnema in System Settings" : selectTip(meta.key)}
        onclick={() => toggleSourceSelected(meta.key)}
      >{meta.label}{#if blocked} <small>no permission</small>{/if}</button>
    {/each}
  {/if}

  <span class="mx-spacer"></span>

  {#if flag}
    {#if flag.kind === "privacy-retry"}
      <span class="mx-flag" data-tone={flag.tone} role="status" use:tip={flag.tip}>{flag.text} <b>{flag.action}</b></span>
    {:else}
      <button
        type="button"
        class="mx-flag"
        data-tone={flag.tone}
        aria-busy={(flag.kind === "privacy-restart" && restarting) || (flag.kind === "permission" && openingPrivacy)}
        disabled={flag.kind === "privacy-restart" && (restarting || captureControls.loadingStop)}
        use:tip={flag.tip}
        onclick={runFlag}
      >{#if flag.text}{flag.text} {/if}<b>{flag.kind === "privacy-restart" && restarting ? "Restarting…" : flag.action}</b></button>
    {/if}
  {:else if rec.mod === "paused" && !rec.clock}
    <span>resumes when you’re back</span>
  {/if}

  {#if engine}
    <button
      type="button"
      class="mx-engine"
      class:mx-engine--error={engine.error}
      use:tip={engine.tip}
      onclick={() => void openSettings("intelligence")}
    >engine <b>{engine.label}</b></button>
    <span class="mx-sep"></span>
  {/if}

  {#if trackedMs !== null}
    <span>today <span class="num">{formatTracked(trackedMs)}</span></span>
  {/if}
  {#if localOnly}
    {#if trackedMs !== null}<span class="mx-sep"></span>{/if}
    <span>local only</span>
  {/if}
  <ShortcutsPop
    {platform}
    {devEnabled}
    isCapturing={captureControls.running}
    isUserPaused={captureControls.isUserPaused}
    {canToggleRecording}
    {canToggleSources}
  />
</footer>
