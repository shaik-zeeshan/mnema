import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export type AppNotificationAction =
	| {
			type: "open_settings_tab";
			tab: "about" | "processing" | "transcription" | "speakers" | "shortcuts";
	  }
	// Only 'microphone' is emitted by the backend and serviceable by the Windows
	// deep-link. Re-widen to screen/systemAudio when those privacy notifications
	// are actually emitted and handled.
	| { type: "open_capture_privacy_settings"; kind: "microphone" };

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

export const appNotifications = {
	get items(): AppNotification[] {
		return notifications;
	},
	get count(): number {
		return notifications.length;
	},
};

export function initAppNotifications(): void {
	if (initialized || typeof window === "undefined") return;
	initialized = true;

	void invoke<AppNotification[]>("get_app_notifications")
		.then((items) => {
			notifications = items;
		})
		.catch(() => {
			notifications = [];
		});

	void listen<AppNotification[]>(APP_NOTIFICATIONS_CHANGED_EVENT, (event) => {
		notifications = event.payload;
	});
}

export async function clearAppNotification(id: string): Promise<void> {
	notifications = await invoke<AppNotification[]>("clear_app_notification", { id });
}

export async function clearAppNotifications(): Promise<void> {
	notifications = await invoke<AppNotification[]>("clear_app_notifications");
}
