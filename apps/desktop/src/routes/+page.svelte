<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import type {
    CaptureSupport,
    CaptureSession,
    GetPermissionsResponse,
    MicrophoneControllerState,
    MicrophoneAutoDisconnectTransitionFailedEvent,
    PermissionsMap,
    PermissionStatus,
    RecordingSettings,
  } from "$lib/types";

  // ─── State ────────────────────────────────────────────────────────────────

  let support = $state<CaptureSupport | null>(null);
  let permissions = $state<PermissionsMap | null>(null);
  let session = $state<CaptureSession | null>(null);
  let recordingSettings = $state<RecordingSettings | null>(null);

  let lastError = $state<string | null>(null);
  let loadingSupport = $state(false);
  let loadingPermissions = $state(false);
  let loadingStart = $state(false);
  let loadingStop = $state(false);
  let loadingSettings = $state(false);

  // ─── Helpers ──────────────────────────────────────────────────────────────

  function clearError() { lastError = null; }

  function setError(err: unknown) {
    lastError = typeof err === "string" ? err : JSON.stringify(err, null, 2);
  }

  function permissionBadgeClass(status: PermissionStatus | undefined): string {
    if (!status) return "badge badge--neutral";
    if (status === "granted") return "badge badge--ok";
    if (status === "denied" || status === "restricted") return "badge badge--err";
    return "badge badge--neutral";
  }

  function supportBadge(val: boolean): string {
    return val ? "badge badge--ok" : "badge badge--err";
  }

  function formatPermission(status: PermissionStatus | undefined): string {
    if (!status) return "unknown";
    return status.replace(/_/g, " ");
  }

  function formatTimestamp(ms: number): string {
    return new Date(ms).toLocaleTimeString();
  }

  // ─── Actions ──────────────────────────────────────────────────────────────

  async function loadSupport() {
    loadingSupport = true;
    clearError();
    try {
      support = await invoke<CaptureSupport>("get_capture_support");
    } catch (err) {
      support = null;
      setError(err);
    } finally {
      loadingSupport = false;
    }
  }

  async function loadPermissions() {
    loadingPermissions = true;
    clearError();
    try {
      const result = await invoke<GetPermissionsResponse>("get_capture_permissions");
      permissions = result.permissions;
      if (result.session) session = result.session;
    } catch (err) {
      permissions = null;
      setError(err);
    } finally {
      loadingPermissions = false;
    }
  }

  async function loadSettings() {
    loadingSettings = true;
    clearError();
    try {
      recordingSettings = await invoke<RecordingSettings>("get_recording_settings");
    } catch (err) {
      setError(err);
    } finally {
      loadingSettings = false;
    }
  }

  async function startCapture() {
    loadingStart = true;
    clearError();
    try {
      // Backend reads from persisted settings — pass an empty/ignored request
      const result = await invoke<{ session: CaptureSession }>("start_native_capture", {
        request: {
          captureScreen: recordingSettings?.captureScreen ?? true,
          captureMicrophone: recordingSettings?.captureMicrophone ?? false,
          captureSystemAudio: recordingSettings?.captureSystemAudio ?? false,
        },
      });
      session = result.session;
    } catch (err) {
      setError(err);
    } finally {
      loadingStart = false;
    }
  }

  async function stopCapture() {
    loadingStop = true;
    clearError();
    try {
      const result = await invoke<{ session: CaptureSession }>("stop_native_capture");
      session = result.session;
    } catch (err) {
      setError(err);
      try {
        const r = await invoke<GetPermissionsResponse>("get_capture_permissions");
        permissions = r.permissions;
        session = r.session;
      } catch { /* best-effort */ }
    } finally {
      loadingStop = false;
    }
  }

  const isCapturing = $derived(session?.isRunning === true);

  // ─── Init ─────────────────────────────────────────────────────────────────

  $effect(() => {
    loadSettings();
    loadPermissions();

    let unlistenControllerChanged: (() => void) | undefined;
    let unlistenAutoDisconnectFailure: (() => void) | undefined;
    let destroyed = false;

    listen<MicrophoneControllerState>("microphone_controller_changed", () => {
      clearError();
    }).then((fn) => {
      if (destroyed) fn();
      else unlistenControllerChanged = fn;
    });

    listen<MicrophoneAutoDisconnectTransitionFailedEvent>(
      "microphone_auto_disconnect_transition_failed",
      (event) => {
        const { context, code, message } = event.payload;
        lastError = `[${context}] [${code}] ${message}`;
      }
    ).then((fn) => {
      if (destroyed) fn();
      else unlistenAutoDisconnectFailure = fn;
    });

    return () => {
      destroyed = true;
      unlistenControllerChanged?.();
      unlistenAutoDisconnectFailure?.();
    };
  });
</script>

<!-- ── Page header ──────────────────────────────────────────────────────── -->
<div class="page-header">
  <h1 class="page-title">Dashboard</h1>
  <p class="page-subtitle">recording status &amp; controls</p>
</div>

<!-- ── Recording status ─────────────────────────────────────────────────── -->
<section class="card">
  <h2 class="card__title">Session</h2>

  <div class="session-status" class:session-status--recording={isCapturing}>
    <span class="rec-dot" class:rec-dot--active={isCapturing}></span>
    <span class="session-label">{isCapturing ? "Recording" : session?.isRunning === false ? "Stopped" : "Idle"}</span>
    {#if session?.sessionId}
      <span class="session-id">{session.sessionId}</span>
    {/if}
  </div>

  {#if session && session.startedAtUnixMs != null}
    <ul class="kv-list kv-list--row">
      <li>
        <span class="kv-key">started</span>
        <span class="kv-val">{formatTimestamp(session.startedAtUnixMs)}</span>
      </li>
      {#if session.requestedSources}
        {#if session.requestedSources.screen}
          <li><span class="badge badge--ok badge--sm">screen</span></li>
        {/if}
        {#if session.requestedSources.microphone}
          <li><span class="badge badge--ok badge--sm">mic</span></li>
        {/if}
        {#if session.requestedSources.systemAudio}
          <li><span class="badge badge--ok badge--sm">sys-audio</span></li>
        {/if}
      {/if}
    </ul>
  {/if}

  {#if recordingSettings && !isCapturing}
    <div class="settings-preview">
      <span class="settings-preview__label">Using persisted settings</span>
      <div class="settings-preview__badges">
        {#if recordingSettings.captureScreen}
          <span class="badge badge--neutral badge--sm">screen</span>
        {/if}
        {#if recordingSettings.captureMicrophone}
          <span class="badge badge--neutral badge--sm">mic</span>
        {/if}
        {#if recordingSettings.captureSystemAudio}
          <span class="badge badge--neutral badge--sm">sys-audio</span>
        {/if}
        <span class="badge badge--neutral badge--sm">{recordingSettings.screenFrameRate} fps</span>
        <span class="badge badge--neutral badge--sm">{recordingSettings.segmentDurationSeconds}s segments</span>
      </div>
    </div>
  {/if}

  <div class="action-row">
    <button
      class="btn btn--primary btn--lg"
      onclick={startCapture}
      disabled={isCapturing || loadingStart || loadingSettings}
    >
      {loadingStart ? "Starting…" : "Start Recording"}
    </button>
    <button
      class="btn btn--danger btn--lg"
      onclick={stopCapture}
      disabled={!isCapturing || loadingStop}
    >
      {loadingStop ? "Stopping…" : "Stop Recording"}
    </button>
  </div>
</section>

<!-- ── Output files ─────────────────────────────────────────────────────── -->
{#if session?.outputFiles}
  {@const files = session.outputFiles}
  {@const hasFiles = files.screenFile || files.screenFiles.length || files.microphoneFiles.length || files.microphoneFile || files.systemAudioFile || files.systemAudioFiles.length}
  {#if hasFiles}
    <section class="card">
      <h2 class="card__title">Output Files</h2>
      <ul class="output-files">
        {#if files.screenFiles.length}
          {#each files.screenFiles as f, i}
            <li class="output-file">
              <span class="output-file__type">screen[{i}]</span>
              <span class="output-file__path">{f}</span>
            </li>
          {/each}
        {:else if files.screenFile}
          <li class="output-file">
            <span class="output-file__type">screen</span>
            <span class="output-file__path">{files.screenFile}</span>
          </li>
        {/if}
        {#if files.microphoneFiles.length}
          {#each files.microphoneFiles as f, i}
            <li class="output-file">
              <span class="output-file__type">mic[{i}]</span>
              <span class="output-file__path">{f}</span>
            </li>
          {/each}
        {:else if files.microphoneFile}
          <li class="output-file">
            <span class="output-file__type">mic[0]</span>
            <span class="output-file__path">{files.microphoneFile}</span>
          </li>
        {/if}
        {#if files.systemAudioFiles.length}
          {#each files.systemAudioFiles as f, i}
            <li class="output-file">
              <span class="output-file__type">sys-audio[{i}]</span>
              <span class="output-file__path">{f}</span>
            </li>
          {/each}
        {:else if files.systemAudioFile}
          <li class="output-file">
            <span class="output-file__type">sys-audio</span>
            <span class="output-file__path">{files.systemAudioFile}</span>
          </li>
        {/if}
      </ul>
    </section>
  {/if}
{/if}

<!-- ── System probe ──────────────────────────────────────────────────────── -->
<section class="card">
  <h2 class="card__title">System Probe</h2>
  <div class="probe-grid">
    <div class="probe-block">
      <div class="probe-block__header">
        <span class="probe-block__name">Support</span>
        <button class="btn btn--ghost btn--sm" onclick={loadSupport} disabled={loadingSupport}>
          {loadingSupport ? "…" : "Query"}
        </button>
      </div>
      {#if support}
        <ul class="kv-list">
          <li>
            <span class="kv-key">platform</span>
            <span class="kv-val">{support.platform}</span>
          </li>
          <li>
            <span class="kv-key">native</span>
            <span class={supportBadge(support.nativeCaptureSupported)}>
              {support.nativeCaptureSupported ? "yes" : "no"}
            </span>
          </li>
          <li>
            <span class="kv-key">screen</span>
            <span class={supportBadge(support.supportedSources.screen)}>
              {support.supportedSources.screen ? "yes" : "no"}
            </span>
          </li>
          <li>
            <span class="kv-key">mic</span>
            <span class={supportBadge(support.supportedSources.microphone)}>
              {support.supportedSources.microphone ? "yes" : "no"}
            </span>
          </li>
          <li>
            <span class="kv-key">sys-audio</span>
            <span class={supportBadge(support.supportedSources.systemAudio)}>
              {support.supportedSources.systemAudio ? "yes" : "no"}
            </span>
          </li>
        </ul>
      {:else}
        <p class="empty">—</p>
      {/if}
    </div>

    <div class="probe-block">
      <div class="probe-block__header">
        <span class="probe-block__name">Permissions</span>
        <button class="btn btn--ghost btn--sm" onclick={loadPermissions} disabled={loadingPermissions}>
          {loadingPermissions ? "…" : "Query"}
        </button>
      </div>
      {#if permissions}
        <ul class="kv-list">
          <li>
            <span class="kv-key">screen</span>
            <span class={permissionBadgeClass(permissions.screen)}>
              {formatPermission(permissions.screen)}
            </span>
          </li>
          <li>
            <span class="kv-key">mic</span>
            <span class={permissionBadgeClass(permissions.microphone)}>
              {formatPermission(permissions.microphone)}
            </span>
          </li>
          <li>
            <span class="kv-key">sys-audio</span>
            <span class={permissionBadgeClass(permissions.systemAudio)}>
              {formatPermission(permissions.systemAudio)}
            </span>
          </li>
        </ul>
      {:else}
        <p class="empty">—</p>
      {/if}
    </div>
  </div>
</section>

<!-- ── Error display ─────────────────────────────────────────────────────── -->
{#if lastError}
  <section class="card card--error">
    <h2 class="card__title">
      Error
      <button class="btn btn--ghost btn--sm" onclick={() => lastError = null}>dismiss</button>
    </h2>
    <pre class="error-pre">{lastError}</pre>
  </section>
{/if}

<style>
  /* ── Page header ───────────────────────────────────────────── */
  .page-header {
    margin-bottom: 4px;
  }

  .page-title {
    font-size: 18px;
    font-weight: 700;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: #f0f0f5;
  }

  .page-subtitle {
    font-size: 10px;
    color: #44445a;
    letter-spacing: 0.06em;
    margin-top: 2px;
  }

  /* ── Cards ─────────────────────────────────────────────────── */
  .card {
    background: #13131a;
    border: 1px solid #1e1e2e;
    border-radius: 6px;
    padding: 20px;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }

  .card--error {
    border-color: #3a1a20;
    background: #0e0a0a;
  }

  .card__title {
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: #44445a;
    display: flex;
    align-items: center;
    gap: 8px;
  }

  /* ── Session status ─────────────────────────────────────────── */
  .session-status {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 14px;
    background: #0d0d14;
    border: 1px solid #1e1e2e;
    border-radius: 5px;
    transition: background 0.2s, border-color 0.2s;
  }

  .session-status--recording {
    background: #0a1410;
    border-color: #1a3020;
  }

  .rec-dot {
    display: block;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: #33334a;
    flex-shrink: 0;
    transition: background 0.2s;
  }

  .rec-dot--active {
    background: #ff4455;
    animation: pulse-rec 1.2s ease-in-out infinite;
  }

  @keyframes pulse-rec {
    0%, 100% { opacity: 1; box-shadow: 0 0 0 0 rgba(255, 68, 85, 0.4); }
    50% { opacity: 0.7; box-shadow: 0 0 0 5px rgba(255, 68, 85, 0); }
  }

  .session-label {
    font-size: 12px;
    font-weight: 700;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: #44445a;
    transition: color 0.2s;
  }

  .session-status--recording .session-label {
    color: #ff6070;
  }

  .session-id {
    font-size: 10px;
    color: #33334a;
    margin-left: auto;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 200px;
  }

  /* ── Settings preview ───────────────────────────────────────── */
  .settings-preview {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .settings-preview__label {
    font-size: 10px;
    color: #33334a;
    letter-spacing: 0.06em;
  }

  .settings-preview__badges {
    display: flex;
    flex-wrap: wrap;
    gap: 5px;
  }

  /* ── Action row ─────────────────────────────────────────────── */
  .action-row {
    display: flex;
    gap: 10px;
    flex-wrap: wrap;
  }

  /* ── Buttons ────────────────────────────────────────────────── */
  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 8px 16px;
    border-radius: 4px;
    font-family: inherit;
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    cursor: pointer;
    border: 1px solid transparent;
    transition: background 0.12s, border-color 0.12s, opacity 0.12s;
    outline: none;
  }

  .btn:disabled {
    opacity: 0.35;
    cursor: not-allowed;
  }

  .btn--lg {
    padding: 10px 20px;
    font-size: 12px;
  }

  .btn--primary {
    background: #0f2e1f;
    color: #3dffa0;
    border-color: #1a4a30;
  }

  .btn--primary:not(:disabled):hover {
    background: #1a3d2a;
    border-color: #3dffa0;
  }

  .btn--danger {
    background: #2e0f14;
    color: #ff6b7a;
    border-color: #4a1a20;
  }

  .btn--danger:not(:disabled):hover {
    background: #3d1a20;
    border-color: #ff6b7a;
  }

  .btn--ghost {
    background: transparent;
    color: #7a7a9a;
    border-color: #2a2a3a;
    font-size: 10px;
  }

  .btn--ghost:not(:disabled):hover {
    background: #1a1a2a;
    color: #a0a0c0;
    border-color: #3a3a5a;
  }

  .btn--sm {
    padding: 3px 8px;
    font-size: 9px;
  }

  /* ── Output files ───────────────────────────────────────────── */
  .output-files {
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 5px;
  }

  .output-file {
    display: flex;
    align-items: baseline;
    gap: 10px;
  }

  .output-file__type {
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: #44445a;
    flex-shrink: 0;
    min-width: 64px;
  }

  .output-file__path {
    font-size: 11px;
    color: #7a7a9a;
    word-break: break-all;
  }

  /* ── Probe grid ─────────────────────────────────────────────── */
  .probe-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 14px;
  }

  .probe-block {
    background: #0e0e16;
    border: 1px solid #1a1a2a;
    border-radius: 4px;
    padding: 12px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .probe-block__header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .probe-block__name {
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: #6a6a88;
  }

  /* ── KV list ────────────────────────────────────────────────── */
  .kv-list {
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 5px;
  }

  .kv-list--row {
    flex-direction: row;
    flex-wrap: wrap;
    gap: 8px;
    align-items: center;
  }

  .kv-list li {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .kv-key {
    color: #3a3a54;
    font-size: 10px;
    white-space: nowrap;
    min-width: 60px;
  }

  .kv-val {
    color: #9090b0;
    font-size: 11px;
  }

  /* ── Badges ─────────────────────────────────────────────────── */
  .badge {
    display: inline-flex;
    align-items: center;
    padding: 1px 6px;
    border-radius: 3px;
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }

  .badge--ok {
    background: #0f2e1f;
    color: #3dffa0;
    border: 1px solid #1a4a30;
  }

  .badge--err {
    background: #2e0f14;
    color: #ff6b7a;
    border: 1px solid #4a1a20;
  }

  .badge--neutral {
    background: #1a1a2a;
    color: #7070a0;
    border: 1px solid #2a2a3a;
  }

  .badge--sm {
    padding: 0 5px;
    font-size: 9px;
  }

  /* ── Error ──────────────────────────────────────────────────── */
  .error-pre {
    background: #0e0a0a;
    border: 1px solid #3a1a20;
    border-radius: 4px;
    padding: 10px 12px;
    font-family: inherit;
    font-size: 11px;
    color: #ff8090;
    white-space: pre-wrap;
    word-break: break-all;
    max-height: 160px;
    overflow-y: auto;
  }

  /* ── Misc ───────────────────────────────────────────────────── */
  .empty {
    color: #2a2a40;
    font-size: 11px;
    font-style: italic;
  }
</style>
