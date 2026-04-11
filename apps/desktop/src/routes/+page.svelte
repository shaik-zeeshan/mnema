<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";

  // ─── Types mirroring Rust structs ─────────────────────────────────────────

  type PermissionStatus = "granted" | "denied" | "not_determined" | "restricted";

  // Microphone controller types (serialized from Rust via serde)
  type MicrophonePreferenceMode = "default" | "specific_device";
  type MicrophoneDisconnectPolicy = "fallback_to_default" | "wait_for_same_device";

  interface MicrophoneDevice {
    id: string;
    name: string;
    isDefault: boolean;
  }

  interface MicrophonePreference {
    mode: MicrophonePreferenceMode;
    deviceId: string | null;
  }

  interface MicrophoneControllerState {
    devices: MicrophoneDevice[];
    preference: MicrophonePreference;
    disconnectPolicy: MicrophoneDisconnectPolicy;
    effectiveDevice: MicrophoneDevice | null;
  }

  interface MicrophoneAutoDisconnectTransitionFailedEvent {
    context: string;
    code: string;
    message: string;
  }

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
    microphoneFiles: string[];
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

  // Microphone controller state
  let micState = $state<MicrophoneControllerState | null>(null);
  let loadingMicState = $state(false);
  let loadingMicUpdate = $state(false);
  // Draft edits (pending apply)
  let draftPreferenceMode = $state<MicrophonePreferenceMode>("default");
  let draftDeviceId = $state<string | null>(null);
  let draftDisconnectPolicy = $state<MicrophoneDisconnectPolicy>("fallback_to_default");

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

  // ─── Microphone controller actions ────────────────────────────────────────

  function syncDraftsFromMicState(state: MicrophoneControllerState) {
    draftPreferenceMode = state.preference.mode;
    draftDeviceId = state.preference.deviceId ?? null;
    draftDisconnectPolicy = state.disconnectPolicy;
  }

  async function loadMicState() {
    loadingMicState = true;
    clearError();
    try {
      const result = await invoke<MicrophoneControllerState>("get_microphone_controller_state");
      micState = result;
      syncDraftsFromMicState(result);
      setResponse(result);
    } catch (err) {
      setError(err);
    } finally {
      loadingMicState = false;
    }
  }

  async function applyMicSettings() {
    loadingMicUpdate = true;
    clearError();
    try {
      const result = await invoke<MicrophoneControllerState>("update_microphone_controller", {
        request: {
          preference: {
            mode: draftPreferenceMode,
            deviceId: draftPreferenceMode === "specific_device" ? draftDeviceId : null,
          },
          disconnectPolicy: draftDisconnectPolicy,
        },
      });
      micState = result;
      syncDraftsFromMicState(result);
      setResponse(result);
    } catch (err) {
      setError(err);
    } finally {
      loadingMicUpdate = false;
    }
  }

  const isCapturing = $derived(session?.isRunning === true);

  // Block Apply when Specific Device mode has no real device selected.
  const applyBlocked = $derived(
    draftPreferenceMode === "specific_device" && !draftDeviceId
  );

  // Load initial state and subscribe to backend-driven change notifications.
  $effect(() => {
    let unlistenControllerChanged: (() => void) | undefined;
    let unlistenAutoDisconnectFailure: (() => void) | undefined;
    let destroyed = false;
    // Fetch state immediately on mount so the panel is populated without
    // requiring a manual Refresh click.
    loadMicState();
    listen<MicrophoneControllerState>("microphone_controller_changed", (event) => {
      micState = event.payload;
      syncDraftsFromMicState(event.payload);
      lastError = null;
    }).then((fn) => {
      // If the effect already cleaned up before listen() resolved, unsubscribe
      // immediately to prevent a permanent leak.
      if (destroyed) {
        fn();
      } else {
        unlistenControllerChanged = fn;
      }
    });

    listen<MicrophoneAutoDisconnectTransitionFailedEvent>(
      "microphone_auto_disconnect_transition_failed",
      (event) => {
        const { context, code, message } = event.payload;
        lastError = `[${context}] [${code}] ${message}`;
      }
    ).then((fn) => {
      if (destroyed) {
        fn();
      } else {
        unlistenAutoDisconnectFailure = fn;
      }
    });
    return () => {
      destroyed = true;
      unlistenControllerChanged?.();
      unlistenAutoDisconnectFailure?.();
    };
  });

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

  <!-- ── Microphone Controller ─────────────────────────────────────────── -->
  <section class="card">
    <h2 class="card__title">
      Microphone Controller
      <button
        class="btn btn--ghost btn--sm"
        onclick={loadMicState}
        disabled={loadingMicState}
        style="margin-left: auto;"
      >
        {loadingMicState ? "…" : "Refresh"}
      </button>
    </h2>

    {#if micState}
      <!-- Effective device banner -->
      <div class="mic-effective">
        <span class="kv-key">effective</span>
        {#if micState.effectiveDevice}
          <span class="kv-val">{micState.effectiveDevice.name}</span>
          {#if micState.effectiveDevice.isDefault}
            <span class="badge badge--neutral badge--sm">default</span>
          {/if}
        {:else}
          <span class="empty">none</span>
        {/if}
      </div>

      <!-- Available devices list -->
      {#if micState.devices.length > 0}
        <div class="mic-devices">
          <span class="mic-section-label">Available Devices</span>
          <ul class="kv-list">
            {#each micState.devices as device (device.id)}
              <li>
                <span class="kv-val mic-device-name">{device.name}</span>
                {#if device.isDefault}
                  <span class="badge badge--neutral badge--sm">default</span>
                {/if}
                {#if micState.effectiveDevice?.id === device.id}
                  <span class="badge badge--ok badge--sm">active</span>
                {/if}
              </li>
            {/each}
          </ul>
        </div>
      {:else}
        <p class="empty">No microphone devices found.</p>
      {/if}

      <!-- Preference controls -->
      <div class="mic-controls">
        <span class="mic-section-label">Preference</span>
        <div class="radio-group">
          <label class="radio-label">
            <input
              type="radio"
              name="mic-mode"
              value="default"
              bind:group={draftPreferenceMode}
            />
            <span class="radio-text">System Default</span>
          </label>
          <label class="radio-label">
            <input
              type="radio"
              name="mic-mode"
              value="specific_device"
              bind:group={draftPreferenceMode}
            />
            <span class="radio-text">Specific Device</span>
          </label>
        </div>

        {#if draftPreferenceMode === "specific_device"}
          <div class="mic-select-wrap">
            <select
              class="mic-select"
              class:mic-select--warn={!draftDeviceId}
              bind:value={draftDeviceId}
            >
              <option value={null}>— pick a device —</option>
              {#each micState.devices as device (device.id)}
                <option value={device.id}>{device.name}{device.isDefault ? " (default)" : ""}</option>
              {/each}
            </select>
          </div>
        {/if}
      </div>

      <!-- Disconnect policy controls -->
      <div class="mic-controls">
        <span class="mic-section-label">On Disconnect</span>
        <div class="radio-group">
          <label class="radio-label">
            <input
              type="radio"
              name="mic-disconnect"
              value="fallback_to_default"
              bind:group={draftDisconnectPolicy}
            />
            <span class="radio-text">Fallback to Default</span>
          </label>
          <label class="radio-label">
            <input
              type="radio"
              name="mic-disconnect"
              value="wait_for_same_device"
              bind:group={draftDisconnectPolicy}
            />
            <span class="radio-text">Wait for Same Device</span>
          </label>
        </div>
      </div>

      <div class="action-row">
        <button
          class="btn btn--primary"
          onclick={applyMicSettings}
          disabled={loadingMicUpdate || applyBlocked}
        >
          {loadingMicUpdate ? "Applying…" : "Apply"}
        </button>
      </div>
      {#if applyBlocked}
        <p class="hint hint--warn">Select a device before applying Specific Device mode.</p>
      {/if}
    {:else}
      <p class="empty">No state loaded. Use Refresh to query the backend.</p>
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
      {#if session.outputFiles?.screenFile || session.outputFiles?.microphoneFiles?.length || session.outputFiles?.microphoneFile || session.outputFiles?.systemAudioFile}
        <div class="output-files">
          <span class="output-files__label">Capture output files</span>
          <ul class="output-files__list">
            {#if session.outputFiles?.screenFile}
              <li class="output-files__item">screen: {session.outputFiles.screenFile}</li>
            {/if}
            {#if session.outputFiles?.microphoneFiles?.length}
              {#each session.outputFiles.microphoneFiles as microphoneFile, index}
                <li class="output-files__item">microphone[{index}]: {microphoneFile}</li>
              {/each}
            {:else if session.outputFiles?.microphoneFile}
              <li class="output-files__item">microphone[0]: {session.outputFiles.microphoneFile}</li>
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

  .hint--warn {
    color: #c47a30;
    font-style: normal;
    font-weight: 600;
  }

  /* ── Microphone Controller ───────────────────────────────────────────── */
  .mic-effective {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    background: #0e0e16;
    border: 1px solid #1a1a2a;
    border-radius: 4px;
  }

  .mic-devices {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .mic-section-label {
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: #44445a;
    display: block;
    margin-bottom: 2px;
  }

  .mic-device-name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .mic-controls {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .radio-group {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .radio-label {
    display: flex;
    align-items: center;
    gap: 8px;
    cursor: pointer;
    user-select: none;
  }

  .radio-label input[type="radio"] {
    appearance: none;
    -webkit-appearance: none;
    width: 14px;
    height: 14px;
    border: 1px solid #2a2a3a;
    border-radius: 50%;
    background: #1a1a2a;
    flex-shrink: 0;
    position: relative;
    cursor: pointer;
    transition: border-color 0.12s, background 0.12s;
  }

  .radio-label input[type="radio"]:checked {
    background: #0f2e1f;
    border-color: #3dffa0;
  }

  .radio-label input[type="radio"]:checked::after {
    content: "";
    display: block;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: #3dffa0;
    position: absolute;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
  }

  .radio-text {
    color: #b0b0c8;
    font-size: 12px;
  }

  .mic-select-wrap {
    margin-top: 2px;
  }

  .mic-select {
    width: 100%;
    background: #0e0e16;
    border: 1px solid #2a2a3a;
    border-radius: 4px;
    padding: 6px 10px;
    color: #c0c0d0;
    font-family: inherit;
    font-size: 12px;
    cursor: pointer;
    outline: none;
    transition: border-color 0.12s;
    appearance: none;
    -webkit-appearance: none;
  }

  .mic-select:focus {
    border-color: #3dffa0;
  }

  .mic-select--warn {
    border-color: #7a4a18;
    color: #c47a30;
  }

  .mic-select--warn:focus {
    border-color: #c47a30;
  }

  .mic-select option {
    background: #13131a;
    color: #c0c0d0;
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
