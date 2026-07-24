// Suggestion-chip fill for the Today front page (Warm Paper redesign, Slice 3).
// Chips are MECHANICAL templates filled from data the page already loaded —
// no LLM call renders the empty state (plan decision). Pure functions only
// (no Svelte, no invoke) so this is unit-testable under `bun test`.

/** One composer suggestion chip. */
export interface SuggestionChip {
	/** Typographic glyph (condition-glyph vocabulary — never emoji): ◉ meeting, ▣ app. */
	glyph: string;
	/** The question the chip prefills into the composer. */
	text: string;
}

export interface ChipFillInput {
	/** The day's activities (any order). Only meetings-category rows are read. */
	activities: {
		title: string;
		category?: string | null;
		startedAtMs: number;
		endedAtMs: number;
	}[];
	/** Usage per app over the day (any order); highest activeMs wins. */
	timePerApp: { app: string; activeMs: number }[];
	nowMs: number;
}

/** Chip text stays a one-line pill; clamp pathological titles. */
export const CHIP_TITLE_MAX = 48;

function clamp(text: string): string {
	const trimmed = text.trim();
	return trimmed.length > CHIP_TITLE_MAX ? `${trimmed.slice(0, CHIP_TITLE_MAX - 1).trimEnd()}…` : trimmed;
}

/**
 * v1 templates (2 chips max, each skipped when its data is missing):
 *   ◉ "Catch me up on <last meeting title>" — most recently started
 *     meetings-category activity that began at or before now.
 *   ▣ "What was I doing in <top app>?" — the app with the most active time.
 */
export function fillChips(input: ChipFillInput): SuggestionChip[] {
	const chips: SuggestionChip[] = [];

	let meeting: ChipFillInput["activities"][number] | null = null;
	for (const a of input.activities) {
		if (a.category !== "meetings") continue;
		if (a.startedAtMs > input.nowMs) continue;
		if (a.title.trim().length === 0) continue;
		if (meeting === null || a.startedAtMs > meeting.startedAtMs) meeting = a;
	}
	if (meeting !== null) {
		chips.push({ glyph: "◉", text: `Catch me up on ${clamp(meeting.title)}` });
	}

	let topApp: ChipFillInput["timePerApp"][number] | null = null;
	for (const t of input.timePerApp) {
		const name = t.app.trim();
		if (t.activeMs <= 0 || name.length === 0 || name === "Unknown") continue;
		if (topApp === null || t.activeMs > topApp.activeMs) topApp = t;
	}
	if (topApp !== null) {
		chips.push({ glyph: "▣", text: `What was I doing in ${clamp(topApp.app)}?` });
	}

	return chips;
}
