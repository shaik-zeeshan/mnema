// Reactive toast store (design frame 14): bottom-right stack of three +
// "+N more in the bell", errors sticky, info/success auto-dismiss, every push
// archived for the bell popover. All list logic is in `toast.ts` (pure,
// bun-testable); this file is only the $state + timer glue.

import {
	autoDismissDelayMs,
	overflowCount,
	pushToastCore,
	removeToastCore,
	visibleToasts,
	type Toast,
	type ToastKind,
	type ToastPushOptions,
} from "./toast";

let stack = $state<Toast[]>([]);
let archive = $state<Toast[]>([]);
let nextId = 1;
const timers = new Map<number, ReturnType<typeof setTimeout>>();

export const toastStore = {
	get visible(): Toast[] {
		return visibleToasts(stack);
	},
	get overflow(): number {
		return overflowCount(stack);
	},
	/** Bell archive, oldest-first (session-local; render newest-first in the popover). */
	get archive(): Toast[] {
		return archive;
	},
};

export function pushToast(
	kind: ToastKind,
	message: string,
	options: ToastPushOptions = {},
): number {
	const toast = pushToastCore(
		stack,
		archive,
		() => nextId++,
		kind,
		message,
		options,
		Date.now(),
	);
	// (Re)arm the auto-dismiss — a coalesced repeat restarts the 6s window.
	const existingTimer = timers.get(toast.id);
	if (existingTimer) clearTimeout(existingTimer);
	const delay = autoDismissDelayMs(toast.kind);
	if (delay !== null) {
		timers.set(
			toast.id,
			setTimeout(() => dismissToast(toast.id), delay),
		);
	}
	return toast.id;
}

/** Remove from the live stack only — the bell archive keeps the record. */
export function dismissToast(id: number): void {
	const timer = timers.get(id);
	if (timer) {
		clearTimeout(timer);
		timers.delete(id);
	}
	removeToastCore(stack, id);
}

export function removeArchivedToast(id: number): void {
	removeToastCore(archive, id);
}

export function clearToastArchive(): void {
	archive.splice(0, archive.length);
}

// Dev-only render/test hook (stripped from production builds): lets a
// playwright run drive the stack without a failing backend.
if (import.meta.env.DEV && typeof window !== "undefined") {
	(window as unknown as Record<string, unknown>).__mnemaPushToast = pushToast;
}
