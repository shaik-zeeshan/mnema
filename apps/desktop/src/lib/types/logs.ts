export interface NativeCaptureDebugLogStatus {
	enabled: boolean;
	path: string;
	exists: boolean;
}

export interface GeneralAppLogStatus {
	path: string;
	exists: boolean;
	/** On-disk size in bytes; null when the file is missing. */
	sizeBytes: number | null;
}
