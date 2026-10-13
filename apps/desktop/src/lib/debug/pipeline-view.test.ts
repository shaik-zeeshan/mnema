// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig, so skip static checking here.
import { describe, expect, it } from "bun:test";
import { appJobsLine, engineWords, flowNodes, frameBatchLine, processorRows, retentionLine } from "./pipeline-view";

const h = (feature, severity) => ({ feature, severity });
const lane = (processor, o = {}) => ({
	processor, queued: 0, retrying: 0, running: 0, completed: 0, failed: 0, failedLast24h: 0,
	averageCompletedSecondsLast24h: null, lastError: null, ...o,
});
const lanesOf = (map) => (p) => map[p] ?? lane(p);
const ISSUES_HEALTH = [h("screen", "ok"), h("microphone", "ok"), h("systemAudio", "warn"), h("reading", "error"), h("transcription", "warn")];
const OFF_HEALTH = [h("screen", "off"), h("microphone", "off"), h("systemAudio", "off")];
const settings = (provider) => ({
	ocr: { provider: "apple_vision", modelId: null },
	transcription: { provider, modelId: null },
	speakerAnalysis: { provider: "speakrs", modelId: null },
	semanticSearch: { provider: "x", modelId: "nomic-embed-text-v1.5" },
});

describe("flowNodes", () => {
	it("maps the issues scenario: waiting = queued (retrying is inside it), tones from failures/retries", () => {
		const nodes = flowNodes(
			ISSUES_HEALTH,
			lanesOf({
				ocr: lane("ocr", { queued: 9, running: 1, failedLast24h: 1 }),
				audio_transcription: lane("audio_transcription", { queued: 14, retrying: 12, failedLast24h: 1 }),
			}),
			{ vectorCount: 12480, backlogCount: 31 },
		);
		expect(nodes.map((n) => n.big)).toEqual(["3", "9", "14", "0", "31"]);
		expect(nodes[0].lines).toEqual([{ text: "sys audio", tone: "warn" }, { text: "recovering", tone: "warn" }]);
		expect(nodes[1].tone).toBe("danger");
		expect(nodes[1].lines[1]).toEqual({ text: "1 failed", tone: "danger" });
		expect(nodes[2].tone).toBe("warn");
		expect(nodes[2].lines[0]).toEqual({ text: "12 retrying", tone: "warn" });
		expect(nodes[2].bar).toBeCloseTo(0.28);
		expect(nodes[3].lines[0].text).toBe("idle");
		expect(nodes[4].tone).toBeUndefined();
		expect(nodes[4].lines[0].text).toBe((12480).toLocaleString() + " done");
	});

	it("does not count retrying jobs twice: they are already inside queued", () => {
		const nodes = flowNodes(OFF_HEALTH, lanesOf({ ocr: lane("ocr", { queued: 5, retrying: 5 }) }), null);
		expect(nodes[1].big).toBe("5");
		expect(nodes[1].bar).toBeCloseTo(5 / 50);
	});

	it("marks draining backlogs info when nothing records, and caps the bar", () => {
		const nodes = flowNodes(OFF_HEALTH, lanesOf({ ocr: lane("ocr", { queued: 80, running: 1 }) }), { vectorCount: 1, backlogCount: 212 });
		expect(nodes[0]).toMatchObject({ big: "0", lines: [{ text: "not recording" }] });
		expect(nodes[1]).toMatchObject({ tone: "info", bar: 1 });
		expect(nodes[4].tone).toBe("info");
	});
});

describe("processorRows", () => {
	it("names the engine, flags an unreachable cloud provider, formats the average", () => {
		const rows = processorRows(
			ISSUES_HEALTH,
			lanesOf({ audio_transcription: lane("audio_transcription", { queued: 3, retrying: 3, averageCompletedSecondsLast24h: 6.24 }), ocr: lane("ocr", { averageCompletedSecondsLast24h: 0.04 }) }),
			settings("deepgram"),
		);
		expect(rows.map((r) => r.key)).toEqual(["reading", "transcription", "speakers", "meaningIndex"]);
		expect(rows[0]).toMatchObject({ tone: "danger", why: "Apple Vision, on this Mac", metric: "40 ms avg" });
		expect(rows[1]).toMatchObject({ tone: "warn", why: "Deepgram, cloud · can’t be reached", metric: "6.2 s avg" });
		expect(rows[3]).toMatchObject({ why: "nomic-embed-text-v1.5, on this Mac", metric: "" });
		expect(engineWords("transcription", settings("parakeet"), null)).toBe("Parakeet, on this Mac");
		expect(engineWords("transcription", null, null)).toBe("");
	});
});

describe("housekeeping lines", () => {
	it("retention", () => {
		expect(retentionLine({ policy: "never", status: "skipped", deletedCaptureSegments: 0, createdAt: null }).value).toBe("keeps everything");
		expect(retentionLine({ policy: "days_30", status: "completed", deletedCaptureSegments: 12, createdAt: "2026-10-13 03:00:00" })).toEqual({
			value: "removed 12 files older than 30 days",
			at: "2026-10-13 03:00:00",
		});
		expect(retentionLine({ policy: "days_7", status: "failed", errorMessage: "disk full\nmore", createdAt: "t" }).value).toBe("failed · disk full");
	});

	it("frame batches and app jobs", () => {
		expect(frameBatchLine([]).value).toBe("none in progress, none failed");
		expect(frameBatchLine([{ status: "open", updatedAt: "a" }, { status: "failed", updatedAt: "b", lastError: "ffmpeg died\nstack" }])).toEqual({
			value: "1 failed · ffmpeg died",
			at: "b",
		});
		expect(appJobsLine([{ kind: "frame_batch_combine", status: "running", updatedAt: "x" }]).value).toBe("1 running · Combining frames");
		expect(appJobsLine([{ kind: "k", status: "completed", updatedAt: "x" }]).value).toBe("none running");
	});
});
