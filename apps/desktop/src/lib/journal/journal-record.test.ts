// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, it } from "bun:test";
import {
	buildActivityRecord,
	frameMarks,
	speakerRoster,
	subjectsForActivity,
} from "./journal-record";
import type { Activity, Conclusion } from "$lib/types/recording";

const T0 = Date.UTC(2026, 7, 5, 9, 0, 0);

function activity(over: Partial<Activity> = {}): Activity {
	return {
		id: 1,
		title: "Chasing the webhook signature mismatch",
		summary: "…",
		category: "creating",
		focus: "deep",
		startedAtMs: T0,
		endedAtMs: T0 + 600_000,
		createdAtMs: T0,
		evidence: [],
		...over,
	};
}

describe("frameMarks", () => {
	it("sorts ascending and drops unparseable stamps", () => {
		const marks = frameMarks([
			{ id: 2, capturedAt: new Date(T0 + 1000).toISOString() },
			{ id: 9, capturedAt: "not-a-date" },
			{ id: 1, capturedAt: new Date(T0).toISOString() },
		] as never);
		expect(marks.map((m) => m.id)).toEqual([1, 2]);
	});
});

describe("buildActivityRecord", () => {
	const marks = [
		{ id: 1, ms: T0 - 5_000 }, // before the span
		{ id: 2, ms: T0 + 1_000 },
		{ id: 3, ms: T0 + 30_000 },
		{ id: 4, ms: T0 + 400_000 }, // > 90s later ⇒ a second capture segment
		{ id: 5, ms: T0 + 900_000 }, // after the span
	];

	it("counts only frames inside the span, and their capture segments", () => {
		const r = buildActivityRecord(activity(), marks);
		expect(r.frameCount).toBe(3);
		expect(r.segmentCount).toBe(2);
		expect(r.firstFrameId).toBe(2);
	});

	it("splits cited evidence and resolves the headline's wall clock", () => {
		const r = buildActivityRecord(
			activity({
				evidence: [
					{ subjectType: "frame", subjectId: 3, isHeadline: true },
					{ subjectType: "frame", subjectId: 2, isHeadline: false },
					{ subjectType: "audio_segment", subjectId: 77, isHeadline: false },
				],
			}),
			marks,
		);
		expect(r.citedFrames).toBe(2);
		expect(r.citedSpoken).toBe(1);
		expect(r.headlineMs).toBe(T0 + 30_000);
	});

	it("reports zero frames (footage expired) without inventing a handoff frame", () => {
		const r = buildActivityRecord(activity({ startedAtMs: T0 + 1e7, endedAtMs: T0 + 2e7 }), marks);
		expect(r.frameCount).toBe(0);
		expect(r.firstFrameId).toBeNull();
	});
});

describe("speakerRoster", () => {
	it("dedupes, keeps first-speech order, and counts the overflow", () => {
		expect(speakerRoster(["You", "Priya", "You", "Tom", "Speaker 4", "Ana"])).toBe(
			"You, Priya, Tom + 2",
		);
		expect(speakerRoster(["You", "Priya"])).toBe("You, Priya");
		expect(speakerRoster([])).toBeNull();
	});
});

describe("subjectsForActivity", () => {
	const conclusion = (subject: string, activityId: number): Conclusion =>
		({
			id: activityId,
			subject,
			statement: "…",
			confidence: 0.7,
			status: "visible",
			pinned: false,
			formedAtMs: T0,
			lastSupportedAtMs: T0,
			updatedAtMs: T0,
			evidence: [{ activityId, stance: "support" }],
		}) as Conclusion;

	it("dedupes case-insensitively and keeps first-seen order", () => {
		const subjects = subjectsForActivity(1, [
			conclusion("Mnema licensing", 1),
			conclusion("mnema Licensing", 1),
			conclusion("Q3 launch", 1),
			conclusion("Unrelated", 2),
		]);
		expect(subjects).toEqual(["Mnema licensing", "Q3 launch"]);
	});
});
