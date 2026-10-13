// Developer-window wire types.
//
// Hand-mirrored from Rust per repo convention (no codegen). Sources:
//   • `DebugSeverity` / `DebugFeature` / `FeatureHealthDto`
//       → apps/desktop/src-tauri/src/debug_health.rs
//   • `AppLogFile` / `AppLogTailDto`
//       → apps/desktop/src-tauri/src/debug_status.rs (tail_app_log)
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
import type { ProcessingJobDto, ProcessingJobStatus } from "./app-infra";

/** A health row's tone. UI: `error` renders as danger; the rest keep their name. */
export type DebugSeverity = "ok" | "info" | "warn" | "error" | "off";

/** One health row. `get_debug_health` always returns all 12, in this order. */
export type DebugFeature =
	| "screen"
	| "microphone"
	| "systemAudio"
	| "reading"
	| "transcription"
	| "speakers"
	| "meaningIndex"
	| "aiEngine"
	| "userContext"
	| "disk"
	| "cliAccess"
	| "startup";

export interface FeatureHealthDto {
	feature: DebugFeature;
	severity: DebugSeverity;
	/** One plain sentence ("Deepgram can’t be reached, retrying on its own"). */
	reason: string;
	/** Short right-column text ("14 waiting", "182 GB", "3rd today", "1.8 s"). */
	metric: string | null;
	/** When set, render a local HH:MM instead of `metric` (AI engine, User Context). */
	metricAtMs: number | null;
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
 * It flattens the *domain* job and then resolves `modelLocked` alongside it, so a
 * parked job (`status === "queued" && modelLocked`) is distinguishable from one
 * that is merely waiting its turn.
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
	/**
	 * The job's model is locked (downloading, absent, or being deleted), so the queue skips it.
	 * `status === "queued" && modelLocked` is the "Preparing" state: waiting for its model rather
	 * than queued behind other work.
	 *
	 * Also on `ProcessingJobDto` now: the desktop commands (`get_processing_job`,
	 * `list_processing_jobs`, `retry_processing_job_now`) resolve it too. Here the
	 * listing wrapper (`crates/app-infra/src/processing/store.rs`) resolves it.
	 */
	modelLocked: boolean;
}

/** Newest-first page of one processor's jobs. `limit` clamps to 0..=500 (default 50). */
export interface ListProcessingJobsRequest {
	/** `ocr` | `audio_transcription` | `speaker_analysis` | `system_audio_speech_activity`. */
	processor: string;
	/** `null`/omitted lists every status. */
	status?: ProcessingJobStatus | null;
	/** `null`/omitted lists every subject — the jobs table's "segment id…" search. */
	subjectId?: number | null;
	limit?: number | null;
	offset?: number | null;
}

/**
 * What `list_processing_jobs_by_processor` returns: one page plus the total
 * behind the same filter (processor + status + subject), so the pager can say
 * "6 of 12 jobs · page 1/2" honestly.
 */
export interface ProcessingJobPage {
	jobs: ProcessingJobListing[];
	total: number;
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
	/**
	 * Context-window occupancy for this report: `input + output`, or the
	 * provider's lump-sum total when it reported no split. Show this rather than
	 * summing the two — a total-only provider reports both as 0.
	 */
	contextTokens: number;
	/** When the turn's agent loop started (unix ms). */
	startedAtMs: number;
	/** Visible tool calls this turn (excludes the `reference_captures` signal). */
	toolCalls: number;
	/** Loop start → usage report, ms (within a stream-tail of the full turn). */
	durationMs: number;
}

// ─── Capture health (get_capture_health) ───────────────────────────────────

export type CaptureSourceState = "off" | "recording" | "paused" | "waiting" | "reconnecting" | "failed";

export interface CaptureSourceHealth {
	requested: boolean;
	state: CaptureSourceState;
	/** Raw reason code, e.g. `capture_display_unavailable`, `inactivity`, `not_requested`. */
	reason: string | null;
	/** dBFS from the normalized peak, clamped to −60; audio only. */
	levelDb: number | null;
	idleMs: number | null;
	/** Not cheaply known today: always `null`. */
	deviceName: string | null;
}

export interface CaptureSuspensionHealth {
	kind: "privacyFilter" | "displayUnavailable" | "lowDisk";
	reason: string;
	recoveryAttempts: number;
	restartRequired: boolean;
	sinceMs: number | null;
	lastErrorMessage: string | null;
}

export type SystemAudioTapRebuildReason =
	| "deviceChanged"
	| "deviceDied"
	| "sampleRateChanged"
	| "excludeListMoved"
	| "silenceWatchdog";

export interface SystemAudioTapHealth {
	/** Since local midnight, within this app run (resets on relaunch). */
	rebuildsToday: number;
	lastReason: SystemAudioTapRebuildReason | null;
	lastRebuildAtMs: number | null;
	/** e.g. "48000 → 16000 Hz" after a sample-rate rebuild. */
	lastDetail: string | null;
	nextSilenceCheckInMs: number | null;
	watchdogDelayMs: number | null;
	sampleRateHz: number | null;
}

export interface CaptureHealthDto {
	recording: boolean;
	startedAtMs: number | null;
	screen: CaptureSourceHealth;
	microphone: CaptureSourceHealth;
	systemAudio: CaptureSourceHealth;
	suspension: CaptureSuspensionHealth | null;
	/** `null` while no system-audio session is live. */
	systemAudioTap: SystemAudioTapHealth | null;
	disk: {
		freeBytes: number | null;
		reserveFloorBytes: number;
		/** `null` while not recording. */
		pauseThresholdBytes: number | null;
		resumeThresholdBytes: number | null;
	};
}
