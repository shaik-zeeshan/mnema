// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig, so skip static checking here.
import { describe, expect, it } from "bun:test";
import { LOG_FEATURE_PATTERNS, filterLogLines, logLineClass } from "./log-filter";

// Real lines, copied from a rust.log tail — the `[target]` field is the same
// macro helper on every one of them, which is exactly why the chips match on
// message text instead.
const LINES = [
	"[2026-07-13][02:25:28][mnema_lib::native_capture::debug_log][INFO] stopped user context derivation worker",
	"[2026-07-13][02:25:28][mnema_lib::native_capture::debug_log][INFO] unloaded 0 cached Local Whisper context(s) after background worker shutdown",
	"[2026-07-13][02:25:28][mnema_lib::native_capture::debug_log][WARN] display unavailable; awaiting return",
	"[2026-07-13][02:25:28][mnema_lib::native_capture::debug_log][ERROR] semantic index backfill failed to embed anchor",
];

describe("filterLogLines", () => {
	it("returns every line when no chip is picked", () => {
		expect(filterLogLines(LINES, null)).toEqual(LINES);
	});

	it("narrows to the picked feature — the acceptance criterion", () => {
		expect(filterLogLines(LINES, "transcription")).toEqual([LINES[1]]);
		expect(filterLogLines(LINES, "userContext")).toEqual([LINES[0]]);
		expect(filterLogLines(LINES, "embeddings")).toEqual([LINES[3]]);
	});

	it("yields nothing rather than everything when a feature has no lines", () => {
		expect(filterLogLines(LINES, "ocr")).toEqual([]);
	});

	it("survives an empty tail (the deleted-log case)", () => {
		expect(filterLogLines([], "capture")).toEqual([]);
		expect(filterLogLines([], null)).toEqual([]);
	});

	it("is stateless across calls — a /g pattern would skip lines here", () => {
		const capture = filterLogLines(LINES, "capture");
		expect(filterLogLines(LINES, "capture")).toEqual(capture);
		expect(filterLogLines(LINES, "capture")).toEqual(capture);
	});
});

describe("LOG_FEATURE_PATTERNS", () => {
	it("covers every chip the section renders, and none carries /g", () => {
		// The chips come from the 9 sections with a `healthFeature`; a missing key
		// would make that chip silently match nothing.
		expect(Object.keys(LOG_FEATURE_PATTERNS).sort()).toEqual([
			"aiRuntime",
			"capture",
			"diarization",
			"embeddings",
			"jobsAndStorage",
			"ocr",
			"privacy",
			"transcription",
			"userContext",
		]);
		for (const pattern of Object.values(LOG_FEATURE_PATTERNS)) {
			expect(pattern.global).toBe(false);
		}
	});
});

describe("logLineClass", () => {
	it("tints by level", () => {
		expect(logLineClass(LINES[3])).toBe("log-line log-line--err");
		expect(logLineClass(LINES[2])).toBe("log-line log-line--warn");
		expect(logLineClass(LINES[0])).toBe("log-line");
	});
});
