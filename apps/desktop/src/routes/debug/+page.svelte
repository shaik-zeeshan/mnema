<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import type {
    CaptureSupport,
    CaptureSession,
    GetPermissionsResponse,
    IdleDebugInfo,
    MicrophoneControllerState,
    MicrophoneAutoDisconnectTransitionFailedEvent,
    PermissionsMap,
    PermissionStatus,
    RecordingSettings,
    AppInfraStatus,
    AppJobDto,
    BackgroundJobStatus,
  } from "$lib/types";
  import { captureSession, setSession } from "$lib/session.svelte";

  // ─── State ────────────────────────────────────────────────────────────────

  let support = $state<CaptureSupport | null>(null);
  let permissions = $state<PermissionsMap | null>(null);
  let recordingSettings = $state<RecordingSettings | null>(null);

  // Read-only alias — writes go through captureSession.value so the shared
  // store (and the layout's activity reporter) always see fresh state.
  const session = $derived(captureSession.value);

  // Generation counter that increments on every *authoritative* session write
  // (start / stop).  Reconciliation polling captures the value before its
  // async IPC and skips the write if the generation advanced while in-flight,
  // preventing a slow response from overwriting a newer stopped state.
  let sessionGeneration = $state(0);

  let lastError = $state<string | null>(null);
  let loadingSupport = $state(false);
  let loadingPermissions = $state(false);
  let loadingStart = $state(false);
  let loadingStop = $state(false);
  let loadingSettings = $state(false);

  // ─── Idle debug ──────────────────────────────────────────────────────────
  let idleDebug = $state<IdleDebugInfo | null>(null);
  let idleDebugError = $state<string | null>(null);

  async function fetchIdleDebug() {
    // Skip the round-trip when the page is hidden or no capture session is active —
    // the debug panel is only meaningful while recording (or briefly after stop).
    if (typeof document !== "undefined" && document.visibilityState === "hidden") return;
    if (!session?.isRunning) return;
    try {
      idleDebug = await invoke<IdleDebugInfo>("get_idle_debug");
      idleDebugError = null;
    } catch (err) {
      idleDebugError = typeof err === "string" ? err : JSON.stringify(err);
    }
  }

  function formatIdleMs(ms: number | null | undefined): string {
    if (ms == null) return "unavailable";
    if (ms < 1000) return `${ms} ms`;
    return `${(ms / 1000).toFixed(1)} s`;
  }

  /**
   * Human-readable label for the activity mode, clarifying hybrid behaviour.
   */
  function formatActivityMode(mode: string): string {
    if (mode === "system_input_only") return "input-only";
    if (mode === "system_input_or_screen") return "hybrid (input + screen)";
    if (mode === "system_input_or_screen_or_audio") return "audio (input + screen + audio)";
    return mode;
  }

  /**
   * Human-readable label for the effective idle source.
   */
  function formatEffectiveSource(src: string): string {
    if (src === "system_input") return "system input";
    if (src === "screen_capture") return "screen activity";
    if (src === "microphone_capture") return "microphone audio";
    if (src === "system_audio_capture") return "system audio";
    if (src === "internal_fallback") return "internal fallback";
    return src;
  }

  function sourceKindLabel(src: string): string {
    if (src === "system_input") return "system input";
    if (src === "screen_capture") return "screen activity";
    if (src === "microphone_capture") return "microphone audio";
    if (src === "system_audio_capture") return "system audio";
    if (src === "internal_fallback") return "internal fallback";
    return src;
  }

  const micActivitySource = $derived(idleDebug?.activitySources?.find((s) => s.kind === "microphone_capture"));
  const sysAudioActivitySource = $derived(idleDebug?.activitySources?.find((s) => s.kind === "system_audio_capture"));

  type RuntimeSourceLane = {
    key: "screen" | "microphone" | "systemAudio";
    label: string;
    glyph: string;
    sample: { lastUnixMs: number | null; idleMs: number | null; level: number | null } | null;
    qualifiedIdleMs: number | null;
    qualifiedThreshold: number | null;
  };

  const runtimeLanes = $derived<RuntimeSourceLane[]>(
    idleDebug
      ? [
          {
            key: "screen",
            label: "Screen",
            glyph: "◉",
            sample: idleDebug.screenActivityLastUnixMs != null
              ? { lastUnixMs: idleDebug.screenActivityLastUnixMs, idleMs: idleDebug.screenActivityIdleMs, level: null }
              : null,
            qualifiedIdleMs: idleDebug.screenActivityIdleMs,
            qualifiedThreshold: null,
          },
          {
            key: "microphone",
            label: "Microphone",
            glyph: "🎙",
            sample: { lastUnixMs: idleDebug.microphoneActivityLastUnixMs, idleMs: null, level: idleDebug.microphoneActivityLevel },
            qualifiedIdleMs: idleDebug.microphoneActivityIdleMs,
            qualifiedThreshold: idleDebug.microphoneActivityThreshold,
          },
          {
            key: "systemAudio",
            label: "System Audio",
            glyph: "🔊",
            sample: { lastUnixMs: idleDebug.systemAudioActivityLastUnixMs, idleMs: null, level: idleDebug.systemAudioActivityLevel },
            qualifiedIdleMs: idleDebug.systemAudioActivityIdleMs,
            qualifiedThreshold: idleDebug.systemAudioActivityThreshold,
          },
        ]
      : []
  );

  /** Compact runtime status word for a source family. */
  function runtimeStateWord(src: { requested: boolean; paused: boolean; sessionActive: boolean | null; writerActive: boolean | null; reason: string | null }): { word: string; cls: string } {
    if (!src.requested) return { word: "off", cls: "rs-state rs-state--off" };
    if (src.sessionActive === null) return { word: src.reason ?? "unknown", cls: "rs-state rs-state--unknown" };
    if (src.paused) return { word: "paused", cls: "rs-state rs-state--paused" };
    if (src.sessionActive && src.writerActive) return { word: "running", cls: "rs-state rs-state--running" };
    if (src.sessionActive && !src.writerActive) return { word: "session only", cls: "rs-state rs-state--partial" };
    return { word: "idle", cls: "rs-state rs-state--idle" };
  }

  function shortenPath(p: string | null | undefined, max = 48): string {
    if (!p) return "—";
    if (p.length <= max) return p;
    const head = p.slice(0, 12);
    const tail = p.slice(-(max - 12 - 1));
    return `${head}…${tail}`;
  }

  function sourceDecisionSummary(available: boolean, selected: boolean, enabled?: boolean): string {
    if (selected) return "selected";
    if (!available) return "unavailable";
    if (enabled === false) return "available, disabled";
    return "available, not selected";
  }

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

  function formatSourceStartedAt(ms: number | null | undefined): string {
    return ms != null ? formatTimestamp(ms) : "—";
  }

  type CaptureSource = "screen" | "microphone" | "systemAudio";
  type SourceSessionLookup = Partial<Record<CaptureSource, { sessionId: string; startedAtUnixMs: number } | null>>;

  function getSourceSession(
    captureSessionValue: CaptureSession | null | undefined,
    source: CaptureSource
  ) {
    const sourceSessions = (captureSessionValue as { sourceSessions?: SourceSessionLookup | null } | null)
      ?.sourceSessions;
    return sourceSessions?.[source] ?? null;
  }

  function getSourceSessionId(
    captureSessionValue: CaptureSession | null | undefined,
    source: CaptureSource
  ): string {
    return getSourceSession(captureSessionValue, source)?.sessionId ?? "—";
  }

  function getSourceSessionStartedAt(
    captureSessionValue: CaptureSession | null | undefined,
    source: CaptureSource
  ): number | null {
    return getSourceSession(captureSessionValue, source)?.startedAtUnixMs ?? null;
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
    const gen = sessionGeneration;
    try {
      const result = await invoke<GetPermissionsResponse>("get_capture_permissions");
      permissions = result.permissions;
      // Only apply the session when no authoritative action (start/stop)
      // occurred while this request was in-flight.
      if (result.session && sessionGeneration === gen) setSession(result.session);
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
      sessionGeneration += 1;
      setSession(result.session);
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
      sessionGeneration += 1;
      setSession(result.session);
    } catch (err) {
      setError(err);
      try {
        const r = await invoke<GetPermissionsResponse>("get_capture_permissions");
        permissions = r.permissions;
        if (r.session) {
          sessionGeneration += 1;
          setSession(r.session);
        }
      } catch { /* best-effort */ }
    } finally {
      loadingStop = false;
    }
  }

  const isCapturing = $derived(session?.isRunning === true);
  const isInactivityPaused = $derived(session?.isInactivityPaused === true);

  // ─── Init ─────────────────────────────────────────────────────────────────
  // Inactivity detection is handled natively by the backend (macOS system-wide
  // idle). This effect only handles data loading and microphone event listeners.
  // NOTE: fetchIdleDebug() reads session?.isRunning, so it must NOT be called
  // synchronously here — doing so would make `session` a reactive dependency of
  // this effect, causing loadPermissions() to re-run on every session change and
  // flickering the Start Recording button's disabled state.

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

  // ─── Idle debug polling ────────────────────────────────────────────────────
  // Kept in a separate effect so that its session-reactivity (fetchIdleDebug
  // reads session?.isRunning) never triggers a re-run of the init effect above.

  $effect(() => {
    fetchIdleDebug();

    const idleDebugInterval = setInterval(fetchIdleDebug, 2000);

    return () => {
      clearInterval(idleDebugInterval);
    };
  });

  // ─── Session reconciliation polling ───────────────────────────────────────
  // While the UI believes capture is running, periodically re-fetch the
  // session from the backend so that an unexpected stop (crash, timeout, etc.)
  // is reflected in the shared session store.  Isolated in its own $effect so
  // that `isCapturing` reactivity here never causes the init effect to re-run.

  $effect(() => {
    // Capture the reactive dep — only poll while the UI thinks we're recording.
    if (!isCapturing) return;

    const RECONCILE_MS = 5_000;

    async function reconcileSession() {
      // Skip when the tab is hidden — avoids unnecessary IPC while inactive.
      if (typeof document !== "undefined" && document.visibilityState === "hidden") return;
      // Snapshot the generation before the async round-trip.  If an
      // authoritative action (start/stop) lands while this request is
      // in-flight, the generation will have advanced and we must discard
      // this (now-stale) response to avoid overwriting the newer state.
      const gen = sessionGeneration;
      try {
        const r = await invoke<GetPermissionsResponse>("get_capture_permissions");
        if (sessionGeneration !== gen) return; // stale — discard
        if (r.session) setSession(r.session);
      } catch {
        // Best-effort — a transient backend error should not crash the UI.
      }
    }

    const interval = setInterval(reconcileSession, RECONCILE_MS);

    return () => {
      clearInterval(interval);
    };
  });

  // ─── App Infra ────────────────────────────────────────────────────────────

  let infraStatus = $state<AppInfraStatus | null>(null);
  let infraStatusError = $state<string | null>(null);
  let loadingInfraStatus = $state(false);

  let jobs = $state<AppJobDto[]>([]);
  let jobsError = $state<string | null>(null);
  let loadingJobs = $state(false);

  let selectedJobId = $state<number | null>(null);
  let selectedJob = $state<AppJobDto | null>(null);
  let loadingSelectedJob = $state(false);
  let selectedJobError = $state<string | null>(null);

  let submitDocName = $state("");
  let submitSourceText = $state("");
  let submitting = $state(false);
  let submitError = $state<string | null>(null);

  // Tracks the active post-submit polling interval so it can be cancelled.
  let postSubmitPollInterval = $state<ReturnType<typeof setInterval> | null>(null);
  let postSubmitPollCount = $state(0);
  const POST_SUBMIT_POLL_MAX = 8;  // poll up to ~8s after submit then stop
  const POST_SUBMIT_POLL_MS = 1000;

  async function fetchInfraStatus() {
    loadingInfraStatus = true;
    infraStatusError = null;
    try {
      infraStatus = await invoke<AppInfraStatus>("get_app_infra_status");
    } catch (err) {
      infraStatusError = typeof err === "string" ? err : JSON.stringify(err);
    } finally {
      loadingInfraStatus = false;
    }
  }

  async function fetchJobs() {
    loadingJobs = true;
    jobsError = null;
    try {
      jobs = await invoke<AppJobDto[]>("list_app_jobs");
      // Keep selected job detail in sync with the refreshed list.
      // If the selected job is now present in the list, update its detail
      // snapshot so status/result are coherent without a separate round-trip.
      if (selectedJobId != null) {
        const match = jobs.find((j) => j.id === selectedJobId);
        if (match) {
          selectedJob = match;
        } else {
          // Job not found in list — clear stale detail explicitly.
          selectedJob = null;
          selectedJobId = null;
        }
      }
    } catch (err) {
      jobsError = typeof err === "string" ? err : JSON.stringify(err);
    } finally {
      loadingJobs = false;
    }
  }

  /** Refresh both infra counts and job list together so they stay in sync. */
  async function refreshAll() {
    await Promise.all([fetchInfraStatus(), fetchJobs()]);
  }

  function stopPostSubmitPolling() {
    if (postSubmitPollInterval != null) {
      clearInterval(postSubmitPollInterval);
      postSubmitPollInterval = null;
      postSubmitPollCount = 0;
    }
  }

  async function selectJob(job: AppJobDto) {
    selectedJobId = job.id;
    selectedJobError = null;
    loadingSelectedJob = true;
    try {
      const result = await invoke<AppJobDto | null>("get_app_job", { request: { jobId: job.id } });
      if (result != null) {
        selectedJob = result;
      } else {
        // Backend says the job no longer exists — clear selection explicitly.
        selectedJob = null;
        selectedJobId = null;
      }
    } catch (err) {
      selectedJobError = typeof err === "string" ? err : JSON.stringify(err);
      selectedJob = job;
    } finally {
      loadingSelectedJob = false;
    }
  }

  async function refreshSelectedJob() {
    if (selectedJobId == null) return;
    selectedJobError = null;
    loadingSelectedJob = true;
    try {
      const result = await invoke<AppJobDto | null>("get_app_job", { request: { jobId: selectedJobId } });
      if (result != null) {
        selectedJob = result;
      } else {
        // Job no longer found — clear stale detail explicitly.
        selectedJob = null;
        selectedJobId = null;
      }
    } catch (err) {
      selectedJobError = typeof err === "string" ? err : JSON.stringify(err);
    } finally {
      loadingSelectedJob = false;
    }
  }

  async function submitDebugJob() {
    submitting = true;
    submitError = null;
    // Cancel any existing post-submit poll before starting a new one.
    stopPostSubmitPolling();
    try {
      const newJob = await invoke<AppJobDto>("submit_debug_cpu_job", {
        request: {
          documentName: submitDocName,
          sourceText: submitSourceText,
        },
      });
      jobs = [newJob, ...jobs];
      submitDocName = "";
      submitSourceText = "";
      // Start a short-lived polling window to catch status updates quickly.
      // The interval is tracked and cleaned up on component destroy or when
      // a new submit replaces it.
      postSubmitPollCount = 0;
      postSubmitPollInterval = setInterval(async () => {
        postSubmitPollCount += 1;
        await refreshAll();
        if (postSubmitPollCount >= POST_SUBMIT_POLL_MAX) {
          stopPostSubmitPolling();
        }
      }, POST_SUBMIT_POLL_MS);
    } catch (err) {
      submitError = typeof err === "string" ? err : JSON.stringify(err);
    } finally {
      submitting = false;
    }
  }

  function jobStatusBadgeClass(status: BackgroundJobStatus): string {
    if (status === "completed") return "badge badge--ok badge--sm";
    if (status === "failed") return "badge badge--err badge--sm";
    if (status === "running") return "badge badge--running badge--sm";
    return "badge badge--neutral badge--sm";
  }

  function normalizeJobTsForDate(ts: string): string {
    const trimmed = ts.trim();
    // SQLite CURRENT_TIMESTAMP is typically "YYYY-MM-DD HH:MM:SS" in UTC.
    // Convert that shape to a browser-safe ISO-8601 string before parsing.
    if (/^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}(?:\.\d+)?$/.test(trimmed)) {
      return trimmed.replace(" ", "T") + "Z";
    }
    // If a timestamp already includes a timezone or is already ISO-like,
    // preserve it and only normalize the date/time separator when needed.
    if (trimmed.includes(" ") && /(?:Z|[+-]\d{2}:\d{2})$/.test(trimmed)) {
      return trimmed.replace(" ", "T");
    }
    return trimmed;
  }

  function formatJobTs(ts: string | null | undefined): string {
    if (!ts) return "—";
    const d = new Date(normalizeJobTsForDate(ts));
    return isNaN(d.getTime()) ? ts : d.toLocaleTimeString();
  }

  $effect(() => {
    fetchInfraStatus();
    fetchJobs();
    // Clean up any in-flight post-submit poll when the component is destroyed.
    return () => {
      stopPostSubmitPolling();
    };
  });

</script>

<!-- ── Page header ──────────────────────────────────────────────────────── -->
<div class="page-header">
  <h1 class="page-title">Debug</h1>
  <p class="page-subtitle">recording status &amp; controls</p>
</div>

<!-- ── Recording status ─────────────────────────────────────────────────── -->
<section class="card">
  <h2 class="card__title">Session</h2>

  <div class="session-status" class:session-status--recording={isCapturing}>
    <span class="rec-dot" class:rec-dot--active={isCapturing}></span>
    <span class="session-label">{isCapturing ? "Recording" : session?.isRunning === false ? "Stopped" : "Idle"}</span>
  </div>

  {#if isInactivityPaused}
    <div class="inactivity-hint">
      <span class="inactivity-hint__dot"></span>
      <span class="inactivity-hint__text">
        Paused — effective idle exceeded timeout; waiting for activity
      </span>
    </div>
  {/if}

  {#if session?.requestedSources}
    <div class="source-session-grid">
      {#if session.requestedSources.screen}
        <div class="source-session-card">
          <div class="source-session-card__header">
            <span class="badge badge--ok badge--sm">screen</span>
          </div>
          <ul class="kv-list">
            <li>
              <span class="kv-key">session</span>
              <span class="kv-val kv-val--mono">{getSourceSessionId(session, "screen")}</span>
            </li>
            <li>
              <span class="kv-key">started</span>
              <span class="kv-val">{formatSourceStartedAt(getSourceSessionStartedAt(session, "screen"))}</span>
            </li>
          </ul>
        </div>
      {/if}

      {#if session.requestedSources.microphone}
        <div class="source-session-card">
          <div class="source-session-card__header">
            <span class="badge badge--ok badge--sm">mic</span>
          </div>
          <ul class="kv-list">
            <li>
              <span class="kv-key">session</span>
              <span class="kv-val kv-val--mono">{getSourceSessionId(session, "microphone")}</span>
            </li>
            <li>
              <span class="kv-key">started</span>
              <span class="kv-val">{formatSourceStartedAt(getSourceSessionStartedAt(session, "microphone"))}</span>
            </li>
          </ul>
        </div>
      {/if}

      {#if session.requestedSources.systemAudio}
        <div class="source-session-card">
          <div class="source-session-card__header">
            <span class="badge badge--ok badge--sm">sys-audio</span>
          </div>
          <ul class="kv-list">
            <li>
              <span class="kv-key">session</span>
              <span class="kv-val kv-val--mono">{getSourceSessionId(session, "systemAudio")}</span>
            </li>
            <li>
              <span class="kv-key">started</span>
              <span class="kv-val">{formatSourceStartedAt(getSourceSessionStartedAt(session, "systemAudio"))}</span>
            </li>
          </ul>
        </div>
      {/if}
    </div>
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

<!-- ── Runtime sources ──────────────────────────────────────────────────── -->
<section class="card card--debug">
  <h2 class="card__title">
    <span class="debug-tag">dbg</span>
    Runtime Sources
    <span class="idle-note">capture session · writer · activity</span>
    <button class="btn btn--ghost btn--sm" onclick={fetchIdleDebug}>↻</button>
  </h2>

  {#if idleDebug && idleDebug.runtimeSources}
    <div class="rs-grid">
      {#each runtimeLanes as lane (lane.key)}
        {@const src = idleDebug.runtimeSources[lane.key]}
        {@const state = runtimeStateWord(src)}
        <article
          class="rs-lane rs-lane--{lane.key}"
          class:rs-lane--off={!src.requested}
          class:rs-lane--paused={src.paused && src.requested}
          class:rs-lane--running={src.requested && !src.paused && src.writerActive === true}
        >
          <header class="rs-lane__head">
            <span class="rs-lane__glyph">{lane.glyph}</span>
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
            <span class="rs-path__val" title={src.outputPath ?? ""}>{shortenPath(src.outputPath)}</span>
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
  {:else if idleDebugError}
    <p class="debug-err">{idleDebugError}</p>
  {:else}
    <p class="empty">runtime status only available while a session is active</p>
  {/if}
</section>

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

<!-- ── Native idle debug ─────────────────────────────────────────────────── -->
<section class="card card--debug">
  <h2 class="card__title">
    <span class="debug-tag">dbg</span>
    Inactivity Policy
    <button class="btn btn--ghost btn--sm" onclick={fetchIdleDebug}>↻</button>
  </h2>

  {#if idleDebugError}
    <p class="debug-err">{idleDebugError}</p>
  {:else if idleDebug}
    <!-- ── Status row ──────────────────────────────────── -->
    <ul class="kv-list">
      <li>
        <span class="kv-key">gating</span>
        <span class={idleDebug.inactivityEnabled ? "badge badge--ok badge--sm" : "badge badge--neutral badge--sm"}>
          {idleDebug.inactivityEnabled ? "enabled" : "disabled"}
        </span>
      </li>
      <li>
        <span class="kv-key">any paused</span>
        <span class={idleDebug.isInactivityPaused ? "badge badge--warn badge--sm" : "badge badge--neutral badge--sm"}>
          {idleDebug.isInactivityPaused ? "yes" : "no"}
        </span>
      </li>
      <li>
        <span class="kv-key">timeout</span>
        <span class="kv-val kv-val--mono">
          {idleDebug.inactivityEnabled ? `${idleDebug.idleTimeoutSeconds}s` : "—"}
        </span>
      </li>
      <li>
        <span class="kv-key">mode</span>
        <span class="kv-val kv-val--mono">{formatActivityMode(idleDebug.activityMode)}</span>
      </li>
    </ul>

    <!-- ── Signal readings ────────────────────────────── -->
    <div class="idle-section-label">Input signals <span class="idle-note">raw / threshold-qualified readings</span></div>
    <ul class="kv-list">
      <li>
        <span class="kv-key kv-key--wide">system input idle</span>
        <span class="kv-val kv-val--mono">{formatIdleMs(idleDebug.systemIdleMs)}</span>
        {#if idleDebug.activityMode !== "system_input_only"}
          <span class="idle-note">keyboard/mouse only</span>
        {/if}
      </li>
      {#if idleDebug.activityMode === "system_input_or_screen" || idleDebug.activityMode === "system_input_or_screen_or_audio"}
        <li>
          <span class="kv-key kv-key--wide">screen activity idle</span>
          <span class="kv-val kv-val--mono">{formatIdleMs(idleDebug.screenActivityIdleMs)}</span>
          <span class="idle-note">time since last screen change</span>
        </li>
      {/if}
      {#if idleDebug.activityMode === "system_input_or_screen_or_audio"}
        <li>
           <span class="kv-key kv-key--wide">mic activity idle</span>
          <span class="kv-val kv-val--mono">{formatIdleMs(idleDebug.microphoneActivityIdleMs)}</span>
          {#if idleDebug.microphoneActivityLevel != null}
            <span class="idle-note">level {(idleDebug.microphoneActivityLevel * 100).toFixed(0)}%</span>
          {/if}
          {#if !idleDebug.microphoneActivityEnabled}
            <span class="badge badge--neutral badge--sm">off</span>
          {/if}
        </li>
        <li>
           <span class="kv-key kv-key--wide">sys audio activity idle</span>
          <span class="kv-val kv-val--mono">{formatIdleMs(idleDebug.systemAudioActivityIdleMs)}</span>
          {#if idleDebug.systemAudioActivityLevel != null}
            <span class="idle-note">level {(idleDebug.systemAudioActivityLevel * 100).toFixed(0)}%</span>
          {/if}
          {#if !idleDebug.systemAudioActivityEnabled}
            <span class="badge badge--neutral badge--sm">off</span>
          {/if}
        </li>
      {/if}
    </ul>

    <!-- ── Per-detector pause status ─────────────────── -->
    <div class="idle-section-label">Detector pause status</div>
    <div class="detector-grid">
      <!-- Screen detector -->
      <div class="detector-card detector-card--screen" class:detector-card--paused={idleDebug.screenPaused}>
        <div class="detector-card__header">
          <span class="detector-card__icon">◉</span>
          <span class="detector-card__name">Screen</span>
          {#if idleDebug.screenPaused}
            <span class="badge badge--warn badge--sm">paused</span>
          {:else}
            <span class="badge badge--ok badge--sm">active</span>
          {/if}
        </div>
        <div class="detector-card__body">
          <div class="detector-card__metric">
            <span class="detector-card__metric-label">effective idle</span>
            <span class="detector-card__metric-value">{formatIdleMs(idleDebug.screenEffectiveIdleMs)}</span>
          </div>
          <div class="detector-card__metric">
            <span class="detector-card__metric-label">source</span>
            <span class="detector-card__metric-source">{formatEffectiveSource(idleDebug.screenEffectiveActivitySource)}</span>
          </div>
        </div>
      </div>

      <!-- Microphone detector -->
      <div class="detector-card detector-card--mic" class:detector-card--paused={idleDebug.microphonePaused && idleDebug.microphoneActivityEnabled} class:detector-card--off={!idleDebug.microphoneActivityEnabled}>
        <div class="detector-card__header">
          <span class="detector-card__icon">🎙</span>
          <span class="detector-card__name">Microphone</span>
          {#if !idleDebug.microphoneActivityEnabled}
            <span class="badge badge--neutral badge--sm">off</span>
          {:else if idleDebug.microphonePaused}
            <span class="badge badge--warn badge--sm">paused</span>
          {:else}
            <span class="badge badge--ok badge--sm">active</span>
          {/if}
        </div>
        <div class="detector-card__body">
          <div class="detector-card__metric">
            <span class="detector-card__metric-label">effective idle</span>
            <span class="detector-card__metric-value">{formatIdleMs(idleDebug.microphoneEffectiveIdleMs)}</span>
          </div>
          <div class="detector-card__metric">
            <span class="detector-card__metric-label">source</span>
            <span class="detector-card__metric-source">{formatEffectiveSource(idleDebug.microphoneEffectiveActivitySource)}</span>
          </div>
          {#if idleDebug.microphoneActivityLevel != null}
            <div class="detector-card__metric">
              <span class="detector-card__metric-label">level</span>
              <span class="detector-card__metric-value">{(idleDebug.microphoneActivityLevel * 100).toFixed(0)}%</span>
            </div>
          {/if}
          {#if idleDebug.microphoneActivitySensitivity != null}
            <div class="detector-card__metric">
              <span class="detector-card__metric-label">sensitivity</span>
              <span class="detector-card__metric-source">{idleDebug.microphoneActivitySensitivity}%{#if idleDebug.microphoneActivityThreshold != null} (thr {(idleDebug.microphoneActivityThreshold * 100).toFixed(1)}%){/if}</span>
            </div>
          {/if}
        </div>
      </div>

      <!-- System audio detector -->
      <div class="detector-card detector-card--sysaudio" class:detector-card--paused={idleDebug.systemAudioPaused && idleDebug.systemAudioActivityEnabled} class:detector-card--off={!idleDebug.systemAudioActivityEnabled}>
        <div class="detector-card__header">
          <span class="detector-card__icon">🔊</span>
          <span class="detector-card__name">System Audio</span>
          {#if !idleDebug.systemAudioActivityEnabled}
            <span class="badge badge--neutral badge--sm">off</span>
          {:else if idleDebug.systemAudioPaused}
            <span class="badge badge--warn badge--sm">paused</span>
          {:else}
            <span class="badge badge--ok badge--sm">active</span>
          {/if}
        </div>
        <div class="detector-card__body">
          <div class="detector-card__metric">
            <span class="detector-card__metric-label">effective idle</span>
            <span class="detector-card__metric-value">{formatIdleMs(idleDebug.systemAudioEffectiveIdleMs)}</span>
          </div>
          <div class="detector-card__metric">
            <span class="detector-card__metric-label">source</span>
            <span class="detector-card__metric-source">{formatEffectiveSource(idleDebug.systemAudioEffectiveActivitySource)}</span>
          </div>
          {#if idleDebug.systemAudioActivityLevel != null}
            <div class="detector-card__metric">
              <span class="detector-card__metric-label">level</span>
              <span class="detector-card__metric-value">{(idleDebug.systemAudioActivityLevel * 100).toFixed(0)}%</span>
            </div>
          {/if}
          {#if idleDebug.systemAudioActivitySensitivity != null}
            <div class="detector-card__metric">
              <span class="detector-card__metric-label">sensitivity</span>
              <span class="detector-card__metric-source">{idleDebug.systemAudioActivitySensitivity}%{#if idleDebug.systemAudioActivityThreshold != null} (thr {(idleDebug.systemAudioActivityThreshold * 100).toFixed(1)}%){/if}</span>
            </div>
          {/if}
        </div>
      </div>
    </div>

    <!-- ── Combined effective (subordinate) ────────────── -->
    <div class="effective-idle-summary">
      <span class="effective-idle-summary__label">combined effective idle</span>
      <span class="effective-idle-summary__value">{formatIdleMs(idleDebug.effectiveIdleMs)}</span>
      <span class="effective-idle-summary__source">via {formatEffectiveSource(idleDebug.effectiveActivitySource)}</span>
    </div>
    {#if idleDebug.activityMode === "system_input_or_screen_or_audio"}
      <p class="effective-idle-block__note">
        Audio mode: combined value is min-over-sources; detector pause states are still tracked per family.
      </p>
    {:else if idleDebug.activityMode === "system_input_or_screen"}
      <p class="effective-idle-block__note">
        Hybrid mode: pause requires <em>both</em> input and screen idle ≥ {idleDebug.idleTimeoutSeconds}s.
      </p>
    {:else}
      <p class="effective-idle-block__note">
        Input-only mode: pause when system input idle ≥ {idleDebug.idleTimeoutSeconds}s.
      </p>
    {/if}

    <div class="idle-section-label">Activity sources <span class="idle-note">combined-policy samples</span></div>
    <ul class="kv-list">
      {#each idleDebug.activitySources as source (source.kind)}
        <li>
          <span class="kv-key kv-key--wide">{sourceKindLabel(source.kind)}</span>
          <span class="kv-val kv-val--mono">{formatIdleMs(source.idleMs)}</span>
          {#if source.latestNormalizedLevel != null}
            <span class="idle-note">lvl {(source.latestNormalizedLevel * 100).toFixed(0)}%{source.activityThreshold != null ? ` / thr ${(source.activityThreshold * 100).toFixed(0)}%` : ""}</span>
          {/if}
          <span class={source.selected ? "badge badge--ok badge--sm" : source.available ? "badge badge--neutral badge--sm" : "badge badge--warn badge--sm"}>
            {sourceDecisionSummary(source.available, source.selected, source.enabled)}
          </span>
        </li>
      {/each}
    </ul>

    <!-- ── Probe info ─────────────────────────────────── -->
    <div class="idle-section-label">Probe</div>
    <ul class="kv-list">
      <li>
        <span class="kv-key kv-key--wide">detector source</span>
        <span class="kv-val kv-val--mono">{idleDebug.detectorSource}</span>
      </li>
      {#if idleDebug.screenActivityLastUnixMs != null}
        <li>
          <span class="kv-key kv-key--wide">screen raw sample</span>
          <span class="kv-val kv-val--mono">{formatTimestamp(idleDebug.screenActivityLastUnixMs)}</span>
        </li>
      {/if}
      {#if idleDebug.microphoneActivitySensitivity != null}
        <li>
          <span class="kv-key kv-key--wide">mic sensitivity</span>
          <span class="kv-val kv-val--mono">{idleDebug.microphoneActivitySensitivity}%</span>
        </li>
      {/if}
      {#if idleDebug.systemAudioActivitySensitivity != null}
        <li>
          <span class="kv-key kv-key--wide">sys audio sensitivity</span>
          <span class="kv-val kv-val--mono">{idleDebug.systemAudioActivitySensitivity}%</span>
        </li>
      {/if}
      {#if idleDebug.microphoneActivityThreshold != null}
        <li>
          <span class="kv-key kv-key--wide">mic threshold</span>
          <span class="kv-val kv-val--mono">{(idleDebug.microphoneActivityThreshold * 100).toFixed(1)}%</span>
          <span class="idle-note">normalised level; audio above this counts as activity</span>
        </li>
      {/if}
      {#if idleDebug.microphoneActivityLastUnixMs != null}
        <li>
          <span class="kv-key kv-key--wide">mic raw sample</span>
          <span class="kv-val kv-val--mono">{formatTimestamp(idleDebug.microphoneActivityLastUnixMs)}</span>
          <span class="idle-note">timestamp, not detector decision</span>
        </li>
      {/if}
      {#if idleDebug.systemAudioActivityLastUnixMs != null}
        <li>
          <span class="kv-key kv-key--wide">sys-audio raw sample</span>
          <span class="kv-val kv-val--mono">{formatTimestamp(idleDebug.systemAudioActivityLastUnixMs)}</span>
          <span class="idle-note">timestamp, not detector decision</span>
        </li>
      {/if}
    </ul>
  {:else}
    <p class="empty">—</p>
  {/if}
</section>

<!-- ── App Infra / Background Jobs ───────────────────────────────────────── -->
<section class="card card--debug">
  <h2 class="card__title">
    <span class="debug-tag">dbg</span>
    App Infra
    <button class="btn btn--ghost btn--sm" onclick={refreshAll} disabled={loadingInfraStatus || loadingJobs}>
      {loadingInfraStatus || loadingJobs ? "…" : "↻"}
    </button>
  </h2>

  {#if infraStatusError}
    <p class="debug-err">{infraStatusError}</p>
  {:else if infraStatus}
    <ul class="kv-list">
      <li>
        <span class="kv-key kv-key--wide">migrations</span>
        <span class={infraStatus.migrationsRan ? "badge badge--ok badge--sm" : "badge badge--warn badge--sm"}>
          {infraStatus.migrationsRan ? "ran" : "pending"}
        </span>
      </li>
      <li>
        <span class="kv-key kv-key--wide">workers</span>
        <span class="kv-val kv-val--mono">{infraStatus.workerThreadCount}</span>
      </li>
      <li>
        <span class="kv-key kv-key--wide">jobs total</span>
        <span class="kv-val kv-val--mono">{infraStatus.jobCounts.total}</span>
      </li>
    </ul>
    <div class="job-count-row">
      {#if infraStatus.jobCounts.queued > 0}
        <span class="badge badge--neutral badge--sm">queued {infraStatus.jobCounts.queued}</span>
      {/if}
      {#if infraStatus.jobCounts.running > 0}
        <span class="badge badge--running badge--sm">running {infraStatus.jobCounts.running}</span>
      {/if}
      {#if infraStatus.jobCounts.completed > 0}
        <span class="badge badge--ok badge--sm">done {infraStatus.jobCounts.completed}</span>
      {/if}
      {#if infraStatus.jobCounts.failed > 0}
        <span class="badge badge--err badge--sm">failed {infraStatus.jobCounts.failed}</span>
      {/if}
    </div>
    <div class="idle-section-label">DB path</div>
    <p class="infra-db-path">{infraStatus.databasePath}</p>
  {:else}
    <p class="empty">—</p>
  {/if}
</section>

<!-- ── Background Jobs ───────────────────────────────────────────────────── -->
<section class="card card--debug">
  <h2 class="card__title">
    <span class="debug-tag">dbg</span>
    Background Jobs
    <button class="btn btn--ghost btn--sm" onclick={refreshAll} disabled={loadingJobs || loadingInfraStatus}>
      {loadingJobs ? "…" : "↻ list"}
    </button>
    {#if postSubmitPollInterval != null}
      <span class="idle-note">polling ({POST_SUBMIT_POLL_MAX - postSubmitPollCount} left)</span>
    {/if}
  </h2>

  <!-- Submit form -->
  <div class="idle-section-label">Submit debug CPU job</div>
  <form class="job-submit-form" onsubmit={(e) => { e.preventDefault(); submitDebugJob(); }}>
    <input
      class="job-input"
      type="text"
      placeholder="document name"
      bind:value={submitDocName}
      disabled={submitting}
    />
    <input
      class="job-input"
      type="text"
      placeholder="source text"
      bind:value={submitSourceText}
      disabled={submitting}
    />
    <button class="btn btn--primary btn--sm" type="submit" disabled={submitting}>
      {submitting ? "…" : "submit"}
    </button>
  </form>
  {#if submitError}
    <p class="debug-err">{submitError}</p>
  {/if}

  <!-- Job list -->
  <div class="idle-section-label">
    Recent jobs
    {#if loadingJobs}<span class="idle-note">loading…</span>{/if}
  </div>
  {#if jobsError}
    <p class="debug-err">{jobsError}</p>
  {:else if jobs.length === 0}
    <p class="empty">no jobs yet</p>
  {:else}
    <ul class="job-list">
      {#each jobs as job (job.id)}
        {@const isSelected = selectedJobId === job.id}
        <li>
          <button
            class="job-row"
            class:job-row--selected={isSelected}
            type="button"
            onclick={() => selectJob(job)}
          >
            <span class="job-row__id">#{job.id}</span>
            <span class="job-row__kind">{job.kind}</span>
            <span class={jobStatusBadgeClass(job.status)}>{job.status}</span>
            <span class="job-row__ts">{formatJobTs(job.createdAt)}</span>
          </button>
        </li>
      {/each}
    </ul>
  {/if}

  <!-- Selected job detail -->
  {#if selectedJobId != null}
    <div class="idle-section-label">
      Job #{selectedJobId}
      <button
        class="btn btn--ghost btn--sm"
        onclick={refreshSelectedJob}
        disabled={loadingSelectedJob}
        style="margin-left: 6px;"
      >
        {loadingSelectedJob ? "…" : "↻"}
      </button>
    </div>
    {#if selectedJobError}
      <p class="debug-err">{selectedJobError}</p>
    {/if}
    {#if selectedJob}
      <ul class="kv-list">
        <li>
          <span class="kv-key kv-key--wide">status</span>
          <span class={jobStatusBadgeClass(selectedJob.status)}>{selectedJob.status}</span>
        </li>
        <li>
          <span class="kv-key kv-key--wide">attempts</span>
          <span class="kv-val kv-val--mono">{selectedJob.attemptCount}</span>
        </li>
        {#if selectedJob.startedAt}
          <li>
            <span class="kv-key kv-key--wide">started</span>
            <span class="kv-val kv-val--mono">{formatJobTs(selectedJob.startedAt)}</span>
          </li>
        {/if}
        {#if selectedJob.finishedAt}
          <li>
            <span class="kv-key kv-key--wide">finished</span>
            <span class="kv-val kv-val--mono">{formatJobTs(selectedJob.finishedAt)}</span>
          </li>
        {/if}
        {#if selectedJob.resultText}
          <li class="kv-list-block">
            <span class="kv-key kv-key--wide">result</span>
            <span class="job-detail-text">{selectedJob.resultText}</span>
          </li>
        {/if}
        {#if selectedJob.lastError}
          <li class="kv-list-block">
            <span class="kv-key kv-key--wide">error</span>
            <span class="job-detail-text job-detail-text--err">{selectedJob.lastError}</span>
          </li>
        {/if}
      </ul>
    {/if}
  {/if}
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

  .source-session-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
    gap: 10px;
  }

  .source-session-card {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px 12px;
    background: #0d0d14;
    border: 1px solid #1a1a28;
    border-radius: 4px;
  }

  .source-session-card__header {
    display: flex;
    align-items: center;
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

  /* ── Debug card ─────────────────────────────────────────────── */
  .card--debug {
    border-style: dashed;
    border-color: #252535;
    background: #0e0e15;
    opacity: 0.92;
  }

  .debug-tag {
    display: inline-flex;
    align-items: center;
    padding: 0 5px;
    background: #1a1a2a;
    border: 1px solid #2a2a40;
    border-radius: 2px;
    font-size: 8px;
    font-weight: 800;
    letter-spacing: 0.1em;
    color: #5a5a7a;
    text-transform: uppercase;
  }

  .kv-key--wide {
    min-width: 120px;
  }

  .kv-val--mono {
    font-family: "SF Mono", "Fira Mono", "Courier New", monospace;
    font-size: 10px;
    color: #8080a8;
  }

  /* ── Idle debug sub-sections ────────────────────────── */
  .idle-section-label {
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: #33334a;
    margin-top: 4px;
  }

  .idle-note {
    font-size: 9px;
    color: #33334a;
    font-style: italic;
    margin-left: 4px;
  }

  /* ── Detector grid ────────────────────────────────────── */
  .detector-grid {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 8px;
  }

  .detector-card {
    background: #0b0b12;
    border: 1px solid #1e1e2e;
    border-radius: 5px;
    padding: 10px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    transition: border-color 0.15s, background 0.15s;
  }

  .detector-card--screen { border-left: 2px solid #5a4aaa; }
  .detector-card--mic { border-left: 2px solid #4a8a6a; }
  .detector-card--sysaudio { border-left: 2px solid #6a7a4a; }

  .detector-card--paused { background: #12100a; }
  .detector-card--off { opacity: 0.55; }

  .detector-card__header {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .detector-card__icon {
    font-size: 11px;
    flex-shrink: 0;
  }

  .detector-card__name {
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: #6a6a88;
    flex: 1;
  }

  .detector-card__body {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .detector-card__metric {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .detector-card__metric-label {
    font-size: 9px;
    color: #3a3a54;
    min-width: 56px;
  }

  .detector-card__metric-value {
    font-family: "SF Mono", "Fira Mono", "Courier New", monospace;
    font-size: 11px;
    font-weight: 700;
    color: #c0b0ff;
    letter-spacing: 0.04em;
  }

  .detector-card--mic .detector-card__metric-value { color: #80d0a8; }
  .detector-card--sysaudio .detector-card__metric-value { color: #b0c080; }

  .detector-card__metric-source {
    font-family: "SF Mono", "Fira Mono", "Courier New", monospace;
    font-size: 9px;
    color: #606080;
  }

  /* ── Effective idle summary (subordinate) ────────────── */
  .effective-idle-summary {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 10px;
    background: #0a0a10;
    border: 1px solid #1a1a28;
    border-radius: 3px;
  }

  .effective-idle-summary__label {
    font-size: 9px;
    color: #3a3a54;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }

  .effective-idle-summary__value {
    font-family: "SF Mono", "Fira Mono", "Courier New", monospace;
    font-size: 11px;
    font-weight: 700;
    color: #8080a8;
  }

  .effective-idle-summary__source {
    font-family: "SF Mono", "Fira Mono", "Courier New", monospace;
    font-size: 9px;
    color: #505068;
  }

  .effective-idle-block__note {
    font-size: 9px;
    color: #4a4470;
    line-height: 1.5;
    margin-top: 4px;
    border-top: 1px solid #1e1a30;
    padding-top: 6px;
  }

  .effective-idle-block__note em {
    font-style: normal;
    color: #7060a8;
    font-weight: 600;
  }

  .debug-err {
    font-size: 10px;
    color: #a05050;
    font-family: "SF Mono", "Fira Mono", "Courier New", monospace;
  }

  .badge--warn {
    background: #201608;
    color: #c09030;
    border: 1px solid #3a2810;
  }

  /* ── Inactivity hint ────────────────────────────────────────── */
  .inactivity-hint {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    background: #0d0c0a;
    border: 1px solid #3a3010;
    border-radius: 4px;
  }

  .inactivity-hint__dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: #a07820;
    flex-shrink: 0;
    animation: pulse-idle 2s ease-in-out infinite;
  }

  @keyframes pulse-idle {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.4; }
  }

  .inactivity-hint__text {
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: #8a6a18;
  }

  /* ── App Infra ──────────────────────────────────────────────── */
  .job-count-row {
    display: flex;
    flex-wrap: wrap;
    gap: 5px;
  }

  .infra-db-path {
    font-family: "SF Mono", "Fira Mono", "Courier New", monospace;
    font-size: 9px;
    color: #44445a;
    word-break: break-all;
    line-height: 1.5;
  }

  /* ── Background Jobs ────────────────────────────────────────── */
  .job-submit-form {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }

  .job-input {
    background: #0d0d14;
    border: 1px solid #2a2a3e;
    border-radius: 3px;
    padding: 4px 8px;
    font-family: inherit;
    font-size: 11px;
    color: #9090b0;
    outline: none;
    flex: 1;
    min-width: 80px;
    transition: border-color 0.12s;
  }

  .job-input:focus {
    border-color: #4a4a7a;
  }

  .job-input::placeholder {
    color: #33334a;
  }

  .job-input:disabled {
    opacity: 0.4;
  }

  .job-list {
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .job-list li {
    display: contents;
  }

  .job-row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 5px 8px;
    border-radius: 3px;
    background: transparent;
    border: 1px solid transparent;
    cursor: pointer;
    width: 100%;
    text-align: left;
    font-family: inherit;
    transition: background 0.1s, border-color 0.1s;
  }

  .job-row:hover {
    background: #0e0e18;
    border-color: #2a2a40;
  }

  .job-row--selected {
    background: #0e0e20;
    border-color: #3a3a60;
  }

  .job-row__id {
    font-family: "SF Mono", "Fira Mono", "Courier New", monospace;
    font-size: 9px;
    color: #44445a;
    min-width: 28px;
    flex-shrink: 0;
  }

  .job-row__kind {
    font-size: 10px;
    color: #6060a0;
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .job-row__ts {
    font-family: "SF Mono", "Fira Mono", "Courier New", monospace;
    font-size: 9px;
    color: #33334a;
    flex-shrink: 0;
    margin-left: auto;
  }

  .badge--running {
    background: #0c1a2e;
    color: #60b0ff;
    border: 1px solid #1a3050;
  }

  .kv-list-block {
    flex-direction: column !important;
    align-items: flex-start !important;
    gap: 4px !important;
  }

  .job-detail-text {
    font-family: "SF Mono", "Fira Mono", "Courier New", monospace;
    font-size: 10px;
    color: #7070a0;
    white-space: pre-wrap;
    word-break: break-all;
    line-height: 1.5;
    max-height: 80px;
    overflow-y: auto;
    display: block;
    padding: 4px 0;
  }

  .job-detail-text--err {
    color: #a05050;
  }

  /* ── Runtime sources ────────────────────────────────────────── */
  .rs-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
    gap: 10px;
  }

  .rs-lane {
    position: relative;
    background: #0a0a11;
    border: 1px solid #1c1c2c;
    border-radius: 5px;
    padding: 12px 12px 10px;
    display: flex;
    flex-direction: column;
    gap: 10px;
    overflow: hidden;
    transition: border-color 0.15s, background 0.15s;
  }

  /* Subtle vertical accent identifying the source family. */
  .rs-lane::before {
    content: "";
    position: absolute;
    left: 0; top: 0; bottom: 0;
    width: 2px;
    opacity: 0.85;
  }
  .rs-lane--screen::before { background: linear-gradient(180deg, #5a4aaa 0%, #2a2050 100%); }
  .rs-lane--microphone::before { background: linear-gradient(180deg, #4a8a6a 0%, #1a3a2a 100%); }
  .rs-lane--systemAudio::before { background: linear-gradient(180deg, #b09040 0%, #4a3a18 100%); }

  .rs-lane--off { opacity: 0.5; }
  .rs-lane--paused { background: #100c08; border-color: #2a2418; }
  .rs-lane--running { border-color: #1f3d2a; }

  .rs-lane__head {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .rs-lane__glyph {
    font-size: 12px;
    line-height: 1;
    flex-shrink: 0;
  }

  .rs-lane__name {
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: #7a7a98;
    flex: 1;
  }

  .rs-state {
    font-size: 8px;
    font-weight: 800;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    padding: 2px 6px;
    border-radius: 2px;
    border: 1px solid transparent;
    white-space: nowrap;
  }
  .rs-state--running { background: #0c2218; color: #4dffa8; border-color: #1a4a30; }
  .rs-state--paused { background: #2a2010; color: #ffb060; border-color: #4a3a18; }
  .rs-state--partial { background: #1a1a3a; color: #a0a0ff; border-color: #2a2a5a; }
  .rs-state--idle { background: #161622; color: #6a6a8a; border-color: #26263a; }
  .rs-state--off { background: #16161e; color: #44445a; border-color: #20202c; }
  .rs-state--unknown { background: #1c0e0e; color: #c08080; border-color: #3a1a1a; }

  /* Truth rows: tiny indicator + label + value. */
  .rs-rows {
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin: 0;
    padding: 0;
  }

  .rs-rows--activity {
    border-top: 1px dashed #1a1a26;
    padding-top: 8px;
  }

  .rs-row {
    display: grid;
    grid-template-columns: 56px 28px 1fr;
    align-items: center;
    gap: 6px;
  }

  .rs-row__label {
    font-size: 8px;
    font-weight: 700;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: #383852;
  }

  /* Indicator bar — colour by state, mimicking a tiny status LED. */
  .rs-row__bar {
    position: relative;
    display: inline-block;
    width: 100%;
    height: 4px;
    border-radius: 2px;
    background: #14141e;
    overflow: hidden;
  }
  .rs-row__bar-fill {
    position: absolute;
    inset: 0;
    border-radius: inherit;
    transition: background 0.15s, opacity 0.15s;
  }
  .rs-row__bar[data-state="on"] .rs-row__bar-fill {
    background: linear-gradient(90deg, #1a4a30 0%, #3dffa0 100%);
    box-shadow: 0 0 6px rgba(61, 255, 160, 0.35);
  }
  .rs-row__bar[data-state="paused"] .rs-row__bar-fill {
    background: linear-gradient(90deg, #4a3618 0%, #ffb060 100%);
    opacity: 0.85;
  }
  .rs-row__bar[data-state="off"] .rs-row__bar-fill { background: #2a2030; opacity: 0.6; }
  .rs-row__bar[data-state="idle"] .rs-row__bar-fill { background: #20202e; }
  .rs-row__bar[data-state="unknown"] .rs-row__bar-fill { background: repeating-linear-gradient(45deg, #3a1a1a 0 3px, #1a0a0a 3px 6px); }

  .rs-row__val {
    font-size: 10px;
    color: #a0a0c0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .rs-row__val--mono {
    font-family: "SF Mono", "Fira Mono", "Courier New", monospace;
    font-size: 10px;
    color: #8a8ab0;
    /* Sample/activity readouts: show full text, allow wrapping. */
    overflow: visible;
    text-overflow: clip;
    white-space: normal;
    word-break: break-word;
  }

  /* Output path block. */
  .rs-path {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 8px;
    background: #07070d;
    border: 1px solid #16161e;
    border-radius: 3px;
  }
  .rs-path__label {
    font-size: 8px;
    font-weight: 700;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: #383852;
  }
  .rs-path__val {
    font-family: "SF Mono", "Fira Mono", "Courier New", monospace;
    font-size: 10px;
    color: #7a7aa8;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1;
  }

  .rs-legend {
    display: flex;
    flex-wrap: wrap;
    gap: 14px;
    align-items: center;
    font-size: 9px;
    color: #4a4a66;
    letter-spacing: 0.04em;
    padding-top: 4px;
    border-top: 1px dashed #1a1a26;
  }
  .rs-legend span { display: inline-flex; align-items: center; gap: 5px; }
  .rs-legend__dot {
    display: inline-block;
    width: 8px;
    height: 4px;
    border-radius: 1px;
  }
  .rs-legend__dot--on { background: linear-gradient(90deg, #1a4a30 0%, #3dffa0 100%); }
  .rs-legend__dot--paused { background: linear-gradient(90deg, #4a3618 0%, #ffb060 100%); }
  .rs-legend__dot--off { background: #2a2030; }

</style>
