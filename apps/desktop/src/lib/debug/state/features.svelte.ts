// State for the five feature summary cards added in slice 6: Transcription,
// Diarization, Embeddings, AI Runtime and User Context.
//
// One store, one poller. The five cards share a data set (the processor lanes
// feed two of them, the config half feeds the rest), so a single tick keeps the
// cards coherent with each other and costs 4 round-trips per second instead of
// one per card.
//
// ── What is polled vs. loaded once ────────────────────────────────────────
// POLLED (1s, cheap DB / in-memory reads): pipeline lanes, semantic index,
// derivation runs, Ask AI last-turn usage.
//
// NOT polled: `get_ai_runtime_status` and `get_user_context_status` both run
// `engine_configured_prerequisite`, which *pings the local engine endpoint* for
// Ollama/Llamafile. At 1s that would hammer the user's local model server twice
// a second forever. Same for the model-status commands, which stat model files
// on disk. All of these describe configuration, not live throughput — so they
// load on mount and refresh on the card's ↻ button.
//
// ponytail: no per-card store and no refcounted shared poller — one store with
// one `startPolling()` matches the health/pipeline idiom already in this folder.

import { invoke } from "@tauri-apps/api/core";
import { humanizeError } from "$lib/format-error";
import type {
	AiRuntimeStatus,
	AiRuntimeTestResult,
	AskAiTokenUsage,
	AudioTranscriptionModelStatusResponse,
	DerivationRun,
	ProcessorPipelineStatus,
	SemanticIndexStatusDto,
	SemanticSearchModelStatusResponse,
	SpeakerAnalysisModelStatusResponse,
	UserContextDerivationRunResult,
	UserContextStatus,
} from "$lib/types";

const POLL_MS = 1000;
/** Only the newest few runs are rendered; keep the polled read small. */
const DERIVATION_RUN_LIMIT = 5;

/** A lane with no jobs at all. See `ProcessorPipelineStatus` on why this exists. */
function emptyLane(processor: string): ProcessorPipelineStatus {
	return {
		processor,
		queued: 0,
		running: 0,
		completed: 0,
		failed: 0,
		failedLast24h: 0,
		averageCompletedSecondsLast24h: null,
		lastError: null,
	};
}

/**
 * A processor's lane out of a `get_processing_pipeline_status` response,
 * defaulted to zeros when the backend omitted it.
 *
 * The response is a `GROUP BY`, so a lane that has never had a job is simply
 * ABSENT — that is "no jobs queued", not an error. Every reader of that command
 * goes through here (`FeaturesStore.lane` for the summary cards, the detail
 * store for the drill-in hero) so the trap is solved exactly once.
 */
export function laneFor(lanes: ProcessorPipelineStatus[], processor: string): ProcessorPipelineStatus {
	return lanes.find((lane) => lane.processor === processor) ?? emptyLane(processor);
}

export function createFeaturesStore() {
	// ─── Polled ──────────────────────────────────────────────────────────────
	let lanes = $state<ProcessorPipelineStatus[]>([]);
	let lanesError = $state<string | null>(null);
	let semanticIndex = $state<SemanticIndexStatusDto | null>(null);
	let semanticIndexError = $state<string | null>(null);
	let derivationRuns = $state<DerivationRun[]>([]);
	let derivationRunsError = $state<string | null>(null);
	let askAiUsage = $state<AskAiTokenUsage | null>(null);

	// ─── Loaded on mount / ↻ ─────────────────────────────────────────────────
	let aiStatus = $state<AiRuntimeStatus | null>(null);
	let aiStatusError = $state<string | null>(null);
	let loadingAiStatus = $state(false);

	let userContextStatus = $state<UserContextStatus | null>(null);
	let userContextStatusError = $state<string | null>(null);
	let loadingUserContextStatus = $state(false);

	let transcriptionModels = $state<AudioTranscriptionModelStatusResponse | null>(null);
	let speakerModels = $state<SpeakerAnalysisModelStatusResponse | null>(null);
	let semanticModels = $state<SemanticSearchModelStatusResponse | null>(null);
	let modelsError = $state<string | null>(null);
	let loadingModels = $state(false);

	// Deepgram is the one cloud transcription provider; its key + auth state
	// live in the keychain, so they are their own tiny read (ADR 0047).
	let deepgramKeyPresent = $state(false);
	let deepgramAuthStatus = $state<string | null>(null);
	let deepgramError = $state<string | null>(null);

	// ─── One-shot actions ────────────────────────────────────────────────────
	let testingAi = $state(false);
	let aiTestResult = $state<AiRuntimeTestResult | null>(null);
	let testingDeepgram = $state(false);
	let deepgramTestResult = $state<{ ok: boolean; message: string } | null>(null);
	let runningDerivation = $state(false);
	let derivationRunMessage = $state<string | null>(null);

	// ─── Loaders ─────────────────────────────────────────────────────────────

	/** The 1s tick. Every read here is a cheap DB or in-memory lookup. */
	async function poll() {
		// Stop polling when the page isn't visible — nothing here is on screen.
		if (typeof document !== "undefined" && document.visibilityState !== "visible") return;
		await Promise.all([
			invoke<ProcessorPipelineStatus[]>("get_processing_pipeline_status")
				.then((v) => { lanes = v; lanesError = null; })
				.catch((err) => { lanesError = humanizeError(err); }),
			invoke<SemanticIndexStatusDto>("get_semantic_index_status")
				.then((v) => { semanticIndex = v; semanticIndexError = null; })
				.catch((err) => { semanticIndexError = humanizeError(err); }),
			invoke<DerivationRun[]>("list_user_context_derivation_runs", { limit: DERIVATION_RUN_LIMIT })
				.then((v) => { derivationRuns = v; derivationRunsError = null; })
				.catch((err) => { derivationRunsError = humanizeError(err); }),
			// `null` until a turn has run — a normal state, not an error.
			invoke<AskAiTokenUsage | null>("get_ask_ai_last_turn_usage")
				.then((v) => { askAiUsage = v; })
				.catch(() => { /* usage is a nicety; never surface a poll error for it */ }),
		]);
	}

	/** 1s poll, paused while the document is hidden. Returns teardown. */
	function startPolling(): () => void {
		void poll();
		const interval = setInterval(() => { void poll(); }, POLL_MS);
		const onVisibility = () => {
			if (document.visibilityState === "visible") void poll();
		};
		document.addEventListener("visibilitychange", onVisibility);
		return () => {
			clearInterval(interval);
			document.removeEventListener("visibilitychange", onVisibility);
		};
	}

	async function loadAiStatus() {
		loadingAiStatus = true;
		aiStatusError = null;
		try {
			aiStatus = await invoke<AiRuntimeStatus>("get_ai_runtime_status");
		} catch (err) {
			aiStatusError = humanizeError(err);
		} finally {
			loadingAiStatus = false;
		}
	}

	async function loadUserContextStatus() {
		loadingUserContextStatus = true;
		userContextStatusError = null;
		try {
			userContextStatus = await invoke<UserContextStatus>("get_user_context_status");
		} catch (err) {
			userContextStatusError = humanizeError(err);
		} finally {
			loadingUserContextStatus = false;
		}
	}

	async function loadModelStatuses() {
		loadingModels = true;
		modelsError = null;
		try {
			[transcriptionModels, speakerModels, semanticModels] = await Promise.all([
				invoke<AudioTranscriptionModelStatusResponse>("get_audio_transcription_model_status"),
				invoke<SpeakerAnalysisModelStatusResponse>("get_speaker_analysis_model_status"),
				invoke<SemanticSearchModelStatusResponse>("get_semantic_search_model_status"),
			]);
		} catch (err) {
			modelsError = humanizeError(err);
		} finally {
			loadingModels = false;
		}
	}

	async function loadDeepgramState() {
		deepgramError = null;
		try {
			deepgramKeyPresent = await invoke<boolean>("transcription_has_deepgram_key");
			deepgramAuthStatus = await invoke<string | null>("transcription_deepgram_auth_status");
		} catch (err) {
			deepgramError = humanizeError(err);
		}
	}

	/** Everything the five cards need that the 1s tick deliberately skips. */
	async function loadConfig() {
		await Promise.all([
			loadAiStatus(),
			loadUserContextStatus(),
			loadModelStatuses(),
			loadDeepgramState(),
		]);
	}

	// ─── Actions ─────────────────────────────────────────────────────────────

	async function testAiConnection() {
		testingAi = true;
		aiTestResult = null;
		try {
			aiTestResult = await invoke<AiRuntimeTestResult>("ai_runtime_test_connection");
		} catch (err) {
			aiTestResult = { ok: false, provider: "—", model: "—", message: humanizeError(err), rawJson: "" };
		} finally {
			testingAi = false;
			await loadAiStatus();
		}
	}

	async function testDeepgram() {
		testingDeepgram = true;
		deepgramTestResult = null;
		try {
			deepgramTestResult = await invoke<{ ok: boolean; message: string }>("transcription_test_deepgram");
		} catch (err) {
			deepgramTestResult = { ok: false, message: humanizeError(err) };
		} finally {
			testingDeepgram = false;
			// The probe may have refreshed the stored auth status.
			await loadDeepgramState();
		}
	}

	async function runDerivationNow() {
		runningDerivation = true;
		derivationRunMessage = null;
		try {
			const result = await invoke<UserContextDerivationRunResult>("user_context_run_derivation_now");
			derivationRunMessage = result.message;
		} catch (err) {
			derivationRunMessage = humanizeError(err);
		} finally {
			runningDerivation = false;
			await loadUserContextStatus();
		}
	}

	return {
		/** This poll's lane for a processor, zero-defaulted — see `laneFor`. */
		lane(processor: string): ProcessorPipelineStatus {
			return laneFor(lanes, processor);
		},
		/**
		 * Every lane this poll saw. Only processors with at least one job appear
		 * (the response is a `GROUP BY`), so this is the honest list of lanes that
		 * exist — which is exactly what the Health card's queue/failure rollup
		 * wants to sum and name.
		 */
		get lanes() { return lanes; },
		get lanesError() { return lanesError; },

		get semanticIndex() { return semanticIndex; },
		get semanticIndexError() { return semanticIndexError; },

		get derivationRuns() { return derivationRuns; },
		get derivationRunsError() { return derivationRunsError; },
		get askAiUsage() { return askAiUsage; },

		get aiStatus() { return aiStatus; },
		get aiStatusError() { return aiStatusError; },
		get loadingAiStatus() { return loadingAiStatus; },

		get userContextStatus() { return userContextStatus; },
		get userContextStatusError() { return userContextStatusError; },
		get loadingUserContextStatus() { return loadingUserContextStatus; },

		get transcriptionModels() { return transcriptionModels; },
		get speakerModels() { return speakerModels; },
		get semanticModels() { return semanticModels; },
		get modelsError() { return modelsError; },
		get loadingModels() { return loadingModels; },

		get deepgramKeyPresent() { return deepgramKeyPresent; },
		get deepgramAuthStatus() { return deepgramAuthStatus; },
		get deepgramError() { return deepgramError; },

		get testingAi() { return testingAi; },
		get aiTestResult() { return aiTestResult; },
		get testingDeepgram() { return testingDeepgram; },
		get deepgramTestResult() { return deepgramTestResult; },
		get runningDerivation() { return runningDerivation; },
		get derivationRunMessage() { return derivationRunMessage; },

		poll,
		startPolling,
		loadConfig,
		loadAiStatus,
		loadUserContextStatus,
		loadModelStatuses,
		loadDeepgramState,
		testAiConnection,
		testDeepgram,
		runDerivationNow,
	};
}

export type FeaturesStore = ReturnType<typeof createFeaturesStore>;
