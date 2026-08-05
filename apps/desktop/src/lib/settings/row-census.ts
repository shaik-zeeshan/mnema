// Where you are in the whole of Settings — the number the sticky headers carry.
//
// Direction 02 deletes the settings rail: the address is a sticky section header
// that names the group AND its position in the total ("9 – 21 of 48"). That
// position has to come from a real enumeration of the rows, not a hand-kept
// count, so it is derived from the same `SETTINGS_ROW_INDEX` the ⌘F filter and
// its completeness test already own (G7).
//
// While filtering, the same walk answers the other half: how many rows in each
// group survive the query, so a group with no hits can drop its header instead
// of leaving a lone "Capture" floating over nothing.
//
// Pure — no runes, no DOM. Tested by `row-census.test.ts`.

import { SETTINGS_GROUPS, type SettingsGroupId } from "./groups";
import { SETTINGS_ROW_INDEX, rowMatchesQuery } from "./settings-index";

export interface GroupCensus {
	id: SettingsGroupId;
	label: string;
	/** The group's section labels, joined — the header's subtitle. */
	sections: string;
	/** 1-based index of this group's first row in the full ordering. */
	first: number;
	/** 1-based index of its last row. */
	last: number;
	/** Rows in this group that match the active query (= `count` with no query). */
	matches: number;
}

export interface SettingsCensus {
	groups: GroupCensus[];
	/** Every indexed row. */
	total: number;
	/** Rows matching the query, or `total` when there is none. */
	matched: number;
	/** Groups with at least one match. */
	matchedGroups: number;
}

/**
 * Walk the groups in render order, counting indexed rows per section.
 *
 * `query` empty/whitespace ⇒ no filtering: every group's `matches` is its full
 * row count. Otherwise `matches` counts only rows `rowMatchesQuery` accepts —
 * the exact predicate `SettingRow` hides itself with, so the header count and
 * the visible rows can never disagree.
 */
export function settingsCensus(query = ""): SettingsCensus {
	const filtering = query.trim() !== "";
	const groups: GroupCensus[] = [];
	let cursor = 0;
	let matched = 0;

	for (const group of SETTINGS_GROUPS) {
		const first = cursor + 1;
		let count = 0;
		let matches = 0;
		for (const section of group.sections) {
			for (const entry of SETTINGS_ROW_INDEX) {
				if (entry.section !== section.id) continue;
				count += 1;
				if (!filtering || rowMatchesQuery(entry.section, entry.label, query)) {
					matches += 1;
				}
			}
		}
		cursor += count;
		matched += matches;
		groups.push({
			id: group.id,
			label: group.label,
			sections: group.sections.map((s) => s.label).join(" · "),
			first,
			last: cursor,
			matches,
		});
	}

	return {
		groups,
		total: cursor,
		matched: filtering ? matched : cursor,
		matchedGroups: groups.filter((g) => g.matches > 0).length,
	};
}
