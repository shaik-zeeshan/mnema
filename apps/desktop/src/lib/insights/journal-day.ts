// Slice 2 of the Journal feature: bucket ONE local-calendar day of already-read
// data (Activities + raw frames + the worker watermark) into a render model for
// the Journal UI (Slice 3). Pure functions only — no Svelte, no `invoke`, no
// side effects — so it is unit-testable under `bun test`. Rendering makes zero
// LLM calls; this file just arranges the reads.

import type { Activity } from "$lib/types/recording";
import type { FrameSummaryDto } from "$lib/types/app-infra";

/**
 * Minimum empty span between two consecutive captured frames, inside the
 * summarized region, that counts as an away-gap (the user stepped away and
 * nothing was captured). Below this we treat the silence as normal capture
 * cadence, not an absence. 5 minutes — matches the max Capture Segment
 * Duration, so a gap this long means at least one whole segment produced
 * nothing.
 */
export const AWAY_GAP_MIN_MS = 300_000;

/** One activity card on the time spine, with how much footage still backs it. */
export interface JournalCardSlot {
	activity: Activity;
	/** Count of day frames whose `capturedAt` ∈ [startedAtMs, endedAtMs). */
	frameCount: number;
	/** `frameCount === 0` → footage aged out under Retention while the summary survives. */
	expired: boolean;
}

/** An away-gap: a covered span with no frames, clamped to the day bounds. */
export interface JournalGap {
	startMs: number;
	endMs: number;
}

/** Why the live-edge slot is pending. The `reason` string is a raw status code — Slice 3 maps codes → human copy. */
export type PendingReason =
	| { kind: "summarizing" } // engine healthy + frames exist past the watermark
	| { kind: "engine_unavailable"; reason: string }; // engine off / no key / budget

/** The trailing "summarizing…" region at the live edge. */
export interface JournalPending {
	active: boolean;
	/** Start of the pending region (the watermark, clamped ≥ day start, or the first frame). `null` when inactive. */
	sinceMs: number | null;
	reason: PendingReason | null;
}

export interface JournalDayModel {
	/** One per activity started within the day, chronological (oldest first). */
	slots: JournalCardSlot[];
	/** Away-gaps within the covered region (no frames, ≥ AWAY_GAP_MIN_MS). */
	gaps: JournalGap[];
	pending: JournalPending;
	/** Frames captured within the day. */
	totalFrameCount: number;
	/** `totalFrameCount > 0` — drives the "nothing captured" empty state. */
	hasAnyCapture: boolean;
}

export interface JournalDayInput {
	activities: Activity[];
	frames: FrameSummaryDto[];
	/** Worker "summarized up to" watermark; frames newer than this aren't summarized. `null` = nothing derived yet. */
	coveredUntilMs: number | null;
	/** Is capture currently running. Carried for Slice 3; the pending rule is derived purely from frames vs. watermark (see below). */
	recording: boolean;
	engineAvailable: boolean;
	engineReason: string | null;
	dayStartMs: number; // local midnight
	dayEndMs: number; // next local midnight (exclusive)
}

/** Parse an RFC3339 `capturedAt` to epoch ms, or null when unparseable. */
function frameTs(frame: FrameSummaryDto): number | null {
	const ms = Date.parse(frame.capturedAt);
	return Number.isNaN(ms) ? null : ms;
}

export function buildJournalDay(input: JournalDayInput): JournalDayModel {
	const { activities, frames, coveredUntilMs, engineAvailable, engineReason, dayStartMs, dayEndMs } =
		input;

	// Day frame timestamps, defensively filtered to [dayStart, dayEnd) and sorted.
	const dayFrameTs = frames
		.map(frameTs)
		.filter((ts): ts is number => ts !== null && ts >= dayStartMs && ts < dayEndMs)
		.sort((a, b) => a - b);

	const totalFrameCount = dayFrameTs.length;
	const hasAnyCapture = totalFrameCount > 0;

	// --- Slots: one per activity that STARTED this day, chronological. ---
	// Ownership is by start day, not overlap: a midnight-crossing activity is
	// already split at the boundary by derivation, so overlap semantics would
	// render yesterday's 11:5x PM half a second time at the top of today.
	// ponytail: O(activities × frames) frame-count scan — trivial for a single
	// day; binary-search the sorted timestamps if a day ever gets huge.
	const slots: JournalCardSlot[] = activities
		.filter((a) => a.startedAtMs >= dayStartMs && a.startedAtMs < dayEndMs)
		.sort((a, b) => a.startedAtMs - b.startedAtMs || a.endedAtMs - b.endedAtMs || a.id - b.id)
		.map((activity) => {
			const frameCount = dayFrameTs.filter(
				(ts) => ts >= activity.startedAtMs && ts < activity.endedAtMs,
			).length;
			return { activity, frameCount, expired: frameCount === 0 };
		});

	// --- Pending region (the live edge). ---
	// Deterministic from inputs (no "now"): pending is active iff there is
	// un-summarized capture to summarize — a frame past the watermark, or the
	// watermark is null (nothing derived yet) while the day has capture. The
	// `recording` flag alone never activates it: recording with everything
	// already summarized shows no pending slot.
	const hasFramesPastWatermark =
		coveredUntilMs !== null && dayFrameTs.some((ts) => ts > coveredUntilMs);
	const pendingActive = hasFramesPastWatermark || (coveredUntilMs === null && hasAnyCapture);

	let pending: JournalPending;
	if (!pendingActive) {
		pending = { active: false, sinceMs: null, reason: null };
	} else {
		const sinceMs =
			coveredUntilMs === null
				? dayFrameTs[0] // whole covered region is pending → first captured frame
				: Math.max(dayStartMs, coveredUntilMs);
		// engine_unavailable takes precedence over the summarizing copy: with the
		// engine down, the river shows the reason instead of a spinner.
		const reason: PendingReason = engineAvailable
			? { kind: "summarizing" }
			: { kind: "engine_unavailable", reason: engineReason ?? "" };
		pending = { active: true, sinceMs, reason };
	}

	// --- Away-gaps: inter-frame gaps within the summarized region only. ---
	// Covered frames are those at or before the watermark; a null watermark means
	// nothing is summarized yet, so there is no covered region and no away-gaps.
	// Excluding pending frames here keeps the trailing pending silence out of the
	// gap list (it's "summarizing", not "away").
	const coveredFrameTs =
		coveredUntilMs === null ? [] : dayFrameTs.filter((ts) => ts <= coveredUntilMs);
	const gaps: JournalGap[] = [];
	for (let i = 1; i < coveredFrameTs.length; i++) {
		const startMs = coveredFrameTs[i - 1];
		const endMs = coveredFrameTs[i];
		if (endMs - startMs >= AWAY_GAP_MIN_MS) {
			gaps.push({
				startMs: Math.max(dayStartMs, startMs),
				endMs: Math.min(dayEndMs, endMs),
			});
		}
	}

	return { slots, gaps, pending, totalFrameCount, hasAnyCapture };
}
