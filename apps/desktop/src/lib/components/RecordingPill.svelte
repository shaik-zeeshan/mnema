<!--
  Recording state pill + transport popover — the title bar's whole recording
  chrome (round-4 redesign slice 5). Replaces the old button cluster (status
  chip + Record/Pause/Stop buttons + three source pills + privacy warning chip)
  with ONE capsule that opens a popover.

  Anatomy is `system.css` §6 `.pill` (dot + elapsed + cost), degrading on the R3
  ladder: cost → timer → single word → dot alone. The dot never goes. Recording
  red is a STATE, never an error — transient liveness tints via `--app-warn` /
  `--app-info`, never `--app-danger`.

  Two-click in-window stop is the accepted design (docs/redesign/DECISIONS.md):
  panic-stop lives on the global shortcut and the tray, both of which keep full
  transport. Idle keeps its ONE-click Record button (the mockups draw idle as a
  button, not a pill) with a chevron beside it as the popover's idle door.

  Popover labels are byte-identical to the tray menu (`status_bar.rs`) — the
  decision record requires one wording everywhere. Per-source toggles work mid
  session: turning one off is a user-scoped mask on that source (paused-flag
  seam), which no liveness recovery ever undoes. A source the session didn't
  start with can't be added, and the last live source can't be turned off.
-->
<script lang="ts">
  import { tip } from "$lib/components/tooltip";
  import {
    captureControls,
    pauseCapture,
    resumeCapture,
    sourceSelection,
    startCapture,
    stopCapture,
    toggleSourceSelected,
    type SourceKey,
  } from "$lib/capture-controls.svelte";
  import { getEffectiveGlobalShortcut, type GlobalShortcutId } from "$lib/global-shortcuts";
  import { formatShortcut, type KeyboardPlatform } from "$lib/keyboard";
  import { fpsToIntervalS } from "$lib/components/capture-rate";
  import { estimateDailyStorageMb } from "$lib/onboarding/disk-estimate";
  import { isPrivacySuspension, resolveRecordingPillState } from "./recording-pill-state";

  interface Props {
    platform: KeyboardPlatform;
  }
  let { platform }: Props = $props();

  // Labels are the tray's, verbatim (`status_bar.rs` SOURCE_*_ID items) — the
  // decision record requires one wording across tray, shortcuts and popover.
  const SOURCES: { key: SourceKey; label: string }[] = [
    { key: "screen", label: "Screen" },
    { key: "microphone", label: "Microphone" },
    { key: "systemAudio", label: "System Audio" },
  ];

  // A requested source only reads as "lost" once the session has had time to
  // come up; every writer is legitimately inactive for the first seconds.
  const SOURCE_SETTLE_MS = 15_000;

  let open = $state(false);
  let anchorEl = $state<HTMLElement | null>(null);
  let popoverEl = $state<HTMLElement | null>(null);
  let restarting = $state(false);

  // One clock for the elapsed readout and the settle grace period. Ticks only
  // while a session is running.
  let now = $state(Date.now());
  $effect(() => {
    if (!captureControls.isRunning) return;
    now = Date.now();
    const handle = setInterval(() => { now = Date.now(); }, 1000);
    return () => clearInterval(handle);
  });

  const startedAtMs = $derived.by<number | null>(() => {
    const sessions = captureControls.sourceSessions;
    if (!sessions) return null;
    const stamps = [sessions.screen, sessions.microphone, sessions.systemAudio]
      .filter((meta) => meta !== null)
      .map((meta) => meta.startedAtUnixMs);
    return stamps.length > 0 ? Math.min(...stamps) : null;
  });

  const elapsedMs = $derived(startedAtMs === null ? 0 : Math.max(0, now - startedAtMs));

  function formatElapsed(ms: number): string {
    const total = Math.floor(ms / 1000);
    const hours = Math.floor(total / 3600);
    const minutes = Math.floor((total % 3600) / 60);
    const seconds = total % 60;
    const pad = (n: number) => String(n).padStart(2, "0");
    return hours > 0 ? `${hours}:${pad(minutes)}:${pad(seconds)}` : `${minutes}:${pad(seconds)}`;
  }

  // Cost readout. The one honest number available in-app today is the DAILY
  // RATE: the measured 270 MB/day anchor scaled by the user's real capture
  // interval (the same function onboarding's consequence lines use). A
  // session-total "MB written" needs real disk facts, which the system-facts
  // slice owns — inventing one here would be exactly the fabricated
  // denominator the decision record forbids.
  const dailyMb = $derived(
    estimateDailyStorageMb(fpsToIntervalS(captureControls.recordingSettings?.screenFrameRate ?? 0)),
  );
  const costLabel = $derived(
    dailyMb >= 1000 ? `${(dailyMb / 1000).toFixed(1)} GB/day` : `${Math.round(dailyMb)} MB/day`,
  );

  const screenReason = $derived(captureControls.runtimeSources?.screen.reason ?? null);
  const privacySuspended = $derived(isPrivacySuspension(screenReason));

  // Only hard denials count. System audio's `possibly_blocked` is an inference,
  // not an answer (ADR 0052) — surfacing it as "not allowed" would be a guess.
  const blockedSource = $derived.by<{ key: SourceKey; label: string } | null>(() => {
    const permissions = captureControls.permissions;
    if (!permissions) return null;
    return (
      SOURCES.find((source) => {
        if (!sourceSelection.isSelected(source.key)) return false;
        const status = permissions[source.key];
        return status === "denied" || status === "restricted";
      }) ?? null
    );
  });

  const lostSource = $derived.by<{ key: SourceKey; label: string } | null>(() => {
    const runtime = captureControls.runtimeSources;
    if (!runtime || elapsedMs < SOURCE_SETTLE_MS) return null;
    return (
      SOURCES.find((source) => {
        const status = runtime[source.key];
        return status.requested && !status.paused && status.sessionActive === false;
      }) ?? null
    );
  });

  const pillState = $derived(
    resolveRecordingPillState({
      running: captureControls.isRunning,
      loadingStart: captureControls.loadingStart,
      loadingStop: captureControls.loadingStop,
      loadingSettings: captureControls.loadingSettings,
      userPaused: captureControls.isUserPaused,
      inactivityPaused: captureControls.isInactivityPaused,
      lowDiskSuspended: captureControls.isLowDiskSuspended,
      screenReason,
      hasBlockedSource: blockedSource !== null,
      hasLostSource: lostSource !== null,
    }),
  );

  // The one word the pill shows beside (or instead of) the timer.
  const word = $derived.by<string | null>(() => {
    switch (pillState) {
      case "starting":
        return "Starting";
      case "stopping":
        return "Stopping";
      case "paused-manual":
        return "Paused";
      case "paused-inactive": {
        const minutes = Math.floor(captureControls.idleMs / 60_000);
        return minutes >= 1 ? `Idle ${minutes}m` : "Idle";
      }
      case "low-disk":
        return "Low disk";
      case "screen-asleep":
        return "screen asleep";
      case "degraded":
        return privacySuspended
          ? "screen stopped"
          : `${lostSource?.label.toLowerCase() ?? "source"} lost`;
      case "permission":
        return `${blockedSource?.label.toLowerCase() ?? "source"} not allowed`;
      // The resting state shows numbers, not a word — except on the ladder's
      // narrow rung, where "Rec" replaces the dropped timer.
      case "recording":
        return "Rec";
      default:
        return null;
    }
  });

  const wordIsRungOnly = $derived(pillState === "recording");
  const showsElapsed = $derived(
    pillState === "recording" ||
      pillState === "paused-manual" ||
      pillState === "screen-asleep" ||
      pillState === "degraded",
  );
  const showsCost = $derived(pillState === "recording");
  const tone = $derived.by<"record" | "quiet" | "warn">(() => {
    switch (pillState) {
      case "starting":
      case "stopping":
      case "paused-manual":
      case "paused-inactive":
        return "quiet";
      case "low-disk":
      case "permission":
        return "warn";
      default:
        return "record";
    }
  });

  // The sentence the tooltip and the popover header both use.
  const detail = $derived.by(() => {
    switch (pillState) {
      case "idle":
        return "Not recording";
      case "starting":
        return "Starting recording…";
      case "stopping":
        return "Stopping recording…";
      case "recording":
        return `Recording · ${formatElapsed(elapsedMs)} · ${costLabel}`;
      case "paused-manual":
        return `Paused · ${formatElapsed(elapsedMs)} elapsed`;
      case "paused-inactive":
        return "Paused while you're away — recording resumes on your next input";
      case "low-disk":
        return "Paused — the recordings volume is low on space. Recording resumes on its own once space frees up.";
      case "screen-asleep":
        return "The display is asleep or locked, so screen capture is waiting for it. Microphone and system audio keep recording.";
      case "degraded":
        if (privacySuspended) {
          return screenReason === "privacy_recovery_restart_required"
            ? "Screen capture stopped: the privacy filter could not be applied. Stop and start recording to resume it."
            : "Screen capture stopped: the privacy filter could not be applied. Mnema is retrying.";
        }
        return `${lostSource?.label ?? "A source"} stopped delivering. Mnema is rebuilding it.`;
      case "permission":
        return `${blockedSource?.label ?? "A source"} is not allowed to record. Grant the permission in System Settings, or turn the source off below.`;
      default:
        return "Recording status";
    }
  });

  function shortcutFor(id: GlobalShortcutId): string {
    const binding = getEffectiveGlobalShortcut(id).bindings[0];
    return binding ? formatShortcut(binding, platform).join("") : "";
  }

  function sourceShortcutId(key: SourceKey): GlobalShortcutId {
    if (key === "screen") return "toggleSourceScreen";
    if (key === "microphone") return "toggleSourceMicrophone";
    return "toggleSourceSystemAudio";
  }

  // A recording needs at least one live source, so the last one standing can't
  // be switched off — stopping is the way out, and the transport above owns it.
  const liveCount = $derived(
    SOURCES.filter((source) => sourceSelection.isSelected(source.key)).length,
  );
  const hasOutOfSessionSource = $derived(
    captureControls.isRunning &&
      SOURCES.some((source) => !sourceSelection.isInSession(source.key)),
  );

  function sourceTip(source: { key: SourceKey; label: string }): string {
    if (captureControls.isRunning && !sourceSelection.isInSession(source.key)) {
      return `${source.label} — not part of this recording`;
    }
    if (sourceSelection.isSelected(source.key) && liveCount <= 1) {
      return `${source.label} — the last source can't be turned off`;
    }
    const shortcut = shortcutFor(sourceShortcutId(source.key));
    return shortcut ? `${source.label} (${shortcut})` : source.label;
  }

  // Transport labels are the tray's, byte-for-byte (`status_bar.rs`).
  const primaryLabel = $derived(
    captureControls.loadingStop
      ? "Stopping..."
      : captureControls.isRunning
        ? "Stop Recording"
        : captureControls.loadingStart
          ? "Starting..."
          : "Start Recording",
  );
  const pauseLabel = $derived(
    captureControls.isUserPaused ? "Resume Recording" : "Pause Recording",
  );
  // Pause is a whole-session control and stays available through an inactivity
  // auto-pause (which is per-source); only a low-disk hold, which the user
  // cannot clear from here, disables it.
  const pauseDisabled = $derived(
    captureControls.loadingPause ||
      (captureControls.isLowDiskSuspended && !captureControls.isUserPaused),
  );

  // `.titlebar { overflow: hidden }` (the tiling-WM spill backstop) clips
  // absolutely-positioned descendants, so the popover is FIXED and anchored
  // from the trigger's measured rect — same escape hatch the notification
  // popover uses, but the pill's x drifts with its neighbours so it is
  // measured rather than hard-coded.
  let anchorLeft = $state(8);
  let anchorTop = $state(44);
  $effect(() => {
    if (!open || !anchorEl) return;
    const rect = anchorEl.getBoundingClientRect();
    anchorLeft = Math.round(rect.left);
    anchorTop = Math.round(rect.bottom + 6);
  });

  function toggleOpen(): void {
    open = !open;
  }

  function onWindowPointerDown(event: PointerEvent): void {
    if (!open) return;
    const target = event.target as Node | null;
    if (!target) return;
    if (popoverEl?.contains(target) || anchorEl?.contains(target)) return;
    open = false;
  }

  function onWindowKeydown(event: KeyboardEvent): void {
    if (open && event.key === "Escape") {
      open = false;
      event.stopPropagation();
    }
  }

  async function runPrimary(): Promise<void> {
    open = false;
    if (captureControls.isRunning) {
      await stopCapture();
      return;
    }
    await startCapture();
  }

  async function runPause(): Promise<void> {
    if (captureControls.isUserPaused) {
      await resumeCapture();
    } else {
      await pauseCapture();
    }
  }

  // Privacy-filter recovery: a stop/start round trip. Both legs funnel their
  // failures into the shared capture-error dialog, so a failed restart is
  // visible rather than silent.
  async function restartForPrivacyRecovery(): Promise<void> {
    if (restarting || !captureControls.isRunning) return;
    restarting = true;
    try {
      await stopCapture();
      if (!captureControls.isRunning) await startCapture();
    } finally {
      restarting = false;
    }
  }
</script>

<svelte:window onpointerdown={onWindowPointerDown} onkeydown={onWindowKeydown} />

<div class="rec" bind:this={anchorEl}>
  {#if pillState === "idle"}
    <!-- Idle keeps a real one-click Record button (the mockups draw idle as a
         button, not a pill); the chevron is the popover's idle door. -->
    <button
      type="button"
      class="btn btn--sm rec__record"
      onclick={() => void startCapture()}
      disabled={captureControls.loadingSettings}
      use:tip={open
        ? ""
        : shortcutFor("toggleRecording")
          ? `Start recording (${shortcutFor("toggleRecording")})`
          : "Start recording"}
      aria-label="Start recording"
    >
      <span class="pill__dot pill__dot--current" aria-hidden="true"></span>
      <span class="rec__record-label">Record</span>
    </button>
    <button
      type="button"
      class="btn btn--ghost btn--sm btn--icon rec__more"
      class:rec__more--open={open}
      aria-label="Recording options"
      aria-haspopup="dialog"
      aria-expanded={open}
      aria-controls="recording-popover"
      onclick={toggleOpen}
      use:tip={open ? "" : "Recording options"}
    >
      <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <path d="m6 9 6 6 6-6" />
      </svg>
    </button>
  {:else}
    <button
      type="button"
      class="pill"
      class:pill--quiet={tone === "quiet"}
      class:pill--warn={tone === "warn"}
      class:pill--open={open}
      aria-haspopup="dialog"
      aria-expanded={open}
      aria-controls="recording-popover"
      aria-label={detail}
      aria-live="polite"
      onclick={toggleOpen}
      use:tip={open ? "" : detail}
    >
      {#if pillState === "starting" || pillState === "stopping"}
        <span class="pill__spin" aria-hidden="true"></span>
      {:else}
        <span
          class="pill__dot"
          class:pill__dot--live={tone === "record"}
          class:pill__dot--off={tone === "quiet"}
          class:pill__dot--warn={tone === "warn"}
          aria-hidden="true"
        ></span>
      {/if}
      {#if showsElapsed}
        <span class="pill__t is-num" class:pill__t--dim={pillState === "paused-manual"}
          >{formatElapsed(elapsedMs)}</span
        >
      {/if}
      {#if word}
        <span
          class="pill__w"
          class:pill__w--rung={wordIsRungOnly}
          class:pill__w--info={pillState === "screen-asleep"}
          class:pill__w--warn={pillState === "degraded"}
        >{word}</span>
      {/if}
      {#if showsCost}
        <span class="pill__gb is-num">{costLabel}</span>
      {/if}
    </button>
  {/if}
</div>

{#if open}
  <div
    id="recording-popover"
    class="recpop"
    role="dialog"
    aria-label="Recording"
    style="left: {anchorLeft}px; top: {anchorTop}px;"
    bind:this={popoverEl}
  >
    <p class="recpop__head">{detail}</p>

    {#if privacySuspended && screenReason === "privacy_recovery_restart_required"}
      <button
        type="button"
        class="btn btn--sm recpop__restart"
        aria-busy={restarting}
        disabled={restarting || captureControls.loadingStart || captureControls.loadingStop}
        onclick={() => void restartForPrivacyRecovery()}
      >{restarting ? "Restarting…" : "Restart Recording"}</button>
    {/if}

    {#if captureControls.isRunning}
      <button
        type="button"
        class="recpop__item"
        disabled={pauseDisabled}
        aria-busy={captureControls.loadingPause}
        onclick={() => void runPause()}
      >
        <svg class="recpop__glyph" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true">
          {#if captureControls.isUserPaused}
            <path d="M7 4.5 19 12 7 19.5Z" fill="currentColor" stroke-linejoin="round" />
          {:else}
            <path d="M9 5v14" /><path d="M15 5v14" />
          {/if}
        </svg>
        <span>{pauseLabel}</span>
        {#if shortcutFor("pauseResumeRecording")}
          <kbd class="kbd recpop__kbd">{shortcutFor("pauseResumeRecording")}</kbd>
        {/if}
      </button>
    {/if}
    <button
      type="button"
      class="recpop__item"
      disabled={captureControls.loadingStart ||
        captureControls.loadingStop ||
        captureControls.loadingSettings}
      aria-busy={captureControls.loadingStart || captureControls.loadingStop}
      onclick={() => void runPrimary()}
    >
      <svg class="recpop__glyph" width="14" height="14" viewBox="0 0 24 24" aria-hidden="true">
        {#if captureControls.isRunning}
          <rect x="6" y="6" width="12" height="12" rx="2" fill="currentColor" />
        {:else}
          <circle cx="12" cy="12" r="6" fill="currentColor" />
        {/if}
      </svg>
      <span>{primaryLabel}</span>
      {#if shortcutFor("toggleRecording")}
        <kbd class="kbd recpop__kbd">{shortcutFor("toggleRecording")}</kbd>
      {/if}
    </button>

    <div class="recpop__sep" role="presentation"></div>
    <p class="recpop__label">Sources</p>
    {#each SOURCES as source (source.key)}
      {@const on = sourceSelection.isSelected(source.key)}
      {@const outOfSession =
        captureControls.isRunning && !sourceSelection.isInSession(source.key)}
      {@const lastOne = on && liveCount <= 1}
      <button
        type="button"
        class="recpop__item"
        role="menuitemcheckbox"
        aria-checked={on}
        disabled={outOfSession ||
          lastOne ||
          captureControls.loadingSettings ||
          sourceSelection.isSaving(source.key)}
        use:tip={sourceTip(source)}
        onclick={() => void toggleSourceSelected(source.key)}
      >
        <span class="recpop__check" aria-hidden="true">
          {#if on}
            <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
              <path d="M2 6.4 4.6 9 10 3" />
            </svg>
          {/if}
        </span>
        <span>{source.label}</span>
      </button>
    {/each}
    {#if captureControls.isRunning && hasOutOfSessionSource}
      <p class="recpop__note">
        Sources this recording didn't start with can't be added — stop and start again to change them.
      </p>
    {/if}
  </div>
{/if}

<style>
  .rec {
    position: relative;
    display: inline-flex;
    align-items: center;
    gap: var(--s-4);
    flex: 0 0 auto;
  }

  /* The record control IS the recording, so it wears record red rather than the
     accent — `system.css` reserves --app-record for exactly this. */
  .rec__record {
    background: var(--app-record-start-bg);
    color: var(--app-record-start-fg);
    /* Borderless like every other button here: the record tint's edge is the
       button's own rim, not a drawn border. */
    box-shadow: 0 1px 1px rgba(0, 0, 0, 0.1),
      inset 0 0 0 var(--hairline) var(--app-record-start-border);
  }
  .rec__record:hover:not(:disabled) {
    background: var(--app-record-start-bg-hover);
    color: var(--app-record-start-fg-hover);
    box-shadow: 0 1px 1px rgba(0, 0, 0, 0.1),
      inset 0 0 0 var(--hairline) var(--app-record-start-border-hover);
  }

  .rec__more {
    color: var(--app-text-muted);
  }
  .rec__more--open {
    background: var(--glass-tint);
    color: var(--app-text-strong);
  }

  /* ── Popover ────────────────────────────────────────────────────────── */
  .recpop {
    /* Fixed, not absolute: `.titlebar { overflow: hidden }` clips absolutely
       positioned descendants out of existence. Anchored from the trigger's
       measured rect. */
    position: fixed;
    z-index: 200;
    min-width: 252px;
    /* NSMenu anatomy on level-5 material (07): 5px padding, 8px radius, 24px
       items, a checkmark gutter, a glyph gutter, and a right-aligned mono key
       hint. Everything in here is a menu label, never prose — so material is
       where it belongs. */
    padding: 5px;
    border: 0;
    border-radius: var(--r-lg);
    background: var(--glass-pop);
    -webkit-backdrop-filter: var(--glass-blur);
    backdrop-filter: var(--glass-blur);
    box-shadow: var(--sh-float), inset 0 0 0 var(--hairline) var(--glass-line);
    display: flex;
    flex-direction: column;
    gap: 0;
  }

  .recpop__head {
    margin: 0;
    padding: var(--s-4) var(--s-8) var(--s-6);
    max-width: 34ch;
    font: var(--w-regular) var(--t-meta) / var(--lh-meta) var(--app-font-sans);
    color: var(--app-text-muted);
  }

  .recpop__label {
    margin: var(--s-4) 0 2px;
    padding: 0 var(--s-8);
    font: var(--w-medium) var(--t-label) / 1 var(--app-font-mono);
    letter-spacing: var(--ls-label);
    text-transform: uppercase;
    color: var(--app-text-faint);
  }

  .recpop__note {
    margin: var(--s-6) 0 0;
    padding: 0 var(--s-8);
    max-width: 34ch;
    font: var(--w-regular) var(--t-meta) / var(--lh-meta) var(--app-font-sans);
    color: var(--app-text-faint);
  }

  .recpop__sep {
    height: 1px;
    margin: 5px var(--s-8);
    background: var(--glass-line);
  }

  .recpop__item {
    display: flex;
    align-items: center;
    gap: var(--s-8);
    width: 100%;
    height: 24px;
    padding: 0 var(--s-8);
    border: 0;
    border-radius: var(--r-sm);
    background: transparent;
    color: var(--app-text-strong);
    font: var(--w-regular) var(--t-ui) / 1 var(--app-font-sans);
    letter-spacing: var(--ls-ui);
    text-align: left;
  }
  /* NSMenu highlights the row under the pointer with a full accent fill; the
     checkmark gutter, not the fill, is what says "on". */
  .recpop__item:hover:not(:disabled),
  .recpop__item:focus-visible {
    background: var(--app-accent);
    color: var(--app-accent-contrast);
  }
  .recpop__item:hover:not(:disabled) .recpop__glyph,
  .recpop__item:hover:not(:disabled) .recpop__check,
  .recpop__item:focus-visible .recpop__glyph,
  .recpop__item:focus-visible .recpop__check {
    color: inherit;
  }
  .recpop__item:focus-visible {
    outline: none;
  }
  .recpop__item:disabled {
    opacity: var(--opacity-disabled);
  }

  /* Both gutters are the same width so a transport row's label and a source
     row's label land on the same x, even though each row carries only one. */
  .recpop__glyph {
    flex: 0 0 auto;
    width: 14px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    color: var(--app-text-muted);
  }
  .recpop__check {
    flex: 0 0 auto;
    width: 14px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    color: var(--app-accent);
  }

  /* In an NSMenu the key equivalent is plain right-aligned mono type, not a
     keycap — the keycap is for teaching a shortcut, this is for recalling one. */
  .recpop__kbd {
    margin-left: auto;
    min-width: 0;
    height: auto;
    padding: 0;
    background: none;
    box-shadow: none;
    font: var(--w-regular) var(--t-meta) / 1 var(--app-font-mono);
    color: var(--app-text-subtle);
  }
  .recpop__item:hover:not(:disabled) .recpop__kbd,
  .recpop__item:focus-visible .recpop__kbd {
    color: inherit;
    opacity: 0.8;
  }

  .recpop__restart {
    margin: 0 var(--s-4) var(--s-4);
    justify-content: center;
  }
</style>
