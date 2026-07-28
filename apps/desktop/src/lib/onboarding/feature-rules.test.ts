// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig, so skip static checking here.
import { describe, expect, it } from "bun:test";
import {
  applyToggle,
  cascadeOf,
  featureLockReason,
  featureNote,
  featureToggleDisabled,
  lockFix,
  normalizeFeatures,
  preview,
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

  it("locks transcription until an audio source is on", () => {
    // Was a SILENT no-op: applyToggle set it, normalizeFeatures unset it, and
    // the switch bounced back with no explanation.
    const noAudio = state();
    expect(featureLockReason(noAudio, "transcription")).toBe("Needs an audio source on");
    expect(featureToggleDisabled(noAudio, "transcription")).toBe(true);
    expect(applyToggle(noAudio, "transcription")).toBe(noAudio);
    const p = preview(noAudio, "transcription");
    expect(p.noop).toBe(true);
    expect(p.lockReason).toBe("Needs an audio source on");
    // Either source clears it.
    for (const source of ["microphone", "systemAudio"]) {
      expect(featureLockReason(state({ [source]: true }), "transcription")).toBeNull();
    }
  });

  it("leaves every other feature unlocked", () => {
    for (const id of ["screen", "systemAudio", "ocr", "semanticSearch", "aiFeatures", "privacy"]) {
      expect(featureLockReason(state({ permissions: { microphone: false, systemAudio: false } }), id)).toBeNull();
    }
  });
});

// ── The cascade, exposed (slice 4) ─────────────────────────────────────────

describe("cascadeOf", () => {
  it("reports both children when the LAST audio source goes off", () => {
    const on = applyToggle(state(), "microphone");
    const off = applyToggle(on, "microphone");
    expect(cascadeOf(on, off, "microphone")).toEqual(["transcription", "speakerSeparation"]);
  });

  it("reports one child when transcription goes off", () => {
    const on = applyToggle(state(), "microphone");
    const off = applyToggle(on, "transcription");
    expect(cascadeOf(on, off, "transcription")).toEqual(["speakerSeparation"]);
  });

  it("reports both children when an audio source comes on", () => {
    const before = state();
    const after = applyToggle(before, "systemAudio");
    expect(cascadeOf(before, after, "systemAudio")).toEqual([
      "transcription",
      "speakerSeparation",
    ]);
  });

  it("reports nothing when one of two sources goes off", () => {
    const both = applyToggle(applyToggle(state(), "microphone"), "systemAudio");
    expect(cascadeOf(both, applyToggle(both, "microphone"), "microphone")).toEqual([]);
  });

  it("reports nothing for a feature with no dependants", () => {
    const before = state({ microphone: true });
    for (const id of ["screen", "ocr", "semanticSearch", "aiFeatures", "privacy"]) {
      expect(cascadeOf(before, applyToggle(before, id), id)).toEqual([]);
    }
  });
});

describe("preview", () => {
  it("sees a cascade before committing it, and commits nothing", () => {
    const before = applyToggle(state(), "microphone");
    const p = preview(before, "microphone");
    expect(p.next).toBe(false);
    expect(p.noop).toBe(false);
    expect(p.lockReason).toBeNull();
    expect(p.cascade).toEqual(["transcription", "speakerSeparation"]);
    // The row values the sentence needs come off `after`.
    expect(p.after.transcription).toBe(false);
    expect(p.after.speakerSeparation).toBe(false);
    // Nothing moved on the previewed state.
    expect(before.transcription).toBe(true);
    expect(before.speakerSeparation).toBe(true);
  });

  it("matches applyToggle exactly", () => {
    const before = state({ microphone: true });
    for (const id of ["screen", "microphone", "systemAudio", "ocr", "transcription", "speakerSeparation", "semanticSearch", "aiFeatures", "privacy"]) {
      expect(preview(before, id).after).toEqual(applyToggle(before, id));
    }
  });

  it("reports a locked enable as a no-op carrying its reason", () => {
    const blocked = state({ permissions: { microphone: false } });
    const p = preview(blocked, "microphone");
    expect(p.noop).toBe(true);
    expect(p.after).toBe(blocked);
    expect(p.next).toBe(false);
    expect(p.lockReason).toBe("Needs Microphone permission");
    expect(p.cascade).toEqual([]);
  });
});

describe("lockFix", () => {
  it("offers the OS grant for the microphone — no row can resolve it", () => {
    const blocked = state({ permissions: { microphone: false } });
    expect(lockFix(blocked, "microphone")).toEqual({
      act: "grant",
      id: "microphone",
      label: "Grant Microphone",
    });
  });

  it("offers the parent row when the parent is flippable", () => {
    // Audio is on, transcription was turned off deliberately → who's speaking
    // is locked on a row the user CAN flip.
    const noTranscript = applyToggle(applyToggle(state(), "microphone"), "transcription");
    expect(lockFix(noTranscript, "speakerSeparation")).toEqual({
      act: "toggle",
      id: "transcription",
      label: "Turn Transcription on",
    });
  });

  it("skips a locked ancestor to reach a flippable one", () => {
    // No audio at all: who's speaking → transcription (itself locked) → system
    // audio, the one source that can never lock (ADR 0052).
    const noAudio = state();
    expect(featureToggleDisabled(noAudio, "transcription")).toBe(true);
    expect(lockFix(noAudio, "speakerSeparation")).toEqual({
      act: "toggle",
      id: "systemAudio",
      label: "Turn System audio on",
    });
    // ...and the fix it offers is not itself a no-op.
    expect(preview(noAudio, "systemAudio").noop).toBe(false);
  });

  it("offers system audio for a locked transcription row", () => {
    expect(lockFix(state(), "transcription")).toEqual({
      act: "toggle",
      id: "systemAudio",
      label: "Turn System audio on",
    });
  });

  it("is null for a row that is not locked", () => {
    const on = applyToggle(state(), "microphone");
    for (const id of ["screen", "microphone", "systemAudio", "ocr", "transcription", "speakerSeparation", "semanticSearch", "aiFeatures", "privacy"]) {
      expect(lockFix(on, id)).toBeNull();
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

// ── Row copy (the point of the screen: never state something false) ────────

describe("featureNote", () => {
  it("never calls the microphone lock 'the one thing missing' while it is ON", () => {
    // `resolveSetup` leaves capture sources ON regardless of the grant, so this
    // is the SHIPPING state: switch on, permission missing, recording nothing.
    const onUngranted = normalizeFeatures({
      ...state({ permissions: { microphone: false } }),
      microphone: true,
    });
    expect(onUngranted.microphone).toBe(true);
    expect(featureNote(onUngranted, "microphone")).toBe(
      "Microphone permission is not granted — stays on, records nothing.",
    );
  });

  it("tells an OFF ungranted microphone row what the grant buys", () => {
    const offUngranted = state({ permissions: { microphone: false } });
    expect(featureNote(offUngranted, "microphone")).toBe(
      "Microphone permission is not granted — grant it to turn this on.",
    );
  });

  it("describes a granted microphone as a source, not as a permission", () => {
    const granted = state({ microphone: true });
    expect(featureNote(granted, "microphone")).toBe(
      "Your voice, from the built-in or a connected mic.",
    );
  });

  it("only says transcription needs audio when it actually does", () => {
    const noAudio = state();
    expect(featureLockReason(noAudio, "transcription")).not.toBeNull();
    expect(featureNote(noAudio, "transcription")).toBe("Needs an audio source above it.");

    // Both sources on, transcription deliberately off — the old copy claimed the
    // two rows above it did not exist.
    const audioOn = applyToggle(
      applyToggle(applyToggle(state(), "microphone"), "systemAudio"),
      "transcription",
    );
    expect(audioOn.transcription).toBe(false);
    expect(featureLockReason(audioOn, "transcription")).toBeNull();
    expect(featureNote(audioOn, "transcription")).toBe(
      "Off — the audio is still recorded, just never turned into text.",
    );

    expect(featureNote(applyToggle(state(), "microphone"), "transcription")).toBe(
      "Runs locally on Whisper base.",
    );
  });

  it("annotates system audio only while macOS cannot confirm the grant", () => {
    const unconfirmed = normalizeFeatures({
      ...state({ permissions: { systemAudio: false } }),
      systemAudio: true,
    });
    expect(featureNote(unconfirmed, "systemAudio")).toContain("can't confirm this grant");
    expect(featureNote(state({ systemAudio: true }), "systemAudio")).not.toContain(
      "can't confirm",
    );
  });

  it("reports the real AI state, including the store's own reason", () => {
    const off = state();
    expect(featureNote(off, "aiFeatures", { configured: false })).toContain("Never pre-ticked");
    expect(featureNote(off, "aiFeatures", { configured: true })).toContain("Ready");
    const on = applyToggle(off, "aiFeatures");
    expect(featureNote(on, "aiFeatures", { configured: false, note: "Verify Ollama first." })).toBe(
      "Verify Ollama first.",
    );
  });

  it("gives every drawn row a sentence", () => {
    for (const id of ["screen", "ocr", "microphone", "systemAudio", "transcription", "speakerSeparation", "semanticSearch", "aiFeatures"]) {
      expect(featureNote(state(), id, { configured: false }).length).toBeGreaterThan(0);
    }
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
