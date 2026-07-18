import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type {
	ActivateLicenseResult,
	LicenseDevices,
	LicenseStatus,
	ResetDevicesOutcome,
} from "$lib/licensing";

// App-wide license/trial status: a snapshot from the deferred-startup gate,
// kept live by the `license_status` event. Slices 6/7/8 render off this; there
// is deliberately no UI here.
const LICENSE_STATUS_EVENT = "license_status";

let initialized = false;
let status = $state<LicenseStatus | null>(null);
// The `license_status` event is the live channel and is always fresher than the
// boot snapshot; once any event has landed, the late-arriving startup snapshot
// must never regress the store back to a stale value.
let gotEvent = false;
// Bumped on every live `license_status` emit — lets one-shot UIs (the license
// deep-link receipt modal) react to "a result landed" even when the payload
// equals the previous status (e.g. re-activating the already-installed key).
let revision = $state(0);

export const licenseStatus = {
	get value(): LicenseStatus | null {
		return status;
	},
	get revision(): number {
		return revision;
	},
};

export function initLicenseStatus(): void {
	if (initialized || typeof window === "undefined") return;
	initialized = true;

	void invoke<LicenseStatus>("get_license_status")
		.then((next) => {
			// A live event may have raced ahead of this boot snapshot — don't
			// clobber it with the older value.
			if (!gotEvent) status = next;
		})
		.catch(() => {
			// Leave `null` — the gate event will backfill once it runs.
		});

	void listen<LicenseStatus>(LICENSE_STATUS_EVENT, (event) => {
		gotEvent = true;
		status = event.payload;
		revision += 1;
	});
}

export async function activateLicense(key: string): Promise<ActivateLicenseResult> {
	const result = await invoke<ActivateLicenseResult>("activate_license", { key });
	status = result.status;
	return result;
}

/** Manual Receipt Refresh: forces a re-activation so a renewal (or freed-up
 * device) lands now. The recomputed status arrives via the `license_status`
 * event; resolving only means the check ran. */
export async function refreshLicenseNow(): Promise<void> {
	await invoke("refresh_license_now");
}

/** "Free up my devices" (over-cap self-service). On success the backend already
 * retried activation and republished the status via the `license_status` event;
 * a rejection carries a human-readable message. */
export async function resetLicenseDevices(): Promise<ResetDevicesOutcome> {
	return invoke<ResetDevicesOutcome>("reset_license_devices");
}

/** Device COUNT from the server (never a list). `null` when there's no key or
 * the server is unreachable — callers render nothing rather than stale numbers. */
export async function getLicenseDevices(): Promise<LicenseDevices | null> {
	return invoke<LicenseDevices | null>("get_license_devices").catch(() => null);
}
