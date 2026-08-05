<script lang="ts">
  // Title-bar recording chrome (frame 11): one calm state pill — dot +
  // elapsed + bytes-captured-today — replacing the round-3 button cluster.
  // All state comes from the shared capture-controls seam; this component
  // only renders it and opens the transport popover.
  import { invoke } from "@tauri-apps/api/core";
  import { tip } from "$lib/components/tooltip";
  import {
    captureControls,
    sourceSelection,
    startCapture,
    subscribeRuntimeSources,
  } from "$lib/capture-controls.svelte";
  import { captureSession } from "$lib/session.svelte";
  import RecordPillPopover from "./RecordPillPopover.svelte";
  import { formatElapsed, formatPillBytes, pillView } from "./record-pill";

  let open = $state(false);
  let nowMs = $state(Date.now());
  let bytesToday = $state<number | null>(null);
  let rootEl = $state<HTMLElement | null>(null);

  const running = $derived(captureControls.running);

  const view = $derived(
    pillView({
      running,
      starting: captureControls.loadingStart,
      stopping: captureControls.loadingStop,
      settingsLoading: captureControls.loadingSettings,
      userPaused: captureControls.isUserPaused,
      inactivityPaused: captureControls.isInactivityPaused,
      lowDisk: captureControls.isLowDiskSuspended,
      idleMs: captureControls.idleMs,
      sources: captureControls.runtimeSources,
      permissions: captureControls.permissions,
      selected: {
        screen: sourceSelection.screen,
        microphone: sourceSelection.microphone,
        systemAudio: sourceSelection.systemAudio,
      },
    }),
  );

  // Session start ≈ earliest source-session anchor (the backend re-anchors it
  // across segment rolls so it stays the session's start, not the segment's).
  const sessionStartMs = $derived.by(() => {
    const sessions = captureSession.value?.sourceSessions;
    if (!sessions) return null;
    const starts = [sessions.screen, sessions.microphone, sessions.systemAudio]
      .filter((s) => s !== null)
      .map((s) => s.startedAtUnixMs);
    return starts.length > 0 ? Math.min(...starts) : null;
  });
  const elapsed = $derived(
    sessionStartMs === null ? "" : formatElapsed(nowMs - sessionStartMs),
  );
  const bytesLabel = $derived(
    bytesToday === null ? "" : formatPillBytes(bytesToday),
  );

  // While recording: live per-source runtime status + a 1s clock tick.
  $effect(() => {
    if (!running) return;
    const release = subscribeRuntimeSources();
    nowMs = Date.now();
    const tick = setInterval(() => {
      nowMs = Date.now();
    }, 1_000);
    return () => {
      release();
      clearInterval(tick);
    };
  });

  // Bytes captured today — since the user's local midnight (the backend sums
  // capture media file sizes). Cheap; refresh every 30s while recording and
  // once whenever the popover opens.
  async function refreshBytes(): Promise<void> {
    const midnight = new Date();
    midnight.setHours(0, 0, 0, 0);
    try {
      bytesToday = await invoke<number>("get_bytes_captured_today", {
        sinceUnixMs: midnight.getTime(),
      });
    } catch {
      // Best-effort readout; keep the last value.
    }
  }
  $effect(() => {
    if (!running && !open) return;
    void refreshBytes();
    const poll = setInterval(() => void refreshBytes(), 30_000);
    return () => clearInterval(poll);
  });

  const stateWord = $derived(
    view.state === "recording"
      ? "Recording"
      : view.state === "permission-missing"
        ? "Not recording"
        : (view.word ?? "Recording"),
  );
  const headline = $derived(
    [stateWord, elapsed || null, bytesLabel ? `${bytesLabel} today` : null]
      .filter((part) => part !== null)
      .join(" · "),
  );
  const pillTitle = $derived(
    view.state === "recording" ? `Recording · ${elapsed}` : headline,
  );

  function onPillClick(): void {
    open = !open;
  }
  function onWindowPointerDown(event: PointerEvent): void {
    if (!open) return;
    if (rootEl?.contains(event.target as Node)) return;
    open = false;
  }
  function onWindowKeydown(event: KeyboardEvent): void {
    if (open && event.key === "Escape") {
      event.stopPropagation();
      open = false;
    }
  }
</script>

<svelte:window onpointerdown={onWindowPointerDown} onkeydown={onWindowKeydown} />

<div class="rp" bind:this={rootEl}>
  {#if view.kind === "button"}
    <button
      type="button"
      class="btn btn--sm btn--primary rp__record"
      disabled={captureControls.loadingSettings}
      aria-busy={captureControls.loadingSettings}
      use:tip={captureControls.loadingSettings
        ? "Preparing recording controls…"
        : "Start recording"}
      onclick={() => void startCapture()}
    >
      <span class="rp__recdot" aria-hidden="true"></span>
      Record
    </button>
  {:else}
    <button
      type="button"
      class="pill"
      class:pill--quiet={view.tone === "quiet"}
      class:pill--warn={view.tone === "warn"}
      aria-expanded={open}
      aria-haspopup="menu"
      use:tip={open ? null : pillTitle}
      aria-label={pillTitle}
      onclick={onPillClick}
    >
      {#if view.dot === "spinner"}
        <span class="rp__spin" aria-hidden="true"></span>
      {:else}
        <i
          class="rp__dot"
          class:rp__dot--idle={view.dot === "idle"}
          class:rp__dot--warn={view.dot === "warn"}
          aria-hidden="true"
        ></i>
      {/if}
      <!-- Frame 11 ordering: quiet pills lead with the word ("Paused 2:14:07"),
           liveness degradations lead with the timer ("2:14:07 screen asleep"). -->
      {#snippet timeSpan()}
        {#if view.showTime}
          <span class="pill__t is-num" class:rp__t--dim={view.tone === "quiet"}>{elapsed}</span>
        {/if}
      {/snippet}
      {#snippet wordSpan()}
        {#if view.word}
          <span
            class="pill__w"
            class:rp__w--info={view.wordTone === "info"}
            class:rp__w--warn={view.wordTone === "warn"}>{view.word}</span
          >
        {/if}
      {/snippet}
      {#if view.tone === "quiet"}
        {@render wordSpan()}{@render timeSpan()}
      {:else}
        {@render timeSpan()}{@render wordSpan()}
      {/if}
      {#if view.state === "recording"}
        <span class="pill__w rp__recword" aria-hidden="true">Rec</span>
      {/if}
      {#if view.showCost && bytesLabel}
        <span class="pill__gb is-num">{bytesLabel}</span>
      {/if}
    </button>
  {/if}
  {#if open}
    <RecordPillPopover {view} {headline} onclose={() => (open = false)} />
  {/if}
</div>

<style>
  .rp {
    display: inline-flex;
    align-items: center;
  }

  .rp__record {
    gap: var(--gap-inline);
  }
  .rp__recdot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: currentColor;
    flex: 0 0 auto;
  }

  .rp__dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--app-record);
    flex: 0 0 auto;
    animation: rp-pulse 2s var(--ease) infinite;
  }
  .rp__dot--idle {
    background: var(--app-text-subtle);
    animation: none;
  }
  .rp__dot--warn {
    background: var(--app-warn);
    animation: none;
  }
  @keyframes rp-pulse {
    0%,
    100% {
      opacity: 1;
    }
    50% {
      opacity: 0.45;
    }
  }

  .rp__spin {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    border: 1.5px solid var(--app-border-strong);
    border-top-color: var(--app-text-muted);
    animation: rp-spin 1s linear infinite;
    flex: 0 0 auto;
  }
  @keyframes rp-spin {
    to {
      transform: rotate(360deg);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .rp__dot,
    .rp__spin {
      animation: none;
    }
  }

  .rp__t--dim {
    color: var(--app-text-subtle);
  }
  .rp__w--info {
    color: var(--app-info);
  }
  .rp__w--warn {
    color: var(--app-warn);
  }

  /* Degradation ladder (frame 11) — by available title-bar width. The dot
     never goes; transport stays in the popover + tray. Container queries
     resolve against the CONTENT box, and `.titlebar` carries 86px of
     horizontal padding, so each threshold is its window width minus 86:
       0 window ≥900: dot + elapsed + cost
       1 window <900 (container 814): drop the cost readout
       2 window <680 (container 594): drop the timer — single "Rec" glyph
       3 window <560 (container 474): dot alone                            */
  .rp__recword {
    display: none;
  }
  @container titlebar (max-width: 814px) {
    .pill__gb {
      display: none;
    }
  }
  @container titlebar (max-width: 594px) {
    .pill__t {
      display: none;
    }
    .rp__recword {
      display: inline;
    }
  }
  @container titlebar (max-width: 474px) {
    .pill__t,
    .pill__w,
    .pill__gb,
    .rp__recword {
      display: none;
    }
  }
</style>
