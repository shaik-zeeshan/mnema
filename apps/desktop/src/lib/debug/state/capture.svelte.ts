// Capture-side debug state: support/permissions probes, recording settings,
// session lifecycle (start/stop + reconciliation + wake resync), the native
// idle debug snapshot and the privacy-filter snapshot.
//
// A behaviour-preserving move of the capture half of the legacy
// routes/debug/+page.svelte <script>. The comments that document *why* each
// guard exists (generation counters, stale-status threshold, the wake-drift
// watchdog) are carried over verbatim — they encode real bugs.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { captureSession, setSession } from "$lib/session.svelte";
import { humanizeError } from "$lib/format-error";
import type {
	CapturePrivacyDebugInfo,
	CaptureSession,
	CaptureSupport,
	GetPermissionsResponse,
	IdleDebugInfo,
	MicrophoneAutoDisconnectTransitionFailedEvent,
	MicrophoneControllerState,
	PermissionsMap,
	RecordingSettings,
} from "$lib/types";

export type CaptureSource = "screen" | "microphone" | "systemAudio";
type SourceSessionLookup = Partial<Record<CaptureSource, { sessionId: string; startedAtUnixMs: number } | null>>;

const RECORDING_SETTINGS_CHANGED_EVENT = "recording_settings_changed";

/**
 * Background reconcile / wake-resync are best-effort and swallow transient
 * errors. But a sustained string of failures means the displayed "Recording"
 * status may no longer reflect the backend — surface a note after N
 * consecutive misses so the user knows the readout may be stale.
 */
const RECONCILE_STALE_THRESHOLD = 3;
const RECONCILE_MS = 5_000;

// Tauri/macOS does not reliably flip `document.visibilityState` on every wake
// (the webview can stay "visible" while the system slept), so the resync also
// watches a wall-clock drift watchdog. The 5s threshold is generous enough that
// normal jank/GC pauses don't trigger a resync; a real sleep is tens of seconds.
const WAKE_DRIFT_THRESHOLD_MS = 5_000;
const WAKE_DRIFT_TICK_MS = 1_000;

const IDLE_POLL_MS = 2_000;

export function createCaptureStore() {
	let support = $state<CaptureSupport | null>(null);
	let permissions = $state<PermissionsMap | null>(null);
	// Per-block probe errors so a failed auto-load (on mount) surfaces inline
	// instead of silently falling back to the "not queried yet" empty state.
	let supportError = $state<string | null>(null);
	let permissionsError = $state<string | null>(null);
	let recordingSettings = $state<RecordingSettings | null>(null);

	/**
	 * Generation counter that increments on every *authoritative* session write
	 * (start / stop). Reconciliation polling captures the value before its async
	 * IPC and skips the write if the generation advanced while in-flight,
	 * preventing a slow response from overwriting a newer stopped state.
	 */
	let sessionGeneration = 0;
	let reconcileFailures = $state(0);

	let lastError = $state<string | null>(null);
	// Start/Stop failures get their own inline chip beside the action buttons so
	// the failure is visible where the user clicked, not only in the page-bottom
	// error card.
	let lifecycleError = $state<string | null>(null);
	let loadingSupport = $state(false);
	let loadingPermissions = $state(false);
	let loadingStart = $state(false);
	let loadingStop = $state(false);
	let loadingSettings = $state(false);

	let idleDebug = $state<IdleDebugInfo | null>(null);
	let idleDebugError = $state<string | null>(null);
	let privacyDebug = $state<CapturePrivacyDebugInfo | null>(null);
	let privacyDebugError = $state<string | null>(null);

	// Per-button loading flags for the manual ↻ refresh controls. The underlying
	// fetch fns are also driven by polling/effects, so each button owns its own
	// flag (set in try/finally) rather than reading a shared one — polling won't
	// flicker the button, and the button disables only itself while in flight.
	let loadingRuntimeSources = $state(false);
	let loadingPrivacyFilter = $state(false);
	let loadingInactivity = $state(false);

	function clearError() {
		lastError = null;
		lifecycleError = null;
	}

	function setError(err: unknown) {
		lastError = humanizeError(err);
	}

	// ─── Probes ──────────────────────────────────────────────────────────────

	async function loadSupport() {
		loadingSupport = true;
		clearError();
		try {
			support = await invoke<CaptureSupport>("get_capture_support");
			supportError = null;
		} catch (err) {
			support = null;
			supportError = humanizeError(err);
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
			permissionsError = null;
			// Only apply the session when no authoritative action (start/stop)
			// occurred while this request was in-flight.
			if (result.session && sessionGeneration === gen) setSession(result.session);
		} catch (err) {
			permissions = null;
			permissionsError = humanizeError(err);
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

	// ─── Live snapshots ──────────────────────────────────────────────────────

	async function fetchIdleDebug() {
		// Skip the round-trip when the page is hidden or no capture session is
		// active — the debug panel is only meaningful while recording (or briefly
		// after stop).
		if (typeof document !== "undefined" && document.visibilityState === "hidden") return;
		if (!captureSession.value?.isRunning) return;
		try {
			idleDebug = await invoke<IdleDebugInfo>("get_idle_debug");
			idleDebugError = null;
		} catch (err) {
			idleDebugError = humanizeError(err);
		}
	}

	async function fetchCapturePrivacyDebug() {
		if (typeof document !== "undefined" && document.visibilityState === "hidden") return;
		if (!captureSession.value?.isRunning) return;
		try {
			privacyDebug = await invoke<CapturePrivacyDebugInfo>("get_capture_privacy_debug");
			privacyDebugError = null;
		} catch (err) {
			privacyDebugError = humanizeError(err);
		}
	}

	async function refreshRuntimeSources() {
		loadingRuntimeSources = true;
		try {
			await Promise.all([fetchIdleDebug(), fetchCapturePrivacyDebug()]);
		} finally {
			loadingRuntimeSources = false;
		}
	}

	async function refreshPrivacyFilter() {
		loadingPrivacyFilter = true;
		try {
			await fetchCapturePrivacyDebug();
		} finally {
			loadingPrivacyFilter = false;
		}
	}

	async function refreshInactivity() {
		loadingInactivity = true;
		try {
			await fetchIdleDebug();
		} finally {
			loadingInactivity = false;
		}
	}

	// ─── Lifecycle ───────────────────────────────────────────────────────────

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
			reconcileFailures = 0;
		} catch (err) {
			setError(err);
			lifecycleError = humanizeError(err);
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
			reconcileFailures = 0;
		} catch (err) {
			setError(err);
			lifecycleError = humanizeError(err);
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

	/**
	 * Re-fetch the session from the backend. Snapshots the generation before the
	 * async round-trip: if an authoritative action (start/stop) lands while the
	 * request is in flight the generation will have advanced and the (now-stale)
	 * response must be discarded to avoid overwriting the newer state.
	 */
	async function resyncSession() {
		const gen = sessionGeneration;
		try {
			const r = await invoke<GetPermissionsResponse>("get_capture_permissions");
			if (sessionGeneration !== gen) return; // stale — discard
			if (r.session) setSession(r.session);
			reconcileFailures = 0;
		} catch {
			// Best-effort — a transient backend error should not crash the UI, but
			// count it so a sustained outage surfaces a "status may be stale" note.
			reconcileFailures += 1;
		}
	}

	// ─── Effect drivers (called from the shell's $effect blocks) ─────────────

	/** Mount-time loaders + microphone/settings event listeners. Returns teardown. */
	function initListeners(): () => void {
		let unlistenControllerChanged: (() => void) | undefined;
		let unlistenAutoDisconnectFailure: (() => void) | undefined;
		let unlistenRecordingSettingsChanged: (() => void) | undefined;
		let destroyed = false;

		listen<MicrophoneControllerState>("microphone_controller_changed", () => {
			clearError();
		}).then((fn) => {
			if (destroyed) fn();
			else unlistenControllerChanged = fn;
		}).catch(() => {
			// Non-fatal: this listener only clears stale errors. Nothing to surface.
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
		}).catch(() => {
			// This is the channel that reports microphone auto-disconnect failures —
			// if we can't subscribe, those failures would go unreported. Surface it
			// once so the operator knows this debug signal is missing.
			if (!destroyed && !lastError) {
				setError("Could not subscribe to microphone auto-disconnect failure events — those failures will not be reported here.");
			}
		});

		listen<RecordingSettings>(RECORDING_SETTINGS_CHANGED_EVENT, (event) => {
			recordingSettings = event.payload;
			clearError();
		}).then((fn) => {
			if (destroyed) fn();
			else unlistenRecordingSettingsChanged = fn;
		}).catch(() => {
			// Non-fatal: settings still load via loadSettings(); live updates only.
		});

		return () => {
			destroyed = true;
			unlistenControllerChanged?.();
			unlistenAutoDisconnectFailure?.();
			unlistenRecordingSettingsChanged?.();
		};
	}

	/** 2s idle + privacy snapshot poll. Returns teardown. */
	function startIdlePolling(): () => void {
		void fetchIdleDebug();
		void fetchCapturePrivacyDebug();
		const interval = setInterval(() => {
			void fetchIdleDebug();
			void fetchCapturePrivacyDebug();
		}, IDLE_POLL_MS);
		return () => clearInterval(interval);
	}

	/** 5s session reconcile while the UI believes capture is running. */
	function startReconcilePolling(): () => void {
		const interval = setInterval(() => {
			// Skip when the tab is hidden — avoids unnecessary IPC while inactive.
			if (typeof document !== "undefined" && document.visibilityState === "hidden") return;
			void resyncSession();
		}, RECONCILE_MS);
		return () => clearInterval(interval);
	}

	/**
	 * Wake/visibility resync. After macOS sleep/wake the native capture pipeline
	 * may have been torn down and restarted while the webview was suspended,
	 * leaving the shared session store stale. The backend-emitted `system_did_wake`
	 * event is the primary reliable trigger; foreground/drift heuristics remain as
	 * backstops. Mirrors the dashboard's wake resync.
	 */
	function startWakeResync(): () => void {
		if (typeof document === "undefined") return () => {};
		const onVisibility = () => {
			if (document.visibilityState !== "visible") return;
			void resyncSession();
		};
		const onFocus = () => { void resyncSession(); };
		let unlistenSystemDidWake: (() => void) | undefined;
		let destroyed = false;

		listen("system_did_wake", () => {
			void resyncSession();
		}).then((fn) => {
			if (destroyed) fn();
			else unlistenSystemDidWake = fn;
		}).catch(() => {
			// Non-fatal: focus/visibility/drift backstops still trigger resync; only
			// the explicit wake event is lost.
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
				void resyncSession();
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
	}

	// ─── Source-session lookups ──────────────────────────────────────────────

	function getSourceSession(value: CaptureSession | null | undefined, source: CaptureSource) {
		const sourceSessions = (value as { sourceSessions?: SourceSessionLookup | null } | null)?.sourceSessions;
		return sourceSessions?.[source] ?? null;
	}

	return {
		// Read-only alias — writes go through captureSession.value so the shared
		// store (and the layout's activity reporter) always see fresh state.
		get session() { return captureSession.value; },
		get isCapturing() { return captureSession.value?.isRunning === true; },
		get isInactivityPaused() { return captureSession.value?.isInactivityPaused === true; },
		get reconcileStale() { return reconcileFailures >= RECONCILE_STALE_THRESHOLD; },

		get support() { return support; },
		get supportError() { return supportError; },
		get permissions() { return permissions; },
		get permissionsError() { return permissionsError; },
		get recordingSettings() { return recordingSettings; },
		get idleDebug() { return idleDebug; },
		get idleDebugError() { return idleDebugError; },
		get privacyDebug() { return privacyDebug; },
		get privacyDebugError() { return privacyDebugError; },

		get lastError() { return lastError; },
		set lastError(v: string | null) { lastError = v; },
		get lifecycleError() { return lifecycleError; },
		get loadingSupport() { return loadingSupport; },
		get loadingPermissions() { return loadingPermissions; },
		get loadingSettings() { return loadingSettings; },
		get loadingStart() { return loadingStart; },
		get loadingStop() { return loadingStop; },
		get loadingRuntimeSources() { return loadingRuntimeSources; },
		get loadingPrivacyFilter() { return loadingPrivacyFilter; },
		get loadingInactivity() { return loadingInactivity; },

		loadSupport,
		loadPermissions,
		loadSettings,
		startCapture,
		stopCapture,
		fetchIdleDebug,
		fetchCapturePrivacyDebug,
		refreshRuntimeSources,
		refreshPrivacyFilter,
		refreshInactivity,
		initListeners,
		startIdlePolling,
		startReconcilePolling,
		startWakeResync,

		getSourceSessionId(value: CaptureSession | null | undefined, source: CaptureSource): string {
			return getSourceSession(value, source)?.sessionId ?? "—";
		},
		getSourceSessionStartedAt(value: CaptureSession | null | undefined, source: CaptureSource): number | null {
			return getSourceSession(value, source)?.startedAtUnixMs ?? null;
		},
	};
}

export type CaptureStore = ReturnType<typeof createCaptureStore>;
