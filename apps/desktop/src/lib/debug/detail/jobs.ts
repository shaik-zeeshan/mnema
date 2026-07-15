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
 * **The command returns a page, not a count** — there is no total anywhere in
 * the wire, so this never invents one. A short page is the end of the list; a
 * full page means "maybe more", which is all `next` can honestly promise.
 */
export function hasNextPage(rows: number, limit: number): boolean {
	return rows >= limit;
}

/** `jobs 26–50` — the range this page covers. No "of N": there is no N. */
export function pageRangeLabel(page: number, rows: number, limit: number): string {
	if (rows === 0) return page === 0 ? "no jobs" : "no more jobs";
	const first = page * limit + 1;
	return `jobs ${first}–${first + rows - 1}`;
}
