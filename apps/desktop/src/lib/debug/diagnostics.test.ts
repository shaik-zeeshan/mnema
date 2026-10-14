// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig, so skip static checking here.
import { describe, expect, it } from "bun:test";
import { formatDiagnostics } from "./diagnostics";

const BASE = {
	version: "0.2.0",
	generatedAtMs: Date.UTC(2026, 9, 13, 14, 28),
	health: [
		{ feature: "screen", severity: "ok", reason: "Recording", metric: null },
		{ feature: "transcription", severity: "warn", reason: "Deepgram can’t be reached, retrying on its own", metric: "14 waiting" },
	],
	pipeline: [
		{ processor: "ocr", queued: 3, retrying: 1, running: 1, completed: 900, failed: 2, failedLast24h: 1,
			averageCompletedSecondsLast24h: 1.84, lastError: null },
	],
	semantic: { vectorCount: 1200, backlogCount: 40, liveDimension: 768, modelLoaded: false,
		consecutiveLoadFailures: 0, quarantinedCount: 0, lastLoadError: null },
	log: { path: "/logs/rust.log", exists: true, lines: ["INFO a", "WARN b"] },
};

describe("formatDiagnostics", () => {
	it("writes version, health rows, queues, index counts and the log tail", () => {
		const text = formatDiagnostics(BASE);
		expect(text).toContain("version: 0.2.0");
		expect(text).toContain("generated: 2026-10-13T14:28:00.000Z");
		expect(text).toContain("transcription  warn  Deepgram can’t be reached, retrying on its own  [14 waiting]");
		expect(text).toContain("screen  ok  Recording\n");
		expect(text).toContain("ocr  queued 3 (retrying 1)  running 1  done 900  failed 2 (24h 1)  avg 1.8s");
		expect(text).toContain("meaning index  vectors 1200  backlog 40  dimension 768");
		expect(text.endsWith("/logs/rust.log\nINFO a\nWARN b\n")).toBe(true);
	});

	it("marks missing sources instead of dropping the section", () => {
		const text = formatDiagnostics({ ...BASE, version: null, health: [], pipeline: null, semantic: null, log: null });
		expect(text).toContain("version: unknown");
		expect(text).toContain("## Health\n(not loaded)");
		expect(text).toContain("## Queues\n(unavailable)");
		expect(text).toContain("## Log (last 200 lines)\n(unavailable)");
		expect(text).not.toContain("meaning index");
	});

	it("says when the log file is missing", () => {
		const text = formatDiagnostics({ ...BASE, log: { path: "/logs/rust.log", exists: false, lines: [] } });
		expect(text).toContain("(missing: /logs/rust.log)");
	});
});
