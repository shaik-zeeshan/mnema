import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { ActivateLicenseResult, LicenseStatus } from "$lib/licensing";

// App-wide license/trial status: a snapshot from the deferred-startup gate,
// kept live by the `license_status` event. Slices 6/7/8 render off this; there
// is deliberately no UI here.
const LICENSE_STATUS_EVENT = "license_status";

let initialized = false;
let status = $state<LicenseStatus | null>(null);

export const licenseStatus = {
	get value(): LicenseStatus | null {
		return status;
	},
};

export function initLicenseStatus(): void {
	if (initialized || typeof window === "undefined") return;
	initialized = true;

	void invoke<LicenseStatus>("get_license_status")
		.then((next) => {
			status = next;
		})
		.catch(() => {
			// Leave `null` — the gate event will backfill once it runs.
		});

	void listen<LicenseStatus>(LICENSE_STATUS_EVENT, (event) => {
		status = event.payload;
	});
}

export async function activateLicense(key: string): Promise<ActivateLicenseResult> {
	const result = await invoke<ActivateLicenseResult>("activate_license", { key });
	status = result.status;
	return result;
}
