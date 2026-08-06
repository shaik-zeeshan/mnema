/**
 * The Settings inspector's subject (direction 02).
 *
 * The Studio Shell carries one 256px inspector on every surface. On Settings it
 * holds the FOCUSED SETTING's detail rather than being navigation — the rail is
 * gone, so this panel is never a nav and must never grow one.
 *
 * What it holds is deliberately only what Mnema actually knows about a row:
 * its label, its section breadcrumb, its description, the search terms it
 * answers to, and — for the rows that have one — what it costs on this machine,
 * measured (never estimated) by `system-facts.ts`. There is no "previous value"
 * or "takes effect at" store behind Settings, so those mockup lines are not
 * invented here (G8's honest-numbers rule applied to prose). The one live
 * history is this session's saved rows, which the row echo already establishes.
 *
 * ponytail: a module singleton, like `settings-find`. Settings is one route
 * with one inspector; a registry would be machinery for a second one that does
 * not exist.
 */

import { untrack } from "svelte";
import type { SettingsSectionId } from "../groups";

export interface InspectedSetting {
	label: string;
	description: string | null;
	section: SettingsSectionId | null;
	/**
	 * What this row costs ON THIS MACHINE, already phrased by `system-facts.ts`
	 * — bytes/day, GB kept, backlog depth, index size. `null` for the rows that
	 * cost nothing measurable, which is most of them: G8 forbids inventing a
	 * denominator, so the inspector simply omits the section.
	 */
	cost: string | null;
}

export interface SettingsChange {
	label: string;
	/** Wall-clock ms — rendered as a local time, never as a duration. */
	atMs: number;
}

/** How many saved rows the "Recent changes" list keeps. */
const RECENT_LIMIT = 6;

class SettingsInspector {
	/** Is the panel shown? (⌥⌘I / the tool-strip toggle.) */
	open = $state(true);
	/** The row the user last focused or touched, or null at rest. */
	subject = $state<InspectedSetting | null>(null);
	/** Rows saved in this session, newest first. */
	recent = $state<SettingsChange[]>([]);

	focus(next: InspectedSetting): void {
		this.subject = next;
	}

	/**
	 * A row's save landed — record it, newest first, de-duplicated by label.
	 *
	 * `untrack` is load-bearing, not decoration. This is called from an `$effect`
	 * in `SettingRow` (the moment the row echo fires); reading `this.recent`
	 * inside that effect would subscribe the effect to the very list it then
	 * writes, and Svelte kills the runaway with `effect_update_depth_exceeded` —
	 * which silently disables OTHER effects in the same flush too. That is
	 * exactly how the save chip and the status strip ended up disagreeing.
	 */
	noteChange(label: string): void {
		const at = Date.now();
		untrack(() => {
			const rest = this.recent.filter((c) => c.label !== label);
			this.recent = [{ label, atMs: at }, ...rest].slice(0, RECENT_LIMIT);
		});
	}

	toggle(): void {
		this.open = !this.open;
	}
}

export const settingsInspector = new SettingsInspector();
