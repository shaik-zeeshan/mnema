// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, it } from "bun:test";
import { CHIP_TITLE_MAX, fillChips } from "./chip-fill";

const NOW = new Date(2026, 6, 23, 14, 47).getTime();
const MIN = 60_000;
const at = (minsAgo) => NOW - minsAgo * MIN;

const meeting = (title, startMinsAgo, over = {}) => ({
	title,
	category: "meetings",
	startedAtMs: at(startMinsAgo),
	endedAtMs: at(startMinsAgo - 30),
	...over,
});

describe("fillChips", () => {
	it("fills the meeting chip from the most recently started meeting", () => {
		const chips = fillChips({
			activities: [
				meeting("Weekly product sync", 280),
				meeting("Design review", 60),
				{ title: "Deep work in Zed", category: "creating", startedAtMs: at(30), endedAtMs: at(5) },
			],
			timePerApp: [],
			nowMs: NOW,
		});
		expect(chips).toEqual([{ glyph: "◉", text: "Catch me up on Design review" }]);
	});

	it("ignores meetings that start in the future and untitled meetings", () => {
		const chips = fillChips({
			activities: [meeting("Upcoming standup", -20), meeting("   ", 60)],
			timePerApp: [],
			nowMs: NOW,
		});
		expect(chips).toEqual([]);
	});

	it("fills the app chip from the app with the most active time", () => {
		const chips = fillChips({
			activities: [],
			timePerApp: [
				{ app: "Safari", activeMs: 20 * MIN },
				{ app: "Zed", activeMs: 95 * MIN },
				{ app: "Figma", activeMs: 40 * MIN },
			],
			nowMs: NOW,
		});
		expect(chips).toEqual([{ glyph: "▣", text: "What was I doing in Zed?" }]);
	});

	it("skips Unknown / empty / zero-time apps", () => {
		const chips = fillChips({
			activities: [],
			timePerApp: [
				{ app: "Unknown", activeMs: 90 * MIN },
				{ app: "  ", activeMs: 60 * MIN },
				{ app: "Mail", activeMs: 0 },
			],
			nowMs: NOW,
		});
		expect(chips).toEqual([]);
	});

	it("orders meeting chip before app chip when both fill", () => {
		const chips = fillChips({
			activities: [meeting("Weekly product sync", 280)],
			timePerApp: [{ app: "Zed", activeMs: 95 * MIN }],
			nowMs: NOW,
		});
		expect(chips.map((c) => c.glyph)).toEqual(["◉", "▣"]);
		expect(chips[0].text).toBe("Catch me up on Weekly product sync");
		expect(chips[1].text).toBe("What was I doing in Zed?");
	});

	it("clamps pathological titles to one-line pills", () => {
		const long = "A".repeat(200);
		const chips = fillChips({
			activities: [meeting(long, 60)],
			timePerApp: [],
			nowMs: NOW,
		});
		expect(chips[0].text.length).toBeLessThanOrEqual("Catch me up on ".length + CHIP_TITLE_MAX);
		expect(chips[0].text.endsWith("…")).toBe(true);
	});
});
