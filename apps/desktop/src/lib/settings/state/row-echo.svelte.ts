/**
 * Row-level "Saved ✓" echo (DECISIONS.md G7).
 *
 * Autosave's chip lives in the top strip and tells you WHETHER a save landed;
 * the echo tells you WHICH row it was. Rows don't know their autosave domain
 * (`SettingRow` is used ~100 times and takes no domain prop), so attribution is
 * by interaction instead of by domain: the row whose control the user last
 * touched is the row that echoes when the next save succeeds.
 *
 * Only `savedRowId` is reactive — `editedRowId`/`editedAtMs` are plain module
 * vars so `noteSaved()` (called from an `$effect` in the chip) never reads
 * reactive state it also writes (the trap `toast.svelte.ts` documents).
 */

/** How long the echo stays up. G7 says ~1.5 s. */
const ECHO_MS = 1500;
/**
 * How long after an interaction a successful save still counts as "that row's".
 * ponytail: a fixed window, not a per-domain correlation — a save that takes
 * longer than this (a retention confirm dialog the user sits on) simply doesn't
 * echo, which is a miss rather than a lie. Thread the domain through SettingRow
 * if that ever matters.
 */
const ATTRIBUTION_WINDOW_MS = 5000;

let nextRowId = 0;
let editedRowId: number | null = null;
let editedAtMs = 0;
let echoTimer: ReturnType<typeof setTimeout> | null = null;
let savedRowId = $state<number | null>(null);

/** One stable id per mounted row. */
export function claimRowId(): number {
	return ++nextRowId;
}

/** A control inside this row was touched (input / change / click). */
export function noteRowEdit(rowId: number): void {
	editedRowId = rowId;
	editedAtMs = Date.now();
}

/** A settings save just succeeded — echo on the row that was last touched. */
export function noteSaved(): void {
	if (editedRowId === null || Date.now() - editedAtMs > ATTRIBUTION_WINDOW_MS) return;
	if (echoTimer !== null) clearTimeout(echoTimer);
	savedRowId = editedRowId;
	editedRowId = null;
	echoTimer = setTimeout(() => {
		savedRowId = null;
		echoTimer = null;
	}, ECHO_MS);
}

/** Is this row currently echoing? (Reactive read — call it from a `$derived`.) */
export function isRowEchoing(rowId: number): boolean {
	return savedRowId === rowId;
}
