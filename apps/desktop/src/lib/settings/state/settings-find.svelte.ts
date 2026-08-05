/**
 * ⌘F row filtering state (DECISIONS.md G7).
 *
 * The filter is nav-agnostic on purpose: settings navigation shape is a
 * per-direction (phase 2) decision, so ⌘F is a state OVER the content pane —
 * the shell renders every group's panel and each row hides itself unless it
 * matches. Hits therefore render in place, with their real, live control:
 * editing a row straight out of the hit list is the same control, so the
 * autosave chip + row echo work unchanged.
 *
 * Section scope: a row's own label is not enough to identify it ("Model" exists
 * in four sections), so each panel section component declares its section once
 * with `setSettingsSection(...)`. `SettingRow` reads it from context — and the
 * index-completeness test reads the same call out of the source file.
 */

import { getContext, setContext } from "svelte";
import type { SettingsSectionId } from "../groups";

const SECTION_KEY = Symbol("settings-section");

/** Declare the section every `SettingRow` below this component belongs to. */
export function setSettingsSection(section: SettingsSectionId): void {
	setContext(SECTION_KEY, section);
}

/** The enclosing section, or null outside a settings panel (e.g. the Debug page). */
export function getSettingsSection(): SettingsSectionId | null {
	return getContext<SettingsSectionId | undefined>(SECTION_KEY) ?? null;
}

// The filter state. A `$state` OBJECT (not a class with `$state` fields): the
// class shape did not notify readers in other components when `query` changed,
// so ⌘F filtered nothing outside the field itself. A `$state` object literal is
// a deep proxy, so every property read anywhere subscribes and every write
// anywhere notifies.
const state = $state({ open: false, query: "" });

export const settingsFind = {
	/** Is the filter field open? (⌘F opens + focuses it, Escape closes it.) */
	get open(): boolean {
		return state.open;
	},
	set open(v: boolean) {
		state.open = v;
	},
	/** The raw query text. */
	get query(): string {
		return state.query;
	},
	set query(v: string) {
		state.query = v;
	},
	/** Filtering is only ON with the field open AND a non-empty query. */
	get active(): boolean {
		return state.open && state.query.trim() !== "";
	},
	close(): void {
		state.open = false;
		state.query = "";
	},
};
