import { describe, expect, test } from "bun:test";
import { featureLockReason, type FeatureLockContext } from "../src/routes/onboarding/feature-model";
import { isFeatureEnabled, featureAttentionFor, type OnboardingFeatureTarget } from "../src/routes/onboarding/onboarding-feature-state";

// specs/ sits outside the `.svelte-kit` tsconfig `include` (src/ only), so
// `bun run check` does not type-check this file — these tests are the only
// thing pinning the contracts below.
const ctx = (over: Partial<FeatureLockContext> = {}): FeatureLockContext => ({
  micGranted: false, transcriptionEnabled: false, ...over,
});
const avail = (available: boolean) => ({ available });
const target = (over: Partial<OnboardingFeatureTarget> = {}): OnboardingFeatureTarget => ({
  draftCaptureMicrophone: false, draftCaptureSystemAudio: false, draftOcrEnabled: false,
  draftTranscriptionEnabled: false, draftSpeakerSeparateSpeakers: false, privacyEnabled: false,
  draftAskAiEnabled: false, draftSemanticSearchEnabled: false,
  permissions: { screen: "granted", microphone: "granted", systemAudio: "not_determined" },
  transcriptionRequestedWhileOff: false,
  selectedOcrModel: avail(true), selectedTranscriptionModel: avail(true),
  selectedSpeakerModel: avail(true), selectedSemanticSearchModel: avail(true),
  selectedOcrDownloadRunning: false, selectedOcrDownloadPercent: null,
  selectedTranscriptionDownloadRunning: false, selectedTranscriptionDownloadPercent: null,
  selectedSpeakerDownloadRunning: false, selectedSpeakerDownloadPercent: null,
  selectedSemanticSearchDownloadRunning: false, selectedSemanticSearchDownloadPercent: null,
  ai: { aiConfigReady: true }, ...over,
}) as unknown as OnboardingFeatureTarget;

describe("featureLockReason (INV-DEP-GATE)", () => {
  test("mic locked until Microphone permission granted", () => {
    expect(featureLockReason("mic", ctx({ micGranted: false }))).toBe("Needs Microphone permission");
    expect(featureLockReason("mic", ctx({ micGranted: true }))).toBeNull();
  });
  // ADR 0052: a Core Audio process tap's grant is unreadable and its prompt only
  // fires at the first tap read, so a lock would never open.
  test("sysaudio never locks — its grant is unreadable", () => {
    expect(featureLockReason("sysaudio", ctx())).toBeNull();
  });
  test("speakers locked until transcription on", () => {
    expect(featureLockReason("speakers", ctx({ transcriptionEnabled: false }))).toBe("Needs Audio transcription on");
    expect(featureLockReason("speakers", ctx({ transcriptionEnabled: true }))).toBeNull();
  });
  test("required + unconditional features never lock", () => {
    for (const id of ["permissions","screen","storage","ocr","transcribe","privacy","askai","semanticSearch","licensing"] as const)
      expect(featureLockReason(id, ctx())).toBeNull();
  });
});

describe("featureAttentionFor permissions (INV-SCREEN-UNSUPPORTED)", () => {
  test("screen ungranted -> attention", () => { expect(featureAttentionFor(target({ permissions: { screen: "denied" } }), "permissions")).toBe(true); });
  test("screen unsupported -> NON-blocking", () => { expect(featureAttentionFor(target({ permissions: { screen: "unsupported" } }), "permissions")).toBe(false); });
  test("screen granted -> no attention", () => { expect(featureAttentionFor(target({ permissions: { screen: "granted" } }), "permissions")).toBe(false); });
  test("sysaudio enabled + unsupported -> NON-blocking", () => { expect(featureAttentionFor(target({ draftCaptureSystemAudio: true, permissions: { systemAudio: "unsupported" } }), "sysaudio")).toBe(false); });
  // ADR 0052 tri-state: only a positive suspicion blocks. "not yet requested" is
  // the normal onboarding state (the prompt fires at the first tap read), so
  // demanding proof would make the attention count unclearable.
  test("sysaudio enabled + possibly_blocked -> attention", () => { expect(featureAttentionFor(target({ draftCaptureSystemAudio: true, permissions: { systemAudio: "possibly_blocked" } }), "sysaudio")).toBe(true); });
  test("sysaudio enabled + not_determined/assumed_working -> no attention", () => {
    expect(featureAttentionFor(target({ draftCaptureSystemAudio: true, permissions: { systemAudio: "not_determined" } }), "sysaudio")).toBe(false);
    expect(featureAttentionFor(target({ draftCaptureSystemAudio: true, permissions: { systemAudio: "assumed_working" } }), "sysaudio")).toBe(false);
  });
  test("sysaudio off + possibly_blocked -> no attention", () => { expect(featureAttentionFor(target({ draftCaptureSystemAudio: false, permissions: { systemAudio: "possibly_blocked" } }), "sysaudio")).toBe(false); });
  test("mic enabled + ungranted -> attention; granted -> none", () => {
    expect(featureAttentionFor(target({ draftCaptureMicrophone: true, permissions: { microphone: "denied" } }), "mic")).toBe(true);
    expect(featureAttentionFor(target({ draftCaptureMicrophone: true, permissions: { microphone: "granted" } }), "mic")).toBe(false);
  });
});

describe("featureAttentionFor model + transcribe-requested", () => {
  test("transcribe master off but a source requests transcript -> attention", () => { expect(featureAttentionFor(target({ transcriptionRequestedWhileOff: true }), "transcribe")).toBe(true); });
  test("ocr on + model unavailable -> attention; available -> none", () => {
    expect(featureAttentionFor(target({ draftOcrEnabled: true, selectedOcrModel: avail(false) }), "ocr")).toBe(true);
    expect(featureAttentionFor(target({ draftOcrEnabled: true, selectedOcrModel: avail(true) }), "ocr")).toBe(false);
  });
  test("askai on + config not ready -> attention", () => { expect(featureAttentionFor(target({ draftAskAiEnabled: true, ai: { aiConfigReady: false } }), "askai")).toBe(true); });
  test("screen/storage/privacy/licensing never raise attention", () => { for (const id of ["screen","storage","privacy","licensing"] as const) expect(featureAttentionFor(target({ privacyEnabled: true }), id)).toBe(false); });
});

describe("isFeatureEnabled", () => {
  test("required features always enabled", () => { for (const id of ["permissions","screen","storage","licensing"] as const) expect(isFeatureEnabled(target(), id)).toBe(true); });
  test("optional features track their draft flag", () => {
    expect(isFeatureEnabled(target({ draftSemanticSearchEnabled: true }), "semanticSearch")).toBe(true);
    expect(isFeatureEnabled(target({ draftSemanticSearchEnabled: false }), "semanticSearch")).toBe(false);
  });
});
