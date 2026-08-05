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

/** Collapsed heights. Bar alone, and bar plus the detached answer piece. */
export const CURRENT_FRAME_BAR_HEIGHT = 96;
/** Extra height for the non-vision disclosure line, which sits under the bar. */
export const CURRENT_FRAME_DISCLOSURE_HEIGHT = 30;
export const CURRENT_FRAME_ANSWER_HEIGHT = 460;

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
