/**
 * The deck — direction 04 "Command Deck"'s one added piece of chrome.
 *
 * A 28px bar pinned to the bottom of the window FRAME (inside `.app-shell`,
 * outside `<main>`), carrying route context on the left and live shortcut
 * hints on the right. Because it is inside the frame and never scrolls, no
 * state it carries can clip off-screen at any window size — which is why the
 * settings autosave state (G7: no bottom save bar, ever) lives in its `status`
 * slot rather than in a save bar.
 *
 * Routes publish into it from an `$effect` and clear on unmount:
 *
 *   $effect(() => {
 *     setDeck({ context: "Timeline · Mon, Aug 3", hints: [...] });
 *     return resetDeck;
 *   });
 */

/** One `⌘K Actions` pair on the right of the deck. */
export interface DeckHint {
	/** Rendered inside a `.kbd` keycap, e.g. "⌘⏎" or "⌃". */
	keys: string;
	/** Sentence-case verb phrase, e.g. "open tile". */
	label: string;
	/** Draw a hairline separator BEFORE this hint. */
	separator?: boolean;
}

export type DeckStatusTone = "quiet" | "ok" | "danger";

/** The deck's one stateful slot — settings autosave uses it; nothing else yet. */
export interface DeckStatus {
	tone: DeckStatusTone;
	text: string;
}

export interface DeckState {
	context: string;
	hints: DeckHint[];
	status: DeckStatus | null;
}

const EMPTY: DeckState = { context: "", hints: [], status: null };

export const deck = $state<DeckState>({ ...EMPTY });

/** Publish a route's deck content. Omitted keys are left untouched. */
export function setDeck(next: Partial<DeckState>): void {
	if (next.context !== undefined) deck.context = next.context;
	if (next.hints !== undefined) deck.hints = next.hints;
	if (next.status !== undefined) deck.status = next.status;
}

/** Clear everything — the cleanup half of a route's `$effect`. */
export function resetDeck(): void {
	deck.context = EMPTY.context;
	deck.hints = [];
	deck.status = null;
}
