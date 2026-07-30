// `finishOnboarding` is the one funnel every onboarding save passes through, and
// issue #195 changed it in two ways that nothing covered:
//
//  1. The `goto("/")` moved to the Finale's own button, so NOTHING unmounts this
//     state on the success path any more — the in-flight flags have to be
//     released here or `OnboardingFlow.busy` stays true forever and Welcome's
//     "Begin setup" is permanently disabled for anyone who walks back.
//  2. Enrolling a voiceprint on the Voice screen turns `recognize_saved_people`
//     ON backend-side. This save is authoritative and is rebuilt from drafts
//     seeded BEFORE the enrollment, so it would write the stale `false` straight
//     back over the flip and leave the voiceprint loaded by nothing.
import { beforeEach, describe, expect, mock, test } from "bun:test";
import type { RecordingSettings } from "../src/lib/types";

const invoked: { command: string; args?: Record<string, unknown> }[] = [];
let ownerPersonId: number | null = null;
let gotoCalls = 0;

mock.module("$app/navigation", () => ({
  goto: async () => {
    gotoCalls += 1;
  },
}));
mock.module("@tauri-apps/api/core", () => ({
  invoke: async (command: string, args?: Record<string, unknown>) => {
    invoked.push({ command, args });
    if (command === "get_account_owner_person_id") return ownerPersonId;
    if (command === "update_recording_settings") {
      return (args as { request: RecordingSettings }).request;
    }
    return null;
  },
}));

const { finishOnboarding } = await import("../src/routes/onboarding/onboarding-lifecycle");
type OnboardingLifecycleTarget =
  import("../src/routes/onboarding/onboarding-lifecycle").OnboardingLifecycleTarget;

const settingsWith = (recognizeSavedPeople: boolean): RecordingSettings =>
  ({
    speakerAnalysis: {
      separateSpeakers: true,
      recognizeSavedPeople,
      autoLabelOwner: true,
      provider: "speakrs",
      modelId: "pyannote-community-1-wespeaker",
      timeoutSeconds: 600,
    },
  }) as unknown as RecordingSettings;

/** The real shape at the Finale: drafts that predate the enrollment. */
const target = (request: RecordingSettings): OnboardingLifecycleTarget =>
  ({
    loading: false,
    saving: false,
    completing: false,
    starting: false,
    errorMessage: null,
    settings: settingsWith(false),
    permissions: { screen: "granted", microphone: "granted", systemAudio: "not_determined" },
    draftCaptureScreen: true,
    draftCaptureMicrophone: false,
    draftCaptureSystemAudio: false,
    canSkipToDashboard: true,
    ai: {},
    appPrivacyExclusion: {
      loadPrivacyAppCandidates() {},
      loadSensitiveCaptureRecommendations() {},
    },
    syncDrafts() {},
    buildSettingsRequest: () => request,
    resetOptionalFeaturesOff() {},
    loadGeckoUrlAccess: async () => {},
  }) as unknown as OnboardingLifecycleTarget;

function savedRequest(): RecordingSettings {
  const call = invoked.find((entry) => entry.command === "update_recording_settings");
  expect(call, "onboarding must commit the settings").toBeDefined();
  return (call!.args as { request: RecordingSettings }).request;
}

beforeEach(() => {
  invoked.length = 0;
  gotoCalls = 0;
  ownerPersonId = null;
});

describe("finishOnboarding in-flight flags", () => {
  test("a successful start releases completing/starting — nothing unmounts them any more", async () => {
    const t = target(settingsWith(false));

    await finishOnboarding(t, true);

    expect(t.errorMessage).toBeNull();
    expect(t.completing).toBe(false);
    expect(t.starting).toBe(false);
    // Navigation is the Finale's, not this function's: a goto here would unmount
    // the page the instant capture started.
    expect(gotoCalls).toBe(0);
    expect(invoked.map((entry) => entry.command)).toContain("start_native_capture");
  });

  test("a successful skip-capture finish releases them too", async () => {
    const t = target(settingsWith(false));

    await finishOnboarding(t, false);

    expect(t.completing).toBe(false);
    expect(t.starting).toBe(false);
    expect(invoked.map((entry) => entry.command)).not.toContain("start_native_capture");
  });
});

describe("finishOnboarding and an enrolled voiceprint", () => {
  test("a voiceprint enrolled during onboarding is not un-recognized by the save", async () => {
    ownerPersonId = 7; // the user enrolled on the Voice screen

    await finishOnboarding(target(settingsWith(false)), false);

    expect(savedRequest().speakerAnalysis.recognizeSavedPeople).toBe(true);
  });

  test("no voiceprint means the draft is persisted untouched", async () => {
    ownerPersonId = null;

    await finishOnboarding(target(settingsWith(false)), false);

    expect(savedRequest().speakerAnalysis.recognizeSavedPeople).toBe(false);
  });

  test("an already-true draft is left alone without asking the backend", async () => {
    ownerPersonId = 7;

    await finishOnboarding(target(settingsWith(true)), false);

    expect(savedRequest().speakerAnalysis.recognizeSavedPeople).toBe(true);
    expect(invoked.map((entry) => entry.command)).not.toContain("get_account_owner_person_id");
  });
});
