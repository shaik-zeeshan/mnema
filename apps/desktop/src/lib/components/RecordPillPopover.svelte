<script lang="ts">
  // The record pill's transport popover (frame 11) — a DOM menu, semantically
  // identical to the tray's transport in status_bar.rs: Stop/Pause first,
  // per-source rows (idle: next-session settings; recording: the mid-session
  // per-source mask, slice 5), a fix row first when the state carries one.
  import { invoke } from "@tauri-apps/api/core";
  import { tip } from "$lib/components/tooltip";
  import {
    captureControls,
    pauseCapture,
    resumeCapture,
    startCapture,
    stopCapture,
    sourceSelection,
    toggleSourceSelected,
    type SourceKey,
  } from "$lib/capture-controls.svelte";
  import {
    getEffectiveGlobalShortcut,
    type GlobalShortcutId,
  } from "$lib/global-shortcuts";
  import { detectKeyboardPlatform, formatShortcut } from "$lib/keyboard";
  import type { PillView } from "./record-pill";

  interface Props {
    view: PillView;
    headline: string;
    onclose: () => void;
  }
  let { view, headline, onclose }: Props = $props();

  const platform = detectKeyboardPlatform();
  function kbd(id: GlobalShortcutId): string {
    const binding = getEffectiveGlobalShortcut(id).bindings[0];
    return binding ? formatShortcut(binding, platform).join("") : "";
  }

  const running = $derived(captureControls.running);
  // Same rules as the replaced cluster: Resume shows for a manual pause or a
  // low-disk hold; the low-disk hold alone can't be cleared from here.
  const showResume = $derived(
    captureControls.isUserPaused || captureControls.isLowDiskSuspended,
  );
  const pauseDisabled = $derived(
    captureControls.loadingPause ||
      (captureControls.isLowDiskSuspended && !captureControls.isUserPaused),
  );
  const stopDisabled = $derived(
    captureControls.loadingStop || captureControls.loadingStart,
  );

  let restarting = $state(false);
  async function restartCapture(): Promise<void> {
    if (restarting || !running) return;
    restarting = true;
    try {
      await stopCapture();
      if (!captureControls.isRunning) await startCapture();
    } finally {
      restarting = false;
      onclose();
    }
  }

  function openPrivacySettings(): void {
    const kind = view.word?.startsWith("mic") ? "microphone" : "screen";
    void invoke("open_capture_privacy_settings", { kind });
    onclose();
  }

  async function run(action: () => Promise<void>): Promise<void> {
    await action();
    onclose();
  }

  const lanes: { key: SourceKey; label: string }[] = [
    { key: "screen", label: "Screen" },
    { key: "microphone", label: "Microphone" },
    { key: "systemAudio", label: "System Audio" },
  ];
  const selectedCount = $derived(
    lanes.filter((lane) => sourceSelection.isSelected(lane.key)).length,
  );
  // Tray parity (`source_item_enabled`) — one behavior everywhere. While idle
  // the rows drive next-session settings; while recording they drive the
  // mid-session per-source mask (slice 5). In both modes the last checked
  // source can't be unchecked (a session needs ≥1 source), and a source that
  // isn't part of the live session can't join it mid-flight.
  function laneDisabled(key: SourceKey): boolean {
    if (sourceSelection.isSaving(key) || captureControls.loadingSettings) return true;
    if (running && !sourceSelection.isRequested(key)) return true;
    return sourceSelection.isSelected(key) && selectedCount === 1;
  }
  function laneTip(key: SourceKey): string {
    if (running) {
      if (!sourceSelection.isRequested(key)) {
        return "Not part of this session — starts with the next recording";
      }
      if (sourceSelection.isSelected(key) && selectedCount === 1) {
        return "At least one source must stay live";
      }
      return sourceSelection.isSelected(key)
        ? "Click to stop this source for the rest of the session"
        : "Click to resume this source";
    }
    if (sourceSelection.isSelected(key) && selectedCount === 1) {
      return "At least one source must stay enabled";
    }
    return sourceSelection.isSelected(key)
      ? "Click to skip on the next recording"
      : "Click to include in the next recording";
  }
</script>

<div class="menu" role="menu" aria-label="Recording">
  <div class="menu__hd is-mono">{headline}</div>

  {#if view.state === "permission-missing"}
    <button type="button" class="menu__i" role="menuitem" onclick={openPrivacySettings}>
      <span class="menu__ck" aria-hidden="true"></span>
      Open System Settings…
    </button>
  {:else if view.state === "source-degraded" && view.word === "restart screen"}
    <button
      type="button"
      class="menu__i"
      role="menuitem"
      disabled={restarting}
      onclick={() => void restartCapture()}
    >
      <span class="menu__ck" aria-hidden="true"></span>
      {restarting ? "Restarting…" : "Restart Recording"}
    </button>
  {/if}

  {#if running}
    <button
      type="button"
      class="menu__i"
      role="menuitem"
      disabled={pauseDisabled}
      onclick={() => void run(showResume ? resumeCapture : pauseCapture)}
    >
      <span class="menu__ck" aria-hidden="true"></span>
      <span class="menu__gl" aria-hidden="true">
        <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
          {#if showResume}
            <path d="M7 4.5 19 12 7 19.5z" fill="currentColor" stroke="none" />
          {:else}
            <path d="M9 5v14" /><path d="M15 5v14" />
          {/if}
        </svg>
      </span>
      {showResume ? "Resume Capture" : "Pause Capture"}
      {#if kbd("pauseResumeRecording")}<kbd class="kbd">{kbd("pauseResumeRecording")}</kbd>{/if}
    </button>
    <button
      type="button"
      class="menu__i"
      role="menuitem"
      disabled={stopDisabled}
      onclick={() => void run(stopCapture)}
    >
      <span class="menu__ck" aria-hidden="true"></span>
      <span class="menu__gl" aria-hidden="true">
        <svg width="13" height="13" viewBox="0 0 24 24" aria-hidden="true">
          <rect x="6" y="6" width="12" height="12" rx="1.5" fill="currentColor" />
        </svg>
      </span>
      Stop &amp; Save
      {#if kbd("toggleRecording")}<kbd class="kbd">{kbd("toggleRecording")}</kbd>{/if}
    </button>
  {:else}
    <button
      type="button"
      class="menu__i"
      role="menuitem"
      disabled={captureControls.loadingStart || captureControls.loadingSettings}
      onclick={() => void run(startCapture)}
    >
      <span class="menu__ck" aria-hidden="true"></span>
      <span class="menu__gl" aria-hidden="true">
        <svg width="13" height="13" viewBox="0 0 24 24" aria-hidden="true">
          <circle cx="12" cy="12" r="6" fill="currentColor" />
        </svg>
      </span>
      Start Recording
      {#if kbd("toggleRecording")}<kbd class="kbd">{kbd("toggleRecording")}</kbd>{/if}
    </button>
  {/if}

  <div class="menu__sep" role="separator"></div>
  <div class="menu__hd">
    {running ? "Sources — this session" : "Sources"}
  </div>
  {#each lanes as lane (lane.key)}
    <button
      type="button"
      class="menu__i"
      role="menuitemcheckbox"
      aria-checked={sourceSelection.isSelected(lane.key)}
      disabled={laneDisabled(lane.key)}
      use:tip={laneTip(lane.key)}
      onclick={() => void toggleSourceSelected(lane.key)}
    >
      <span class="menu__ck" aria-hidden="true">
        {#if sourceSelection.isSelected(lane.key)}
          <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round">
            <path d="M4 12.5 9.5 18 20 6.5" />
          </svg>
        {/if}
      </span>
      <span class="menu__gl menu__gl--{lane.key}" aria-hidden="true">
        {#if lane.key === "screen"}
          <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <rect x="2" y="4" width="20" height="13" rx="2" /><path d="M8 21h8" /><path d="M12 17v4" />
          </svg>
        {:else if lane.key === "microphone"}
          <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <rect x="9" y="2.5" width="6" height="12" rx="3" /><path d="M5.5 11a6.5 6.5 0 0 0 13 0" /><path d="M12 17.5v3.5" /><path d="M9 21h6" />
          </svg>
        {:else}
          <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M11 5 6.5 9H3v6h3.5L11 19z" /><path d="M15.5 8.5a5 5 0 0 1 0 7" /><path d="M18.5 5.5a9 9 0 0 1 0 13" />
          </svg>
        {/if}
      </span>
      {lane.label}
    </button>
  {/each}
</div>

<style>
  /* NSMenu anatomy (frame 11): 24px rows, 20px checkmark gutter, right kbd
     column, floating shadow. Fixed position — `.titlebar { overflow:hidden }`
     clips absolute descendants (see the notification popover). */
  .menu {
    position: fixed;
    top: calc(var(--app-titlebar-height) + 6px);
    right: 8px;
    z-index: 200;
    min-width: 238px;
    padding: 5px;
    border-radius: var(--r-md);
    background: var(--app-surface-raised);
    box-shadow:
      var(--shadow-popover),
      0 0 0 var(--hairline) var(--app-border-strong);
  }
  .menu__i {
    width: 100%;
    height: 24px;
    display: flex;
    align-items: center;
    padding: 0 var(--s-8) 0 0;
    border: 0;
    border-radius: 5px;
    background: transparent;
    font: var(--w-regular) var(--t-ui) / 1 var(--app-font-sans);
    letter-spacing: var(--ls-ui);
    color: var(--app-text-strong);
    white-space: nowrap;
    cursor: default;
  }
  .menu__i:not(:disabled):hover {
    background: var(--app-accent);
    color: var(--app-accent-contrast);
  }
  .menu__i:not(:disabled):hover .kbd {
    background: transparent;
    color: inherit;
    opacity: 0.8;
  }
  .menu__i:not(:disabled):hover .menu__gl {
    color: inherit;
  }
  .menu__i:disabled {
    color: var(--app-text-subtle);
  }
  .menu__i:focus-visible {
    outline: none;
    box-shadow: var(--ring);
  }
  .menu__ck {
    width: 20px;
    flex: 0 0 auto;
    display: inline-flex;
    justify-content: center;
  }
  .menu__gl {
    width: 18px;
    flex: 0 0 auto;
    margin-right: var(--s-4);
    display: inline-flex;
    justify-content: center;
    opacity: 0.85;
  }
  .menu__gl--screen {
    color: var(--app-src-screen, currentColor);
  }
  .menu__gl--microphone {
    color: var(--app-src-mic, currentColor);
  }
  .menu__gl--systemAudio {
    color: var(--app-src-sys, currentColor);
  }
  .menu__i:disabled .menu__gl {
    color: inherit;
  }
  .menu__i .kbd {
    margin-left: auto;
  }
  .menu__sep {
    height: var(--hairline);
    background: var(--app-border-strong);
    margin: 5px var(--s-8);
  }
  .menu__hd {
    padding: var(--s-4) var(--s-8) var(--s-4) 20px;
    font: var(--w-medium) var(--t-meta) / var(--lh-meta) var(--app-font-sans);
    color: var(--app-text-subtle);
    white-space: nowrap;
  }
  .menu__hd.is-mono {
    font-family: var(--app-font-mono);
    font-variant-numeric: tabular-nums;
  }
</style>
