// Current-frame ask: the frontend half of "what's on screen right now"
// (round-4 decisions G1–G3).
//
// Hand-mirrored from `crates/capture-types/src/current_frame.rs` — no codegen,
// so this file and that one move together (the Rust serde round-trip test plus
// `bun run check` are the only guards).
//
// The frontend decides NOTHING here. It invokes the capture, renders the chip
// from what came back, and hands the same record straight back to
// `ask_ai_start` / `ask_ai_followup`; the backend re-resolves pixels-vs-OCR-text
// at send time.

import { invoke } from "@tauri-apps/api/core";

export type CurrentFrameCapture = {
  imagePath: string;
  capturedAtUnixMs: number;
  appName: string | null;
  windowTitle: string | null;
  /** Privacy-listed apps that were on screen and got blanked. Named, never hidden. */
  excludedAppNames: string[];
  /** Whether this turn's model can read images. Drives the upfront disclosure. */
  visionSupported: boolean;
  modelLabel: string;
};

/** Take the live screenshot. Throws with a humane reason when Ask AI is unset. */
export function captureCurrentFrame(): Promise<CurrentFrameCapture> {
  return invoke<CurrentFrameCapture>("capture_current_frame");
}

/**
 * Resize the Quick Access window itself — the same window collapses to the bar
 * (G3), never a second one. `null` restores the full launcher.
 */
export function setQuickRecallCollapsed(height: number | null): Promise<void> {
  return invoke<void>("quick_recall_set_collapsed", { height });
}

/**
 * Collapsed heights. Bar alone, and bar plus the detached answer piece.
 *
 * Direction 05's bar is three stacked pieces — the control pill ("Seeing your
 * screen"), the composer holding the frame chip in its sentence, and the
 * freshness readout — so it is taller than the plain phase-1 bar was.
 */
export const CURRENT_FRAME_BAR_HEIGHT = 136;
/** Extra height for the non-vision disclosure line, which sits under the bar. */
export const CURRENT_FRAME_DISCLOSURE_HEIGHT = 30;
export const CURRENT_FRAME_ANSWER_HEIGHT = 500;

/**
 * When the grab stops being trustworthy. Past this the control pill flips to
 * its warn face and offers a re-grab — the screen has almost certainly moved on.
 */
export const FRAME_STALE_MS = 45_000;

/**
 * The freshness readout's age, phrased at the precision the clock can support.
 * This is a MEASURED elapsed time, not an ETA, so tenths are honest under ten
 * seconds; past a minute nobody cares about the seconds (G8: round coarsely).
 */
export function frameAgePhrase(ageMs: number): string {
	const ms = Math.max(0, ageMs);
	if (ms < 10_000) return `${(Math.round(ms / 100) / 10).toFixed(1)} s`;
	if (ms < 60_000) return `${Math.round(ms / 1000)} s`;
	const minutes = Math.round(ms / 60_000);
	return `${minutes} min`;
}

/** The chip's primary label: what the shot is of. */
export function frameChipLabel(frame: CurrentFrameCapture): string {
  const app = frame.appName?.trim();
  return app && app.length > 0 ? app : "This screen";
}

/**
 * The blanked-apps naming the chip carries. Never a refusal and never silent —
 * an excluded frontmost app still produced a shot, it is just named as missing.
 */
export function frameExclusionNote(frame: CurrentFrameCapture): string | null {
  const names = frame.excludedAppNames.filter((name) => name.trim().length > 0);
  if (names.length === 0) {
    return null;
  }
  if (names.length === 1) {
    return `${names[0]} excluded`;
  }
  if (names.length === 2) {
    return `${names[0]} and ${names[1]} excluded`;
  }
  return `${names.slice(0, -1).join(", ")} and ${names[names.length - 1]} excluded`;
}

/**
 * The upfront non-vision disclosure (G2). Present on the chip BEFORE the user
 * types — the feature stays available, it just sends text instead of pixels.
 */
export function frameVisionNote(frame: CurrentFrameCapture): string | null {
  if (frame.visionSupported) {
    return null;
  }
  const model = frame.modelLabel.trim();
  return `${model.length > 0 ? model : "This model"} can't see images — sending text from this screen`;
}
