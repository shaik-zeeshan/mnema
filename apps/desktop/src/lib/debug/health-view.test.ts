// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig, so skip static checking here.
import { describe, expect, it } from "bun:test";
import { group, metricText, miniFooter, rowView } from "./health-view";

const row = (feature, severity, reason = `${feature} ${severity}`, metric = "m", metricAtMs = null) => ({
	feature,
	severity,
	reason,
	metric,
	metricAtMs,
});

describe("group", () => {
	it("buckets by tone, ranks within a group, drops empty groups", () => {
		const groups = group([
			row("screen", "ok"),
			row("systemAudio", "warn"),
			row("reading", "error"),
			row("microphone", "off"),
			row("meaningIndex", "info"),
		]);
		expect(groups.map((g) => [g.title, g.rows.map((r) => r.feature)])).toEqual([
			["Needs a look", ["reading", "systemAudio"]],
			["Right now", ["meaningIndex", "microphone"]],
			["Running fine", ["screen"]],
		]);
		expect(group([row("screen", "ok")]).map((g) => g.title)).toEqual(["Running fine"]);
	});
});

describe("rowView", () => {
	it("a troubled processor opens its drill-in; everything else its tab", () => {
		expect(rowView(row("transcription", "warn")).target).toEqual({ processor: "transcription" });
		expect(rowView(row("transcription", "ok")).target).toEqual({ tab: "pipeline" });
		expect(rowView(row("systemAudio", "warn")).target).toEqual({ tab: "capture" });
		expect(rowView(row("startup", "error")).target).toEqual({ tab: "logs" });
	});

	it("metric: metricAtMs renders a clock, the AI engine prefixes 'used '", () => {
		const at = new Date(2026, 0, 1, 14, 28).getTime();
		expect(metricText(row("aiEngine", "ok", "", null, at))).toMatch(/^used .*2:28/);
		expect(metricText(row("userContext", "ok", "", null, at))).toMatch(/^\d{1,2}:28/);
		expect(metricText(row("disk", "ok", "", "182 GB"))).toBe("182 GB");
		expect(metricText(row("disk", "ok", "", null))).toBe("");
	});
});

describe("miniFooter", () => {
	it("names the worst row", () => {
		expect(miniFooter([row("transcription", "warn", "Deepgram offline"), row("reading", "error", "1 failed")])).toEqual({
			tone: "danger",
			text: "Reading: 1 failed",
		});
	});

	it("falls back to the verdict sentence when nothing needs a look", () => {
		expect(miniFooter([row("screen", "ok")])).toEqual({ tone: "ok", text: "Everything is running" });
		const stopped = [row("screen", "off"), row("microphone", "off"), row("systemAudio", "off"), row("reading", "info")];
		expect(miniFooter(stopped)).toEqual({ tone: "off", text: "Not recording · finishing earlier work" });
	});
});
