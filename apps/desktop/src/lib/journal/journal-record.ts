// The inspector's record for one selected activity — pure, so it is bun-testable
// and the panel component stays markup.
//
// Everything here is counted from data already loaded for the river (the day's
// frames, the activity's own evidence refs, the standing conclusions). Nothing
// is invented: a figure that cannot be counted comes back `null`/`0` and the
// inspector drops that row (**G8**).

import type { Activity, ActivityEvidenceRef, Conclusion } from "$lib/types/recording";
import type { FrameSummaryDto } from "$lib/types/app-infra";
import { partitionEvidence } from "$lib/insights/receipt-audio";
import { countCaptureSegments } from "$lib/insights/receipt-playback";

/** A day frame reduced to what the record needs. */
export interface FrameMark {
	id: number;
	ms: number;
}

export interface ActivityRecord {
	/** Frames still on disk inside the activity's span. 0 ⇒ footage expired. */
	frameCount: number;
	/** Capture segments those frames fall into (90s inter-frame gap rule). */
	segmentCount: number;
	citedFrames: number;
	citedSpoken: number;
	/** Wall-clock of the headline (poster) frame, when the engine cited one. */
	headlineMs: number | null;
	/** The frame "Show in Timeline" hands off; null when nothing survives. */
	firstFrameId: number | null;
}

/** Day frames → ascending `{id, ms}`, dropping unparseable stamps. */
export function frameMarks(frames: FrameSummaryDto[]): FrameMark[] {
	return frames
		.map((f) => ({ id: f.id, ms: Date.parse(f.capturedAt) }))
		.filter((f) => Number.isFinite(f.ms))
		.sort((a, b) => a.ms - b.ms);
}

function headlineMsOf(refs: ActivityEvidenceRef[], marks: FrameMark[]): number | null {
	const headline = refs.find((e) => e.isHeadline);
	if (!headline) return null;
	// The frame's own capture time wins; the ref's stamp is the fallback for a
	// frame retention has already removed.
	const mark = marks.find((m) => m.id === headline.subjectId);
	return mark?.ms ?? headline.capturedAtMs ?? null;
}

export function buildActivityRecord(activity: Activity, dayMarks: FrameMark[]): ActivityRecord {
	const inSpan = dayMarks.filter(
		(m) => m.ms >= activity.startedAtMs && m.ms < activity.endedAtMs,
	);
	const { frames: frameRefs, audio: audioRefs } = partitionEvidence(activity.evidence);
	return {
		frameCount: inSpan.length,
		segmentCount: countCaptureSegments(inSpan.map((m) => m.ms)),
		citedFrames: frameRefs.length,
		citedSpoken: audioRefs.length,
		headlineMs: headlineMsOf(frameRefs, dayMarks),
		firstFrameId: inSpan[0]?.id ?? null,
	};
}

/**
 * "You, Priya, Tom + 2" — the inspector's 256px-wide speaker line. Distinct
 * names in the order they first spoke; everyone past `max` is counted, never
 * dropped silently. The receipt's own `turnSpeakerRoster` stays the full-width
 * form (it also carries the "name in Timeline" nudge, which does not fit here).
 */
export function speakerRoster(names: string[], max = 3): string | null {
	const seen: string[] = [];
	for (const n of names) if (!seen.includes(n)) seen.push(n);
	if (seen.length === 0) return null;
	const head = seen.slice(0, max).join(", ");
	return seen.length > max ? `${head} + ${seen.length - max}` : head;
}

/**
 * The Subjects this activity is evidence for — read off the standing
 * Conclusions' evidence refs, deduplicated, in first-seen order. Empty when the
 * distillation has not linked it to anything (the inspector then drops the row
 * rather than claiming "none").
 */
export function subjectsForActivity(activityId: number, conclusions: Conclusion[]): string[] {
	const seen = new Set<string>();
	const out: string[] = [];
	for (const c of conclusions) {
		if (!c.evidence.some((e) => e.activityId === activityId)) continue;
		const key = c.subject.toLowerCase();
		if (seen.has(key)) continue;
		seen.add(key);
		out.push(c.subject);
	}
	return out;
}
