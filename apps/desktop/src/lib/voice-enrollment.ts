// Voice enrollment — the pure half: the sentence the user reads, the clip
// length, the wire shape of the backend's verdict, and the words each verdict
// and each recognition state get.
//
// No Svelte, no `invoke` — unit-testable, and shared by both enrollment doors
// (Settings → Intelligence → Speakers, and the onboarding Voice screen).
//
// ALL enrollment judgment lives in the Rust embedder
// (`speaker_analysis::embed_enrollment_clip`). Nothing here re-judges a take;
// these functions only put the backend's verdict into words.

import type { PersonProfileDto } from "$lib/types/app-infra";

/** Clip length asked of `record_bounded_microphone_clip`. */
export const ENROLLMENT_CLIP_MS = 15_000;

/**
 * The supplied sentence. Users should not have to improvise, and an
 * improvised take is usually too short — this reads out in roughly the clip
 * length at a normal pace.
 */
export const ENROLLMENT_SENTENCE =
  "I keep my work on this machine, and I would like Mnema to know my voice. " +
  "I work on a few things at once, and I would like the transcript to say who said what. " +
  "Reading this out loud takes about fifteen seconds.";

/** Mirrors `VoiceEnrollmentOutcomeDto` in `src-tauri/src/voice_enrollment.rs`. */
export type VoiceEnrollmentOutcome =
  | { status: "enrolled"; profile: PersonProfileDto }
  | { status: "tooShort"; durationMs: number }
  | { status: "noSpeech" }
  | { status: "multipleSpeakers"; speakerCount: number };

/** The three rejections, i.e. every outcome that is not `enrolled`. */
export type VoiceEnrollmentRejection = Exclude<VoiceEnrollmentOutcome, { status: "enrolled" }>;

/** The backend's verdict, in words, with what to do about it. */
export function rejectionMessage(rejection: VoiceEnrollmentRejection): string {
  switch (rejection.status) {
    case "tooShort": {
      const seconds = Math.max(1, Math.round(rejection.durationMs / 1000));
      return `That take had only ${seconds} second${seconds === 1 ? "" : "s"} of speech in it. Read the whole sentence out loud, then try again.`;
    }
    case "noSpeech":
      return "Mnema heard no speech in that take. Check the right microphone is selected and that you are not muted, then try again.";
    case "multipleSpeakers":
      return `Mnema heard ${rejection.speakerCount} voices in that take. It can only learn one, so record somewhere quieter — or ask the other person for fifteen seconds of silence.`;
  }
}

export interface RecognitionReadout {
  /** Whether an account-owner voiceprint exists. */
  enrolled: boolean;
  displayName: string | null;
  separateSpeakers: boolean;
  recognizeSavedPeople: boolean;
  autoLabelOwner: boolean;
}

/**
 * One plain-language sentence covering both questions the enrollment surface
 * has to answer: is there a voiceprint, and is recognition actually on. The
 * order of the checks is the order the user would hit them — a voiceprint is
 * useless with separation off, and recognition is useless with saved-people
 * matching off.
 */
export function recognitionReadout(state: RecognitionReadout): string {
  if (!state.enrolled) {
    return "No voiceprint saved. Your turns stay labelled “Speaker 1” until you record one.";
  }
  const who = state.displayName?.trim() || "you";
  if (!state.separateSpeakers) {
    return `Voiceprint saved for ${who}. Speaker separation is off, so nothing is being labelled.`;
  }
  if (!state.recognizeSavedPeople) {
    return `Voiceprint saved for ${who}. Recognising saved people is off, so the voiceprint is not being used.`;
  }
  return state.autoLabelOwner
    ? `Voiceprint saved for ${who}. Your turns are labelled with your name on their own when the match is confident.`
    : `Voiceprint saved for ${who}. Mnema suggests your name and waits for you to confirm it.`;
}
