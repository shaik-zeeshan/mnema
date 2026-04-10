<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";

  // ─── Types mirroring Rust structs ─────────────────────────────────────────

  type PermissionStatus = "granted" | "denied" | "not_determined" | "restricted";

  interface SupportedSources {
    screen: boolean;
    microphone: boolean;
    systemAudio: boolean;
  }

  interface CaptureSupport {
    platform: string;
    nativeCaptureSupported: boolean;
    supportedSources: SupportedSources;
  }

  interface PermissionsMap {
    screen: PermissionStatus;
    microphone: PermissionStatus;
    systemAudio: PermissionStatus;
  }

  interface RequestedSources {
    screen: boolean;
    microphone: boolean;
    systemAudio: boolean;
  }

  interface CaptureOutputFiles {
    screenFile: string | null;
    microphoneFile: string | null;
    systemAudioFile: string | null;
  }

  interface CaptureSession {
    isRunning: boolean;
    sessionId: string | null;
    startedAtUnixMs: number | null;
    requestedSources: RequestedSources | null;
    outputFiles: CaptureOutputFiles | null;
  }

  interface GetPermissionsResponse {
    permissions: PermissionsMap;
    session: CaptureSession | null;
  }

  interface StartCaptureResponse {
    session: CaptureSession;
  }

  interface StopCaptureResponse {
    session: CaptureSession;
  }

  // ─── State ────────────────────────────────────────────────────────────────

  let support = $state<CaptureSupport | null>(null);
  let permissions = $state<PermissionsMap | null>(null);
  let session = $state<CaptureSession | null>(null);

  let captureScreen = $state(true);
  let captureMicrophone = $state(false);
  let captureSystemAudio = $state(false);

  let lastResponse = $state<unknown>(null);
  let lastError = $state<string | null>(null);
  let loadingSupport = $state(false);
  let loadingPermissions = $state(false);
  let loadingStart = $state(false);
  let loadingStop = $state(false);

  // ─── Helpers ──────────────────────────────────────────────────────────────

  function clearError() {
    lastError = null;
  }

  function setResponse(data: unknown) {
    lastResponse = data;
    lastError = null;
  }

  function setError(err: unknown) {
    lastError = typeof err === "string" ? err : JSON.stringify(err, null, 2);
    lastResponse = null;
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

  // ─── Actions ──────────────────────────────────────────────────────────────

  async function loadSupport() {
    loadingSupport = true;
    clearError();
    try {
      const result = await invoke<CaptureSupport>("get_capture_support");
      support = result;
      setResponse(result);
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
      if (result.session) {
        session = result.session;
      }
      setResponse(result);
    } catch (err) {
      permissions = null;
      setError(err);
    } finally {
      loadingPermissions = false;
    }
  }

  async function startCapture() {
    loadingStart = true;
    clearError();
    try {
      const result = await invoke<StartCaptureResponse>("start_native_capture", {
        request: {
          captureScreen,
          captureMicrophone,
          captureSystemAudio,
        },
      });
      session = result.session;
      setResponse(result);
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
      const result = await invoke<StopCaptureResponse>("stop_native_capture");
      session = result.session;
      setResponse(result);
    } catch (err) {
      setError(err);
      try {
        const permissionsResult = await invoke<GetPermissionsResponse>("get_capture_permissions");
        permissions = permissionsResult.permissions;
        session = permissionsResult.session;
      } catch {
        // Keep the original stop error visible; best-effort UI sync only.
      }
    } finally {
      loadingStop = false;
    }
  }

  const isCapturing = $derived(session?.isRunning === true);

  function formatTimestamp(ms: number): string {
    return new Date(ms).toLocaleTimeString();
  }
</script>

<main>
  <header>
    <div class="wordmark">
      <span class="wordmark__dot"></span>
      <span class="wordmark__label">capture</span>
    </div>
    <p class="subtitle">native capture control surface · macOS PoC</p>
  </header>

  <!-- ── Probe row ─────────────────────────────────────────────────────── -->
  <section class="card">
    <h2 class="card__title">Probe</h2>
    <div class="probe-grid">
      <!-- Support -->
      <div class="probe-block">
        <div class="probe-block__header">
          <span class="probe-block__name">Support</span>
          <button
            class="btn btn--ghost btn--sm"
            onclick={loadSupport}
            disabled={loadingSupport}
          >
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
              <span class="kv-key">native&nbsp;capture</span>
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
              <span class="kv-key">microphone</span>
              <span class={supportBadge(support.supportedSources.microphone)}>
                {support.supportedSources.microphone ? "yes" : "no"}
              </span>
            </li>
            <li>
              <span class="kv-key">system&nbsp;audio</span>
              <span class={supportBadge(support.supportedSources.systemAudio)}>
                {support.supportedSources.systemAudio ? "yes" : "no"}
              </span>
            </li>
          </ul>
        {:else}
          <p class="empty">—</p>
        {/if}
      </div>

      <!-- Permissions -->
      <div class="probe-block">
        <div class="probe-block__header">
          <span class="probe-block__name">Permissions</span>
          <button
            class="btn btn--ghost btn--sm"
            onclick={loadPermissions}
            disabled={loadingPermissions}
          >
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
              <span class="kv-key">microphone</span>
              <span class={permissionBadgeClass(permissions.microphone)}>
                {formatPermission(permissions.microphone)}
              </span>
            </li>
            <li>
              <span class="kv-key">system&nbsp;audio</span>
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

  <!-- ── Config ────────────────────────────────────────────────────────── -->
  <section class="card">
    <h2 class="card__title">Config</h2>
    <div class="toggle-row">
      <label class="toggle" class:toggle--disabled={isCapturing}>
        <input
          type="checkbox"
          bind:checked={captureScreen}
          disabled={isCapturing}
        />
        <span class="toggle__track"><span class="toggle__thumb"></span></span>
        <span class="toggle__label">Screen</span>
      </label>

      <label class="toggle" class:toggle--disabled={isCapturing}>
        <input
          type="checkbox"
          bind:checked={captureMicrophone}
          disabled={isCapturing}
        />
        <span class="toggle__track"><span class="toggle__thumb"></span></span>
        <span class="toggle__label">Microphone</span>
      </label>

      <label class="toggle" class:toggle--disabled={isCapturing}>
        <input
          type="checkbox"
          bind:checked={captureSystemAudio}
          disabled={isCapturing}
        />
        <span class="toggle__track"><span class="toggle__thumb"></span></span>
        <span class="toggle__label">System Audio</span>
      </label>
    </div>
    {#if isCapturing}
      <p class="hint">Stop the active session to change config.</p>
    {/if}
  </section>

  <!-- ── Session controls ──────────────────────────────────────────────── -->
  <section class="card">
    <h2 class="card__title">Session</h2>

    {#if session}
      <div class="session-status" class:session-status--idle={!session.isRunning}>
        <span class="recording-indicator" class:recording-indicator--idle={!session.isRunning}></span>
        <span class="session-status__label" class:session-status__label--idle={!session.isRunning}>
          {session.isRunning ? "Recording" : "Stopped"}
        </span>
        <span class="session-status__id">{session.sessionId ?? "—"}</span>
      </div>
      <ul class="kv-list kv-list--inline">
        <li>
          <span class="kv-key">started</span>
          <span class="kv-val">{session.startedAtUnixMs != null ? formatTimestamp(session.startedAtUnixMs) : "—"}</span>
        </li>
        {#if session.requestedSources}
          <li>
            <span class="kv-key">screen</span>
            <span class={supportBadge(session.requestedSources.screen)}>
              {session.requestedSources.screen ? "on" : "off"}
            </span>
          </li>
          <li>
            <span class="kv-key">mic</span>
            <span class={supportBadge(session.requestedSources.microphone)}>
              {session.requestedSources.microphone ? "on" : "off"}
            </span>
          </li>
          <li>
            <span class="kv-key">sys&nbsp;audio</span>
            <span class={supportBadge(session.requestedSources.systemAudio)}>
              {session.requestedSources.systemAudio ? "on" : "off"}
            </span>
          </li>
        {/if}
      </ul>
      {#if session.outputFiles?.screenFile || session.outputFiles?.microphoneFile || session.outputFiles?.systemAudioFile}
        <div class="output-files">
          <span class="output-files__label">Capture output files</span>
          <ul class="output-files__list">
            {#if session.outputFiles?.screenFile}
              <li class="output-files__item">screen: {session.outputFiles.screenFile}</li>
            {/if}
            {#if session.outputFiles?.microphoneFile}
              <li class="output-files__item">microphone: {session.outputFiles.microphoneFile}</li>
            {/if}
            {#if session.outputFiles?.systemAudioFile}
              <li class="output-files__item">system-audio: {session.outputFiles.systemAudioFile}</li>
            {/if}
          </ul>
        </div>
      {/if}
    {:else}
      <p class="empty">No active session.</p>
    {/if}

    <div class="action-row">
      <button
        class="btn btn--primary"
        onclick={startCapture}
        disabled={isCapturing || loadingStart}
      >
        {loadingStart ? "Starting…" : "Start Capture"}
      </button>
      <button
        class="btn btn--danger"
        onclick={stopCapture}
        disabled={!isCapturing || loadingStop}
      >
        {loadingStop ? "Stopping…" : "Stop Capture"}
      </button>
    </div>
  </section>

  <!-- ── Response inspector ─────────────────────────────────────────────── -->
  <section class="card card--inspector">
    <h2 class="card__title">
      Last Response
      {#if lastError}<span class="badge badge--err badge--sm">error</span>{/if}
    </h2>
    {#if lastError}
      <pre class="inspector inspector--error">{lastError}</pre>
    {:else if lastResponse !== null}
      <pre class="inspector">{JSON.stringify(lastResponse, null, 2)}</pre>
    {:else}
      <p class="empty">No response yet. Use the controls above.</p>
    {/if}
  </section>
</main>

<style>
  /* ── Reset / global ──────────────────────────────────────────────────── */
  :global(*, *::before, *::after) {
    box-sizing: border-box;
    margin: 0;
    padding: 0;
  }

  :global(html) {
    height: 100%;
  }

  :global(body) {
    min-height: 100%;
    background-color: #0c0c0e;
    color: #e2e2e8;
    font-family: "Berkeley Mono", "TX-02", "Monaspace Neon", ui-monospace,
      "Cascadia Code", "Fira Code", monospace;
    font-size: 13px;
    line-height: 1.6;
    -webkit-font-smoothing: antialiased;
  }

  /* ── Layout ──────────────────────────────────────────────────────────── */
  main {
    max-width: 640px;
    margin: 0 auto;
    padding: 32px 20px 64px;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  /* ── Header ──────────────────────────────────────────────────────────── */
  header {
    margin-bottom: 8px;
  }

  .wordmark {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-bottom: 4px;
  }

  .wordmark__dot {
    display: block;
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: #3dffa0;
    flex-shrink: 0;
  }

  .wordmark__label {
    font-size: 20px;
    font-weight: 700;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: #f0f0f5;
  }

  .subtitle {
    color: #555566;
    font-size: 11px;
    letter-spacing: 0.06em;
    padding-left: 20px;
  }

  /* ── Cards ───────────────────────────────────────────────────────────── */
  .card {
    background: #13131a;
    border: 1px solid #1e1e2e;
    border-radius: 6px;
    padding: 20px;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }

  .card--inspector {
    border-color: #1a1a2e;
  }

  .card__title {
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: #55556a;
    display: flex;
    align-items: center;
    gap: 8px;
  }

  /* ── Probe grid ──────────────────────────────────────────────────────── */
  .probe-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 16px;
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
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: #7a7a92;
  }

  /* ── KV list ─────────────────────────────────────────────────────────── */
  .kv-list {
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 5px;
  }

  .kv-list--inline {
    flex-direction: row;
    flex-wrap: wrap;
    gap: 8px;
  }

  .kv-list li {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .kv-key {
    color: #4a4a60;
    font-size: 11px;
    white-space: nowrap;
    min-width: 80px;
  }

  .kv-val {
    color: #c0c0d0;
  }

  /* ── Badges ──────────────────────────────────────────────────────────── */
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
    color: #8888a8;
    border: 1px solid #2a2a3a;
  }

  .badge--sm {
    padding: 0px 5px;
    font-size: 9px;
  }

  /* ── Toggles ─────────────────────────────────────────────────────────── */
  .toggle-row {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .toggle {
    display: flex;
    align-items: center;
    gap: 10px;
    cursor: pointer;
    user-select: none;
  }

  .toggle--disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  .toggle input {
    position: absolute;
    opacity: 0;
    width: 0;
    height: 0;
  }

  .toggle__track {
    position: relative;
    display: block;
    width: 32px;
    height: 18px;
    background: #1a1a2a;
    border: 1px solid #2a2a3a;
    border-radius: 9px;
    transition: background 0.15s, border-color 0.15s;
    flex-shrink: 0;
  }

  .toggle__thumb {
    position: absolute;
    top: 2px;
    left: 2px;
    width: 12px;
    height: 12px;
    border-radius: 50%;
    background: #44445a;
    transition: transform 0.15s, background 0.15s;
  }

  .toggle input:checked ~ .toggle__track {
    background: #0f2e1f;
    border-color: #1a4a30;
  }

  .toggle input:checked ~ .toggle__track .toggle__thumb {
    transform: translateX(14px);
    background: #3dffa0;
  }

  .toggle__label {
    color: #b0b0c8;
    font-size: 12px;
  }

  /* ── Session status ──────────────────────────────────────────────────── */
  .session-status {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 12px;
    background: #0f1e14;
    border: 1px solid #1a3020;
    border-radius: 4px;
  }

  .session-status--idle {
    background: #141420;
    border-color: #1e1e30;
  }

  .recording-indicator {
    display: block;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: #ff4455;
    flex-shrink: 0;
    animation: pulse 1.2s ease-in-out infinite;
  }

  .recording-indicator--idle {
    background: #44445a;
    animation: none;
  }

  @keyframes pulse {
    0%, 100% { opacity: 1; box-shadow: 0 0 0 0 rgba(255, 68, 85, 0.4); }
    50% { opacity: 0.7; box-shadow: 0 0 0 5px rgba(255, 68, 85, 0); }
  }

  .session-status__label {
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: #ff4455;
  }

  .session-status__label--idle {
    color: #55556a;
  }

  .session-status__id {
    font-size: 10px;
    color: #44445a;
    margin-left: auto;
  }

  /* ── Output files ────────────────────────────────────────────────────── */
  .output-files {
    background: #0a0a12;
    border: 1px solid #1a1a2a;
    border-radius: 4px;
    padding: 10px 12px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .output-files__label {
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: #44445a;
  }

  .output-files__list {
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }

  .output-files__item {
    font-size: 11px;
    color: #8888aa;
    word-break: break-all;
  }

  /* ── Buttons ─────────────────────────────────────────────────────────── */
  .action-row {
    display: flex;
    gap: 10px;
    flex-wrap: wrap;
  }

  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 8px 16px;
    border-radius: 4px;
    font-family: inherit;
    font-size: 12px;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    cursor: pointer;
    border: 1px solid transparent;
    transition: background 0.12s, border-color 0.12s, opacity 0.12s;
  }

  .btn:disabled {
    opacity: 0.35;
    cursor: not-allowed;
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
  }

  .btn--ghost:not(:disabled):hover {
    background: #1a1a2a;
    color: #a0a0c0;
    border-color: #3a3a5a;
  }

  .btn--sm {
    padding: 4px 10px;
    font-size: 10px;
  }

  /* ── Inspector ───────────────────────────────────────────────────────── */
  .inspector {
    background: #0a0a12;
    border: 1px solid #1a1a2a;
    border-radius: 4px;
    padding: 12px;
    font-family: inherit;
    font-size: 11px;
    line-height: 1.7;
    color: #8888aa;
    white-space: pre-wrap;
    word-break: break-all;
    max-height: 200px;
    overflow-y: auto;
  }

  .inspector--error {
    border-color: #3a1a20;
    color: #ff8090;
    background: #0e0a0a;
  }

  /* ── Misc ────────────────────────────────────────────────────────────── */
  .empty {
    color: #33334a;
    font-size: 11px;
    font-style: italic;
  }

  .hint {
    color: #44445a;
    font-size: 11px;
    font-style: italic;
  }

  /* ── Scrollbar ───────────────────────────────────────────────────────── */
  .inspector::-webkit-scrollbar {
    width: 4px;
  }
  .inspector::-webkit-scrollbar-track {
    background: transparent;
  }
  .inspector::-webkit-scrollbar-thumb {
    background: #2a2a3a;
    border-radius: 2px;
  }
</style>
