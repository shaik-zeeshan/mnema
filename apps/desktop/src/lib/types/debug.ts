// Debug-page wire types.
//
// Hand-mirrored from Rust per repo convention (no codegen). Sources:
//   • `DebugSeverity` / `DebugFeature` / `FeatureHealthDto`
//       → apps/desktop/src-tauri/src/debug_health.rs
//   • `AppLogFile` / `AppLogTailDto`
//       → apps/desktop/src-tauri/src/debug_status.rs (tail_app_log)
//   • `CapturePrivacyDebugInfo`
//       → apps/desktop/src-tauri/src/native_capture/metadata.rs (get_capture_privacy_debug)
//   • `OcrBudgetDebug` + its events
//       → apps/desktop/src-tauri/src/ocr_budget.rs (get_ocr_budget_debug)
//   • `ProcessorPipelineStatus` / `ProcessingJobListing`
//       → crates/app-infra/src/processing/store.rs
//         (get_processing_pipeline_status / list_processing_jobs_by_processor)
//   • `SemanticIndexStatusDto`
//       → apps/desktop/src-tauri/src/debug_status.rs (get_semantic_index_status)
//   • `DerivationRun`
//       → crates/app-infra/src/user_context/store.rs (list_user_context_derivation_runs)
//   • `AskAiTokenUsage`
//       → apps/desktop/src-tauri/src/ask_ai.rs (get_ask_ai_last_turn_usage)
//
// The privacy/OCR shapes were page-local `type` aliases inside
// routes/debug/+page.svelte before the slice-5 split; they moved here verbatim
// because the section components and the state stores both need them now.

import type { ProcessingJobDto, ProcessingJobStatus } from "./app-infra";

/** Dock health dot. Wire: serde `rename_all = "camelCase"` over the Rust enum. */
export type DebugSeverity = "ok" | "warn" | "error";

/**
 * A debug section that has a health rollup. Note there are **9** of these but
 * **11** sections — `health` and `logs` have no health entry (see
 * `lib/debug/sections.ts`, where `healthFeature` is nullable).
 */
export type DebugFeature =
	| "capture"
	| "privacy"
	| "ocr"
	| "transcription"
	| "diarization"
	| "embeddings"
	| "aiRuntime"
	| "userContext"
	| "jobsAndStorage";

export interface FeatureHealthDto {
	feature: DebugFeature;
	severity: DebugSeverity;
	/** Short plain-language sentence — rendered in the dock tooltip. */
	reason: string;
}

// ─── Log tail (tail_app_log) ───────────────────────────────────────────────

/** The two logs the app writes. Wire: serde `rename_all = "camelCase"`. */
export type AppLogFile = "rust" | "nativeCapture";

export interface AppLogTailDto {
	/** Resolved on-disk path — shown so the reader knows where this came from. */
	path: string;
	/**
	 * `false` when the file is missing. NOT an error: the log may simply never
	 * have been written, or the user just deleted it from Settings. The command
	 * returns an empty tail, and the viewer renders a calm empty state and keeps
	 * polling so it recovers when the file reappears.
	 */
	exists: boolean;
	/** Up to the requested line count, oldest first. Empty when missing. */
	lines: string[];
}

// ─── Privacy filter debug (get_capture_privacy_debug) ──────────────────────

export interface PrivacyFilterDecision {
	excludedBundleIds: string[];
	excludedBundleSourceIds?: Record<string, string>;
	matchedRuleIds: string[];
	metadataRedactionReason: string | null;
	privacyFilterApplied: boolean;
}

export interface CapturePrivacyDebugInfo {
	metadataEnabled: boolean;
	browserUrlMode: string;
	browserUrlMetadataSource: string;
	privacyDebug: {
		latestSnapshot: {
			appBundleId: string | null;
			appName: string | null;
			windowTitle: string | null;
			browserUrl: string | null;
			displayId: number | null;
			metadataRedactionReason: string | null;
			metadataRedactionSourceId: string | null;
		} | null;
		latestDecision: PrivacyFilterDecision;
		latestAppliedDecision: PrivacyFilterDecision;
		currentlyExcludedBundleIds: string[];
		privacyFilterApplied: boolean;
	};
}

// ─── OCR budget debug (get_ocr_budget_debug) ───────────────────────────────

export interface OcrAdmissionSignals {
	firstCandidateInScope: boolean;
	contextChanged: boolean;
	lowQueuePressure: boolean;
	representativeDue: boolean;
	highQueuePressure: boolean;
	fingerprintNovelInScope: boolean;
	noveltyAdmissionAvailable: boolean;
}

export interface OcrAdmissionDebugEvent {
	occurredAt: string;
	sessionId: string;
	workspaceScope: string;
	frameId: number;
	outcome: string;
	reason: string;
	queuePressureCount: number;
	recordingActive: boolean;
	jobId: number | null;
	relatedFrameId: number | null;
	signals: OcrAdmissionSignals;
}

export interface OcrExecutionDebugEvent {
	occurredAt: string;
	jobId: number;
	frameId: number | null;
	provider: string;
	modelId: string | null;
	recognitionMode: string | null;
	status: string;
	runDurationMs: number;
	queueWaitMs: number | null;
	resultTextLength: number | null;
	observationCount: number | null;
	lastError: string | null;
}

export interface OcrBudgetDebug {
	summary: {
		queuedOrRunningCount: number;
		executionState: string;
		cooldownRemainingMs: number;
		lastRunDurationMs: number | null;
		lastRunStatus: string | null;
		lastPacingMode: string | null;
	};
	admissionEvents: OcrAdmissionDebugEvent[];
	executionEvents: OcrExecutionDebugEvent[];
}

// ─── Pipeline status (get_processing_pipeline_status) ──────────────────────

/**
 * One processor lane's job counts.
 *
 * **The response is a `GROUP BY processor`, so a lane with zero jobs is ABSENT
 * from the array — not returned as a zero row.** On a fresh install there is no
 * `audio_transcription` entry at all. The per-feature cards are fixed, so they
 * must read lanes through `FeaturesStore.lane(processor)`, which defaults a
 * missing lane to zeros. Absence means "nothing queued", never an error.
 */
export interface ProcessorPipelineStatus {
	/** `ocr` | `audio_transcription` | `speaker_analysis` | `system_audio_speech_activity`. */
	processor: string;
	queued: number;
	/**
	 * The subset of `queued` serving a retry backoff (`nextAttemptAt` in the
	 * future) — the same derived "retrying" as `lib/debug/detail/jobs.ts`.
	 */
	retrying: number;
	running: number;
	completed: number;
	failed: number;
	failedLast24h: number;
	/** `null` when nothing completed in the window (nothing to average). */
	averageCompletedSecondsLast24h: number | null;
	lastError: string | null;
}

// ─── Processor job listing (list_processing_jobs_by_processor) ─────────────

/**
 * One `processing_jobs` row as the detail view's jobs table lists it.
 *
 * Wire: Rust's `ProcessingJobListing` `#[serde(flatten)]`s the job, so the
 * fields land **flat** — there is no nested `job` key — with `nextAttemptAt`
 * alongside them. Hence `extends ProcessingJobDto` rather than a `job` member.
 */
export interface ProcessingJobListing extends ProcessingJobDto {
	/**
	 * When the queue may re-claim this job. Set by the bounded failure-retry lane
	 * (and by the transient-liveness requeue) to `now + backoff`; `null` on a job
	 * that is claimable now, was never retried, or is terminally failed.
	 *
	 * A `queued` job with a *future* `nextAttemptAt` is what the UI calls
	 * **retrying**: a retry reverts status to `queued`, so "retrying" is a
	 * derived state, not a wire status (see `lib/debug/detail/jobs.ts`).
	 */
	nextAttemptAt: string | null;
}

/** Newest-first page of one processor's jobs. `limit` clamps to 0..=500 (default 50). */
export interface ListProcessingJobsRequest {
	/** `ocr` | `audio_transcription` | `speaker_analysis` | `system_audio_speech_activity`. */
	processor: string;
	/** `null`/omitted lists every status. */
	status?: ProcessingJobStatus | null;
	limit?: number | null;
	offset?: number | null;
}

// ─── Semantic index (get_semantic_index_status) ────────────────────────────

export interface SemanticIndexStatusDto {
	/** Stored vectors — the live index size. */
	vectorCount: number;
	/** Anchors still lacking a vector (the backfill backlog). */
	backlogCount: number;
	/** Live `vec0` column width. `null` = table absent, i.e. index unusable. */
	liveDimension: number | null;
	/** `false` is normal: the worker drops the embedder after an idle grace period. */
	modelLoaded: boolean | null;
	consecutiveLoadFailures: number | null;
	/** In-memory, cleared by a restart — the UI must label this "since app start". */
	quarantinedCount: number | null;
	/** `null` once a load succeeds. */
	lastLoadError: string | null;
}

// ─── User Context derivation runs (list_user_context_derivation_runs) ──────

export interface DerivationRun {
	id: number;
	/** `activity` | `conclusion` | `confidence` | `backfill`. */
	kind: string;
	windowStartMs: number | null;
	windowEndMs: number | null;
	/** `running` | `completed` | `failed` | `skipped`. */
	status: string;
	activitiesDerived: number;
	conclusionsDerived: number;
	inputTokens: number;
	outputTokens: number;
	provider: string | null;
	model: string | null;
	error: string | null;
	ungrounded: number;
	guardrailSuppressed: number;
	belowFormationBar: number;
	resurfaceBlocked: number;
	superseded: number;
	supersedeDegraded: number;
	supersedeBlocked: number;
	createdAtMs: number;
}

// ─── Ask AI usage (get_ask_ai_last_turn_usage) ─────────────────────────────

/** Last-turn token usage. The command returns `null` until a turn has run. */
export interface AskAiTokenUsage {
	inputTokens: number;
	outputTokens: number;
	/** When the turn's agent loop started (unix ms). */
	startedAtMs: number;
	/** Visible tool calls this turn (excludes the `reference_captures` signal). */
	toolCalls: number;
	/** Loop start → usage report, ms (within a stream-tail of the full turn). */
	durationMs: number;
}
