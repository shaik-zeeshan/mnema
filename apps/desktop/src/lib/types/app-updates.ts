export type AppUpdateChannel = "stable" | "preview";

export type AppUpdateState =
	| "idle"
	| "checking"
	| "upToDate"
	| "available"
	| "downloading"
	| "installing"
	| "restartRequired"
	| "recordingBlocked"
	| "incompatible"
	| "failed";

export type AppUpdateErrorKind =
	| "network"
	| "feed"
	| "incompatible"
	| "verification"
	| "install"
	| "recordingActive"
	| "unknown";

export interface AppUpdateStatus {
	app: {
		productName: string;
		version: string;
		identifier: string;
		platform: string;
		arch: string;
	};
	channel: AppUpdateChannel;
	state: AppUpdateState;
	update?: {
		version: string;
		date?: string | null;
		notes?: string | null;
		channel: AppUpdateChannel;
	} | null;
	progress?: {
		downloadedBytes: number;
		contentLengthBytes?: number | null;
	} | null;
	error?: {
		kind: AppUpdateErrorKind;
		message: string;
	} | null;
	lastCheckedAtUnixMs?: number | null;
	recordingActive: boolean;
}
