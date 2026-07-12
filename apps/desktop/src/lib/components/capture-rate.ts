// Human ladder for the screen capture rate. The wire format stays fps (f64,
// `screenFrameRate`); the UI speaks in "one snapshot every X". Backend bounds
// are 1/60..=10 fps (native_capture_settings.rs MIN/MAX_SCREEN_FRAME_RATE) —
// the ladder endpoints must stay inside them.
export const CAPTURE_INTERVAL_LADDER_S = [
	0.1, 0.5, 1, 2, 3, 5, 10, 15, 30, 45, 60,
] as const;

// 2s between snapshots = the 0.5 fps default in default_recording_settings().
export const DEFAULT_CAPTURE_INTERVAL_S = 2;

export function fpsToIntervalS(fps: number): number {
	return fps > 0 ? 1 / fps : DEFAULT_CAPTURE_INTERVAL_S;
}

export function intervalSToFps(intervalS: number): number {
	return 1 / intervalS;
}

// Nearest ladder stop by log distance (the ladder is log-spaced), so any
// persisted fps — including legacy values like 7.5 that are no longer on the
// ladder — maps to a sensible slider position without mutating the setting.
export function nearestLadderIndex(fps: number): number {
	const target = Math.log(fpsToIntervalS(fps));
	let best = 0;
	let bestDistance = Number.POSITIVE_INFINITY;
	CAPTURE_INTERVAL_LADDER_S.forEach((intervalS, i) => {
		const distance = Math.abs(Math.log(intervalS) - target);
		if (distance < bestDistance) {
			bestDistance = distance;
			best = i;
		}
	});
	return best;
}

// Readout phrase for the control header: "every 2 seconds", "once per minute".
export function captureIntervalPhrase(intervalS: number): string {
	if (intervalS < 1) return `${Math.round(1 / intervalS)} times a second`;
	if (intervalS === 1) return "every second";
	if (intervalS === 60) return "once per minute";
	return `every ${intervalS} seconds`;
}

// Short form for summary lines that previously said "0.5 fps".
export function captureRateShortLabel(fps: number): string {
	const intervalS = CAPTURE_INTERVAL_LADDER_S[nearestLadderIndex(fps)]!;
	if (intervalS < 1) return `${Math.round(1 / intervalS)} snapshots/sec`;
	if (intervalS === 1) return "1 snapshot/sec";
	if (intervalS === 60) return "1 snapshot/min";
	return `1 snapshot every ${intervalS}s`;
}

// Storage (and encode CPU) scale linearly with fps for preset bitrates —
// compute_effective_screen_bitrate_bps multiplies by the frame rate.
export function relativeStorageLabel(intervalS: number): string {
	const ratio = DEFAULT_CAPTURE_INTERVAL_S / intervalS;
	if (ratio >= 1) return `${Math.round(ratio * 10) / 10}×`;
	return `1/${Math.round(1 / ratio)}×`;
}

// Timeline app-run split threshold. Frames land one capture interval apart, so
// the gap must comfortably exceed one interval or a single skipped frame splits
// a run. Derived from the live rate; the 10s floor preserves the old behaviour
// for fast rates and for history captured before a rate change. Unset/zero fps
// falls back to the 2s default interval.
export function timelineGapMs(fps: number | undefined): number {
	const intervalMs = fps && fps > 0 ? 1000 / fps : DEFAULT_CAPTURE_INTERVAL_S * 1000;
	return Math.max(10_000, 3 * intervalMs);
}
