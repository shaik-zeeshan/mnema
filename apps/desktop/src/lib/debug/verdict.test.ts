// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig, so skip static checking here.
import { describe, expect, it } from "bun:test";
import { formatRunning, healthVerdict, toneOf, verdict } from "./verdict";

const row = (feature, severity, reason = `${feature} ${severity}`) => ({ feature, severity, reason });
const RECORDING = [row("screen", "ok"), row("microphone", "ok"), row("systemAudio", "ok")];
const STOPPED = [row("screen", "off"), row("microphone", "off"), row("systemAudio", "off")];

describe("verdict", () => {
	it("counts warn + error rows, tone = the worst", () => {
		expect(verdict([...RECORDING, row("reading", "error"), row("transcription", "warn")])).toEqual({
			tone: "danger",
			text: "2 things need a look",
		});
		expect(verdict([...RECORDING, row("transcription", "warn")])).toEqual({ tone: "warn", text: "1 thing needs a look" });
	});

	it("problems win over not recording", () => {
		expect(verdict([...STOPPED, row("disk", "warn")]).text).toBe("1 thing needs a look");
	});

	it("names the first info row and says its reason", () => {
		expect(
			verdict([row("screen", "info", "Waiting, display asleep"), row("microphone", "ok"), row("reading", "info", "second")]),
		).toEqual({ tone: "info", text: "Screen: Waiting, display asleep" });
	});

	it("reads all-off capture as not recording, naming the draining queues", () => {
		expect(verdict([...STOPPED, row("reading", "info", "Finishing the last recording")])).toEqual({
			tone: "off",
			text: "Not recording · finishing earlier work",
		});
		expect(verdict([...STOPPED, row("reading", "ok")])).toEqual({ tone: "off", text: "Not recording" });
	});

	it("is quiet when everything is ok", () => {
		expect(verdict([...RECORDING, row("reading", "ok")])).toEqual({ tone: "ok", text: "Everything is running" });
	});
});

describe("healthVerdict", () => {
	it("a failing read wins over the last good rows", () => {
		expect(healthVerdict(RECORDING, "boom")).toEqual({ tone: "danger", text: "Can’t read health" });
		expect(healthVerdict(RECORDING, null)).toEqual({ tone: "ok", text: "Everything is running" });
		expect(healthVerdict([], null)).toEqual({ tone: "off", text: "Checking…" });
	});
});

describe("toneOf", () => {
	it("maps error to danger and passes the rest through", () => {
		expect(toneOf("error")).toBe("danger");
		expect(toneOf("warn")).toBe("warn");
		expect(toneOf("off")).toBe("off");
		expect(toneOf("ok")).toBe("ok");
	});
});

describe("formatRunning", () => {
	it("hours and minutes", () => {
		expect(formatRunning((2 * 60 + 14) * 60_000 + 5_000)).toBe("2h 14m");
		expect(formatRunning(14 * 60_000)).toBe("14m");
	});
});
