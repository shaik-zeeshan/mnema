// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig, so skip static checking here.
import { describe, expect, it } from "bun:test";
import {
  applyToggle,
  featureLockReason,
  featureToggleDisabled,
  normalizeFeatures,
  systemAudioNeedsRequest,
} from "./feature-rules";

function state(overrides = {}) {
  const { permissions, ...rest } = overrides;
  return normalizeFeatures({
    permissions: {
      screen: true,
      microphone: true,
      systemAudio: true,
      ...(permissions ?? {}),
    },
    screen: true,
    microphone: false,
    systemAudio: false,
    ocr: true,
    transcription: false,
    speakerSeparation: false,
    semanticSearch: true,
    aiFeatures: false,
    privacy: false,
    transcribeMicrophone: false,
    transcribeSystemAudio: false,
    recognizeSavedPeople: false,
    ...rest,
  });
}

// ── Upward cascades (new) ──────────────────────────────────────────────────

describe("applyToggle — upward cascades", () => {
  it("enabling the microphone enables transcription and speaker separation", () => {
    const next = applyToggle(state(), "microphone");
    expect(next.microphone).toBe(true);
    expect(next.transcription).toBe(true);
    expect(next.transcribeMicrophone).toBe(true);
    expect(next.speakerSeparation).toBe(true);
  });

  it("enabling system audio enables transcription and speaker separation", () => {
    const next = applyToggle(state(), "systemAudio");
    expect(next.systemAudio).toBe(true);
    expect(next.transcription).toBe(true);
    expect(next.transcribeSystemAudio).toBe(true);
    expect(next.speakerSeparation).toBe(true);
  });

  it("enabling a second source transcribes it too", () => {
    const next = applyToggle(applyToggle(state(), "microphone"), "systemAudio");
    expect(next.transcribeMicrophone).toBe(true);
    expect(next.transcribeSystemAudio).toBe(true);
  });

  it("enabling transcription binds the sources already on", () => {
    const base = state({ microphone: true, systemAudio: true, transcription: false });
    expect(base.transcribeMicrophone).toBe(false);
    const next = applyToggle(base, "transcription");
    expect(next.transcription).toBe(true);
    expect(next.transcribeMicrophone).toBe(true);
    expect(next.transcribeSystemAudio).toBe(true);
  });

  it("transcription cannot be enabled with no audio source at all", () => {
    const next = applyToggle(state(), "transcription");
    expect(next.transcription).toBe(false);
    expect(next.speakerSeparation).toBe(false);
  });
});

// ── Downward cascades (existing — must not regress) ────────────────────────

describe("applyToggle — downward cascades", () => {
  it("disabling the LAST audio source disables transcription and speakers", () => {
    const on = applyToggle(state(), "microphone");
    const next = applyToggle(on, "microphone");
    expect(next.microphone).toBe(false);
    expect(next.transcription).toBe(false);
    expect(next.transcribeMicrophone).toBe(false);
    expect(next.speakerSeparation).toBe(false);
    expect(next.recognizeSavedPeople).toBe(false);
  });

  it("disabling one of two sources keeps transcription on for the other", () => {
    const both = applyToggle(applyToggle(state(), "microphone"), "systemAudio");
    const next = applyToggle(both, "microphone");
    expect(next.transcription).toBe(true);
    expect(next.transcribeMicrophone).toBe(false);
    expect(next.transcribeSystemAudio).toBe(true);
    expect(next.speakerSeparation).toBe(true);
  });

  it("disabling transcription clears the per-source flags and speaker separation", () => {
    const on = applyToggle(state(), "microphone");
    const next = applyToggle(on, "transcription");
    expect(next.transcription).toBe(false);
    expect(next.transcribeMicrophone).toBe(false);
    expect(next.transcribeSystemAudio).toBe(false);
    expect(next.speakerSeparation).toBe(false);
    expect(next.recognizeSavedPeople).toBe(false);
    // The capture source survives — audio stays worth keeping.
    expect(next.microphone).toBe(true);
  });

  it("disabling speaker separation clears saved-people recognition", () => {
    const on = applyToggle(state(), "microphone");
    const enrolled = normalizeFeatures({ ...on, recognizeSavedPeople: true });
    expect(enrolled.recognizeSavedPeople).toBe(true);
    const next = applyToggle(enrolled, "speakerSeparation");
    expect(next.speakerSeparation).toBe(false);
    expect(next.recognizeSavedPeople).toBe(false);
  });
});

// ── Independent features ───────────────────────────────────────────────────

describe("applyToggle — features with no dependants", () => {
  for (const id of ["screen", "ocr", "semanticSearch", "aiFeatures", "privacy"]) {
    it(`${id} flips both ways and cascades to nothing`, () => {
      const before = state({ microphone: true, transcription: true });
      const off = applyToggle(before, id);
      expect(off[id]).toBe(!before[id]);
      const back = applyToggle(off, id);
      expect(back[id]).toBe(before[id]);
      expect(back.transcription).toBe(before.transcription);
      expect(back.speakerSeparation).toBe(before.speakerSeparation);
    });
  }
});

// ── Locks ──────────────────────────────────────────────────────────────────

describe("featureLockReason", () => {
  it("locks the microphone until its permission is granted", () => {
    const blocked = state({ permissions: { microphone: false } });
    expect(featureLockReason(blocked, "microphone")).toBe(
      "Needs Microphone permission",
    );
    expect(featureLockReason(state(), "microphone")).toBeNull();
  });

  it("NEVER locks system audio — not on intent, not on screen (ADR 0052)", () => {
    for (const permissions of [
      { systemAudio: false },
      { systemAudio: true },
      { screen: false, systemAudio: false },
    ]) {
      expect(featureLockReason(state({ permissions }), "systemAudio")).toBeNull();
    }
  });

  it("regression: system audio off with no intent can still be toggled back on", () => {
    // The user skipped the Permissions screen, saw it on, turned it off. A lock
    // here would be a gate we can never open — the flow has no route back.
    const noIntent = state({ permissions: { systemAudio: false } });
    expect(featureToggleDisabled(noIntent, "systemAudio")).toBe(false);
    const back = applyToggle(noIntent, "systemAudio");
    expect(back.systemAudio).toBe(true);
    // ...and the upward cascade still runs.
    expect(back.transcription).toBe(true);
    expect(back.speakerSeparation).toBe(true);
  });

  it("locks speaker separation until transcription is on", () => {
    expect(featureLockReason(state(), "speakerSeparation")).toBe(
      "Needs Audio transcription on",
    );
    const on = applyToggle(state(), "microphone");
    expect(featureLockReason(on, "speakerSeparation")).toBeNull();
  });

  it("leaves every other feature unlocked", () => {
    for (const id of ["screen", "systemAudio", "ocr", "transcription", "semanticSearch", "aiFeatures", "privacy"]) {
      expect(featureLockReason(state({ permissions: { microphone: false, systemAudio: false } }), id)).toBeNull();
    }
  });
});

describe("applyToggle — locked enables are refused, disables never are", () => {
  it("refuses to enable the microphone without permission", () => {
    const blocked = state({ permissions: { microphone: false } });
    expect(applyToggle(blocked, "microphone")).toBe(blocked);
  });

  it("refuses to enable speaker separation without transcription", () => {
    const blocked = state();
    expect(applyToggle(blocked, "speakerSeparation")).toBe(blocked);
  });

  it("always permits turning a locked feature OFF", () => {
    // A source enabled before the permission was revoked must still be turnable off.
    const on = normalizeFeatures({
      ...state({ permissions: { microphone: false } }),
      microphone: true,
      transcription: true,
    });
    const next = applyToggle(on, "microphone");
    expect(next.microphone).toBe(false);
    expect(next.transcription).toBe(false);
  });
});

describe("featureToggleDisabled", () => {
  it("is true only when the feature is OFF and its lock is unmet", () => {
    const blocked = state({ permissions: { microphone: false, systemAudio: false } });
    expect(featureToggleDisabled(blocked, "microphone")).toBe(true);
    expect(featureToggleDisabled(blocked, "systemAudio")).toBe(false);
    expect(featureToggleDisabled(blocked, "speakerSeparation")).toBe(true);
    expect(featureToggleDisabled(blocked, "ocr")).toBe(false);

    const on = normalizeFeatures({ ...blocked, microphone: true });
    expect(featureToggleDisabled(on, "microphone")).toBe(false);
  });
});

describe("systemAudioNeedsRequest", () => {
  it("annotates an on-but-unconfirmed row, and nothing else", () => {
    const onNoIntent = normalizeFeatures({
      ...state({ permissions: { systemAudio: false } }),
      systemAudio: true,
    });
    expect(systemAudioNeedsRequest(onNoIntent)).toBe(true);
    // Asked for → no annotation. macOS still never confirms, but the prompt was
    // raised, so the row stops offering "Request".
    expect(systemAudioNeedsRequest({ ...onNoIntent, permissions: { ...onNoIntent.permissions, systemAudio: true } })).toBe(false);
    // Off → nothing to annotate.
    expect(systemAudioNeedsRequest(state({ permissions: { systemAudio: false } }))).toBe(false);
  });
});

describe("applyToggle — purity", () => {
  it("never mutates the input state", () => {
    const before = state();
    const snapshot = JSON.parse(JSON.stringify(before));
    applyToggle(before, "microphone");
    expect(before).toEqual(snapshot);
  });
});

describe("normalizeFeatures", () => {
  it("is idempotent", () => {
    const once = normalizeFeatures(state({ microphone: true, transcription: true }));
    expect(normalizeFeatures(once)).toEqual(once);
  });
});
