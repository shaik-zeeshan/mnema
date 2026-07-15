// The log tail's client-side filtering — pure, so it is bun-testable.
//
// `tail_app_log` is a poll-and-replace snapshot with no server-side filtering,
// so the filter chips narrow the lines the backend already returned. Split out
// of `state/logs.svelte.ts` because a runes module can't be imported by
// `bun test` (same reason `capture-rate.ts` / `segmented-nav.ts` sit beside
// their components).

import type { DebugFeature } from "$lib/types";

/**
 * Feature → line matcher for the filter chips.
 *
 * Keyword matching on the message text, deliberately: every Rust log line is
 * emitted through one macro helper, so the `[target]` field reads
 * `mnema_lib::native_capture::debug_log` for essentially the whole file and
 * cannot discriminate. These are triage hints, not a taxonomy — a chip narrows
 * the tail, it does not promise completeness.
 *
 * No `/g` flag anywhere: a global regex carries `lastIndex` between `.test()`
 * calls and would skip matching lines at random.
 */
export const LOG_FEATURE_PATTERNS: Record<DebugFeature, RegExp> = {
	capture: /capture|segment|session|frame|display|screen|recording|microphone|audio/i,
	privacy: /privacy|exclusion|excluded|redact|inactivity|idle/i,
	ocr: /\bocr\b|recognition|recognizer|vision|tesseract/i,
	transcription: /transcri|whisper|deepgram/i,
	diarization: /diariz|speaker|speakrs/i,
	embeddings: /embed|semantic|vector|\bindex\b|backfill|quarantin/i,
	aiRuntime: /ai runtime|ask ai|ai_runtime|ask_ai|\brig\b|\bllm\b|anthropic|openai|ollama|llamafile/i,
	userContext: /user context|user_context|distill|derivation|subject|belief|digest/i,
	jobsAndStorage: /\bjobs?\b|worker|migration|sqlite|database|storage|cleanup|retention/i,
};

/** The rows to render: every line when no chip is picked, else the matches. */
export function filterLogLines(lines: string[], feature: DebugFeature | null): string[] {
	if (feature == null) return lines;
	const pattern = LOG_FEATURE_PATTERNS[feature];
	return lines.filter((line) => pattern.test(line));
}

/** ERROR/WARN tint for one line — cheap enough to run per row, per poll. */
export function logLineClass(line: string): string {
	if (line.includes("[ERROR]")) return "log-line log-line--err";
	if (line.includes("[WARN]")) return "log-line log-line--warn";
	return "log-line";
}
