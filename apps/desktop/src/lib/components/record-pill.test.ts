// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, test } from "bun:test";
import { formatElapsed, formatPillBytes, pillView } from "./record-pill";

const src = (over = {}) => ({
	requested: true,
	paused: false,
	sessionActive: true,
	writerActive: true,
	outputPath: null,
	reason: null,
	...over,
});

const inputs = (over = {}) => ({
	running: true,
	starting: false,
	stopping: false,
	settingsLoading: false,
	userPaused: false,
	inactivityPaused: false,
	lowDisk: false,
	idleMs: null,
	sources: { screen: src(), microphone: src(), systemAudio: src() },
	permissions: {
		screen: "granted",
		microphone: "granted",
		systemAudio: "assumed_working",
	},
	selected: { screen: true, microphone: true, systemAudio: true },
	...over,
});

describe("pillView — the nine frame-11 states", () => {
	test("idle renders the Record button, never a pill", () => {
		const v = pillView(inputs({ running: false }));
		expect(v.state).toBe("idle");
		expect(v.kind).toBe("button");
	});

	test("starting is a quiet spinner pill", () => {
		const v = pillView(inputs({ running: false, starting: true }));
		expect(v).toMatchObject({ state: "starting", tone: "quiet", dot: "spinner", word: "Starting" });
	});

	test("recording is the resting state: red dot + time + cost", () => {
		const v = pillView(inputs());
		expect(v).toMatchObject({
			state: "recording",
			tone: "record",
			dot: "rec",
			showTime: true,
			showCost: true,
			word: null,
		});
	});

	test("manual pause is quiet with the timer kept", () => {
		const v = pillView(inputs({ userPaused: true }));
		expect(v).toMatchObject({ state: "paused-user", tone: "quiet", dot: "idle", word: "Paused", showTime: true });
	});

	test("inactivity pause reads the idle minutes", () => {
		const v = pillView(inputs({ inactivityPaused: true, idleMs: 12 * 60_000 }));
		expect(v).toMatchObject({ state: "paused-inactivity", word: "Idle 12m", tone: "quiet" });
		expect(pillView(inputs({ inactivityPaused: true })).word).toBe("Idle");
	});

	test("low disk is warn-tinted and outranks the pause words", () => {
		const v = pillView(inputs({ lowDisk: true, userPaused: true, inactivityPaused: true }));
		expect(v).toMatchObject({ state: "low-disk", tone: "warn", dot: "warn", word: "Low disk" });
	});

	test("display unavailable keeps the red dot + timer, info word", () => {
		const v = pillView(
			inputs({
				sources: {
					screen: src({ reason: "capture_display_unavailable", sessionActive: false, writerActive: false }),
					microphone: src(),
					systemAudio: src(),
				},
			}),
		);
		expect(v).toMatchObject({
			state: "display-unavailable",
			tone: "record",
			dot: "rec",
			showTime: true,
			word: "screen asleep",
			wordTone: "info",
		});
	});

	test("a lost source is a warn word while capture continues", () => {
		const v = pillView(
			inputs({
				sources: { screen: src(), microphone: src({ writerActive: false }), systemAudio: src() },
			}),
		);
		expect(v).toMatchObject({ state: "source-degraded", word: "mic lost", wordTone: "warn", showTime: true });
	});

	test("permission missing (idle) is a warn pill, not danger", () => {
		const v = pillView(
			inputs({ running: false, permissions: { screen: "denied", microphone: "granted", systemAudio: "unknown" } }),
		);
		expect(v).toMatchObject({ state: "permission-missing", tone: "warn", word: "screen not allowed" });
	});

	test("an unselected source's denied permission does not warn", () => {
		const v = pillView(
			inputs({
				running: false,
				permissions: { screen: "denied", microphone: "granted", systemAudio: "unknown" },
				selected: { screen: false, microphone: true, systemAudio: true },
			}),
		);
		expect(v.state).toBe("idle");
	});

	test("privacy recovery escalation asks for a restart via warn tone", () => {
		const v = pillView(
			inputs({
				sources: {
					screen: src({ reason: "privacy_recovery_restart_required", sessionActive: false }),
					microphone: src(),
					systemAudio: src(),
				},
			}),
		);
		expect(v).toMatchObject({ state: "source-degraded", tone: "warn", word: "restart screen" });
	});
});

describe("formatElapsed / formatPillBytes", () => {
	test("h:mm:ss with unpadded hours", () => {
		expect(formatElapsed(0)).toBe("0:00:00");
		expect(formatElapsed(41_000)).toBe("0:00:41");
		expect(formatElapsed((2 * 3600 + 14 * 60 + 7) * 1000)).toBe("2:14:07");
	});

	test("whole megabytes, one-decimal gigabytes", () => {
		expect(formatPillBytes(0)).toBe("0 MB");
		expect(formatPillBytes(270_400_000)).toBe("270 MB");
		expect(formatPillBytes(1_400_000_000)).toBe("1.4 GB");
		expect(formatPillBytes(-1)).toBe("");
	});
});
