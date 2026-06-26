import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { humanizeError } from "$lib/format-error";

export type AppNotificationAction = {
	type: "open_settings_tab";
	tab: "about" | "processing" | "transcription" | "speakers" | "shortcuts";
};

export interface AppNotification {
	id: string;
	severity: "info" | "warning" | "error";
	title: string;
	message: string;
	createdAtUnixMs: number;
	action?: AppNotificationAction | null;
}

const APP_NOTIFICATIONS_CHANGED_EVENT = "app_notifications_changed";

let initialized = false;
let notifications = $state<AppNotification[]>([]);
// Distinguishes "the initial fetch failed" from "there are no notifications":
// without it a load failure collapses to the silent no-bell state. Cleared
// whenever a fresh list arrives (reload or the backend change event).
let loadError = $state<string | null>(null);
// Transient feedback for a failed clear / action so the user perceives that
// the dismissal did not stick. Cleared on the next successful mutation.
let actionError = $state<string | null>(null);

function serializeError(err: unknown): string {
	return humanizeError(err);
}

export const appNotifications = {
	get items(): AppNotification[] {
		return notifications;
	},
	get count(): number {
		return notifications.length;
	},
	get loadError(): string | null {
		return loadError;
	},
	get actionError(): string | null {
		return actionError;
	},
};

export function initAppNotifications(): void {
	if (initialized || typeof window === "undefined") return;
	initialized = true;

	void reloadAppNotifications();

	void listen<AppNotification[]>(APP_NOTIFICATIONS_CHANGED_EVENT, (event) => {
		notifications = event.payload;
		loadError = null;
	});
}

export async function reloadAppNotifications(): Promise<void> {
	try {
		notifications = await invoke<AppNotification[]>("get_app_notifications");
		loadError = null;
	} catch (err) {
		// Keep any list we already have and retain a recoverable error state so
		// the bell can offer a retry rather than vanishing.
		loadError = serializeError(err);
	}
}

export function dismissAppNotificationError(): void {
	actionError = null;
}

/**
 * Surface a transient action error from a caller (e.g. a failed navigation
 * triggered by a notification action) through the same channel as clear
 * failures, so the popover renders one consistent error row.
 */
export function noteAppNotificationError(message: string): void {
	actionError = message;
}

/**
 * Clear a single notification. Returns `true` when the backend confirmed the
 * removal; on failure the item is kept and a brief `actionError` is surfaced so
 * callers can decide whether to keep the popover open.
 */
export async function clearAppNotification(id: string): Promise<boolean> {
	try {
		notifications = await invoke<AppNotification[]>("clear_app_notification", { id });
		actionError = null;
		return true;
	} catch (err) {
		actionError = serializeError(err);
		return false;
	}
}

export async function clearAppNotifications(): Promise<boolean> {
	try {
		notifications = await invoke<AppNotification[]>("clear_app_notifications");
		actionError = null;
		return true;
	} catch (err) {
		actionError = serializeError(err);
		return false;
	}
}
