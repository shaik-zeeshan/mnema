// The health rollup behind the verdict line, the Overview rows and the mini
// view — the one thing that polls `get_debug_health`, in both window modes.
//
// The backend composes the whole rollup in one command so this costs one
// round-trip per tick.

import { invoke } from "@tauri-apps/api/core";
import { humanizeError } from "$lib/format-error";
import type { DebugFeature, DebugSeverity, FeatureHealthDto } from "$lib/types";
import type { PollFn } from "./controller.svelte";

const HEALTH_POLL_MS = 2000;

export function createHealthStore(poll: PollFn) {
	let entries = $state<FeatureHealthDto[]>([]);
	let error = $state<string | null>(null);
	let loaded = $state(false);
	let fetchedAtMs = $state<number | null>(null);

	async function fetch() {
		try {
			entries = await invoke<FeatureHealthDto[]>("get_debug_health");
			error = null;
			loaded = true;
			// Stamped on success only, so a failed poll leaves the last good time
			// standing rather than advancing to a tick that fetched nothing.
			fetchedAtMs = Date.now();
		} catch (err) {
			error = humanizeError(err);
		}
	}

	/** 2 s poll through `controller.poll`. Returns teardown. */
	function startPolling(): () => void {
		return poll(fetch, HEALTH_POLL_MS);
	}

	return {
		get entries() { return entries; },
		get error() { return error; },
		get loaded() { return loaded; },
		/** Wall-clock ms of the last *successful* rollup fetch; null until one lands. */
		get fetchedAtMs() { return fetchedAtMs; },

		/** A feature's severity, or `null` when it is missing from the rollup. */
		severityFor(feature: DebugFeature | null): DebugSeverity | null {
			if (feature == null) return null;
			return entries.find((e) => e.feature === feature)?.severity ?? null;
		},
		reasonFor(feature: DebugFeature | null): string | null {
			if (feature == null) return null;
			return entries.find((e) => e.feature === feature)?.reason ?? null;
		},
		fetch,
		startPolling,
	};
}

export type HealthStore = ReturnType<typeof createHealthStore>;
