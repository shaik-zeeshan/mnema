// Pure helpers for the detail view's jobs table + inspector. Kept out of the
// components so the two rules that are easy to get wrong — what "retrying"
// means, and where a page ends without a total — are unit-testable.

import { parseJobTs } from "../format";
import type { ProcessingJobListing, ProcessingJobStatus } from "$lib/types";

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
export type JobState = ProcessingJobStatus | "retrying";

export function jobState(
	job: Pick<ProcessingJobListing, "status" | "nextAttemptAt">,
	nowMs: number,
): JobState {
	if (job.status !== "queued") return job.status;
	const next = parseJobTs(job.nextAttemptAt);
	return next != null && next > nowMs ? "retrying" : "queued";
}

export function jobStateBadgeClass(state: JobState): string {
	if (state === "completed") return "badge badge--ok badge--sm";
	if (state === "failed") return "badge badge--err badge--sm";
	if (state === "retrying") return "badge badge--warn badge--sm";
	if (state === "running") return "badge badge--running badge--sm";
	return "badge badge--neutral badge--sm";
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

/** Total pages behind the filter. Never 0: an empty list is still "page 1/1". */
export function pageCount(total: number, limit: number): number {
	return Math.max(1, Math.ceil(total / limit));
}

/** The wire now carries the filter's total, so `next` is exact — no guessing. */
export function hasNextPage(page: number, total: number, limit: number): boolean {
	return page + 1 < pageCount(total, limit);
}

/** `6 of 12 jobs` — this page's rows against the filter's total. */
export function pageTotalsLabel(rows: number, total: number): string {
	if (total === 0) return "no jobs";
	return `${rows} of ${total} job${total === 1 ? "" : "s"}`;
}
