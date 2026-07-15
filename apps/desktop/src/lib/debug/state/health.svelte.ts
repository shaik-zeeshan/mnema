// Dock health dots — the only thing that polls `get_debug_health`.
//
// The backend composes the whole rollup in one command precisely so the dock
// costs one round-trip per tick (PLAN: "the dock polls only this").

import { invoke } from "@tauri-apps/api/core";
import { humanizeError } from "$lib/format-error";
import type { DebugFeature, DebugSeverity, FeatureHealthDto } from "$lib/types";

const HEALTH_POLL_MS = 1000;

export function createHealthStore() {
	let entries = $state<FeatureHealthDto[]>([]);
	let error = $state<string | null>(null);
	let loaded = $state(false);
	let fetchedAtMs = $state<number | null>(null);

	async function fetch() {
		// Stop polling when the page isn't visible — the dock is off-screen anyway.
		if (typeof document !== "undefined" && document.visibilityState !== "visible") return;
		try {
			entries = await invoke<FeatureHealthDto[]>("get_debug_health");
			error = null;
			loaded = true;
			// Stamped on success only: the Health card's hint reports how fresh the
			// rollup is, so a failed poll must leave the last good time standing
			// rather than advance to a tick that fetched nothing.
			fetchedAtMs = Date.now();
		} catch (err) {
			error = humanizeError(err);
		}
	}

	/** 1s poll while mounted. Returns teardown. */
	function startPolling(): () => void {
		void fetch();
		const interval = setInterval(() => { void fetch(); }, HEALTH_POLL_MS);
		const onVisibility = () => {
			if (document.visibilityState === "visible") void fetch();
		};
		document.addEventListener("visibilitychange", onVisibility);
		return () => {
			clearInterval(interval);
			document.removeEventListener("visibilitychange", onVisibility);
		};
	}

	return {
		get entries() { return entries; },
		get error() { return error; },
		get loaded() { return loaded; },
		/** Wall-clock ms of the last *successful* rollup fetch; null until one lands. */
		get fetchedAtMs() { return fetchedAtMs; },

		/**
		 * The dot state for a section. `null` feature (Health, Logs) and any
		 * feature missing from the rollup read as `idle` — there are 9 health
		 * features but 11 sections, so absence is normal, not an error.
		 */
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
