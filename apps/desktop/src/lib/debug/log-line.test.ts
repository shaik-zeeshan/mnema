// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig, so skip static checking here.
import { describe, expect, it } from "bun:test";
import { groupLogLines, levelMatches, parseLogLine } from "./log-line";

const pad = (n: number) => String(n).padStart(2, "0");
const localHms = (d: Date) => `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;

describe("parseLogLine", () => {
	it("parses a rust.log line, converting the UTC stamp to local time", () => {
		const line = parseLogLine("[2026-10-13][15:01:26][capture_system_audio::rebuild][WARN] system-audio-tap: rebuilding tap");
		expect(line).toEqual({
			time: localHms(new Date(Date.UTC(2026, 9, 13, 15, 1, 26))),
			level: "WARN",
			target: "capture_system_audio::rebuild",
			prefix: "system-audio-tap:",
			message: "rebuilding tap",
		});
	});

	it("keeps millis when the stamp has them", () => {
		const line = parseLogLine("[2026-10-13][15:01:26.07][a::b][INFO] x");
		expect(line?.time).toBe(`${localHms(new Date(Date.UTC(2026, 9, 13, 15, 1, 26)))}.070`);
	});

	it("takes a bracket tag, else the target's last segment, as the prefix", () => {
		expect(parseLogLine("[2026-10-13][15:01:26][app_infra::jobs][INFO] [app-infra] worker started")?.prefix).toBe("[app-infra]");
		const plain = parseLogLine("[2026-10-13][15:01:26][mnema_lib::semantic_search_worker][INFO] embedded 64 passages");
		expect(plain?.prefix).toBe("semantic_search_worker");
		expect(plain?.message).toBe("embedded 64 passages");
	});

	it("returns null for a continuation line", () => {
		expect(parseLogLine("   0: std::backtrace::Backtrace::create")).toBeNull();
		expect(parseLogLine("")).toBeNull();
	});
});

describe("groupLogLines", () => {
	it("keeps continuation lines under their head, and orphans at the top in a headless entry", () => {
		const entries = groupLogLines([
			"orphan tail of an older message",
			"[2026-10-13][15:01:26][a::b][ERROR] panic on thread 'x'",
			"   0: frame one",
			"[2026-10-13][15:01:27][a::b][INFO] next",
		]);
		expect(entries.map((e) => [e.head?.level ?? null, e.more])).toEqual([
			[null, ["orphan tail of an older message"]],
			["ERROR", ["   0: frame one"]],
			["INFO", []],
		]);
		expect(entries[1].raw).toBe("[2026-10-13][15:01:26][a::b][ERROR] panic on thread 'x'\n   0: frame one");
	});
});

describe("levelMatches", () => {
	it("Warnings = WARN + ERROR, Errors = ERROR", () => {
		expect(["INFO", "WARN", "ERROR", null].filter((l) => levelMatches(l, "warn"))).toEqual(["WARN", "ERROR"]);
		expect(["INFO", "WARN", "ERROR", null].filter((l) => levelMatches(l, "error"))).toEqual(["ERROR"]);
		expect(["INFO", null].every((l) => levelMatches(l, "all"))).toBe(true);
	});
});
