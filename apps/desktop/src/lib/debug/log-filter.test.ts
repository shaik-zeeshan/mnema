// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig, so skip static checking here.
import { describe, expect, it } from "bun:test";
import { LOG_FEATURE_PATTERNS } from "./log-filter";

// Real lines, copied from a rust.log tail — the `[target]` field is the same
// macro helper on every one of them, which is exactly why the chips match on
// message text instead.
const LINES = [
	"[2026-07-13][02:25:28][mnema_lib::native_capture::debug_log][INFO] stopped user context derivation worker",
	"[2026-07-13][02:25:28][mnema_lib::native_capture::debug_log][INFO] unloaded 0 cached Local Whisper context(s) after background worker shutdown",
	"[2026-07-13][02:25:28][mnema_lib::native_capture::debug_log][WARN] display unavailable; awaiting return",
	"[2026-07-13][02:25:28][mnema_lib::native_capture::debug_log][ERROR] semantic index backfill failed to embed anchor",
];

describe("LOG_FEATURE_PATTERNS", () => {
	it("narrows to the picked feature", () => {
		const match = (f) => LINES.filter((l) => LOG_FEATURE_PATTERNS[f].test(l));
		expect(match("transcription")).toEqual([LINES[1]]);
		expect(match("userContext")).toEqual([LINES[0]]);
		expect(match("embeddings")).toEqual([LINES[3]]);
		expect(match("ocr")).toEqual([]);
	});

	it("covers every LogFeature, and none carries /g", () => {
		// A missing key would make that chip silently match nothing.
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
