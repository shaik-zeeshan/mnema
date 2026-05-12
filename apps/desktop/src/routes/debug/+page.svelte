<script lang="ts">
  import { tick } from "svelte";
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
    SegmentWorkspaceCleanupDebugInfoDto,
    SegmentWorkspaceCleanupDisposition,
    FrameBatchStatus,
    ProcessingJobStatus,
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
  const RECORDING_SETTINGS_CHANGED_EVENT = "recording_settings_changed";
  let loadingSettings = $state(false);

  // ─── Idle debug ──────────────────────────────────────────────────────────
  let idleDebug = $state<IdleDebugInfo | null>(null);
  let idleDebugError = $state<string | null>(null);

  type PrivacyFilterDecision = {
    excludedBundleIds: string[];
    excludedWindowIds: number[];
    matchedRuleIds: string[];
    metadataRedactionReason: string | null;
    privacyFilterApplied: boolean;
  };

  type CapturePrivacyDebugInfo = {
    metadataEnabled: boolean;
    browserUrlMode: string;
    privateBrowserExclusionEnabled: boolean;
    privacyDebug: {
      latestSnapshot: {
        appBundleId: string | null;
        appName: string | null;
        windowTitle: string | null;
        browserUrl: string | null;
        displayId: number | null;
        metadataRedactionReason: string | null;
      } | null;
      latestDecision: PrivacyFilterDecision;
      latestAppliedDecision: PrivacyFilterDecision;
      websitePrivacyHoldBundleIds: string[];
      websitePrivacyHolds: Array<{ bundleId: string; reason: string }>;
      currentlyExcludedBundleIds: string[];
      currentlyExcludedWindowIds: number[];
      privacyFilterApplied: boolean;
      metadataRedactionReason: string | null;
    };
  };

  let privacyDebug = $state<CapturePrivacyDebugInfo | null>(null);
  let privacyDebugError = $state<string | null>(null);

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

  async function fetchCapturePrivacyDebug() {
    if (typeof document !== "undefined" && document.visibilityState === "hidden") return;
    if (!session?.isRunning) return;
    try {
      privacyDebug = await invoke<CapturePrivacyDebugInfo>("get_capture_privacy_debug");
      privacyDebugError = null;
    } catch (err) {
      privacyDebugError = typeof err === "string" ? err : JSON.stringify(err);
    }
  }

  function formatDebugList(values: Array<string | number> | null | undefined): string {
    if (!values || values.length === 0) return "none";
    return values.join(", ");
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
            sample: { lastUnixMs: idleDebug.microphoneActivitySample.lastUnixMs, idleMs: null, level: idleDebug.microphoneActivitySample.level },
            qualifiedIdleMs: idleDebug.microphoneActivityDecision.idleMs,
            qualifiedThreshold: idleDebug.microphoneActivityDecision.activityThreshold,
          },
          {
            key: "systemAudio",
            label: "System Audio",
            glyph: "🔊",
            sample: { lastUnixMs: idleDebug.systemAudioActivitySample.lastUnixMs, idleMs: null, level: idleDebug.systemAudioActivitySample.level },
            qualifiedIdleMs: idleDebug.systemAudioActivityDecision.idleMs,
            qualifiedThreshold: idleDebug.systemAudioActivityDecision.activityThreshold,
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
    let unlistenRecordingSettingsChanged: (() => void) | undefined;
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

    listen<RecordingSettings>(RECORDING_SETTINGS_CHANGED_EVENT, (event) => {
      recordingSettings = event.payload;
      clearError();
    }).then((fn) => {
      if (destroyed) fn();
      else unlistenRecordingSettingsChanged = fn;
    });

    return () => {
      destroyed = true;
      unlistenControllerChanged?.();
      unlistenAutoDisconnectFailure?.();
      unlistenRecordingSettingsChanged?.();
    };
  });

  // ─── Idle debug polling ────────────────────────────────────────────────────
  // Kept in a separate effect so that its session-reactivity (fetchIdleDebug
  // reads session?.isRunning) never triggers a re-run of the init effect above.

  $effect(() => {
    fetchIdleDebug();
    fetchCapturePrivacyDebug();

    const idleDebugInterval = setInterval(() => {
      fetchIdleDebug();
      fetchCapturePrivacyDebug();
    }, 2000);

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

  // ─── Wake/visibility resync ───────────────────────────────────────────────
  // After macOS sleep/wake the native capture pipeline may have been torn
  // down and restarted while the webview was suspended, leaving the shared
  // session store stale. The backend-emitted `system_did_wake` event is the
  // primary reliable trigger; foreground/drift heuristics remain as backstops.
  // Every resync snapshots the generation so a wake-triggered refresh can
  // never overwrite a newer authoritative action.
  //
  // Tauri/macOS does not reliably flip `document.visibilityState` on every
  // wake (the webview can stay "visible" while the system slept), so we
  // listen to a small union of triggers in addition to `visibilitychange`:
  // window `focus`, `pageshow`, `online`, and a wall-clock drift watchdog
  // (a 1Hz tick whose late arrival flags a process suspension). The 5s
  // threshold is generous enough that normal jank/GC pauses don't trigger a
  // resync; a real sleep is typically tens of seconds or more. Mirrors the
  // dashboard's wake resync.
  const WAKE_DRIFT_THRESHOLD_MS = 5_000;
  const WAKE_DRIFT_TICK_MS = 1_000;
  $effect(() => {
    if (typeof document === "undefined") return;
    async function resyncCaptureSession() {
      const gen = sessionGeneration;
      try {
        const r = await invoke<GetPermissionsResponse>("get_capture_permissions");
        if (sessionGeneration !== gen) return; // superseded by start/stop
        if (r.session) setSession(r.session);
      } catch {
        // Best-effort — the periodic reconcile still covers steady-state drift.
      }
    }
    const onVisibility = () => {
      if (document.visibilityState !== "visible") return;
      void resyncCaptureSession();
    };
    const onFocus = () => { void resyncCaptureSession(); };
    let unlistenSystemDidWake: (() => void) | undefined;
    let destroyed = false;

    listen("system_did_wake", () => {
      void resyncCaptureSession();
    }).then((fn) => {
      if (destroyed) fn();
      else unlistenSystemDidWake = fn;
    });

    document.addEventListener("visibilitychange", onVisibility);
    window.addEventListener("focus", onFocus);
    window.addEventListener("pageshow", onFocus);
    window.addEventListener("online", onFocus);

    let lastTick = Date.now();
    const driftTimer = setInterval(() => {
      const now = Date.now();
      const drift = now - lastTick - WAKE_DRIFT_TICK_MS;
      lastTick = now;
      if (drift >= WAKE_DRIFT_THRESHOLD_MS) {
        // Wall-clock jumped — process was suspended. Treat as a wake.
        void resyncCaptureSession();
      }
    }, WAKE_DRIFT_TICK_MS);

    return () => {
      destroyed = true;
      unlistenSystemDidWake?.();
      document.removeEventListener("visibilitychange", onVisibility);
      window.removeEventListener("focus", onFocus);
      window.removeEventListener("pageshow", onFocus);
      window.removeEventListener("online", onFocus);
      clearInterval(driftTimer);
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

  // ─── Hidden segment workspace classifier ──────────────────────────────────

  let workspaceDirInput = $state("");
  let workspaceClassification = $state<SegmentWorkspaceCleanupDebugInfoDto | null>(null);
  // `null` here means "no path looked like a hidden segment workspace" (the
  // backend returned `Option::None`); distinct from "have not run yet".
  let workspaceClassificationLoaded = $state(false);
  let workspaceClassificationError = $state<string | null>(null);
  let loadingWorkspaceClassification = $state(false);

  async function classifyWorkspace() {
    const trimmed = workspaceDirInput.trim();
    if (!trimmed) {
      workspaceClassificationError = "workspace path is required";
      return;
    }
    loadingWorkspaceClassification = true;
    workspaceClassificationError = null;
    try {
      const result = await invoke<SegmentWorkspaceCleanupDebugInfoDto | null>(
        "classify_hidden_segment_workspace",
        { request: { workspaceDir: trimmed } }
      );
      workspaceClassification = result;
      workspaceClassificationLoaded = true;
    } catch (err) {
      workspaceClassification = null;
      workspaceClassificationLoaded = false;
      workspaceClassificationError = typeof err === "string" ? err : JSON.stringify(err);
    } finally {
      loadingWorkspaceClassification = false;
    }
  }

  function dispositionLabel(d: SegmentWorkspaceCleanupDisposition): string {
    switch (d) {
      case "referenced_by_incomplete_batch": return "referenced by incomplete batch";
      case "referenced_by_nonterminal_ocr": return "referenced by non-terminal OCR";
      case "missing_visible_segment_sibling": return "missing visible segment sibling";
      case "completed_only": return "completed only";
      case "no_references": return "no references";
      default: return d;
    }
  }

  function dispositionBadgeClass(d: SegmentWorkspaceCleanupDisposition): string {
    switch (d) {
      case "completed_only":
      case "no_references":
        return "badge badge--ok badge--sm";
      case "referenced_by_incomplete_batch":
      case "referenced_by_nonterminal_ocr":
        return "badge badge--warn badge--sm";
      case "missing_visible_segment_sibling":
        return "badge badge--err badge--sm";
      default:
        return "badge badge--neutral badge--sm";
    }
  }

  function batchStatusBadgeClass(status: FrameBatchStatus): string {
    if (status === "completed") return "badge badge--ok badge--sm";
    if (status === "failed") return "badge badge--err badge--sm";
    if (status === "processing") return "badge badge--running badge--sm";
    return "badge badge--neutral badge--sm";
  }

  function ocrStatusBadgeClass(status: ProcessingJobStatus): string {
    if (status === "completed") return "badge badge--ok badge--sm";
    if (status === "failed") return "badge badge--err badge--sm";
    if (status === "running") return "badge badge--running badge--sm";
    return "badge badge--neutral badge--sm";
  }

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

  // ─── Section tabs ────────────────────────────────────────────────────────
  // Reorganize the dense Debug page into discrete sections so only one is
  // mounted at a time. This keeps the dedicated debug window focused, lets
  // long-lived backgrounds (probe polling, idle polling, etc.) ride next to
  // the section that owns them, and dramatically reduces what the renderer
  // paints when several heavy lists are present at once.
  type DebugTab =
    | "session"
    | "runtime"
    | "probe"
    | "inactivity"
    | "infra"
    | "workspaces"
    | "jobs";

  const debugTabs: { id: DebugTab; label: string }[] = [
    { id: "session", label: "Session" },
    { id: "runtime", label: "Runtime" },
    { id: "probe", label: "Probe" },
    { id: "inactivity", label: "Inactivity" },
    { id: "infra", label: "Infra" },
    { id: "workspaces", label: "Workspaces" },
    { id: "jobs", label: "Jobs" },
  ];

  let activeTab = $state<DebugTab>("session");

  // Scroll-region element. The wrapper persists across tab switches (only
  // the inner `{#if activeTab === ...}` panel re-mounts), so without an
  // explicit reset the previous tab's `scrollTop` would carry over.
  // Reset to the top whenever `activeTab` changes.
  let scrollRegion = $state<HTMLDivElement | null>(null);

  $effect(() => {
    activeTab;
    scrollRegion?.scrollTo({ top: 0, behavior: "auto" });
  });

  let initialTabFocusDone = false;
  let initialTabFocusScheduled = false;

  $effect(() => {
    activeTab;
    if (initialTabFocusDone || initialTabFocusScheduled || typeof document === "undefined") return;
    initialTabFocusScheduled = true;
    void tick().then(() => {
      initialTabFocusScheduled = false;
      if (initialTabFocusDone) return;
      document.getElementById(`debug-tab-${activeTab}`)?.focus({ preventScroll: true });
      initialTabFocusDone = true;
    });
  });

  function handleDebugTabKeydown(event: KeyboardEvent): void {
    const focusedTab = event.target instanceof Element
      ? event.target.closest<HTMLElement>('[role="tab"]')
      : null;
    const focusedTabId = focusedTab?.id?.replace(/^debug-tab-/, "") ?? null;
    const focusedIndex = debugTabs.findIndex((tab) => tab.id === focusedTabId);
    const currentIndex = focusedIndex >= 0
      ? focusedIndex
      : debugTabs.findIndex((tab) => tab.id === activeTab);
    if (currentIndex === -1) return;
    let nextIndex: number | null = null;
    if (event.key === "ArrowRight" || event.key === "ArrowDown") {
      nextIndex = (currentIndex + 1) % debugTabs.length;
    } else if (event.key === "ArrowLeft" || event.key === "ArrowUp") {
      nextIndex = (currentIndex - 1 + debugTabs.length) % debugTabs.length;
    } else if (event.key === "Home") {
      nextIndex = 0;
    } else if (event.key === "End") {
      nextIndex = debugTabs.length - 1;
    }
    if (nextIndex === null) return;
    event.preventDefault();
    event.stopPropagation();
    const nextTab = debugTabs[nextIndex];
    activeTab = nextTab.id;
    document.getElementById(`debug-tab-${nextTab.id}`)?.focus();
  }

  // ─── Jobs pagination ─────────────────────────────────────────────────────
  // Recent-jobs can grow unbounded; render a fixed-size window. The selected
  // job detail panel is rendered outside the paginated list so the user can
  // still see it after paging away from the row that owns it.
  const JOBS_PAGE_SIZE = 5;
  let jobsPage = $state(0);
  const jobsPageCount = $derived(Math.max(1, Math.ceil(jobs.length / JOBS_PAGE_SIZE)));
  // Clamp the current page when the underlying job list shrinks (e.g. after
  // a refresh that drops a previously-listed job). This avoids landing on an
  // empty page that confusingly shows no rows despite jobs.length > 0.
  $effect(() => {
    if (jobsPage > jobsPageCount - 1) {
      jobsPage = jobsPageCount - 1;
    }
    if (jobsPage < 0) jobsPage = 0;
  });
  const jobsPageStart = $derived(jobsPage * JOBS_PAGE_SIZE);
  const pagedJobs = $derived(jobs.slice(jobsPageStart, jobsPageStart + JOBS_PAGE_SIZE));
  const selectedJobPage = $derived.by(() => {
    if (selectedJobId == null) return null;
    const idx = jobs.findIndex((j) => j.id === selectedJobId);
    if (idx < 0) return null;
    return Math.floor(idx / JOBS_PAGE_SIZE);
  });
  const selectedJobOnAnotherPage = $derived(
    selectedJobPage != null && selectedJobPage !== jobsPage,
  );

  function goToSelectedJobPage() {
    if (selectedJobPage != null) jobsPage = selectedJobPage;
  }

</script>

<!-- ── Page header ──────────────────────────────────────────────────────── -->
<header class="page-header">
  <div class="page-header__row">
    <div class="page-header__title-block">
      <h1 class="page-header__title">Debug</h1>
    </div>
  </div>
  <p class="page-subtitle">Recording status &amp; controls.</p>
</header>

<!-- Wrapper carries the dashed separator below the tab chips so the
     pinned head of the dedicated Debug window matches `.page-header`'s
     bottom divider. The chip card itself keeps its own border. -->
<div class="debug-tabs-wrap">
<div class="debug-tabs" role="tablist" aria-label="Debug sections" tabindex="-1" onkeydown={handleDebugTabKeydown}>
    {#each debugTabs as tab (tab.id)}
      <button
        type="button"
        role="tab"
        id="debug-tab-{tab.id}"
        aria-selected={activeTab === tab.id}
        aria-controls="debug-panel-{tab.id}"
        tabindex={activeTab === tab.id ? 0 : -1}
        class="debug-tabs__btn"
        class:debug-tabs__btn--active={activeTab === tab.id}
        onkeydown={handleDebugTabKeydown}
        onclick={() => (activeTab = tab.id)}
      >
        {tab.label}
      </button>
    {/each}
</div>
</div>

<!-- ── Scroll region ──────────────────────────────────────────────────────
     Only the panel area below the tab strip scrolls. The page header and
     tabs stay pinned at the top of the dedicated Debug window so tab
     switches never push the controls off-screen. See the matching
     `.settings-scroll` block in settings/+page.svelte for the same
     pattern; both rely on `.app-shell--dedicated` being viewport-pinned
     in +layout.svelte. -->
<div class="debug-scroll" bind:this={scrollRegion}>

<!-- ── Recording status ─────────────────────────────────────────────────── -->
{#if activeTab === "session"}
<div class="card" id="debug-panel-session" role="tabpanel" aria-labelledby="debug-tab-session" tabindex="0">
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
</div>
{/if}

<!-- ── Runtime sources ──────────────────────────────────────────────────── -->
{#if activeTab === "runtime"}
<div class="card card--debug" id="debug-panel-runtime" role="tabpanel" aria-labelledby="debug-tab-runtime" tabindex="0">
  <h2 class="card__title">
    <span class="debug-tag">dbg</span>
    Runtime Sources
    <span class="idle-note">capture session · writer · activity</span>
    <button class="btn btn--ghost btn--sm" onclick={() => { fetchIdleDebug(); fetchCapturePrivacyDebug(); }}>↻</button>
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
</div>

<div class="card card--debug" id="debug-panel-privacy" aria-labelledby="debug-tab-runtime">
  <h2 class="card__title">
    <span class="debug-tag">dbg</span>
    Privacy Filter
    <span class="idle-note">last successfully applied ScreenCaptureKit exclusions</span>
    <button class="btn btn--ghost btn--sm" onclick={fetchCapturePrivacyDebug}>↻</button>
  </h2>

  {#if privacyDebugError}
    <p class="debug-err">{privacyDebugError}</p>
  {:else if privacyDebug}
    <ul class="kv-list">
      <li>
        <span class="kv-key kv-key--wide">filter</span>
        <span class={privacyDebug.privacyDebug.privacyFilterApplied ? "badge badge--warn badge--sm" : "badge badge--ok badge--sm"}>
          {privacyDebug.privacyDebug.privacyFilterApplied ? "active" : "empty"}
        </span>
      </li>
      <li>
        <span class="kv-key kv-key--wide">metadata</span>
        <span class="kv-val kv-val--mono">{privacyDebug.metadataEnabled ? "enabled" : "disabled"} · URL {privacyDebug.browserUrlMode}</span>
      </li>
      <li>
        <span class="kv-key kv-key--wide">private windows</span>
        <span class="kv-val kv-val--mono">{privacyDebug.privateBrowserExclusionEnabled ? "enabled" : "disabled"}</span>
      </li>
      <li>
        <span class="kv-key kv-key--wide">reason</span>
        <span class="kv-val kv-val--mono">{privacyDebug.privacyDebug.metadataRedactionReason ?? "none"}</span>
      </li>
      <li>
        <span class="kv-key kv-key--wide">applied bundles</span>
        <span class="kv-val kv-val--mono privacy-debug-list">{formatDebugList(privacyDebug.privacyDebug.currentlyExcludedBundleIds)}</span>
      </li>
      <li>
        <span class="kv-key kv-key--wide">applied windows</span>
        <span class="kv-val kv-val--mono privacy-debug-list">{formatDebugList(privacyDebug.privacyDebug.currentlyExcludedWindowIds)}</span>
      </li>
      <li>
        <span class="kv-key kv-key--wide">website hold</span>
        <span class="kv-val kv-val--mono privacy-debug-list">{formatDebugList(privacyDebug.privacyDebug.websitePrivacyHoldBundleIds)}</span>
      </li>
      <li>
        <span class="kv-key kv-key--wide">hold reasons</span>
        <span class="kv-val kv-val--mono privacy-debug-list">
          {formatDebugList(privacyDebug.privacyDebug.websitePrivacyHolds.map((hold) => `${hold.bundleId}: ${hold.reason}`))}
        </span>
      </li>
    </ul>

    <div class="privacy-debug-grid">
      <div>
        <div class="idle-section-label">evaluated decision</div>
        <p class="privacy-debug-json">{JSON.stringify(privacyDebug.privacyDebug.latestDecision, null, 2)}</p>
      </div>
      <div>
        <div class="idle-section-label">applied decision</div>
        <p class="privacy-debug-json">{JSON.stringify(privacyDebug.privacyDebug.latestAppliedDecision, null, 2)}</p>
      </div>
    </div>

    <div class="idle-section-label">latest metadata snapshot</div>
    {#if privacyDebug.privacyDebug.latestSnapshot}
      <p class="privacy-debug-json">{JSON.stringify(privacyDebug.privacyDebug.latestSnapshot, null, 2)}</p>
    {:else}
      <p class="empty">no metadata snapshot captured yet</p>
    {/if}
  {:else}
    <p class="empty">privacy filter status has not loaded yet</p>
  {/if}
</div>
{/if}

<!-- ── System probe ──────────────────────────────────────────────────────── -->
{#if activeTab === "probe"}
<div class="card" id="debug-panel-probe" role="tabpanel" aria-labelledby="debug-tab-probe" tabindex="0">
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
</div>
{/if}

<!-- ── Native idle debug ─────────────────────────────────────────────────── -->
{#if activeTab === "inactivity"}
<div class="card card--debug" id="debug-panel-inactivity" role="tabpanel" aria-labelledby="debug-tab-inactivity" tabindex="0">
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
          <span class="kv-val kv-val--mono">{formatIdleMs(idleDebug.microphoneActivityDecision.idleMs)}</span>
          {#if idleDebug.microphoneActivitySample.level != null}
            <span class="idle-note">level {(idleDebug.microphoneActivitySample.level * 100).toFixed(0)}%</span>
          {/if}
          {#if !idleDebug.microphoneActivityDecision.enabled}
            <span class="badge badge--neutral badge--sm">off</span>
          {/if}
        </li>
        <li>
           <span class="kv-key kv-key--wide">sys audio activity idle</span>
          <span class="kv-val kv-val--mono">{formatIdleMs(idleDebug.systemAudioActivityDecision.idleMs)}</span>
          {#if idleDebug.systemAudioActivitySample.level != null}
            <span class="idle-note">level {(idleDebug.systemAudioActivitySample.level * 100).toFixed(0)}%</span>
          {/if}
          {#if !idleDebug.systemAudioActivityDecision.enabled}
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
      <div class="detector-card detector-card--mic" class:detector-card--paused={idleDebug.microphonePaused && idleDebug.microphoneActivityDecision.enabled} class:detector-card--off={!idleDebug.microphoneActivityDecision.enabled}>
        <div class="detector-card__header">
          <span class="detector-card__icon">🎙</span>
          <span class="detector-card__name">Microphone</span>
          {#if !idleDebug.microphoneActivityDecision.enabled}
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
          {#if idleDebug.microphoneActivitySample.level != null}
            <div class="detector-card__metric">
              <span class="detector-card__metric-label">level</span>
              <span class="detector-card__metric-value">{(idleDebug.microphoneActivitySample.level * 100).toFixed(0)}%</span>
            </div>
          {/if}
          {#if idleDebug.microphoneActivitySensitivity != null}
            <div class="detector-card__metric">
              <span class="detector-card__metric-label">sensitivity</span>
              <span class="detector-card__metric-source">{idleDebug.microphoneActivitySensitivity}%{#if idleDebug.microphoneActivityDecision.activityThreshold != null} (thr {(idleDebug.microphoneActivityDecision.activityThreshold * 100).toFixed(1)}%){/if}</span>
            </div>
          {/if}
        </div>
      </div>

      <!-- System audio detector -->
      <div class="detector-card detector-card--sysaudio" class:detector-card--paused={idleDebug.systemAudioPaused && idleDebug.systemAudioActivityDecision.enabled} class:detector-card--off={!idleDebug.systemAudioActivityDecision.enabled}>
        <div class="detector-card__header">
          <span class="detector-card__icon">🔊</span>
          <span class="detector-card__name">System Audio</span>
          {#if !idleDebug.systemAudioActivityDecision.enabled}
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
          {#if idleDebug.systemAudioActivitySample.level != null}
            <div class="detector-card__metric">
              <span class="detector-card__metric-label">level</span>
              <span class="detector-card__metric-value">{(idleDebug.systemAudioActivitySample.level * 100).toFixed(0)}%</span>
            </div>
          {/if}
          {#if idleDebug.systemAudioActivitySensitivity != null}
            <div class="detector-card__metric">
              <span class="detector-card__metric-label">sensitivity</span>
              <span class="detector-card__metric-source">{idleDebug.systemAudioActivitySensitivity}%{#if idleDebug.systemAudioActivityDecision.activityThreshold != null} (thr {(idleDebug.systemAudioActivityDecision.activityThreshold * 100).toFixed(1)}%){/if}</span>
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
      <li>
        <span class="kv-key kv-key--wide">mic VAD</span>
        <span class="kv-val kv-val--mono">
          {idleDebug.microphoneVad.configuredAdapter} -> {idleDebug.microphoneVad.effectiveAdapter}
        </span>
        {#if idleDebug.microphoneVad.fallbackReason}
          <span class="idle-note">{idleDebug.microphoneVad.fallbackReason}</span>
        {/if}
      </li>
      {#if idleDebug.systemAudioActivitySensitivity != null}
        <li>
          <span class="kv-key kv-key--wide">sys audio sensitivity</span>
          <span class="kv-val kv-val--mono">{idleDebug.systemAudioActivitySensitivity}%</span>
        </li>
      {/if}
      {#if idleDebug.microphoneActivityDecision.activityThreshold != null}
        <li>
          <span class="kv-key kv-key--wide">mic threshold</span>
          <span class="kv-val kv-val--mono">{(idleDebug.microphoneActivityDecision.activityThreshold * 100).toFixed(1)}%</span>
          <span class="idle-note">normalised level; audio above this counts as activity</span>
        </li>
      {/if}
      {#if idleDebug.microphoneActivitySample.lastUnixMs != null}
        <li>
          <span class="kv-key kv-key--wide">mic raw sample</span>
          <span class="kv-val kv-val--mono">{formatTimestamp(idleDebug.microphoneActivitySample.lastUnixMs)}</span>
          <span class="idle-note">timestamp, not detector decision</span>
        </li>
      {/if}
      {#if idleDebug.systemAudioActivitySample.lastUnixMs != null}
        <li>
          <span class="kv-key kv-key--wide">sys-audio raw sample</span>
          <span class="kv-val kv-val--mono">{formatTimestamp(idleDebug.systemAudioActivitySample.lastUnixMs)}</span>
          <span class="idle-note">timestamp, not detector decision</span>
        </li>
      {/if}
    </ul>
  {:else}
    <p class="empty">—</p>
  {/if}
</div>
{/if}

<!-- ── App Infra / Background Jobs ───────────────────────────────────────── -->
{#if activeTab === "infra"}
<div class="card card--debug" id="debug-panel-infra" role="tabpanel" aria-labelledby="debug-tab-infra" tabindex="0">
  <h2 class="card__title">
    <span class="card__num">05</span>
    <span class="debug-tag">dbg</span>
    App Infra
    <button class="btn btn--ghost btn--sm card__title-action" onclick={refreshAll} disabled={loadingInfraStatus || loadingJobs}>
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
</div>
{/if}

<!-- ── Hidden segment workspace classifier ───────────────────────────────── -->
{#if activeTab === "workspaces"}
<div class="card card--debug" id="debug-panel-workspaces" role="tabpanel" aria-labelledby="debug-tab-workspaces" tabindex="0">
  <h2 class="card__title">
    <span class="debug-tag">dbg</span>
    Segment Workspace Cleanup
    <span class="idle-note">classify a hidden segment workspace dir</span>
  </h2>

  <form
    class="job-submit-form"
    onsubmit={(e) => {
      e.preventDefault();
      classifyWorkspace();
    }}
  >
    <input
      class="job-input"
      type="text"
      placeholder="/…/recordings/YYYY/MM/DD/.session-segment-####"
      bind:value={workspaceDirInput}
      disabled={loadingWorkspaceClassification}
      spellcheck="false"
      autocomplete="off"
    />
    <button
      class="btn btn--primary btn--sm"
      type="submit"
      disabled={loadingWorkspaceClassification || workspaceDirInput.trim() === ""}
    >
      {loadingWorkspaceClassification ? "…" : "classify"}
    </button>
  </form>

  {#if workspaceClassificationError}
    <p class="debug-err">{workspaceClassificationError}</p>
  {:else if workspaceClassificationLoaded && workspaceClassification == null}
    <p class="empty">
      not a hidden segment workspace path (expected a directory named
      <code>.&lt;session&gt;-segment-####</code>)
    </p>
  {:else if workspaceClassification}
    {@const info = workspaceClassification}
    <ul class="kv-list">
      <li>
        <span class="kv-key kv-key--wide">disposition</span>
        <span class={dispositionBadgeClass(info.disposition)}>
          {dispositionLabel(info.disposition)}
        </span>
      </li>
      <li>
        <span class="kv-key kv-key--wide">safe to remove</span>
        <span class={info.safeToRemove ? "badge badge--ok badge--sm" : "badge badge--warn badge--sm"}>
          {info.safeToRemove ? "yes" : "no"}
        </span>
      </li>
      <li>
        <span class="kv-key kv-key--wide">visible segment</span>
        <span class={info.visibleSegmentExists ? "badge badge--ok badge--sm" : "badge badge--err badge--sm"}>
          {info.visibleSegmentExists ? "present" : "missing"}
        </span>
        <span class="kv-val kv-val--mono" title={info.paths.visibleSegmentPath}>
          {shortenPath(info.paths.visibleSegmentPath)}
        </span>
      </li>
      <li>
        <span class="kv-key kv-key--wide">frame count</span>
        <span class="kv-val kv-val--mono">{info.frameCount}</span>
      </li>
      <li>
        <span class="kv-key kv-key--wide">workspace</span>
        <span class="kv-val kv-val--mono" title={info.paths.workspaceDir}>
          {shortenPath(info.paths.workspaceDir)}
        </span>
      </li>
      <li>
        <span class="kv-key kv-key--wide">frames dir</span>
        <span class="kv-val kv-val--mono" title={info.paths.framesDir}>
          {shortenPath(info.paths.framesDir)}
        </span>
      </li>
    </ul>

    <div class="idle-section-label">
      Batch references
      <span class="idle-note">{info.batchReferences.length}</span>
    </div>
    {#if info.batchReferences.length === 0}
      <p class="empty">none</p>
    {:else}
      <ul class="kv-list">
        {#each info.batchReferences as ref (ref.batchId)}
          <li>
            <span class="kv-key kv-key--wide">batch #{ref.batchId}</span>
            <span class={batchStatusBadgeClass(ref.status)}>{ref.status}</span>
          </li>
        {/each}
      </ul>
    {/if}

    <div class="idle-section-label">
      Non-terminal OCR references
      <span class="idle-note">{info.nonterminalOcrReferences.length}</span>
    </div>
    {#if info.nonterminalOcrReferences.length === 0}
      <p class="empty">none</p>
    {:else}
      <ul class="kv-list">
        {#each info.nonterminalOcrReferences as ref (ref.jobId)}
          <li>
            <span class="kv-key kv-key--wide">frame #{ref.frameId} · job #{ref.jobId}</span>
            <span class={ocrStatusBadgeClass(ref.status)}>{ref.status}</span>
          </li>
        {/each}
      </ul>
    {/if}
  {:else}
    <p class="empty">enter a hidden segment workspace path to classify</p>
  {/if}
</div>
{/if}

<!-- ── Background Jobs ───────────────────────────────────────────────────── -->
{#if activeTab === "jobs"}
<div class="card card--debug" id="debug-panel-jobs" role="tabpanel" aria-labelledby="debug-tab-jobs" tabindex="0">
  <h2 class="card__title">
    <span class="card__num">06</span>
    <span class="debug-tag">dbg</span>
    Background Jobs
    <button class="btn btn--ghost btn--sm card__title-action" onclick={refreshAll} disabled={loadingJobs || loadingInfraStatus}>
      {loadingJobs ? "…" : "↻ list"}
    </button>
    {#if postSubmitPollInterval != null}
      <span class="idle-note">polling ({POST_SUBMIT_POLL_MAX - postSubmitPollCount} left)</span>
    {/if}
  </h2>

  <!-- Submit form -->
  <details class="advanced">
    <summary class="advanced__summary">Submit debug CPU job</summary>
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
  </details>

  <!-- Job list -->
  <div class="idle-section-label">
    Recent jobs
    {#if loadingJobs}<span class="idle-note">loading…</span>{/if}
    {#if jobs.length > 0}
      <span class="idle-note">
        {jobsPageStart + 1}–{Math.min(jobsPageStart + JOBS_PAGE_SIZE, jobs.length)} of {jobs.length}
      </span>
    {/if}
  </div>
  {#if jobsError}
    <p class="debug-err">{jobsError}</p>
  {:else if jobs.length === 0}
    <p class="empty">no jobs yet</p>
  {:else}
    <ul class="job-list">
      {#each pagedJobs as job (job.id)}
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
    {#if jobsPageCount > 1}
      <div class="job-pager" role="group" aria-label="Recent jobs pagination">
        <button
          type="button"
          class="btn btn--ghost btn--sm"
          onclick={() => (jobsPage = Math.max(0, jobsPage - 1))}
          disabled={jobsPage === 0}
          aria-label="Previous page"
        >
          ‹ prev
        </button>
        <span class="job-pager__info">
          page {jobsPage + 1} / {jobsPageCount}
        </span>
        <button
          type="button"
          class="btn btn--ghost btn--sm"
          onclick={() => (jobsPage = Math.min(jobsPageCount - 1, jobsPage + 1))}
          disabled={jobsPage >= jobsPageCount - 1}
          aria-label="Next page"
        >
          next ›
        </button>
      </div>
    {/if}
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
      {#if selectedJobOnAnotherPage}
        <button
          type="button"
          class="btn btn--ghost btn--sm"
          onclick={goToSelectedJobPage}
          style="margin-left: 6px;"
          aria-label="Jump to the page containing the selected job"
        >
          show in list
        </button>
      {/if}
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
</div>
{/if}

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

</div><!-- /.debug-scroll -->

<style>
  /* ── Scroll region ────────────────────────────────────────────────────
     Wraps every tab panel + the bottom error card so the page header and
     `.debug-tabs` strip stay pinned while only this region scrolls.
     `flex: 1 1 0` claims the leftover height inside `.app-content`, and
     `min-height: 0` lets the child shrink below its intrinsic content
     height in the flex column (without it the whole dedicated Debug
     window would scroll). */
  .debug-scroll {
    flex: 1 1 0;
    min-height: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }

  /* ── Page header ───────────────────────────────────────────── */
  .page-header {
    display: flex;
    flex-direction: column;
    gap: 12px;
    margin-bottom: 6px;
    padding-bottom: 12px;
    border-bottom: 1px dashed var(--app-border);
  }

  .page-header__row {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    gap: 12px;
  }

  .page-header__title-block {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .page-header__title {
    font-size: 14px;
    font-weight: 700;
    letter-spacing: 0.04em;
    color: var(--app-text-strong);
    line-height: 1.1;
  }

  .page-header__close {
    min-width: 72px;
  }

  .page-header__meta {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .page-header__pill {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 3px 9px;
    border-radius: 999px;
    background: var(--app-surface);
    border: 1px solid var(--app-border);
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: var(--app-text-muted);
  }

  .page-header__pill--rec {
    background: var(--app-danger-bg-soft);
    border-color: var(--app-danger-border);
    color: var(--app-danger);
  }

  .page-header__pill--warn {
    background: var(--app-warn-bg);
    border-color: var(--app-warn-border);
    color: var(--app-warn);
  }

  .page-header__pill-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--app-text-faint);
  }

  .page-header__pill-dot--rec {
    background: var(--app-danger-strong);
    animation: pulse-rec 1.2s ease-in-out infinite;
  }

  .page-subtitle {
    font-size: 10px;
    color: var(--app-text-subtle);
    letter-spacing: 0.06em;
  }

  /* ── Cards ─────────────────────────────────────────────────── */
  .card {
    background: var(--app-surface-raised);
    border: 1px solid var(--app-border);
    border-radius: 6px;
    padding: 18px 20px;
    display: flex;
    flex-direction: column;
    gap: 14px;
    position: relative;
  }

  .card::before {
    content: "";
    position: absolute;
    left: 14px;
    right: 14px;
    top: 0;
    height: 1px;
    background: linear-gradient(90deg, transparent 0%, var(--app-border-strong) 30%, var(--app-border-strong) 70%, transparent 100%);
    opacity: 0.6;
  }

  .card--debug::before {
    background: linear-gradient(90deg, transparent 0%, var(--app-border-strong) 50%, transparent 100%);
  }

  .card--error {
    border-color: var(--app-danger-border);
    background: var(--app-danger-bg-soft);
  }

  .card__title {
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: var(--app-text-subtle);
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .card__num {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 22px;
    height: 16px;
    padding: 0 4px;
    background: var(--app-surface);
    border: 1px solid var(--app-border);
    border-radius: 2px;
    font-size: 9px;
    font-weight: 800;
    letter-spacing: 0.08em;
    color: var(--app-text-muted);
  }

  .card__title-action {
    margin-left: auto;
  }

  /* ── Collapsible advanced blocks ─────────────────────────── */
  .advanced {
    border-top: 1px dashed var(--app-border);
    padding-top: 10px;
    margin-top: 2px;
  }

  .advanced > :global(*) {
    margin-top: 10px;
  }

  .advanced > :global(*:first-child) {
    margin-top: 0;
  }

  .advanced__summary {
    list-style: none;
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    gap: 8px;
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: var(--app-text-muted);
    user-select: none;
    transition: color 0.12s;
  }

  .advanced__summary::-webkit-details-marker { display: none; }

  .advanced__summary::before {
    content: "▸";
    display: inline-block;
    font-size: 9px;
    color: var(--app-text-subtle);
    transition: transform 0.15s;
  }

  .advanced[open] > .advanced__summary::before {
    transform: rotate(90deg);
  }

  .advanced__summary:hover {
    color: var(--app-text);
  }

  /* ── Session status ─────────────────────────────────────────── */
  .session-status {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 14px;
    background: var(--app-surface);
    border: 1px solid var(--app-border);
    border-radius: 5px;
    transition: background 0.2s, border-color 0.2s;
  }

  .session-status--recording {
    background: var(--app-source-mic-bg);
    border-color: var(--app-source-mic-border);
  }

  .rec-dot {
    display: block;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--app-text-faint);
    flex-shrink: 0;
    transition: background 0.2s;
  }

  .rec-dot--active {
    background: var(--app-danger-strong);
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
    color: var(--app-text-subtle);
    transition: color 0.2s;
  }

  .session-status--recording .session-label {
    color: var(--app-danger);
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
    background: var(--app-surface);
    border: 1px solid var(--app-border);
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
    color: var(--app-text-faint);
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
    background: var(--app-accent-bg);
    color: var(--app-accent);
    border-color: var(--app-accent-border);
  }

  .btn--primary:not(:disabled):hover {
    background: var(--app-surface-active);
    border-color: var(--app-accent);
  }

  .btn--danger {
    background: var(--app-danger-bg);
    color: var(--app-danger);
    border-color: var(--app-danger-border);
  }

  .btn--danger:not(:disabled):hover {
    background: var(--app-danger-bg-soft);
    border-color: var(--app-danger);
  }

  .btn--ghost {
    background: transparent;
    color: var(--app-text-muted);
    border-color: var(--app-border-strong);
    font-size: 10px;
  }

  .btn--ghost:not(:disabled):hover {
    background: var(--app-surface-hover);
    color: var(--app-text);
    border-color: var(--app-border-hover);
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
    background: var(--app-surface);
    border: 1px solid var(--app-border);
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
    color: var(--app-text-muted);
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
    color: var(--app-text-subtle);
    font-size: 10px;
    white-space: nowrap;
    min-width: 60px;
  }

  .kv-val {
    color: var(--app-text);
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
    background: var(--app-accent-bg);
    color: var(--app-accent);
    border: 1px solid var(--app-accent-border);
  }

  .badge--err {
    background: var(--app-danger-bg);
    color: var(--app-danger);
    border: 1px solid var(--app-danger-border);
  }

  .badge--neutral {
    background: var(--app-neutral-bg);
    color: var(--app-neutral-text);
    border: 1px solid var(--app-neutral-border);
  }

  .badge--sm {
    padding: 0 5px;
    font-size: 9px;
  }

  /* ── Error ──────────────────────────────────────────────────── */
  .error-pre {
    background: var(--app-danger-bg-soft);
    border: 1px solid var(--app-danger-border);
    border-radius: 4px;
    padding: 10px 12px;
    font-family: inherit;
    font-size: 11px;
    color: var(--app-danger-text);
    white-space: pre-wrap;
    word-break: break-all;
    max-height: 160px;
    overflow-y: auto;
  }

  /* ── Misc ───────────────────────────────────────────────────── */
  .empty {
    color: var(--app-text-faint);
    font-size: 11px;
    font-style: italic;
  }

  /* ── Debug card ─────────────────────────────────────────────── */
  .card--debug {
    border-style: dashed;
    border-color: var(--app-border-strong);
    background: var(--app-surface);
    opacity: 0.92;
  }

  .debug-tag {
    display: inline-flex;
    align-items: center;
    padding: 0 5px;
    background: var(--app-neutral-bg);
    border: 1px solid var(--app-neutral-border);
    border-radius: 2px;
    font-size: 8px;
    font-weight: 800;
    letter-spacing: 0.1em;
    color: var(--app-text-muted);
    text-transform: uppercase;
  }

  .kv-key--wide {
    min-width: 120px;
  }

  .kv-val--mono {
    font-family: "SF Mono", "Fira Mono", "Courier New", monospace;
    font-size: 10px;
    color: var(--app-text);
  }

  .privacy-debug-list {
    overflow-wrap: anywhere;
    line-height: 1.5;
  }

  .privacy-debug-grid {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 10px;
    margin-top: 12px;
  }

  .privacy-debug-json {
    margin: 4px 0 0;
    padding: 8px;
    border: 1px solid var(--app-border);
    border-radius: 4px;
    background: var(--app-surface-raised);
    color: var(--app-text-muted);
    font-family: "SF Mono", "Fira Mono", "Courier New", monospace;
    font-size: 10px;
    line-height: 1.45;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  @media (max-width: 760px) {
    .privacy-debug-grid {
      grid-template-columns: 1fr;
    }
  }

  /* ── Idle debug sub-sections ────────────────────────── */
  .idle-section-label {
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: var(--app-text-faint);
    margin-top: 4px;
  }

  .idle-note {
    font-size: 9px;
    color: var(--app-text-faint);
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
    background: var(--app-surface);
    border: 1px solid var(--app-border);
    border-radius: 5px;
    padding: 10px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    transition: border-color 0.15s, background 0.15s;
  }

  .detector-card--screen { border-left: 2px solid var(--app-source-screen-strong); }
  .detector-card--mic { border-left: 2px solid var(--app-source-mic-strong); }
  .detector-card--sysaudio { border-left: 2px solid var(--app-source-sysaudio-strong); }

  .detector-card--paused { background: var(--app-warn-bg); }
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
    color: var(--app-text-muted);
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
    color: var(--app-text-subtle);
    min-width: 56px;
  }

  .detector-card__metric-value {
    font-family: "SF Mono", "Fira Mono", "Courier New", monospace;
    font-size: 11px;
    font-weight: 700;
    color: var(--app-source-screen);
    letter-spacing: 0.04em;
  }

  .detector-card--mic .detector-card__metric-value { color: var(--app-source-mic); }
  .detector-card--sysaudio .detector-card__metric-value { color: var(--app-source-sysaudio); }

  .detector-card__metric-source {
    font-family: "SF Mono", "Fira Mono", "Courier New", monospace;
    font-size: 9px;
    color: var(--app-text-muted);
  }

  /* ── Effective idle summary (subordinate) ────────────── */
  .effective-idle-summary {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 10px;
    background: var(--app-surface-raised);
    border: 1px solid var(--app-border);
    border-radius: 3px;
  }

  .effective-idle-summary__label {
    font-size: 9px;
    color: var(--app-text-subtle);
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }

  .effective-idle-summary__value {
    font-family: "SF Mono", "Fira Mono", "Courier New", monospace;
    font-size: 11px;
    font-weight: 700;
    color: var(--app-text);
  }

  .effective-idle-summary__source {
    font-family: "SF Mono", "Fira Mono", "Courier New", monospace;
    font-size: 9px;
    color: var(--app-text-muted);
  }

  .effective-idle-block__note {
    font-size: 9px;
    color: var(--app-text-muted);
    line-height: 1.5;
    margin-top: 4px;
    border-top: 1px solid var(--app-border);
    padding-top: 6px;
  }

  .effective-idle-block__note em {
    font-style: normal;
    color: var(--app-source-screen);
    font-weight: 600;
  }

  .debug-err {
    font-size: 10px;
    color: var(--app-danger);
    font-family: "SF Mono", "Fira Mono", "Courier New", monospace;
  }

  .badge--warn {
    background: var(--app-warn-bg);
    color: var(--app-warn);
    border: 1px solid var(--app-warn-border);
  }

  /* ── Inactivity hint ────────────────────────────────────────── */
  .inactivity-hint {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    background: var(--app-warn-bg);
    border: 1px solid var(--app-warn-border);
    border-radius: 4px;
  }

  .inactivity-hint__dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--app-warn-strong);
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
    color: var(--app-warn);
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
    color: var(--app-text-subtle);
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
    background: var(--app-surface);
    border: 1px solid var(--app-border-strong);
    border-radius: 3px;
    padding: 4px 8px;
    font-family: inherit;
    font-size: 11px;
    color: var(--app-text);
    outline: none;
    flex: 1;
    min-width: 80px;
    transition: border-color 0.12s;
  }

  .job-input:focus {
    border-color: var(--app-info-strong);
  }

  .job-input::placeholder {
    color: var(--app-text-faint);
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
    background: var(--app-surface-raised);
    border-color: var(--app-border-strong);
  }

  .job-row--selected {
    background: var(--app-info-bg);
    border-color: var(--app-info-border);
  }

  .job-row__id {
    font-family: "SF Mono", "Fira Mono", "Courier New", monospace;
    font-size: 9px;
    color: var(--app-text-subtle);
    min-width: 28px;
    flex-shrink: 0;
  }

  .job-row__kind {
    font-size: 10px;
    color: var(--app-info-strong);
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .job-row__ts {
    font-family: "SF Mono", "Fira Mono", "Courier New", monospace;
    font-size: 9px;
    color: var(--app-text-faint);
    flex-shrink: 0;
    margin-left: auto;
  }

  .badge--running {
    background: var(--app-info-bg);
    color: var(--app-info);
    border: 1px solid var(--app-info-border);
  }

  .kv-list-block {
    flex-direction: column !important;
    align-items: flex-start !important;
    gap: 4px !important;
  }

  .job-detail-text {
    font-family: "SF Mono", "Fira Mono", "Courier New", monospace;
    font-size: 10px;
    color: var(--app-neutral-text);
    white-space: pre-wrap;
    word-break: break-all;
    line-height: 1.5;
    max-height: 80px;
    overflow-y: auto;
    display: block;
    padding: 4px 0;
  }

  .job-detail-text--err {
    color: var(--app-danger);
  }

  /* ── Runtime sources ────────────────────────────────────────── */
  .rs-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
    gap: 10px;
  }

  .rs-lane {
    position: relative;
    background: var(--app-surface);
    border: 1px solid var(--app-border);
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
  .rs-lane--screen::before { background: linear-gradient(180deg, var(--app-source-screen-strong) 0%, var(--app-source-screen-border) 100%); }
  .rs-lane--microphone::before { background: linear-gradient(180deg, var(--app-source-mic-strong) 0%, var(--app-source-mic-border) 100%); }
  .rs-lane--systemAudio::before { background: linear-gradient(180deg, var(--app-source-sysaudio-strong) 0%, var(--app-source-sysaudio-border) 100%); }

  .rs-lane--off { opacity: 0.5; }
  .rs-lane--paused { background: var(--app-warn-bg); border-color: var(--app-warn-border); }
  .rs-lane--running { border-color: var(--app-accent-border); }

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
    color: var(--app-text-muted);
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
  .rs-state--running { background: var(--app-source-mic-bg); color: var(--app-source-mic); border-color: var(--app-source-mic-border); }
  .rs-state--paused { background: var(--app-warn-bg); color: var(--app-warn); border-color: var(--app-warn-border); }
  .rs-state--partial { background: var(--app-source-screen-bg); color: var(--app-source-screen); border-color: var(--app-source-screen-border); }
  .rs-state--idle { background: var(--app-neutral-bg); color: var(--app-neutral-text); border-color: var(--app-neutral-border); }
  .rs-state--off { background: var(--app-surface-raised); color: var(--app-text-subtle); border-color: var(--app-border); }
  .rs-state--unknown { background: var(--app-danger-bg-soft); color: var(--app-danger-text); border-color: var(--app-danger-border); }

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
    border-top: 1px dashed var(--app-border);
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
    color: var(--app-text-subtle);
  }

  /* Indicator bar — colour by state, mimicking a tiny status LED. */
  .rs-row__bar {
    position: relative;
    display: inline-block;
    width: 100%;
    height: 4px;
    border-radius: 2px;
    background: var(--app-neutral-bg);
    overflow: hidden;
  }
  .rs-row__bar-fill {
    position: absolute;
    inset: 0;
    border-radius: inherit;
    transition: background 0.15s, opacity 0.15s;
  }
  .rs-row__bar[data-state="on"] .rs-row__bar-fill {
    background: linear-gradient(90deg, var(--app-source-mic-strong) 0%, var(--app-source-mic) 100%);
    box-shadow: 0 0 6px var(--app-accent-glow);
  }
  .rs-row__bar[data-state="paused"] .rs-row__bar-fill {
    background: linear-gradient(90deg, var(--app-warn-strong) 0%, var(--app-warn) 100%);
    opacity: 0.85;
  }
  .rs-row__bar[data-state="off"] .rs-row__bar-fill { background: var(--app-neutral-border); opacity: 0.6; }
  .rs-row__bar[data-state="idle"] .rs-row__bar-fill { background: var(--app-neutral-text); }
  .rs-row__bar[data-state="unknown"] .rs-row__bar-fill { background: repeating-linear-gradient(45deg, var(--app-danger-border) 0 3px, var(--app-danger-bg-soft) 3px 6px); }

  .rs-row__val {
    font-size: 10px;
    color: var(--app-text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .rs-row__val--mono {
    font-family: "SF Mono", "Fira Mono", "Courier New", monospace;
    font-size: 10px;
    color: var(--app-text-muted);
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
    background: var(--app-surface-raised);
    border: 1px solid var(--app-border);
    border-radius: 3px;
  }
  .rs-path__label {
    font-size: 8px;
    font-weight: 700;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: var(--app-text-subtle);
  }
  .rs-path__val {
    font-family: "SF Mono", "Fira Mono", "Courier New", monospace;
    font-size: 10px;
    color: var(--app-text-muted);
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
    color: var(--app-text-muted);
    letter-spacing: 0.04em;
    padding-top: 4px;
    border-top: 1px dashed var(--app-border);
  }
  .rs-legend span { display: inline-flex; align-items: center; gap: 5px; }
  .rs-legend__dot {
    display: inline-block;
    width: 8px;
    height: 4px;
    border-radius: 1px;
  }
  .rs-legend__dot--on { background: linear-gradient(90deg, var(--app-source-mic-strong) 0%, var(--app-source-mic) 100%); }
  .rs-legend__dot--paused { background: linear-gradient(90deg, var(--app-warn-strong) 0%, var(--app-warn) 100%); }
  .rs-legend__dot--off { background: var(--app-neutral-border); }

  /* ── Section tabs ──────────────────────────────────────────── */
  .debug-tabs-wrap {
    padding-bottom: 12px;
    border-bottom: 1px dashed var(--app-border);
  }
  .debug-tabs {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    padding: 4px;
    border: 1px solid var(--app-border);
    border-radius: 6px;
    background: var(--app-surface);
  }

  .debug-tabs__btn {
    appearance: none;
    border: 1px solid transparent;
    background: transparent;
    color: var(--app-text-muted);
    font: inherit;
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    padding: 6px 10px;
    border-radius: 4px;
    cursor: pointer;
    transition: color 0.12s, background 0.12s, border-color 0.12s;
  }

  .debug-tabs__btn:hover {
    color: var(--app-text);
    background: var(--app-surface-raised);
  }

  .debug-tabs__btn:focus-visible {
    outline: none;
    border-color: var(--app-accent);
    box-shadow: 0 0 0 2px var(--app-accent-glow);
  }

  .debug-tabs__btn--active {
    color: var(--app-text-strong);
    background: var(--app-surface-raised);
    border-color: var(--app-border-strong);
  }

  /* ── Jobs pager ─────────────────────────────────────────────── */
  .job-pager {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-top: 8px;
    padding-top: 8px;
    border-top: 1px dashed var(--app-border);
  }

  .job-pager__info {
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: var(--app-text-muted);
  }

</style>
