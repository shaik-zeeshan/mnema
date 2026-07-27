// The drawer's waveform scrubber (`waveform-peaks.svelte.ts`). Three contracts,
// none of which `cargo check` or `bun run check` can see:
//
//  1. Empty peaks are the FALLBACK, not an error state — a rejected/garbage
//     `invoke` must leave `value` as `[]` (plain scrub bar), never throw.
//  2. A late response from the PREVIOUS segment must never paint over the new
//     segment's transcript.
//  3. The effect depends on the segment id VALUE, not on the row object it was
//     read off. `refreshAudioSegments()` (the ~1.5s head poll while capturing)
//     reassigns `audioSegments` with freshly mapped DTOs, so an identity-level
//     dependency re-fires every poll — and every re-fire is a full AVFoundation
//     decode (1.09–1.25s measured) of the open segment on the blocking pool.
//
// Runes can't run under bun test, so specs/_reactivity/build.mjs precompiles the
// REAL module with Svelte's compiler under node (same harness as
// jumper-reactivity.test.ts / licensing-store-race.test.ts). The caller side is
// built here from the same `svelte/internal/client` primitives the compiler
// emits, mirroring routes/+page.svelte's `audioSegments.find(...)` selection.
import { test, expect, mock, beforeAll } from "bun:test";
import { spawnSync } from "child_process";
import { flushSync } from "svelte";
import { resolve } from "path";
import * as $ from "svelte/internal/client";

const here = resolve(import.meta.dir, "_reactivity");

type InvokeCall = { cmd: string; args: unknown };
const calls: InvokeCall[] = [];
let invokeImpl: (cmd: string, args: unknown) => Promise<unknown> = async () => [];

mock.module("@tauri-apps/api/core", () => ({
	invoke: (cmd: string, args: unknown) => {
		calls.push({ cmd, args });
		return invokeImpl(cmd, args);
	},
	// bun module mocks fix the export-name set process-wide; keep parity with
	// the other specs' mock of this module.
	convertFileSrc: (p: string) => p,
}));

type Module = {
	waveformPeaks: (id: () => number, buckets?: number) => { readonly value: number[] };
	WAVEFORM_BUCKETS: number;
};
let waveformPeaks: Module["waveformPeaks"];
let WAVEFORM_BUCKETS: number;

beforeAll(async () => {
	const built = spawnSync("node", [resolve(here, "build.mjs")], { encoding: "utf8" });
	if (built.status !== 0) {
		throw new Error(`rune precompile failed: ${built.stderr || built.stdout}`);
	}
	({ waveformPeaks, WAVEFORM_BUCKETS } = (await import(
		resolve(here, "gen/waveform-peaks.js")
	)) as Module);
});

type Row = { id: number };

/**
 * The production caller shape: rows live in reactive state, the selected row is
 * derived out of them by id (routes/+page.svelte L922-929), and the drawer hands
 * `waveformPeaks` a closure that reads the id off that row.
 */
function drive(rows: Row[], selectedId: number) {
	const rowsState = $.state(rows);
	const idState = $.state(selectedId);
	let peaks!: { readonly value: number[] };
	const stop = $.effect_root(() => {
		const selected = $.derived(() =>
			($.get(rowsState) as Row[]).find((r) => r.id === ($.get(idState) as number)),
		);
		peaks = waveformPeaks(() => ($.get(selected) as Row).id);
	});
	flushSync();
	return {
		get value() {
			return [...peaks.value];
		},
		/** One `refreshAudioSegments()` head-poll tick: same ids, new objects. */
		repoll(next: Row[]) {
			$.set(rowsState, next);
			flushSync();
		},
		select(id: number) {
			$.set(idState, id);
			flushSync();
		},
		stop,
	};
}

/** Let the mocked invoke's promise settle, then flush the effects it woke. */
async function settle() {
	await new Promise((r) => setTimeout(r, 0));
	flushSync();
	await new Promise((r) => setTimeout(r, 0));
}

test("renders real peaks, and falls back to empty on a failed decode", async () => {
	invokeImpl = async () => [0.1, 0.9];
	const ok = drive([{ id: 42 }], 42);
	await settle();
	expect(ok.value).toEqual([0.1, 0.9]);
	ok.stop();

	// A rejected decode is the plain scrub bar, not an error state or a throw.
	invokeImpl = async () => {
		throw new Error("decode failed");
	};
	const failed = drive([{ id: 42 }], 42);
	await settle();
	expect(failed.value).toEqual([]);
	failed.stop();

	// Same for a response that isn't an array (the Array.isArray guard).
	invokeImpl = async () => "nope";
	const garbage = drive([{ id: 42 }], 42);
	await settle();
	expect(garbage.value).toEqual([]);
	garbage.stop();
});

test("pins the get_audio_segment_waveform_peaks request shape", async () => {
	invokeImpl = async () => [];
	calls.length = 0;
	const d = drive([{ id: 42 }], 42);
	await settle();

	// Rust boundary: `request` arg name + camelCase serde fields
	// (app_infra.rs GetAudioSegmentWaveformPeaksRequest). A rename here is a
	// runtime-only failure both `cargo check` and `bun run check` pass.
	expect(calls).toEqual([
		{
			cmd: "get_audio_segment_waveform_peaks",
			args: { request: { audioSegmentId: 42, bucketCount: WAVEFORM_BUCKETS } },
		},
	]);
	d.stop();
});

test("clears peaks when the segment changes and ignores the stale response", async () => {
	const pending: Array<(v: unknown) => void> = [];
	invokeImpl = () => new Promise((r) => pending.push(r));
	calls.length = 0;

	const d = drive([{ id: 41 }, { id: 42 }], 41);
	await settle();
	expect(pending.length).toBe(1);

	// Switch before 41's decode comes back: the scrubber must go blank rather
	// than keep drawing 41's shape over 42's transcript.
	d.select(42);
	await settle();
	expect(d.value).toEqual([]);
	expect(pending.length).toBe(2);

	// 41's decode lands late — it must be dropped, not painted.
	pending[0]([0.41, 0.41]);
	await settle();
	expect(d.value).toEqual([]);

	// 42's own decode is what shows.
	pending[1]([0.42, 0.42]);
	await settle();
	expect(d.value).toEqual([0.42, 0.42]);
	d.stop();
});

test("does not re-fetch when the segment row object is replaced by an equal row", async () => {
	invokeImpl = async (_cmd, args) =>
		(args as { request: { audioSegmentId: number } }).request.audioSegmentId === 42
			? [0.42]
			: [0.41];
	calls.length = 0;

	const d = drive([{ id: 41 }, { id: 42 }], 42);
	await settle();
	expect(calls.length).toBe(1);
	expect(d.value).toEqual([0.42]);

	// Three head-poll ticks: the same segments arrive as brand-new objects.
	for (let i = 0; i < 3; i++) {
		d.repoll([{ id: 41 }, { id: 42 }]);
		await settle();
	}
	expect(calls.length).toBe(1);
	expect(d.value).toEqual([0.42]);

	// ...and a real segment switch still fetches.
	d.select(41);
	await settle();
	expect(calls.length).toBe(2);
	expect(d.value).toEqual([0.41]);
	d.stop();
});
