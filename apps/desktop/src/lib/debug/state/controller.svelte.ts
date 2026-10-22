// One controller owning the developer window's stores, navigation and the
// single polling primitive, shared with every tab via Svelte context.
//
// The shell (routes/debug/+page.svelte) builds it once; tabs read it via
// `getDebugController()` and start their own polls with `controller.poll` in
// their own `$effect`, so a poll lives exactly as long as its tab is mounted.

import { getContext, setContext, untrack } from "svelte";
import { createCaptureStore } from "./capture.svelte";
import { createPipelineStore } from "./pipeline.svelte";
import { createHealthStore } from "./health.svelte";
import { createFeaturesStore } from "./features.svelte";
import { createDetailStore } from "./detail.svelte";
import type { ProcessorKey } from "../detail/specs";

export type DevTab = "overview" | "pipeline" | "capture" | "ai" | "logs";
export type DevMode = "full" | "mini";

/**
 * Run `fn` now and then every `ms`. A tick is skipped while live updates are
 * paused, while the document is hidden, and while the previous call is still in
 * flight. Returns the teardown.
 */
export type PollFn = (fn: () => unknown, ms: number) => () => void;

export function createDebugController() {
	let live = $state(true);
	let pausedAtMs = $state<number | null>(null);
	let mode = $state<DevMode>("full");
	const nav = $state({
		tab: "overview" as DevTab,
		processor: null as ProcessorKey | null,
		jobId: null as number | null,
		logNeedle: "",
	});

	// Every running poll's tick, so Resume can refresh them all at once instead
	// of leaving the window stale until each interval comes round.
	const ticks = new Set<() => void>();

	const poll: PollFn = (fn, ms) => {
		let inFlight = false;
		const tick = () => {
			if (!live || inFlight) return;
			if (typeof document !== "undefined" && document.visibilityState === "hidden") return;
			inFlight = true;
			void (async () => {
				try {
					await fn();
				} finally {
					inFlight = false;
				}
			})();
		};
		ticks.add(tick);
		// Untracked: callers start polls inside an `$effect`, and neither `live`
		// nor whatever `fn` reads before its first await may become that effect's
		// dependency (a Resume would otherwise restart the poll and double-fire).
		untrack(tick);
		const interval = setInterval(tick, ms);
		return () => {
			clearInterval(interval);
			ticks.delete(tick);
		};
	};

	return {
		capture: createCaptureStore(),
		pipeline: createPipelineStore(),
		health: createHealthStore(poll),
		features: createFeaturesStore(),
		detail: createDetailStore(),

		nav,
		poll,

		get live() { return live; },
		/** Wall-clock ms the window was paused at; null while live. */
		get pausedAtMs() { return pausedAtMs; },
		toggleLive() {
			live = !live;
			pausedAtMs = live ? null : Date.now();
			if (live) for (const tick of ticks) tick();
		},

		get mode() { return mode; },
		set mode(next: DevMode) { mode = next; },

		/** Switch tab; a tab switch always lands on the tab's top level. */
		go(tab: DevTab) {
			nav.tab = tab;
			nav.processor = null;
			nav.jobId = null;
		},
		openProcessor(processor: ProcessorKey) {
			nav.tab = "pipeline";
			nav.processor = processor;
			nav.jobId = null;
		},
		openJob(id: number) {
			nav.tab = "pipeline";
			nav.jobId = id;
		},
		/** One level up: job → processor → tab. */
		back() {
			if (nav.jobId != null) nav.jobId = null;
			else nav.processor = null;
		},
		showInLog(needle: string) {
			nav.tab = "logs";
			nav.processor = null;
			nav.jobId = null;
			nav.logNeedle = needle;
		},
	};
}

export type DebugController = ReturnType<typeof createDebugController>;

const KEY = Symbol("debug-controller");

export function setDebugController(controller: DebugController): void {
	setContext(KEY, controller);
}

export function getDebugController(): DebugController {
	const controller = getContext<DebugController | undefined>(KEY);
	if (!controller) throw new Error("DebugController missing — call setDebugController() in the debug shell");
	return controller;
}
