// Compile-time mirror guard for the processing-job wire shapes. No runtime
// behaviour: `bun run check` is the assertion.
//
// `ProcessingJobDto` mirrors Rust's `ProcessingJob`
// (crates/app-infra/src/processing/job.rs) — what `get_processing_job` and
// `list_processing_jobs` return. `ProcessingJobListing` mirrors the debug-page
// wrapper (crates/app-infra/src/processing/store.rs), which `#[serde(flatten)]`s
// the job and resolves `nextAttemptAt` + `modelLocked` alongside it.
//
// A field added to the wrapper but declared on the DTO would make every plain
// job DTO claim a property the backend never sends. These two literals are the
// exact wire payloads; if the mirrors drift, this file stops compiling.
import type { ProcessingJobDto } from "./app-infra";
import type { ProcessingJobListing } from "./debug";

const WIRE_JOB: ProcessingJobDto = {
	id: 1,
	subjectType: "audio_segment",
	subjectId: 7,
	processor: "speaker_analysis",
	status: "queued",
	attemptCount: 0,
	failureCount: 0,
	payloadJson: null,
	lastError: null,
	createdAt: "2026-07-15 14:30:00",
	queuedAt: "2026-07-15 14:30:00",
	updatedAt: "2026-07-15 14:30:00",
	startedAt: null,
	finishedAt: null,
};

const WIRE_LISTING: ProcessingJobListing = {
	...WIRE_JOB,
	nextAttemptAt: null,
	modelLocked: true,
};

export const PROCESSING_JOB_WIRE_MIRROR = { WIRE_JOB, WIRE_LISTING };
