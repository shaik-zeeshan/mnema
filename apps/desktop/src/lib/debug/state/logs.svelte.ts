// Log tail state — backs `lib/debug/LogTail.svelte` (the Logs section today,
// the per-feature "Log tail" sub-tab in slice 7's detail views next).
//
// `tail_app_log` is a poll-and-replace snapshot: there is no server-side
// filtering and no tail-follow push. So this store owns three things — which
// file to read, the poll loop, and the client-side feature filter over the
// lines the backend hands back.
//
// One store per viewer (created inside the component, not on the debug
// controller): the Logs section and a detail view's log sub-tab want
// independent file/filter/follow state.

import { invoke } from "@tauri-apps/api/core";
import { humanizeError } from "$lib/format-error";
import { filterLogLines } from "../log-filter";
import type { AppLogFile, AppLogTailDto, DebugFeature } from "$lib/types";

/**
 * Lines requested per poll. The backend clamps to 1..=2000; this is the whole
 * window the viewer has, so it is also the render cap — surfaced in the UI
 * ("last N lines") so a truncated view can never read as "that's all there is".
 */
export const LOG_TAIL_LINES = 500;

/**
 * ponytail: 2s, not the page's usual 1s. Each tick re-reads the file front to
 * back Rust-side and re-renders up to LOG_TAIL_LINES rows; logs are written at
 * human pace, so a 1s tick would pay that twice to show the same lines.
 */
const LOG_POLL_MS = 2000;

/** The real on-disk file names — a debug tool should name what it reads. */
export const LOG_FILE_OPTIONS: { value: AppLogFile; label: string }[] = [
	{ value: "rust", label: "rust.log" },
	{ value: "nativeCapture", label: "native-capture-debug.log" },
];

export function createLogTailStore(
	init: { file?: AppLogFile; feature?: DebugFeature | null; needle?: string } = {},
) {
	let file = $state<AppLogFile>(init.file ?? "rust");
	let feature = $state<DebugFeature | null>(init.feature ?? null);
	/** Free-text substring filter (case-insensitive) — e.g. a job id to grep for. */
	let needle = $state(init.needle ?? "");
	/** Pin the view to the newest line. The user disengages by scrolling up. */
	let follow = $state(true);
	let tail = $state<AppLogTailDto | null>(null);
	let error = $state<string | null>(null);

	async function fetch() {
		// Nothing to tail into an off-screen view.
		if (typeof document !== "undefined" && document.visibilityState !== "visible") return;
		const requested = file;
		try {
			const next = await invoke<AppLogTailDto>("tail_app_log", {
				file: requested,
				lines: LOG_TAIL_LINES,
			});
			// A file switch landed while this was in flight — that request owns the
			// view now; dropping this one keeps rust.log's lines off native-capture's
			// path label.
			if (requested !== file) return;
			tail = next;
			error = null;
		} catch (err) {
			// A missing log is `exists: false`, not a throw — anything that lands
			// here is a real read failure.
			error = humanizeError(err);
		}
	}

	/** Poll while mounted + visible. Returns teardown. */
	function startPolling(): () => void {
		void fetch();
		const interval = setInterval(() => { void fetch(); }, LOG_POLL_MS);
		const onVisibility = () => {
			if (document.visibilityState === "visible") void fetch();
		};
		document.addEventListener("visibilitychange", onVisibility);
		return () => {
			clearInterval(interval);
			document.removeEventListener("visibilitychange", onVisibility);
		};
	}

	// The rendered rows. One filter pass over ≤LOG_TAIL_LINES strings per change
	// — no memo cache, so nothing to keep in a WeakMap and nothing the template
	// can write back into.
	const lines = $derived.by(() => {
		const byFeature = filterLogLines(tail?.lines ?? [], feature);
		const query = needle.trim().toLowerCase();
		if (!query) return byFeature;
		return byFeature.filter((line) => line.toLowerCase().includes(query));
	});

	return {
		get file() { return file; },
		set file(next: AppLogFile) {
			if (next === file) return;
			file = next;
			// Drop the old file's lines immediately rather than showing them under
			// the new file's name until the next tick lands.
			tail = null;
			error = null;
			void fetch();
		},

		get feature() { return feature; },
		set feature(next: DebugFeature | null) { feature = next; },

		get needle() { return needle; },
		set needle(next: string) { needle = next; },

		get follow() { return follow; },
		set follow(next: boolean) { follow = next; },

		get tail() { return tail; },
		get error() { return error; },
		/** Filtered lines, oldest first — what the viewer renders. */
		get lines() { return lines; },
		/** Unfiltered count, so the header can say "42 of 500". */
		get totalLines() { return tail?.lines.length ?? 0; },

		fetch,
		startPolling,
	};
}

export type LogTailStore = ReturnType<typeof createLogTailStore>;
