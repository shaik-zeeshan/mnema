// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig, so skip static checking here.
import { describe, expect, it } from "bun:test";
import {
	CAPTURE_INTERVAL_LADDER_S,
	captureIntervalPhrase,
	captureRateShortLabel,
	fpsToIntervalS,
	intervalSToFps,
	nearestLadderIndex,
	relativeStorageLabel,
	snapshotsPerMinute,
	timelineGapMs,
} from "./capture-rate";

// Backend bounds are 1/60..=10 fps (native_capture_settings.rs).
const MIN_FPS = 1 / 60;
const MAX_FPS = 10;

describe("fpsToIntervalS", () => {
	it("inverts a positive fps", () => {
		expect(fpsToIntervalS(0.5)).toBe(2);
		expect(fpsToIntervalS(10)).toBe(0.1);
	});
	it("falls back to the 2s default for non-positive / NaN fps", () => {
		expect(fpsToIntervalS(0)).toBe(2);
		expect(fpsToIntervalS(-1)).toBe(2);
		expect(fpsToIntervalS(NaN)).toBe(2);
	});
});

describe("ladder ↔ backend bounds", () => {
	it("keeps every ladder stop inside the backend [1/60, 10] fps range", () => {
		for (const intervalS of CAPTURE_INTERVAL_LADDER_S) {
			const fps = intervalSToFps(intervalS);
			expect(fps).toBeGreaterThanOrEqual(MIN_FPS);
			expect(fps).toBeLessThanOrEqual(MAX_FPS);
		}
	});
	it("round-trips index → fps → index for every ladder stop (no thumb jump)", () => {
		CAPTURE_INTERVAL_LADDER_S.forEach((intervalS, i) => {
			expect(nearestLadderIndex(intervalSToFps(intervalS))).toBe(i);
		});
	});
});

describe("nearestLadderIndex", () => {
	it("maps the 0.5 fps default to the 2s stop", () => {
		expect(CAPTURE_INTERVAL_LADDER_S[nearestLadderIndex(0.5)]).toBe(2);
	});
	it("maps a legacy off-ladder fps to the nearest log stop without mutating it", () => {
		const legacy = 7.5;
		const idx = nearestLadderIndex(legacy);
		expect(CAPTURE_INTERVAL_LADDER_S[idx]).toBe(0.1); // ~10 snapshots/sec
		expect(legacy).toBe(7.5);
	});
	it("returns a valid in-range index for degenerate inputs", () => {
		for (const fps of [0, -1, NaN, Infinity, 1e-9, 1e12]) {
			const idx = nearestLadderIndex(fps);
			expect(idx).toBeGreaterThanOrEqual(0);
			expect(idx).toBeLessThan(CAPTURE_INTERVAL_LADDER_S.length);
			expect(CAPTURE_INTERVAL_LADDER_S[idx]).toBeDefined();
		}
	});
});

describe("captureIntervalPhrase", () => {
	it("covers each phrasing branch", () => {
		expect(captureIntervalPhrase(0.5)).toBe("2 times a second");
		expect(captureIntervalPhrase(1)).toBe("every second");
		expect(captureIntervalPhrase(2)).toBe("every 2 seconds");
		expect(captureIntervalPhrase(60)).toBe("once per minute");
	});
});

describe("captureRateShortLabel", () => {
	it("covers each short-form branch (snaps fps to the nearest ladder stop)", () => {
		expect(captureRateShortLabel(10)).toBe("10 snapshots/sec");
		expect(captureRateShortLabel(1)).toBe("1 snapshot/sec");
		expect(captureRateShortLabel(1 / 60)).toBe("1 snapshot/min");
		expect(captureRateShortLabel(0.5)).toBe("1 snapshot every 2s");
	});
});

describe("relativeStorageLabel", () => {
	it("shows ≥1× and 1/N× branches relative to the 2s default", () => {
		expect(relativeStorageLabel(2)).toBe("1×");
		expect(relativeStorageLabel(1)).toBe("2×");
		expect(relativeStorageLabel(0.1)).toBe("20×");
		expect(relativeStorageLabel(60)).toBe("1/30×");
	});
});

// The value ChangeSettingsScreen hands its capture-rate `Select`, and the
// option values it picks from. Through the ladder, never a raw 1/fps.
const optionValues = CAPTURE_INTERVAL_LADDER_S.map((s) => String(s));
const selectValue = (fps: number) => String(CAPTURE_INTERVAL_LADDER_S[nearestLadderIndex(fps)]);

describe("capture rate Select value", () => {
	it("resolves a legacy off-ladder rate to an option that exists", () => {
		// 1/7.5 = 0.1333s matched no option, so the picker rendered blank.
		expect(optionValues).not.toContain(String(fpsToIntervalS(7.5)));
		expect(selectValue(7.5)).toBe("0.1");
		expect(optionValues).toContain(selectValue(7.5));
	});
	it("never lands outside the option list, for any stored rate", () => {
		for (const fps of [0, -1, NaN, 0.001, 1 / 45.5, 0.3, 0.9, 7.5, 9.9, 60, 1e12]) {
			expect(optionValues).toContain(selectValue(fps));
		}
	});
	it("round-trips every ladder stop unchanged", () => {
		for (const intervalS of CAPTURE_INTERVAL_LADDER_S) {
			expect(selectValue(intervalSToFps(intervalS))).toBe(String(intervalS));
		}
	});
});

describe("snapshotsPerMinute", () => {
	it("counts the snapshot at t=0, so the 45s stop holds two", () => {
		expect(snapshotsPerMinute(45)).toBe(2);
	});
	it("keeps the closing edge exclusive at one a minute", () => {
		expect(snapshotsPerMinute(60)).toBe(1);
	});
	it("matches a hand count across the whole ladder", () => {
		expect(CAPTURE_INTERVAL_LADDER_S.map(snapshotsPerMinute)).toEqual([
			600, 120, 60, 30, 20, 12, 6, 4, 2, 2, 1,
		]);
	});
	it("agrees with simulating one minute of capture", () => {
		for (const intervalS of CAPTURE_INTERVAL_LADDER_S) {
			let taken = 0;
			for (let t = 0; t < 60 - 1e-9; t += intervalS) taken += 1;
			expect(snapshotsPerMinute(intervalS)).toBe(taken);
		}
	});
	it("never undercounts the old Math.round(60 / s) reading", () => {
		for (const intervalS of CAPTURE_INTERVAL_LADDER_S) {
			expect(snapshotsPerMinute(intervalS)).toBeGreaterThanOrEqual(Math.round(60 / intervalS));
		}
	});
});

describe("timelineGapMs", () => {
	it("floors at 10s for unset / fast rates, opens up for slow rates", () => {
		expect(timelineGapMs(undefined)).toBe(10_000); // fallback 2s → 6000, floored
		expect(timelineGapMs(0)).toBe(10_000);
		expect(timelineGapMs(10)).toBe(10_000); // 3×100ms floored
		expect(timelineGapMs(0.5)).toBe(10_000); // default 2s → 6000, floored
		expect(timelineGapMs(1 / 60)).toBe(180_000); // 3×60s exceeds floor
	});
});
