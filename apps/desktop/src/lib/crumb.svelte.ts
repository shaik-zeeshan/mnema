/**
 * Title-bar breadcrumb — direction 04's "the title bar grows one breadcrumb
 * chip" (README, pages 08–10). Journal, Subjects and Context are destinations
 * opened from Overview; while one is showing, the titlebar renders a
 * `› Journal esc` chip after the surface switcher and `esc` returns.
 *
 * Same publish/reset contract as the deck: the destination route sets its
 * trail from an `$effect` and clears it on unmount. The layout only renders.
 */

export interface Crumb {
	label: string;
	/** Clicking a linked crumb navigates there (e.g. "Subjects" over a detail). */
	href?: string;
}

export const crumb = $state<{ trail: Crumb[] }>({ trail: [] });

export function setCrumbs(trail: Crumb[]): void {
	crumb.trail = trail;
}

export function resetCrumbs(): void {
	crumb.trail = [];
}
