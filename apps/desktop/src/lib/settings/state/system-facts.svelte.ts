// The one reader of `get_system_facts` (round-4 decision **G8**).
//
// App-global, read-only, and the same for every row that quotes it, so it is a
// module singleton rather than another field on the settings controller —
// sibling in shape to `$lib/notifications.svelte.ts`.
//
// Failure is silent on purpose: `facts` stays `null`, every derived phrase in
// `./system-facts.ts` returns `null`, and the affected rows render no number.
// A missing denominator is the designed G8 outcome, not an error to report.

import { invoke } from "@tauri-apps/api/core";
import type { SystemFacts } from "$lib/types";

let facts = $state<SystemFacts | null>(null);
let inFlight: Promise<void> | null = null;

export const systemFacts = {
	get value(): SystemFacts | null {
		return facts;
	},
	/**
	 * Load once per app run. The figures move on the scale of a capture day (or
	 * a job queue), so several settings rows mounting at once must not each stat
	 * the capture volume — later callers await the first flight.
	 */
	ensureLoaded(): Promise<void> {
		if (facts !== null) return Promise.resolve();
		inFlight ??= invoke<SystemFacts>("get_system_facts")
			.then((next) => {
				facts = next;
			})
			.catch(() => {
				facts = null;
			})
			.finally(() => {
				inFlight = null;
			});
		return inFlight;
	},
	/** Re-read after something that moves a figure (a retention cleanup). */
	refresh(): Promise<void> {
		facts = null;
		inFlight = null;
		return this.ensureLoaded();
	},
};
