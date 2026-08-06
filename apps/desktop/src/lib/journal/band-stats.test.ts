// @ts-nocheck — run under `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (same as journal-view.test.ts).
import { describe, expect, it } from "bun:test";
import type { RiverRow } from "$lib/insights/journal-view";
import { bandStats, relativeAgo } from "./band-stats";

function card(startedAtMs: number, endedAtMs: number, id: number): RiverRow {
	return {
		kind: "card",
		atMs: startedAtMs,
		slot: {
			activity: {
				id,
				title: "t",
				summary: "s",
				startedAtMs,
				endedAtMs,
				createdAtMs: startedAtMs,
				evidence: [],
			},
			frameCount: 0,
			expired: true,
		},
	};
}

describe("bandStats", () => {
	it("counts only cards and sums their durations", () => {
		const rows: RiverRow[] = [
			card(0, 60_000, 1),
			{ kind: "gap", atMs: 60_000, gap: { startMs: 60_000, endMs: 600_000 } },
			card(600_000, 900_000, 2),
		];
		expect(bandStats(rows)).toEqual({ count: 2, totalMs: 360_000 });
	});

	it("is zero for a band of nothing but gaps", () => {
		expect(bandStats([{ kind: "gap", atMs: 1, gap: { startMs: 1, endMs: 2 } }])).toEqual({
			count: 0,
			totalMs: 0,
		});
	});
});

describe("relativeAgo", () => {
	const now = 1_000_000_000_000;
	it("rounds coarsely", () => {
		expect(relativeAgo(now - 30_000, now)).toBe("just now");
		expect(relativeAgo(now - 12 * 60_000, now)).toBe("12 min ago");
		expect(relativeAgo(now - 3 * 3_600_000, now)).toBe("3h ago");
		expect(relativeAgo(now - 50 * 3_600_000, now)).toBe("2d ago");
	});
	it("says nothing about a missing timestamp", () => {
		expect(relativeAgo(0, now)).toBe("");
	});
});
