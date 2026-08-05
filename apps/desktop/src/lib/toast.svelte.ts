/**
 * App-wide toasts — the ONE non-blocking placement for app-level/async events
 * (system.css §6). Bottom-right, at most three visible, never reflows content.
 *
 * Two rules the surfaces depend on:
 *  - `error` NEVER auto-dismisses (DECISIONS.md G7: the settings save-failure
 *    toast stays until the user acts on it). `info`/`success` fade on a timer.
 *  - Nothing is lost when it leaves the stack. A toast that is dismissed,
 *    expires, or is pushed out by a fourth arrival moves into the archive that
 *    the title-bar bell already renders, alongside backend notifications.
 *
 * Inline field validation stays inline (`.field__help`) — this is not that.
 */

import { untrack } from "svelte";

export type ToastTone = "info" | "success" | "error";

export interface ToastAction {
	label: string;
	run: () => void | Promise<void>;
}

export interface Toast {
	id: string;
	tone: ToastTone;
	title: string;
	message?: string;
	action?: ToastAction;
	/** Runs when the toast leaves the stack, however it leaves. */
	onDismiss?: () => void;
	createdAtUnixMs: number;
}

export interface ToastInput {
	tone?: ToastTone;
	title: string;
	message?: string;
	action?: ToastAction;
	/**
	 * Called when the toast is dismissed. For a toast that MIRRORS a piece of
	 * app state (a settings save failure), this is where that state gets
	 * cleared, so dismissing the toast can't leave an orphaned error behind.
	 */
	onDismiss?: () => void;
	/**
	 * Stable identity for a repeatable condition (a failing autosave, a failing
	 * poll). Raising the same id again REPLACES the live toast instead of
	 * stacking a duplicate; without it every retry would add a row.
	 */
	id?: string;
}

const MAX_VISIBLE = 3;
const ARCHIVE_MAX = 50;
const AUTO_DISMISS_MS = 6000;

let live = $state<Toast[]>([]);
let archived = $state<Toast[]>([]);
const timers = new Map<string, ReturnType<typeof setTimeout>>();

// Every mutation reads the current lists through `untrack`. Callers raise
// toasts from inside `$effect`s (a settings save failure mirrors state into a
// toast); without this, reading `live` there makes the effect depend on the
// state it is about to write and Svelte kills it with effect_update_depth_
// exceeded. Untracking here fixes it once for every caller.
const currentLive = (): Toast[] => untrack(() => live);
const currentArchived = (): Toast[] => untrack(() => archived);

export const toasts = {
	/** Newest last — the stack grows upward from the bottom-right corner. */
	get visible(): Toast[] {
		return live;
	},
	/** Newest first — dismissed, expired and overflowed toasts. */
	get archived(): Toast[] {
		return archived;
	},
	get archivedCount(): number {
		return archived.length;
	},
};

function clearTimer(id: string): void {
	const timer = timers.get(id);
	if (timer !== undefined) {
		clearTimeout(timer);
		timers.delete(id);
	}
}

function archive(toast: Toast): void {
	clearTimer(toast.id);
	// The archive row is a record, not a control — drop the closures rather than
	// retaining whatever they captured.
	archived = [
		{ ...toast, action: undefined, onDismiss: undefined },
		...currentArchived(),
	].slice(0, ARCHIVE_MAX);
}

/** Raise a toast. Returns its id so a caller can dismiss it early. */
export function toast(input: ToastInput): string {
	const tone = input.tone ?? "info";
	const id = input.id ?? `toast-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
	const next: Toast = {
		id,
		tone,
		title: input.title,
		message: input.message,
		action: input.action,
		onDismiss: input.onDismiss,
		createdAtUnixMs: Date.now(),
	};

	clearTimer(id);
	const rest = currentLive().filter((t) => t.id !== id);
	// A fourth arrival retires the oldest rather than growing the stack.
	const overflow = rest.slice(0, Math.max(0, rest.length + 1 - MAX_VISIBLE));
	for (const old of overflow) archive(old);
	live = [...rest.slice(overflow.length), next];

	if (tone !== "error") {
		timers.set(
			id,
			setTimeout(() => dismissToast(id), AUTO_DISMISS_MS),
		);
	}
	return id;
}

export function dismissToast(id: string): void {
	const current = currentLive();
	const hit = current.find((t) => t.id === id);
	if (!hit) return;
	live = current.filter((t) => t.id !== id);
	archive(hit);
	// Only an actual dismissal (user or timer) clears mirrored state — being
	// pushed out by a fourth toast must not mutate anything.
	hit.onDismiss?.();
}

export function clearArchivedToast(id: string): void {
	archived = currentArchived().filter((t) => t.id !== id);
}

export function clearToastArchive(): void {
	archived = [];
}
