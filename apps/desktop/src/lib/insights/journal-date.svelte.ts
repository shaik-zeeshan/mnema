// The Journal's viewed day. Page 08 puts the date control in the CHROME, not in
// the content, so the control (<JournalDateStepper/>, mounted by the layout via
// <JournalTitlebarControls/>) and the surface that loads the day (<DayTimeline/>)
// live in two different component trees. This module is the seam between them.
//
// Deliberately a module-level singleton: there is exactly one Journal
// destination in exactly one window, so a store is the whole machinery needed —
// no context, no props threading through the layout.

import { shiftAnchor, windowFor } from "$lib/insights/activity-helpers";

class JournalDate {
	/** Any instant inside the viewed day; the range is derived from it. */
	anchorMs = $state<number>(Date.now());

	/** Local-midnight bounds of the viewed day. */
	get range(): { startMs: number; endMs: number } {
		return windowFor(this.anchorMs, "day");
	}

	/** True while the viewed day is the current day — the "next" step is then off. */
	get atLatest(): boolean {
		return Date.now() < this.range.endMs;
	}

	get dayLabel(): string {
		return new Date(this.range.startMs).toLocaleDateString(undefined, {
			weekday: "short",
			month: "short",
			day: "numeric",
		});
	}

	step(dir: -1 | 1): void {
		if (dir === 1 && this.atLatest) return; // never step into the future
		this.anchorMs = shiftAnchor(this.anchorMs, "day", dir);
	}

	/** Jump to a picked calendar day. Local noon dodges DST-boundary midnights. */
	setDay(year: number, month1: number, day: number): void {
		this.anchorMs = new Date(year, month1 - 1, day, 12).getTime();
	}
}

export const journalDate = new JournalDate();
