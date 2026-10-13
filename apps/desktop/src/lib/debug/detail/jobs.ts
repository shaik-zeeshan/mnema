// Pure helpers for the detail view's jobs table + inspector. Kept out of the
// components so the two rules that are easy to get wrong — what "retrying"
// means, and where a page ends without a total — are unit-testable.

import { parseJobTs } from "../format";
import type { ProcessingJobListing, ProcessingJobStatus, ProcessorPipelineStatus } from "$lib/types";

/**
 * What a row *says*. Four of these are the wire status; `retrying` is derived.
 *
 * The bounded failure-retry lane reverts a failed job to **queued** and defers
 * it with `next_attempt_at = now + backoff` (so does the transient-liveness
 * requeue, ADR 0048). So a queued job with a future `nextAttemptAt` is not
 * "waiting its turn", it is serving a backoff — the single most useful thing
 * this table can tell you, and invisible from `status` alone.
 *
 * A `failed` row is therefore always terminal: the retry lane already had its
 * chance and declined (cap reached, or the processor has no retry policy).
 */
export type JobState = ProcessingJobStatus | "retrying" | "preparing";

export function jobState(
	job: Pick<ProcessingJobListing, "status" | "nextAttemptAt" | "modelLocked">,
	nowMs: number,
): JobState {
	if (job.status !== "queued") return job.status;
	// A locked model means the job is PARKED: the claim predicate skips it, so it
	// is not waiting its turn and is not serving a backoff. It burns no attempt
	// while it waits, which is the whole point of claim-time gating — but without
	// its own state it looks identical to a queued job that is about to run.
	// Checked before the backoff so a job that is both parked and deferred reads
	// as parked: the model is the reason it will not run next.
	if (job.modelLocked) return "preparing";
	const next = parseJobTs(job.nextAttemptAt);
	return next != null && next > nowMs ? "retrying" : "queued";
}

/** `4m 12s` / `18s` — a backoff window, not a wall clock. */
function formatDelta(ms: number): string {
	const seconds = Math.round(ms / 1000);
	if (seconds < 60) return `${seconds}s`;
	return `${Math.floor(seconds / 60)}m ${seconds % 60}s`;
}

/**
 * "in 4m 12s" / "due now", or `null` when no attempt is scheduled — which is
 * the common case and must read as absence, not as zero.
 */
export function nextAttemptLabel(
	nextAttemptAt: string | null | undefined,
	nowMs: number,
): string | null {
	const next = parseJobTs(nextAttemptAt);
	if (next == null) return null;
	const delta = next - nowMs;
	return delta <= 0 ? "due now" : `in ${formatDelta(delta)}`;
}

/**
 * The `provider` a job was enqueued for, read out of its payload JSON. Only
 * some processors stamp one (transcription does; OCR payloads may not) — `null`
 * renders as "—", meaning "this job carries no provider", never an error.
 */
export function jobProvider(payloadJson: string | null | undefined): string | null {
	if (!payloadJson) return null;
	try {
		const payload = JSON.parse(payloadJson);
		const provider = payload?.provider;
		return typeof provider === "string" && provider !== "" ? provider : null;
	} catch {
		return null;
	}
}

/** The jobs table's filter. `retrying` is derived, so it is not a wire status. */
export type JobFilter = "all" | "queued" | "running" | "retrying" | "failed";

/** The lane counts each filter segment shows. A lane's `queued` INCLUDES `retrying`. */
export function filterCounts(
	lane: Pick<ProcessorPipelineStatus, "queued" | "retrying" | "running" | "completed" | "failed">,
): Record<JobFilter, number> {
	return {
		all: lane.queued + lane.running + lane.completed + lane.failed,
		queued: lane.queued - lane.retrying,
		running: lane.running,
		retrying: lane.retrying,
		failed: lane.failed,
	};
}

/** Does a row belong under `filter`? Queued and retrying share the wire status. */
export function matchesFilter(state: JobState, filter: JobFilter): boolean {
	if (filter === "all") return true;
	if (filter === "queued") return state === "queued" || state === "preparing";
	return state === filter;
}

/** "Segment #2291" / "Frame #918244". */
export function subjectLabel(job: Pick<ProcessingJobListing, "subjectType" | "subjectId">): string {
	const noun =
		job.subjectType === "audio_segment" ? "Segment" : job.subjectType === "frame" ? "Frame" : job.subjectType.replaceAll("_", " ");
	return `${noun} #${job.subjectId}`;
}

/** The payload's model id, if the processor froze one into it. */
export function jobModel(payloadJson: string | null | undefined): string | null {
	if (!payloadJson) return null;
	try {
		const payload = JSON.parse(payloadJson);
		const model = payload?.modelId ?? payload?.model_id;
		return typeof model === "string" && model !== "" ? model : null;
	} catch {
		return null;
	}
}

/** "deepgram" → "Deepgram". */
export function providerName(provider: string): string {
	const words = provider.replaceAll("_", " ");
	return words.charAt(0).toUpperCase() + words.slice(1);
}

// ponytail: string sniffing over the raw error; add a typed error kind on the
// wire if these start to misfire.
const TRANSIENT = /timed? ?out|timeout|unreachable|could not connect|connection|network|dns|offline|temporar|(http|status)\s*(429|5\d\d)\b/i;
const CUT_OFF = /moov|decode|truncat|unexpected (eof|end)|end of file|invalid data/i;
const NO_MODEL = /model.*(missing|not found|not installed|absent|not downloaded)|no such model/i;

/** Does this error look like a provider that will come back on its own? */
export function isTransientError(error: string | null | undefined): boolean {
	return !!error && TRANSIENT.test(error);
}

/** A plain title for the few error shapes we recognise; `null` for anything else. */
export function knownErrorTitle(error: string | null | undefined, provider: string | null): string | null {
	if (!error) return null;
	if (CUT_OFF.test(error)) return "The recording was cut off";
	if (NO_MODEL.test(error)) return "The model isn’t installed";
	if (TRANSIENT.test(error)) return `${provider ? providerName(provider) : "The provider"} can’t be reached`;
	return null;
}

/** The inspector's notice title: always plain, falling back to a generic one. */
export function errorTitle(error: string | null | undefined, provider: string | null): string {
	return knownErrorTitle(error, provider) ?? "This job failed";
}

export function firstLine(text: string): string {
	return text.split("\n", 1)[0].trim();
}

/** The line under a row: why it is where it is. `null` = no line. */
export function jobMessage(job: ProcessingJobListing, state: JobState, nowMs: number, laneRetrying: number): string | null {
	const provider = jobProvider(job.payloadJson);
	switch (state) {
		case "retrying": {
			const why = knownErrorTitle(job.lastError, provider) ?? (job.lastError ? firstLine(job.lastError) : null);
			const next = `Next try ${nextAttemptLabel(job.nextAttemptAt, nowMs) ?? "soon"}`;
			return why ? `${next} · ${why}` : next;
		}
		case "failed":
			return job.lastError ? (knownErrorTitle(job.lastError, provider) ?? firstLine(job.lastError)) : "Failed";
		case "preparing":
			return "Waiting for its model";
		case "queued":
			return laneRetrying > 0 ? `Waiting behind ${laneRetrying} ${laneRetrying === 1 ? "retry" : "retries"}` : "Waiting its turn";
		default:
			return null;
	}
}

/** The main-window target for Open in Timeline, or `null` for a subject it can't show. */
export function timelineTarget(job: Pick<ProcessingJobListing, "subjectType" | "subjectId">) {
	if (job.subjectType === "frame")
		return { kind: "frame", frameId: job.subjectId, audioSegmentId: null };
	if (job.subjectType === "audio_segment")
		return { kind: "audio", frameId: null, audioSegmentId: job.subjectId };
	return null;
}
