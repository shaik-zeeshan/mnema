// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig, so skip static checking here.
import { describe, expect, test } from "bun:test";
import { SETTINGS_ROW_INDEX } from "./settings-index";
import { settingsCensus } from "./row-census";

// The sticky headers' "9 – 21 of 48" is a claim about position. If the ranges
// overlap, gap, or stop short of the index, the address is a lie.

describe("settingsCensus", () => {
	test("the group ranges tile the whole index with no gaps or overlaps", () => {
		const census = settingsCensus();
		expect(census.total).toBe(SETTINGS_ROW_INDEX.length);
		let expected = 1;
		for (const group of census.groups) {
			expect(group.first).toBe(expected);
			expect(group.last).toBeGreaterThanOrEqual(group.first - 1);
			expected = group.last + 1;
		}
		expect(expected - 1).toBe(census.total);
	});

	test("with no query every group matches its whole range", () => {
		for (const group of settingsCensus().groups) {
			expect(group.matches).toBe(group.last - group.first + 1);
		}
	});

	test("a query narrows to the groups that actually hold hits", () => {
		// "aud" is the mockup's own example: audio rows in Capture, transcription
		// rows in Intelligence — and nothing in About.
		const census = settingsCensus("aud");
		expect(census.matched).toBeGreaterThan(0);
		expect(census.matched).toBeLessThan(census.total);
		expect(census.groups.find((g) => g.id === "about")?.matches).toBe(0);
		expect(census.matchedGroups).toBe(
			census.groups.filter((g) => g.matches > 0).length,
		);
		// The per-group matches always sum to the reported total.
		const sum = census.groups.reduce((n, g) => n + g.matches, 0);
		expect(sum).toBe(census.matched);
	});

	test("a query that matches nothing leaves every group empty", () => {
		const census = settingsCensus("zzzznotasetting");
		expect(census.matched).toBe(0);
		expect(census.matchedGroups).toBe(0);
	});
});
