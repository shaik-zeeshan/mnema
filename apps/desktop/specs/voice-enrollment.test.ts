import { describe, expect, test } from "bun:test";
import {
  rejectionMessage,
  recognitionReadout,
  ENROLLMENT_CLIP_MS,
} from "../src/lib/voice-enrollment";

describe("enrollment rejections are rendered, never re-judged", () => {
  test("each typed rejection gets its own actionable words", () => {
    expect(rejectionMessage({ status: "tooShort", durationMs: 2400 })).toContain("2 seconds");
    expect(rejectionMessage({ status: "tooShort", durationMs: 900 })).toContain("1 second");
    expect(rejectionMessage({ status: "noSpeech" })).toContain("no speech");
    expect(rejectionMessage({ status: "multipleSpeakers", speakerCount: 3 })).toContain("3 voices");
  });

  test("the three messages are distinct — a retry has to know which to fix", () => {
    const messages = new Set([
      rejectionMessage({ status: "tooShort", durationMs: 1000 }),
      rejectionMessage({ status: "noSpeech" }),
      rejectionMessage({ status: "multipleSpeakers", speakerCount: 2 }),
    ]);
    expect(messages.size).toBe(3);
  });
});

describe("recognition read-out", () => {
  const enrolledOn = {
    enrolled: true,
    displayName: "Zeeshan",
    separateSpeakers: true,
    recognizeSavedPeople: true,
    autoLabelOwner: true,
  };

  test("no voiceprint says so, and says what happens instead", () => {
    const text = recognitionReadout({ ...enrolledOn, enrolled: false });
    expect(text).toContain("No voiceprint");
    expect(text).toContain("Speaker 1");
  });

  test("names the first thing that is actually off", () => {
    expect(recognitionReadout({ ...enrolledOn, separateSpeakers: false })).toContain(
      "Speaker separation is off",
    );
    expect(recognitionReadout({ ...enrolledOn, recognizeSavedPeople: false })).toContain(
      "Recognising saved people is off",
    );
  });

  test("auto-labelling on vs off reads differently", () => {
    expect(recognitionReadout(enrolledOn)).toContain("on their own");
    expect(recognitionReadout({ ...enrolledOn, autoLabelOwner: false })).toContain("waits for you");
  });

  test("a nameless profile still reads as a sentence", () => {
    expect(recognitionReadout({ ...enrolledOn, displayName: "  " })).toContain(
      "Voiceprint saved for you",
    );
  });
});

test("the clip is the 15 s the surface promises", () => {
  expect(ENROLLMENT_CLIP_MS).toBe(15_000);
});
