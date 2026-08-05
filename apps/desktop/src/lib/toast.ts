// Toast core (design frame 14): pure stack/archive logic, kept free of Svelte
// runes so `bun test` can exercise it directly. The reactive shell lives in
// `toast.svelte.ts`; the markup in `components/Toast.svelte`.

export type ToastKind = "success" | "info" | "danger";

export interface ToastAction {
	label: string;
	run: () => void;
}

export interface Toast {
	id: number;
	kind: ToastKind;
	message: string;
	detail: string | null;
	action: ToastAction | null;
	createdAtUnixMs: number;
	/** Coalesced consecutive repeats of the same kind+message (see coalesceNewest). */
	count: number;
}

export interface ToastPushOptions {
	detail?: string | null;
	action?: ToastAction | null;
}

/** Max toasts rendered in the stack; the rest collapse to "+N more in the bell". */
export const TOAST_VISIBLE_MAX = 3;
export const TOAST_AUTO_DISMISS_MS = 6000;

/**
 * Errors never auto-dismiss (frame 14 rule); info/success acks clear after 6s.
 * Returns null for sticky kinds.
 */
export function autoDismissDelayMs(kind: ToastKind): number | null {
	return kind === "danger" ? null : TOAST_AUTO_DISMISS_MS;
}

/** Newest toasts win the three visible slots; newest renders nearest the corner. */
export function visibleToasts(stack: Toast[]): Toast[] {
	return stack.slice(-TOAST_VISIBLE_MAX);
}

export function overflowCount(stack: Toast[]): number {
	return Math.max(0, stack.length - TOAST_VISIBLE_MAX);
}

/**
 * Coalesce-consecutive-identical: when the newest entry matches kind+message,
 * bump it in place (repeat count, freshest detail + timestamp) instead of
 * stacking a duplicate — a re-failing job repeats one toast, not thirty.
 * Returns the bumped entry, or null when nothing coalesced.
 */
export function coalesceNewest(
	list: Toast[],
	kind: ToastKind,
	message: string,
	detail: string | null,
	nowMs: number,
): Toast | null {
	const newest = list[list.length - 1];
	if (!newest || newest.kind !== kind || newest.message !== message) return null;
	newest.count += 1;
	newest.detail = detail;
	newest.createdAtUnixMs = nowMs;
	return newest;
}

/**
 * Push into the live stack + the bell archive (every toast is archived; the
 * archive entry is an independent copy so dismissing the toast keeps the
 * record). Returns the live stack entry (new or coalesced).
 */
export function pushToastCore(
	stack: Toast[],
	archive: Toast[],
	nextId: () => number,
	kind: ToastKind,
	message: string,
	options: ToastPushOptions,
	nowMs: number,
): Toast {
	const detail = options.detail ?? null;

	if (!coalesceNewest(archive, kind, message, detail, nowMs)) {
		archive.push({
			id: nextId(),
			kind,
			message,
			detail,
			action: null,
			createdAtUnixMs: nowMs,
			count: 1,
		});
	}

	const coalesced = coalesceNewest(stack, kind, message, detail, nowMs);
	if (coalesced) return coalesced;

	const toast: Toast = {
		id: nextId(),
		kind,
		message,
		detail,
		action: options.action ?? null,
		createdAtUnixMs: nowMs,
		count: 1,
	};
	stack.push(toast);
	return toast;
}

/** Remove a toast from a list by id. Returns whether anything was removed. */
export function removeToastCore(list: Toast[], id: number): boolean {
	const index = list.findIndex((toast) => toast.id === id);
	if (index < 0) return false;
	list.splice(index, 1);
	return true;
}
