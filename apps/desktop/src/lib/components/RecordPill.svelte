<script lang="ts">
  // The record pill — the titlebar door for the capture model's "on / off the
  // record" state (Warm Paper Slice 7; mockup story-first-v5.html frames 1–3).
  // Three states, one pill: breathing dot while on the record, amber dashed
  // dot + live countdown for a timed off-the-record window, muted ring for an
  // indefinite one. Clicking opens the go-off-the-record menu (timed options
  // auto-resume) or, when off, "Back on the record".
  import { tip } from "$lib/components/tooltip";
  import {
    captureControls,
    pauseCapture,
    resumeCapture,
    startCapture,
  } from "$lib/capture-controls.svelte";
  import { captureSession } from "$lib/session.svelte";

  const offRecord = $derived(captureControls.offTheRecord);
  const deadline = $derived(captureControls.offRecordDeadlineUnixMs);
  const pillState = $derived(offRecord ? (deadline !== null ? "timed" : "off") : "on");
  const busy = $derived(
    captureControls.loadingPause || captureControls.loadingStart || captureControls.loadingStop,
  );

  // Live countdown: tick only while a timed window is armed.
  let nowMs = $state(Date.now());
  $effect(() => {
    if (deadline === null) return;
    const timer = setInterval(() => {
      nowMs = Date.now();
    }, 1000);
    return () => clearInterval(timer);
  });

  function clock(ms: number): string {
    return new Date(ms).toLocaleTimeString(undefined, {
      hour: "numeric",
      minute: "2-digit",
      hour12: true,
    });
  }

  function countdown(msLeft: number): string {
    const totalMin = Math.max(1, Math.ceil(msLeft / 60_000));
    if (totalMin < 60) return `${totalMin}m`;
    const h = Math.floor(totalMin / 60);
    const m = totalMin % 60;
    return m === 0 ? `${h}h` : `${h}h ${m}m`;
  }

  // "since 8:47 AM" — earliest source session start of the live session.
  const sinceMs = $derived.by<number | null>(() => {
    const sessions = captureSession.value?.sourceSessions;
    if (!sessions) return null;
    const starts = [sessions.screen, sessions.microphone, sessions.systemAudio]
      .filter((s) => s !== null)
      .map((s) => s.startedAtUnixMs);
    return starts.length > 0 ? Math.min(...starts) : null;
  });

  const label = $derived.by<string>(() => {
    if (pillState === "on") {
      return sinceMs !== null ? `On the record · since ${clock(sinceMs)}` : "On the record";
    }
    if (pillState === "timed" && deadline !== null) {
      return `Off the record · ${countdown(deadline - nowMs)} left`;
    }
    return "Off the record";
  });

  const tooltip = $derived.by<string>(() => {
    if (pillState === "on") {
      // Automatic suspensions stay "on the record" but the tooltip is honest.
      if (captureControls.isLowDiskSuspended) {
        return "On the record — capture is waiting on free disk space";
      }
      if (captureControls.isInactivityPaused) {
        return "On the record — idle, capture resumes with activity";
      }
      return "The record — go off the record";
    }
    if (pillState === "timed" && deadline !== null) {
      return `Off the record — back on at ${clock(deadline)}`;
    }
    return "Off the record — until you turn it back on";
  });

  // ── Menu ──────────────────────────────────────────────────────────────
  let open = $state(false);
  let rootEl = $state<HTMLDivElement | null>(null);
  let menuLeft = $state(78);

  function toggle(): void {
    if (!open) {
      menuLeft = rootEl?.getBoundingClientRect().left ?? 78;
    }
    open = !open;
  }

  // Dismiss on outside pointerdown / Escape. Capture-phase window listeners:
  // WKWebView doesn't focus buttons on click, so element-level key handling
  // never sees the keys.
  $effect(() => {
    if (!open) return;
    const onPointerDown = (event: PointerEvent) => {
      const target = event.target as Node | null;
      if (target && rootEl?.contains(target)) return;
      open = false;
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      event.stopPropagation();
      open = false;
    };
    window.addEventListener("pointerdown", onPointerDown, { capture: true });
    window.addEventListener("keydown", onKeyDown, { capture: true });
    return () => {
      window.removeEventListener("pointerdown", onPointerDown, { capture: true });
      window.removeEventListener("keydown", onKeyDown, { capture: true });
    };
  });

  const OFF_OPTIONS = [
    { minutes: 15, label: "For 15 minutes" },
    { minutes: 30, label: "For 30 minutes" },
    { minutes: 60, label: "For 1 hour" },
  ] as const;

  async function goOff(minutes: number | null): Promise<void> {
    open = false;
    if (minutes === null) {
      await pauseCapture();
    } else {
      await pauseCapture(Date.now() + minutes * 60_000);
    }
  }

  async function backOn(): Promise<void> {
    open = false;
    // A live user-paused session resumes; a stopped session (or a startup
    // hold carrying a deadline without a session) starts fresh — the start
    // clears the armed deadline backend-side.
    if (captureControls.isRunning) {
      await resumeCapture();
    } else {
      await startCapture();
    }
  }
</script>

<div class="record" bind:this={rootEl}>
  <button
    type="button"
    class="pill pill--{pillState}"
    aria-haspopup="menu"
    aria-expanded={open}
    aria-live="polite"
    use:tip={open ? null : tooltip}
    onclick={toggle}
  >
    <span class="dot" aria-hidden="true"></span>
    <span class="label">{label}</span>
  </button>

  {#if open}
    <div class="menu" role="menu" aria-label="The record" style="left: {menuLeft}px">
      {#if !offRecord}
        <div class="menu-head" aria-hidden="true">Go off the record</div>
      {/if}
      {#if offRecord}
        <button type="button" role="menuitem" disabled={busy} onclick={() => void backOn()}>
          Back on the record
        </button>
      {:else}
        {#each OFF_OPTIONS as option (option.minutes)}
          <button type="button" role="menuitem" disabled={busy} onclick={() => void goOff(option.minutes)}>
            {option.label}
            <span class="when">{clock(nowMs + option.minutes * 60_000)}</span>
          </button>
        {/each}
        <button type="button" role="menuitem" disabled={busy} onclick={() => void goOff(null)}>
          Until I turn it back on
        </button>
      {/if}
    </div>
  {/if}
</div>

<style>
  .record {
    display: flex;
    align-items: center;
    min-width: 0;
  }

  .pill {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    min-width: 0;
    padding: 3px 11px 3px 9px;
    border-radius: 999px;
    border: 1px solid;
    background: transparent;
    font-family: var(--app-font-mono);
    font-size: var(--text-xs);
    letter-spacing: 0.03em;
    white-space: nowrap;
    cursor: pointer;
    font-variant-numeric: tabular-nums;
  }
  .pill:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
  }
  .pill .label {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .pill .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    flex: 0 0 auto;
  }

  /* On the record — rec tint, breathing dot. */
  .pill--on {
    color: var(--app-status-running-fg);
    border-color: var(--app-status-running-border);
    background: var(--app-status-bg);
  }
  .pill--on .dot {
    background: var(--app-status-running-dot);
    box-shadow: 0 0 0 3px var(--app-status-running-dot-glow);
    animation: record-breathe 2.4s ease-in-out infinite;
  }
  @keyframes record-breathe {
    0%,
    100% {
      opacity: 1;
      box-shadow: 0 0 0 3px var(--app-status-running-dot-glow);
    }
    50% {
      opacity: 0.55;
      box-shadow: 0 0 0 5px transparent;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .pill--on .dot {
      animation: none;
    }
  }

  /* Timed off-the-record — amber, dashed hollow dot, counting down. */
  .pill--timed {
    color: var(--app-warn);
    border-color: var(--app-warn-border);
    background: var(--app-warn-bg);
  }
  .pill--timed .dot {
    background: transparent;
    border: 1.5px dashed var(--app-warn);
  }

  /* Indefinite off-the-record — muted hollow dot. */
  .pill--off {
    color: var(--app-text-subtle);
    border-color: var(--app-border);
    background: var(--app-surface-raised);
  }
  .pill--off .dot {
    background: transparent;
    border: 1.5px solid var(--app-text-subtle);
  }

  /* Fixed, not absolute: `.titlebar { overflow: hidden }` clips absolutely
     positioned descendants (same escape as .notification-popover). */
  .menu {
    position: fixed;
    top: calc(var(--app-titlebar-height) + 6px);
    z-index: 200;
    width: 218px;
    padding: 5px;
    background: var(--app-surface-raised);
    border: 1px solid var(--app-border-strong);
    border-radius: 11px;
    box-shadow: var(--app-shadow-popover);
  }
  .menu-head {
    padding: 6px 10px 4px;
    font-family: var(--app-font-mono);
    font-size: var(--text-xs);
    letter-spacing: 0.09em;
    text-transform: uppercase;
    color: var(--app-text-faint);
  }
  .menu button {
    display: flex;
    align-items: baseline;
    gap: 8px;
    width: 100%;
    border: 0;
    background: transparent;
    border-radius: 7px;
    padding: 7px 10px;
    cursor: pointer;
    text-align: left;
    font: inherit;
    font-size: var(--text-sm);
    color: var(--app-text-strong);
    white-space: nowrap;
  }
  .menu button:hover {
    background: var(--app-surface-hover);
  }
  .menu button:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
  }
  .menu button:disabled {
    opacity: var(--app-disabled-opacity);
    cursor: not-allowed;
  }
  .menu .when {
    margin-left: auto;
    font-family: var(--app-font-mono);
    font-size: var(--text-xs);
    color: var(--app-text-faint);
  }
</style>
