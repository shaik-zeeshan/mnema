// Onboarding re-entry: per-source transcribe flags must reconcile to the master.
//
// Regression for the P0 deadlock: a returning user (settings saved, onboarding
// not completed) with saved `transcription.enabled=false, microphoneEnabled=true`
// and `captureMicrophone=true` re-enters onboarding. Before the fix, `syncDraftsInto`
// rehydrated the per-source `draftTranscriptionMicrophoneEnabled=true` even though
// the master was off, so `transcriptionRequestedWhileOff` lit phantom attention and
// both finale CTAs (`canProceedToFinale`/`canComplete`) stayed disabled forever.
//
// `syncDraftsInto` only WRITES the draft target (it reads `theme` only inside the
// sibling `buildSettingsRequestFrom`), so we exercise it against a minimal plain
// object cast to its `OnboardingDraftTarget` view.

import { describe, expect, mock, test } from "bun:test";
import type { RecordingSettings } from "../src/lib/types";

// `onboarding-settings-sync` has a runtime `import { theme } from "$lib/theme.svelte"`
// (read only inside the sibling `buildSettingsRequestFrom`), and `theme.svelte`'s
// module body evaluates Svelte runes (`$state`) that bun-test can't run. Stub the
// module so importing the round-trip transforms doesn't drag in the runes runtime.
mock.module("$lib/theme.svelte", () => ({
  theme: { loaded: false, appearance: "system", resolved: "dark" },
}));

const { syncDraftsInto } = await import(
  "../src/routes/onboarding/onboarding-settings-sync"
);
type OnboardingDraftTarget =
  import("../src/routes/onboarding/onboarding-settings-sync").OnboardingDraftTarget;

// A `RecordingSettings` carrying only the slices `syncDraftsInto` reads. Missing
// nested objects exercise the `?.` fallbacks; the cast keeps the fixture terse.
const settingsWith = (over: Partial<RecordingSettings>): RecordingSettings =>
  ({
    captureScreen: false,
    captureMicrophone: false,
    captureSystemAudio: false,
    screenFrameRate: 1,
    segmentDurationSeconds: 60,
    screenResolution: { mode: "preset", preset: "1080p" },
    videoBitrate: { mode: "preset", preset: "medium", customMbps: null },
    saveDirectory: "",
    autoStart: false,
    pauseCaptureOnInactivity: false,
    idleTimeoutSeconds: 0,
    ...over,
  }) as unknown as RecordingSettings;

// A draft target whose every field is irrelevant except the transcribe flags we
// assert on; cast to the structural view the controller satisfies at runtime.
const draftTarget = (): OnboardingDraftTarget =>
  ({
    ai: { syncFromSettings() {} },
    selectedSemanticSearchModel: null,
  }) as unknown as OnboardingDraftTarget;

describe("syncDraftsInto transcribe rehydrate", () => {
  // The exact P0 scenario.
  test("master off + saved per-source mic on -> per-source mic zeroed", () => {
    const draft = draftTarget();
    syncDraftsInto(
      draft,
      settingsWith({
        captureMicrophone: true,
        transcription: {
          enabled: false,
          microphoneEnabled: true,
          systemAudioEnabled: false,
        },
      } as Partial<RecordingSettings>),
    );
    expect(draft.draftTranscriptionEnabled).toBe(false);
    expect(draft.draftTranscriptionMicrophoneEnabled).toBe(false);
    expect(draft.draftTranscriptionSystemAudioEnabled).toBe(false);
  });

  test("master off + saved per-source sysaudio on -> per-source sysaudio zeroed", () => {
    const draft = draftTarget();
    syncDraftsInto(
      draft,
      settingsWith({
        captureSystemAudio: true,
        transcription: {
          enabled: false,
          microphoneEnabled: false,
          systemAudioEnabled: true,
        },
      } as Partial<RecordingSettings>),
    );
    expect(draft.draftTranscriptionMicrophoneEnabled).toBe(false);
    expect(draft.draftTranscriptionSystemAudioEnabled).toBe(false);
  });

  // Master on must still round-trip the saved per-source selections (not the fix's
  // forced-false branch), preserving prior behavior.
  test("master on -> per-source flags round-trip", () => {
    const draft = draftTarget();
    syncDraftsInto(
      draft,
      settingsWith({
        transcription: {
          enabled: true,
          microphoneEnabled: true,
          systemAudioEnabled: true,
        },
      } as Partial<RecordingSettings>),
    );
    expect(draft.draftTranscriptionEnabled).toBe(true);
    expect(draft.draftTranscriptionMicrophoneEnabled).toBe(true);
    expect(draft.draftTranscriptionSystemAudioEnabled).toBe(true);
  });

  // Master on with absent per-source fields -> the documented defaults.
  test("master on + absent per-source fields -> mic defaults true, sys false", () => {
    const draft = draftTarget();
    syncDraftsInto(draft, settingsWith({ transcription: { enabled: true } } as Partial<RecordingSettings>));
    expect(draft.draftTranscriptionMicrophoneEnabled).toBe(true);
    expect(draft.draftTranscriptionSystemAudioEnabled).toBe(false);
  });
});
