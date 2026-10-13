// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig, so skip static checking here.
import { describe, expect, it } from "bun:test";
import { diskAt, formatWait, gb, laneDetail, meterBars, rebuildReasonText, reasonPhrase } from "./capture-view";

const lane = (state, reason = null, deviceName = null) => ({ requested: true, state, reason, levelDb: null, idleMs: null, deviceName });

describe("capture view", () => {
	it("maps the suspension reasons to plain words", () => {
		expect(reasonPhrase("capture_display_unavailable")).toBe("display asleep");
		expect(reasonPhrase("privacy_filter_apply_failed")).toBe("privacy filter retrying");
		expect(reasonPhrase("privacy_recovery_restart_required")).toBe("privacy filter needs a restart");
		expect(reasonPhrase("capture_low_disk")).toBe("disk almost full");
		expect(reasonPhrase("something_new")).toBeNull();
		expect(reasonPhrase(null)).toBeNull();
	});

	it("lane detail prefers the device, else the reason, plus the tap rate", () => {
		expect(laneDetail(lane("waiting", "capture_display_unavailable"))).toBe("display asleep");
		expect(laneDetail(lane("waiting", "capture_display_unavailable", "Built-in Retina Display"))).toBe("display asleep");
		expect(laneDetail(lane("recording", null, "AirPods Pro"), 48_000)).toBe("AirPods Pro · 48 kHz");
		expect(laneDetail(lane("recording"), 48_000)).toBe("48 kHz");
		expect(laneDetail(lane("off"), 48_000)).toBe("");
	});

	it("meter lights clamp((dB + 60) / 60 · 12)", () => {
		expect(meterBars(null)).toBe(0);
		expect(meterBars(-60)).toBe(0);
		expect(meterBars(-30)).toBe(6);
		expect(meterBars(0)).toBe(12);
		expect(meterBars(6)).toBe(12);
	});

	it("formats waits, GB and the headroom scale", () => {
		expect(formatWait(6 * 60_000 + 5_000)).toBe("6 min");
		expect(formatWait(134 * 60_000)).toBe("2h 14m");
		expect(gb(182 * 1024 ** 3)).toBe("182");
		expect(gb(2.1 * 1024 ** 3)).toBe("2.1");
		expect(gb(5 * 1024 ** 3)).toBe("5");
		expect(gb(41.23 * 1024 ** 3)).toBe("41.2");
		expect(diskAt(5 * 1024 ** 3)).toBe(0.2);
		expect(diskAt(500 * 1024 ** 3)).toBe(1);
	});

	it("reads a sample-rate rebuild as the headset changing mode", () => {
		expect(rebuildReasonText("sampleRateChanged", "48000 → 16000 Hz")).toEqual({ label: "Headset switched to call quality", detail: "48 → 16 kHz" });
		expect(rebuildReasonText("sampleRateChanged", "16000 → 48000 Hz")).toEqual({ label: "Headset back to full quality", detail: "16 → 48 kHz" });
		expect(rebuildReasonText("sampleRateChanged", "48000 → 44100 Hz")).toEqual({ label: "Sample rate changed", detail: "48 → 44.1 kHz" });
		expect(rebuildReasonText("sampleRateChanged", null)).toEqual({ label: "Sample rate changed", detail: null });
		expect(rebuildReasonText("deviceDied", null)).toEqual({ label: "Output device went away", detail: null });
		expect(rebuildReasonText(null)).toEqual({ label: "—", detail: null });
	});
});
