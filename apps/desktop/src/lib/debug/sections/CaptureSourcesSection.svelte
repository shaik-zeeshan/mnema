<script lang="ts">
  // Capture Sources — migrated from the legacy "Capture" tab (session card +
  // runtime-source lanes) plus the "System" tab's support/permissions probe,
  // which is capture capability and belongs with the sources it describes.

  import type { Component } from "svelte";
  import { tip } from "$lib/components/tooltip";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import IconMonitor from "~icons/lucide/monitor";
  import IconMic from "~icons/lucide/mic";
  import IconVolume2 from "~icons/lucide/volume-2";
  import { anchor } from "../sections";
  import { getDebugController } from "../state/controller.svelte";
  import {
    formatIdleMs,
    formatPermission,
    formatSourceStartedAt,
    formatTimestamp,
    permissionBadgeClass,
    runtimeStateWord,
    severityBadgeClass,
    severityCardClass,
    severityLabel,
    shortenPath,
    supportBadge,
  } from "../format";
  import type { CaptureSource } from "../state/capture.svelte";

  const SOURCE_ICONS: Record<CaptureSource, Component> = {
    screen: IconMonitor,
    microphone: IconMic,
    systemAudio: IconVolume2,
  };

  const { capture, health } = getDebugController();

  const severity = $derived(health.severityFor("capture"));

  // Pulled out of the markup: `{@const}` needs a block parent, and reading
  // through `capture.session?.` inline wouldn't narrow inside the `{#if}`.
  const session = $derived(capture.session);
  const requestedSources = $derived(session?.requestedSources);

  /** The session row's word + tone: the same states the Health card reports. */
  const sessionState = $derived.by(() => {
    if (!capture.isCapturing) {
      return session?.isRunning === false
        ? { word: "stopped", cls: "badge badge--neutral" }
        : { word: "idle", cls: "badge badge--neutral" };
    }
    if (session?.isLowDiskSuspended) return { word: "suspended", cls: "badge badge--warn" };
    if (session?.isUserPaused) return { word: "paused", cls: "badge badge--warn" };
    if (capture.isInactivityPaused) return { word: "paused", cls: "badge badge--warn" };
    return { word: "recording", cls: "badge badge--ok" };
  });

  /** Which sources this session asked for, or (while stopped) will ask for. */
  const sourceSummary = $derived.by(() => {
    const settings = capture.recordingSettings;
    const live = requestedSources;
    const on = live
      ? [live.screen && "screen", live.microphone && "mic", live.systemAudio && "sys-audio"]
      : [settings?.captureScreen && "screen", settings?.captureMicrophone && "mic", settings?.captureSystemAudio && "sys-audio"];
    const names = on.filter((name): name is string => typeof name === "string");
    if (names.length === 0) return "no sources selected";
    return `${names.join(" · ")}${live ? "" : " (persisted settings)"}`;
  });

  type RuntimeSourceLane = {
    key: CaptureSource;
    label: string;
    sample: { lastUnixMs: number | null; idleMs: number | null; level: number | null } | null;
    qualifiedIdleMs: number | null;
    qualifiedThreshold: number | null;
  };

  const runtimeLanes = $derived.by<RuntimeSourceLane[]>(() => {
    const idleDebug = capture.idleDebug;
    if (!idleDebug) return [];
    return [
      {
        key: "screen",
        label: "Screen",
        sample: idleDebug.screenActivityLastUnixMs != null
          ? { lastUnixMs: idleDebug.screenActivityLastUnixMs, idleMs: idleDebug.screenActivityIdleMs, level: null }
          : null,
        qualifiedIdleMs: idleDebug.screenActivityIdleMs,
        qualifiedThreshold: null,
      },
      {
        key: "microphone",
        label: "Microphone",
        sample: { lastUnixMs: idleDebug.microphoneActivitySample.lastUnixMs, idleMs: null, level: idleDebug.microphoneActivitySample.level },
        qualifiedIdleMs: idleDebug.microphoneActivityDecision.idleMs,
        qualifiedThreshold: idleDebug.microphoneActivityDecision.activityThreshold,
      },
      {
        key: "systemAudio",
        label: "System Audio",
        sample: { lastUnixMs: idleDebug.systemAudioActivitySample.lastUnixMs, idleMs: null, level: idleDebug.systemAudioActivitySample.level },
        qualifiedIdleMs: idleDebug.systemAudioActivityDecision.idleMs,
        qualifiedThreshold: idleDebug.systemAudioActivityDecision.activityThreshold,
      },
    ];
  });
</script>

<SettingGroup
  title="Capture Sources"
  hint="session · runtime sources · permissions"
  hintInline
  id={anchor("capture")}
  cardClass={severityCardClass(severity)}
>
  {#snippet titleExtra()}
    <span class={severityBadgeClass(severity)} use:tip={health.reasonFor("capture") ?? ""}>
      <span class="b-dot" aria-hidden="true"></span>{severityLabel(severity)}
    </span>
  {/snippet}

  <!-- ── Session ───────────────────────────────────────────────────────── -->
  <div class="row">
    <div class="row__main">
      <div class="row__label">Session</div>
      <div class="row__desc">{sourceSummary}</div>
    </div>
    <div class="row__value">
      <span class="rec-dot" class:rec-dot--active={capture.isCapturing}></span>
      <span class={sessionState.cls}>{sessionState.word}</span>
    </div>
  </div>

  {#if capture.recordingSettings}
    {@const settings = capture.recordingSettings}
    <div class="row">
      <div class="row__main">
        <div class="row__label">Capture settings</div>
        <div class="row__desc">what a new session would be started with</div>
      </div>
      <div class="row__value">
        <span class="badge badge--neutral">{settings.screenFrameRate} fps</span>
        <span class="badge badge--neutral">{settings.segmentDurationSeconds}s segments</span>
      </div>
    </div>
  {/if}

  {#if capture.isInactivityPaused}
    <p class="debug-warnline" role="status" aria-live="polite">
      Paused on inactivity timeout; waiting for activity.
    </p>
  {/if}

  {#if requestedSources}
    <div class="debug-body">
      <div class="source-session-grid">
        {#if requestedSources.screen}
          <div class="source-session-card">
            <div class="source-session-card__header"><span class="badge badge--ok badge--sm">screen</span></div>
            <ul class="kv-list">
              <li>
                <span class="kv-key">session</span>
                <span class="kv-val kv-val--mono">{capture.getSourceSessionId(session, "screen")}</span>
              </li>
              <li>
                <span class="kv-key">started</span>
                <span class="kv-val">{formatSourceStartedAt(capture.getSourceSessionStartedAt(session, "screen"))}</span>
              </li>
            </ul>
          </div>
        {/if}

        {#if requestedSources.microphone}
          <div class="source-session-card">
            <div class="source-session-card__header"><span class="badge badge--ok badge--sm">mic</span></div>
            <ul class="kv-list">
              <li>
                <span class="kv-key">session</span>
                <span class="kv-val kv-val--mono">{capture.getSourceSessionId(session, "microphone")}</span>
              </li>
              <li>
                <span class="kv-key">started</span>
                <span class="kv-val">{formatSourceStartedAt(capture.getSourceSessionStartedAt(session, "microphone"))}</span>
              </li>
            </ul>
          </div>
        {/if}

        {#if requestedSources.systemAudio}
          <div class="source-session-card">
            <div class="source-session-card__header"><span class="badge badge--ok badge--sm">sys-audio</span></div>
            <ul class="kv-list">
              <li>
                <span class="kv-key">session</span>
                <span class="kv-val kv-val--mono">{capture.getSourceSessionId(session, "systemAudio")}</span>
              </li>
              <li>
                <span class="kv-key">started</span>
                <span class="kv-val">{formatSourceStartedAt(capture.getSourceSessionStartedAt(session, "systemAudio"))}</span>
              </li>
            </ul>
          </div>
        {/if}
      </div>
    </div>
  {/if}

  <!-- ── Runtime sources ─────────────────────────────────────────────────
       Heads the lane grid below it, so the rule sits above this row, not
       between the row and its own block. -->
  <div class="row row--head">
    <div class="row__main">
      <div class="row__label">Runtime sources</div>
      <div class="row__desc">capture session · writer · activity</div>
    </div>
    <div class="row__value">
      {#if !capture.idleDebug?.runtimeSources && !capture.idleDebugError}
        <span class="dim">{capture.isCapturing ? "not loaded" : "session inactive"}</span>
      {/if}
      <button
        class="btn btn--ghost btn--sm"
        onclick={capture.refreshRuntimeSources}
        disabled={capture.loadingRuntimeSources || !capture.isCapturing}
        aria-label="Refresh runtime sources"
        use:tip={capture.isCapturing ? "Refresh runtime sources" : "No active capture session — start recording to refresh"}
      >
        <span class="refresh-glyph" class:refresh-glyph--spin={capture.loadingRuntimeSources} aria-hidden="true">↻</span>
      </button>
    </div>
  </div>

  {#if capture.idleDebug && capture.idleDebug.runtimeSources}
    {@const runtimeSources = capture.idleDebug.runtimeSources}
    <div class="debug-body">
      <div class="rs-grid">
        {#each runtimeLanes as lane (lane.key)}
          {@const src = runtimeSources[lane.key]}
          {@const state = runtimeStateWord(src)}
          {@const LaneIcon = SOURCE_ICONS[lane.key]}
          <article
            class="rs-lane rs-lane--{lane.key}"
            class:rs-lane--off={!src.requested}
            class:rs-lane--paused={src.paused && src.requested}
            class:rs-lane--running={src.requested && !src.paused && src.writerActive === true}
          >
            <header class="rs-lane__head">
              <span class="rs-lane__glyph" aria-hidden="true"><LaneIcon /></span>
              <span class="rs-lane__name">{lane.label}</span>
              <span class={state.cls}>{state.word}</span>
            </header>

            <!-- Truth rows: source · session · writer -->
            <ul class="rs-rows">
              <li class="rs-row">
                <span class="rs-row__label">source</span>
                <span class="rs-row__bar" data-state={src.requested ? "on" : "off"}>
                  <span class="rs-row__bar-fill"></span>
                </span>
                <span class="rs-row__val">{src.requested ? "requested" : "not requested"}</span>
              </li>
              <li class="rs-row">
                <span class="rs-row__label">session</span>
                <span class="rs-row__bar" data-state={src.sessionActive === null ? "unknown" : src.sessionActive ? (src.paused ? "paused" : "on") : "off"}>
                  <span class="rs-row__bar-fill"></span>
                </span>
                <span class="rs-row__val">
                  {#if src.sessionActive === null}
                    {src.reason ?? "—"}
                  {:else if src.sessionActive}
                    {src.paused ? "running (paused)" : "running"}
                  {:else}
                    {src.requested ? "detached" : "—"}
                  {/if}
                </span>
              </li>
              <li class="rs-row">
                <span class="rs-row__label">writer</span>
                <span class="rs-row__bar" data-state={src.writerActive === null ? "unknown" : src.writerActive ? "on" : src.requested ? "off" : "idle"}>
                  <span class="rs-row__bar-fill"></span>
                </span>
                <span class="rs-row__val">
                  {#if src.writerActive === null}
                    {src.reason ?? "—"}
                  {:else if src.writerActive}
                    attached
                  {:else if src.requested && src.paused}
                    finalized (paused)
                  {:else if src.requested}
                    detached
                  {:else}
                    —
                  {/if}
                </span>
              </li>
            </ul>

            <!-- Output path -->
            <div class="rs-path">
              <span class="rs-path__label">out</span>
              <span class="rs-path__val" use:tip={src.outputPath ?? ""}>{shortenPath(src.outputPath)}</span>
            </div>

            <!-- Activity readouts: distinguish raw sample vs threshold-qualified -->
            <ul class="rs-rows rs-rows--activity">
              <li class="rs-row">
                <span class="rs-row__label">sample</span>
                <span class="rs-row__val rs-row__val--mono">
                  {#if lane.sample == null || lane.sample.lastUnixMs == null}
                    —
                  {:else}
                    {formatTimestamp(lane.sample.lastUnixMs)}
                    {#if lane.sample.level != null}
                      <span class="idle-note">lvl {(lane.sample.level * 100).toFixed(0)}%</span>
                    {/if}
                  {/if}
                </span>
              </li>
              <li class="rs-row">
                <span class="rs-row__label">activity</span>
                <span class="rs-row__val rs-row__val--mono">
                  {#if lane.qualifiedIdleMs == null}
                    none
                  {:else}
                    idle {formatIdleMs(lane.qualifiedIdleMs)}
                    {#if lane.qualifiedThreshold != null}
                      <span class="idle-note">thr {(lane.qualifiedThreshold * 100).toFixed(0)}%</span>
                    {/if}
                  {/if}
                </span>
              </li>
            </ul>
          </article>
        {/each}
      </div>
      <p class="rs-legend">
        <span><b class="rs-legend__dot rs-legend__dot--on"></b> attached / running</span>
        <span><b class="rs-legend__dot rs-legend__dot--paused"></b> requested but paused</span>
        <span><b class="rs-legend__dot rs-legend__dot--off"></b> detached / not requested</span>
        <span class="idle-note">sample = raw probe timestamp · activity = threshold-qualified idle</span>
      </p>
    </div>
  {:else if capture.idleDebugError}
    <p class="debug-errline" role="alert" aria-live="polite">{capture.idleDebugError}</p>
  {/if}

  <!-- ── System probe ──────────────────────────────────────────────────── -->
  <details class="disclosure">
    <summary class="row row--head">
      <div class="row__main">
        <div class="row__label">System probe</div>
        <div class="row__desc">support &amp; permissions</div>
      </div>
      <div class="row__value"><span class="disclosure__chevron" aria-hidden="true">›</span></div>
    </summary>
    <div class="debug-body">
      <div class="probe-grid">
        <div class="probe-block">
          <div class="probe-block__header">
            <span class="probe-block__name">Support</span>
            <button class="btn btn--ghost btn--sm" onclick={capture.loadSupport} disabled={capture.loadingSupport}>
              Query{#if capture.loadingSupport}&nbsp;<span class="refresh-glyph refresh-glyph--spin" aria-hidden="true">↻</span>{/if}
            </button>
          </div>
          {#if capture.support}
            {@const support = capture.support}
            <ul class="kv-list">
              <li><span class="kv-key">platform</span><span class="kv-val">{support.platform}</span></li>
              <li>
                <span class="kv-key">native</span>
                <span class={supportBadge(support.nativeCaptureSupported)}>{support.nativeCaptureSupported ? "yes" : "no"}</span>
              </li>
              <li>
                <span class="kv-key">screen</span>
                <span class={supportBadge(support.supportedSources.screen)}>{support.supportedSources.screen ? "yes" : "no"}</span>
              </li>
              <li>
                <span class="kv-key">mic</span>
                <span class={supportBadge(support.supportedSources.microphone)}>{support.supportedSources.microphone ? "yes" : "no"}</span>
              </li>
              <li>
                <span class="kv-key">sys-audio</span>
                <span class={supportBadge(support.supportedSources.systemAudio)}>{support.supportedSources.systemAudio ? "yes" : "no"}</span>
              </li>
            </ul>
          {:else if capture.supportError}
            <p class="debug-err" role="alert" aria-live="polite">{capture.supportError}</p>
          {:else}
            <p class="empty">not queried yet — press Query</p>
          {/if}
        </div>

        <div class="probe-block">
          <div class="probe-block__header">
            <span class="probe-block__name">Permissions</span>
            <button class="btn btn--ghost btn--sm" onclick={capture.loadPermissions} disabled={capture.loadingPermissions}>
              Query{#if capture.loadingPermissions}&nbsp;<span class="refresh-glyph refresh-glyph--spin" aria-hidden="true">↻</span>{/if}
            </button>
          </div>
          {#if capture.permissions}
            {@const permissions = capture.permissions}
            <ul class="kv-list">
              <li><span class="kv-key">screen</span><span class={permissionBadgeClass(permissions.screen)}>{formatPermission(permissions.screen)}</span></li>
              <li><span class="kv-key">mic</span><span class={permissionBadgeClass(permissions.microphone)}>{formatPermission(permissions.microphone)}</span></li>
              <li><span class="kv-key">sys-audio</span><span class={permissionBadgeClass(permissions.systemAudio)}>{formatPermission(permissions.systemAudio)}</span></li>
            </ul>
          {:else if capture.permissionsError}
            <p class="debug-err" role="alert" aria-live="polite">{capture.permissionsError}</p>
          {:else}
            <p class="empty">not queried yet — press Query</p>
          {/if}
        </div>
      </div>
    </div>
  </details>

  <div class="actions">
    <button class="btn btn--primary" onclick={capture.startCapture} disabled={capture.isCapturing || capture.loadingStart || capture.loadingSettings}>
      {capture.loadingStart ? "Starting…" : "Start Recording"}
    </button>
    <button class="btn btn--danger" onclick={capture.stopCapture} disabled={!capture.isCapturing || capture.loadingStop}>
      {capture.loadingStop ? "Stopping…" : "Stop Recording"}
    </button>
    {#if capture.lifecycleError}
      <span class="lifecycle-error" role="alert" aria-live="assertive" use:tip={capture.lifecycleError}>
        <span class="lifecycle-error__tag" aria-hidden="true">✕</span>
        <span class="lifecycle-error__text">{capture.lifecycleError}</span>
      </span>
    {/if}
  </div>
</SettingGroup>
