export type MicrophonePreferenceMode = "default" | "specific_device";
export type MicrophoneDisconnectPolicy = "fallback_to_default" | "wait_for_same_device";

export interface MicrophoneDevice {
	id: string;
	name: string;
	isDefault: boolean;
}

export interface MicrophonePreference {
	mode: MicrophonePreferenceMode;
	deviceId: string | null;
}

export interface MicrophoneControllerState {
	devices: MicrophoneDevice[];
	preference: MicrophonePreference;
	disconnectPolicy: MicrophoneDisconnectPolicy;
	effectiveDevice: MicrophoneDevice | null;
}

export interface MicrophoneAutoDisconnectTransitionFailedEvent {
	context: string;
	code: string;
	message: string;
}
