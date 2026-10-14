// Log tail state for the Logs tab.
//
// `tail_app_log` is a poll-and-replace snapshot: there is no server-side
// filtering and no tail-follow push. So this store owns which file to read,
// the poll, and the client-side level / feature / text filters over the lines
// the backend hands back.
//
// Created inside the tab (not on the controller), so it lives and polls only
// while the tab is mounted.

import { invoke } from "@tauri-apps/api/core";
import { humanizeError } from "$lib/format-error";
import { LOG_FEATURE_PATTERNS, type LogFeature } from "../log-filter";
import { groupLogLines, levelMatches, type LogLevelFilter } from "../log-line";
import type { PollFn } from "./controller.svelte";
import type { AppLogFile, AppLogTailDto } from "$lib/types";

/**
 * Lines requested per poll. The backend clamps to 1..=2000; this is the whole
 * window the viewer has, so it is surfaced in the UI ("last N lines") so a
 * truncated view never reads as "that's all there is".
 */
export const LOG_TAIL_LINES = 500;

/** Logs are written at human pace; each tick re-reads the file Rust-side. */
const LOG_POLL_MS = 2000;

function sameTail(a: AppLogTailDto | null, b: AppLogTailDto): boolean {
	return a != null && a.path === b.path && a.exists === b.exists &&
		a.lines.length === b.lines.length && a.lines.every((line, i) => line === b.lines[i]);
}

export function createLogTailStore(poll: PollFn, init: { needle?: string } = {}) {
	let file = $state<AppLogFile>("rust");
	let level = $state<LogLevelFilter>("all");
	let feature = $state<LogFeature | null>(null);
	/** Free-text substring filter (case-insensitive), e.g. a job id. */
	let needle = $state(init.needle ?? "");
	/** Stick to the newest line on each poll. */
	let follow = $state(true);
	let tail = $state<AppLogTailDto | null>(null);
	let error = $state<string | null>(null);

	async function fetch() {
		const requested = file;
		try {
			const next = await invoke<AppLogTailDto>("tail_app_log", { file: requested, lines: LOG_TAIL_LINES });
			// A file switch landed while this was in flight; that request owns the view.
			if (requested !== file) return;
			// An unchanged poll must not re-derive and re-render every row.
			if (!sameTail(tail, next)) tail = next;
			error = null;
		} catch (err) {
			// A missing log is `exists: false`, not a throw.
			error = humanizeError(err);
		}
	}

	const grouped = $derived(groupLogLines(tail?.lines ?? []));
	const entries = $derived.by(() => {
		const pattern = feature ? LOG_FEATURE_PATTERNS[feature] : null;
		const query = needle.trim().toLowerCase();
		return grouped.filter((entry) =>
			levelMatches(entry.head?.level ?? null, level) &&
			(!pattern || pattern.test(entry.raw)) &&
			(!query || entry.raw.toLowerCase().includes(query)));
	});

	return {
		get file() { return file; },
		set file(next: AppLogFile) {
			if (next === file) return;
			file = next;
			// Drop the old file's lines rather than showing them under the new name.
			tail = null;
			error = null;
			void fetch();
		},
		get level() { return level; },
		set level(next: LogLevelFilter) { level = next; },
		get feature() { return feature; },
		set feature(next: LogFeature | null) { feature = next; },
		get needle() { return needle; },
		set needle(next: string) { needle = next; },
		get follow() { return follow; },
		set follow(next: boolean) { follow = next; },

		get tail() { return tail; },
		get error() { return error; },
		/** Filtered entries, oldest first. */
		get entries() { return entries; },
		/** Raw line count the backend returned. */
		get totalLines() { return tail?.lines.length ?? 0; },

		startPolling: () => poll(fetch, LOG_POLL_MS),
	};
}

export type LogTailStore = ReturnType<typeof createLogTailStore>;
