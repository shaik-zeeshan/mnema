<!--
  Capture tab (docs/mockups/redesign/developer.html, view "capture"): the run
  line with the window's only Record/Stop, three source lanes with live level
  meters, the suspension notice, the system-audio tap spec, what is kept out of
  recordings, and disk headroom. Polls `get_capture_health` at 1 s.
-->
<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import type { Component } from "svelte";
  import { tip } from "$lib/components/tooltip";
  import { fpsToIntervalS } from "$lib/components/capture-rate";
  import { getEffectiveGlobalShortcut } from "$lib/global-shortcuts";
  import { detectKeyboardPlatform, formatShortcut } from "$lib/keyboard";
  import { formatClock } from "$lib/time-format";
  import type { StorageUsage } from "$lib/types";
  import type { CaptureSourceHealth } from "$lib/types/debug";
  import IconMonitor from "~icons/lucide/monitor";
  import IconMic from "~icons/lucide/mic";
  import IconVolume from "~icons/lucide/volume-2";
  import IconInbox from "~icons/lucide/inbox";
  import IconSquare from "~icons/lucide/square";
  import IconCircle from "~icons/lucide/circle";
  import IconAlert from "~icons/lucide/triangle-alert";
  import { getDebugController } from "../state/controller.svelte";
  import { formatRunning } from "../verdict";
  import {
    LANE_WORD, WEB_ADDRESS, diskAt, formatSeconds, formatWait, gb, laneDetail, meterBars, rebuildReasonLabel,
  } from "../capture-view";

  const controller = getDebugController();
  const { capture } = controller;

  let now = $state(Date.now());
  let storage = $state<StorageUsage | null>(null);

  $effect(() => capture.initListeners());
  $effect(() => {
    void capture.loadSettings();
    invoke<StorageUsage>("get_storage_usage").then((u) => (storage = u)).catch(() => {});
  });
  $effect(() =>
    controller.poll(async () => {
      await capture.fetchCaptureHealth();
      now = Date.now();
    }, 1_000),
  );

  const health = $derived(capture.captureHealth);
  const recording = $derived(health?.recording ?? capture.isCapturing);
  // Reconcile the shared session only while recording, only from this tab.
  $effect(() => {
    if (recording) return controller.poll(capture.resyncSession, 5_000);
  });

  const settings = $derived(capture.recordingSettings);
  const tap = $derived(health?.systemAudioTap ?? null);
  const suspension = $derived(health?.suspension ?? null);
  const platform = detectKeyboardPlatform();
  const keys = $derived(formatShortcut(getEffectiveGlobalShortcut("toggleRecording").bindings[0], platform).join(""));
  const actionTip = $derived(`${recording ? "Stop" : "Start"} recording${keys ? ` · ${keys}` : ""}`);

  function screenRight(src: CaptureSourceHealth): string {
    if (src.state === "waiting" && suspension?.sinceMs != null) return formatWait(now - suspension.sinceMs);
    if (src.state !== "recording" || !settings) return "—";
    const s = fpsToIntervalS(settings.screenFrameRate);
    return s < 60 ? `every ${Math.round(s * 10) / 10} s` : `every ${Math.round(s / 60)} min`;
  }

  type Lane = { name: string; icon: Component; c: string; src: CaptureSourceHealth; audio: boolean; rate: number | null };
  const lanes = $derived<Lane[]>(
    health
      ? [
          { name: "Screen", icon: IconMonitor, c: "var(--app-source-screen)", src: health.screen, audio: false, rate: null },
          { name: "Microphone", icon: IconMic, c: "var(--app-source-mic)", src: health.microphone, audio: true, rate: null },
          { name: "System audio", icon: IconVolume, c: "var(--app-source-sysaudio)", src: health.systemAudio, audio: true, rate: tap?.sampleRateHz ?? null },
        ]
      : [],
  );

  const kept = $derived((settings?.privacy.excludedApps ?? []).filter((a) => a.enabled));
  const urlMode = $derived(settings ? (settings.metadata.enabled ? settings.metadata.browserUrlMode : "off") : null);

  const free = $derived(health?.disk.freeBytes ?? storage?.freeBytes ?? null);
  const pauseAt = $derived(health ? (health.disk.pauseThresholdBytes ?? health.disk.reserveFloorBytes) : null);
  const resumeAt = $derived(health ? (health.disk.resumeThresholdBytes ?? health.disk.reserveFloorBytes) : null);
  // Not recording: no live thresholds, so one tick at the reserve floor.
  const split = $derived(pauseAt != null && resumeAt != null && resumeAt > pauseAt);
  const used = $derived(
    storage ? storage.screenBytes + storage.microphoneBytes + storage.systemAudioBytes + storage.indexBytes + storage.modelsBytes : null,
  );
  const db = (v: number) => `${v < 0 ? "−" : ""}${Math.abs(Math.round(v))} dB`;
</script>

<section class="dv-sec">
  <div class="dv-run">
    <span class="dv-dot" data-tone={recording ? "ok" : "off"}></span>
    <b>{recording ? "Recording" : "Not recording"}</b>
    {#if recording && health?.startedAtMs != null}
      <span class="num">{formatRunning(now - health.startedAtMs)} · since {formatClock(health.startedAtMs)}</span>
    {/if}
    <span class="mx-spacer"></span>
    {#if recording}
      <button type="button" class="mx-btn mx-btn--sm" use:tip={actionTip} disabled={capture.loadingStop}
        onclick={async () => { await capture.stopCapture(); void capture.fetchCaptureHealth(); }}>
        <IconSquare width="8" height="8" class="dv-glyph" style="color:var(--app-text-strong)" aria-hidden="true" />Stop
      </button>
    {:else}
      <button type="button" class="mx-btn mx-btn--sm" use:tip={actionTip} disabled={capture.loadingStart}
        onclick={async () => { await capture.startCapture(); void capture.fetchCaptureHealth(); }}>
        <IconCircle width="7" height="7" class="dv-glyph" style="color:var(--app-record-glyph-start)" aria-hidden="true" />Record
      </button>
    {/if}
  </div>
  {#if capture.lifecycleError}
    <p class="mx-inline" data-tone="danger"><IconAlert width="13" height="13" aria-hidden="true" />{capture.lifecycleError}</p>
  {/if}
  <div class="dv-lanes">
    {#each lanes as lane (lane.name)}
      {@const word = LANE_WORD[lane.src.state]}
      {@const live = lane.src.state === "recording" && lane.src.levelDb != null}
      {@const Icon = lane.icon}
      <div class="dv-lane" class:dv-lane--off={lane.src.state === "off"} style="--c:{lane.c}">
        <span class="dv-lane__icon"><Icon width="14" height="14" aria-hidden="true" /></span>
        <div>
          <div class="dv-lane__name">{lane.name}</div>
          <div class="dv-lane__state" data-tone={word.tone}><b>{word.word}</b>{laneDetail(lane.src, lane.rate)}</div>
        </div>
        <div class="dv-lane__right">
          {#if lane.audio}
            {@const lit = live ? meterBars(lane.src.levelDb) : 0}
            <span class="dv-meter" aria-hidden="true">{#each { length: 12 } as _, i}<i class:on={i < lit}></i>{/each}</span>
            <span class="num">{live ? db(lane.src.levelDb ?? 0) : "—"}</span>
          {:else}
            <span class="num">{screenRight(lane.src)}</span>
          {/if}
        </div>
      </div>
    {/each}
  </div>
</section>

{#if suspension?.kind === "displayUnavailable"}
  <div class="mx-notice" data-tone="info">
    <span class="mx-notice__icon"><IconMonitor width="15" height="15" aria-hidden="true" /></span>
    <b class="mx-notice__title">Resumes on its own when the display wakes</b>
    <p class="mx-notice__text">
      {#if suspension.sinceMs != null}Since <span class="num">{formatClock(suspension.sinceMs)}</span>, checked{:else}Checked{/if}
      <span class="num">{suspension.recoveryAttempts}</span> {suspension.recoveryAttempts === 1 ? "time" : "times"}. The last few
      minutes were saved; microphone and system audio keep recording.
    </p>
  </div>
{:else if suspension?.kind === "lowDisk"}
  <div class="mx-notice" data-tone="warn">
    <span class="mx-notice__icon"><IconAlert width="15" height="15" aria-hidden="true" /></span>
    <b class="mx-notice__title">Recording paused until {resumeAt != null ? `${gb(resumeAt)} GB` : "more space"} is free</b>
    <p class="mx-notice__text">It picks up on its own once there is room again.</p>
  </div>
{:else if suspension?.kind === "privacyFilter" && suspension.restartRequired}
  <div class="mx-notice" data-tone="danger">
    <span class="mx-notice__icon"><IconAlert width="15" height="15" aria-hidden="true" /></span>
    <b class="mx-notice__title">The privacy filter needs a restart</b>
    <p class="mx-notice__text">The screen stays off until then. Press Stop, then Record.</p>
  </div>
{/if}

{#if health && !recording}
  <div class="mx-empty mx-empty--compact dv-cap-empty">
    <span class="mx-empty__glyph"><IconInbox width="14" height="14" aria-hidden="true" /></span>
    <b class="mx-empty__title">Nothing is being captured</b>
    <p class="mx-empty__text">Earlier recordings are still being read and indexed. See Pipeline.</p>
  </div>
{/if}

{#if recording && tap}
  <section class="dv-sec">
    <h3 class="dv-h">System audio connection</h3>
    <dl class="mx-spec">
      <div>
        <dt>Rebuilt today</dt>
        <dd><span class="num">{tap.rebuildsToday}</span> {tap.rebuildsToday === 1 ? "time" : "times"}{health?.systemAudio.state === "reconnecting" ? " · rebuilding now" : ""}</dd>
        <dd class="mx-spec__note">each opens a new file</dd>
      </div>
      <div>
        <dt>Last reason</dt>
        <dd>{rebuildReasonLabel(tap.lastReason)}{#if tap.lastDetail}, <span class="num">{tap.lastDetail}</span>{/if}</dd>
        <dd class="mx-spec__note">{tap.lastRebuildAtMs != null ? formatClock(tap.lastRebuildAtMs) : ""}</dd>
      </div>
      <div>
        <dt>Silence check</dt>
        <dd>{#if tap.nextSilenceCheckInMs != null}next in <span class="num">{formatSeconds(tap.nextSilenceCheckInMs)}</span>{:else}—{/if}</dd>
        <dd class="mx-spec__note">{tap.watchdogDelayMs != null ? `every ${formatSeconds(tap.watchdogDelayMs)}` : ""}</dd>
      </div>
    </dl>
  </section>
{/if}

{#if settings}
  <section class="dv-sec">
    <h3 class="dv-h">Kept out of recordings<span class="num">{kept.length} {kept.length === 1 ? "app" : "apps"}</span></h3>
    {#if kept.length}
      <div class="dv-apps">
        {#each kept as app (app.id)}
          <span class="dv-app"><i>{app.displayName.trim().charAt(0).toUpperCase() || "?"}</i>{app.displayName}</span>
        {/each}
      </div>
    {/if}
    {#if urlMode}
      <div class="dv-pline">Web addresses <b>{WEB_ADDRESS[urlMode]}</b></div>
    {/if}
  </section>
{/if}

<section class="dv-sec">
  <h3 class="dv-h">Disk<span class="mx-spacer"></span><span class="num dv-free">{free != null ? `${gb(free)} GB free` : "—"}</span></h3>
  {#if free != null && pauseAt != null && resumeAt != null}
    <div
      class="dv-head"
      style="--v:{diskAt(free)}"
      data-tone={free < resumeAt ? "warn" : undefined}
      role="img"
      aria-label="{gb(free)} GB free; recording pauses below {gb(pauseAt)} GB{split ? ` and resumes at ${gb(resumeAt)} GB` : ""}"
    >
      <i></i><span class="dv-tick" style="--at:{diskAt(pauseAt)}"></span>
      {#if split}<span class="dv-tick dv-tick--resume" style="--at:{diskAt(resumeAt)}"></span>{/if}
    </div>
    <div class="dv-head__legend">
      <!-- A pause tick hard against 0 takes over the "0" label rather than overlap it. -->
      {#if diskAt(pauseAt) < 0.12}<span style="--at:0">pause {gb(pauseAt)}</span>
      {:else}<span style="--at:0">0</span><span style="--at:{diskAt(pauseAt)}">pause {gb(pauseAt)}</span>{/if}
      {#if split}<span style="--at:{diskAt(resumeAt)}">resume {gb(resumeAt)}</span>{/if}<span>25 GB +</span>
    </div>
  {/if}
  {#if storage && used != null}
    <p class="dv-uses">
      Mnema uses <span class="num">{gb(used)} GB</span>: screen <span class="num">{gb(storage.screenBytes)}</span> · audio
      <span class="num">{gb(storage.microphoneBytes + storage.systemAudioBytes)}</span> · meaning index
      <span class="num">{gb(storage.indexBytes)}</span> · models <span class="num">{gb(storage.modelsBytes)}</span>
    </p>
  {/if}
</section>

<style>
  .dv-run { display: flex; align-items: center; gap: var(--s-2); }
  .dv-run b { font: 500 var(--text-md)/1 var(--font-sans); color: var(--app-text-strong); }
  .dv-run .num { font-size: var(--text-base); color: var(--app-text-muted); }
  .dv-run .mx-spacer { flex: 1; }
  /* the mockup draws the stop square and record dot solid */
  .dv-run :global(.dv-glyph *) { fill: currentColor; stroke: none; }
  .dv-lanes { display: grid; border-top: 1px solid var(--mx-hairline); }
  .dv-lane { display: grid; grid-template-columns: 26px minmax(0, 1fr) auto; align-items: center; gap: 10px; padding: 8px 2px; border-bottom: 1px solid var(--mx-hairline); }
  .dv-lane__icon { width: 26px; height: 26px; display: grid; place-items: center; border-radius: var(--r-sm); color: var(--c);
    background: color-mix(in srgb, var(--c) 12%, transparent); box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--c) 24%, transparent); }
  .dv-lane--off .dv-lane__icon { color: var(--app-text-subtle); background: none; box-shadow: inset 0 0 0 1px var(--app-border-strong); }
  .dv-lane__name { font: 500 var(--text-md)/1.25 var(--font-sans); color: var(--app-text-strong); }
  .dv-lane__state { display: flex; align-items: center; gap: 6px; margin-top: 2px; font: 400 var(--text-base)/1.3 var(--font-sans); color: var(--app-text-subtle); white-space: nowrap; overflow: hidden; }
  .dv-lane__state b { font-weight: 500; color: var(--tone, var(--app-text)); }
  .dv-lane__right { display: flex; align-items: center; gap: var(--s-2); }
  .dv-lane__right .num { min-width: 52px; text-align: right; font-size: var(--text-sm); color: var(--app-text-muted); }
  .dv-meter { display: flex; align-items: flex-end; gap: 2px; height: 12px; }
  .dv-meter i { width: 3px; height: 100%; border-radius: 1px; background: var(--mx-wash-strong); }
  .dv-meter i.on { background: var(--c); }

  .dv-cap-empty { padding: 10px 0; }

  .dv-apps { display: flex; flex-wrap: wrap; gap: 5px; }
  .dv-app { display: inline-flex; align-items: center; gap: 6px; height: 22px; padding: 0 8px 0 3px; border-radius: var(--r-sm);
    border: 1px solid var(--app-border); background: var(--app-surface-subtle); font: 400 var(--text-base)/1 var(--font-sans); color: var(--app-text); }
  .dv-app i { width: 16px; height: 16px; display: grid; place-items: center; border-radius: 4px; font: 600 var(--text-xs)/1 var(--font-sans); font-style: normal;
    color: var(--app-text-muted); background: var(--mx-wash-strong); }
  .dv-pline { display: flex; align-items: baseline; gap: var(--s-2); font: 400 var(--text-base)/1.4 var(--font-sans); color: var(--app-text-subtle); }
  .dv-pline b { font-weight: 500; color: var(--app-text); }

  .dv-free { color: var(--app-text); }
  .dv-head { position: relative; height: 8px; border-radius: 4px; background: var(--mx-wash-strong); }
  .dv-head > i { position: absolute; inset: 0 auto 0 0; width: calc(var(--v) * 100%); border-radius: inherit; background: color-mix(in srgb, var(--app-text-subtle) 50%, transparent); }
  .dv-head[data-tone] > i { background: var(--tone); }
  .dv-tick { position: absolute; top: -3px; bottom: -3px; left: calc(var(--at) * 100%); width: 2px; margin-left: -1px; border-radius: 1px; background: var(--app-text-muted); }
  .dv-tick--resume { background: var(--app-text-subtle); }
  .dv-head__legend { position: relative; height: 14px; margin-top: 5px; font: 450 var(--text-xs)/1 var(--font-mono); color: var(--app-text-subtle); }
  .dv-head__legend span { position: absolute; left: calc(var(--at) * 100%); translate: -50% 0; white-space: nowrap; }
  .dv-head__legend span:first-child { translate: 0 0; }
  .dv-head__legend span:last-child { left: auto; right: 0; translate: 0 0; }
  .dv-uses { margin: 2px 0 0; font: 400 var(--text-base)/1.5 var(--font-sans); color: var(--app-text-subtle); }
  .dv-uses .num { color: var(--app-text-muted); }
</style>
