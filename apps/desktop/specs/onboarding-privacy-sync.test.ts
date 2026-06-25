// Onboarding privacy-slice sync regression.
//
// `syncPrivacyDraftInto()` is the ONE place the onboarding-only `privacyEnabled`
// flag is derived from the saved excluded-apps list, and the ONLY draft slice the
// privacy controller's `onSettingsUpdated` is allowed to touch (so add/remove/
// recommend commands never clobber other in-progress feature toggles).
//
// It lives in its own `import type`-only module precisely so it can be exercised
// here without dragging in Svelte runes / theme.svelte. We build minimal
// plain-object targets typed as the function's `PrivacyDraftTarget`.

import { describe, expect, test } from "bun:test";
import {
  syncPrivacyDraftInto,
  type PrivacyDraftTarget,
} from "../src/routes/onboarding/onboarding-privacy-sync";
import type { ExcludedAppEntry, RecordingSettings } from "../src/lib/types";

const app = (bundleId: string): ExcludedAppEntry =>
  ({ bundleId, name: bundleId }) as ExcludedAppEntry;

// A `RecordingSettings` with just the privacy slice set; the function only reads
// `next.privacy?.excludedApps`, so the rest is irrelevant.
const settingsWith = (excludedApps?: ExcludedAppEntry[]): RecordingSettings =>
  ({ privacy: excludedApps === undefined ? undefined : { excludedApps } }) as unknown as RecordingSettings;

const target = (over: Partial<PrivacyDraftTarget> = {}): PrivacyDraftTarget => ({
  draftExcludedApps: [],
  privacyEnabled: false,
  ...over,
});

describe("syncPrivacyDraftInto", () => {
  // #1 load fix: a returning user's saved excluded apps must LIGHT the row.
  test("absent privacy -> privacyEnabled false, empty draft", () => {
    const draft = target();
    syncPrivacyDraftInto(draft, settingsWith(undefined));
    expect(draft.privacyEnabled).toBe(false);
    expect(draft.draftExcludedApps).toEqual([]);
  });

  test("empty excludedApps -> privacyEnabled false", () => {
    const draft = target();
    syncPrivacyDraftInto(draft, settingsWith([]));
    expect(draft.privacyEnabled).toBe(false);
  });

  test("non-empty excludedApps from false -> privacyEnabled true", () => {
    const draft = target({ privacyEnabled: false });
    syncPrivacyDraftInto(draft, settingsWith([app("com.foo.bar")]));
    expect(draft.privacyEnabled).toBe(true);
    expect(draft.draftExcludedApps).toEqual([app("com.foo.bar")]);
  });

  // MONOTONIC: removing the last app must NOT auto-turn-OFF (would dim mid-edit).
  test("already-true + zero apps -> stays true (no auto-off)", () => {
    const draft = target({ privacyEnabled: true });
    syncPrivacyDraftInto(draft, settingsWith([]));
    expect(draft.privacyEnabled).toBe(true);
    expect(draft.draftExcludedApps).toEqual([]);
  });

  test("draftExcludedApps is a COPY, not the same array reference", () => {
    const apps = [app("com.a"), app("com.b")];
    const next = settingsWith(apps);
    const draft = target();
    syncPrivacyDraftInto(draft, next);
    expect(draft.draftExcludedApps).toEqual(apps);
    expect(draft.draftExcludedApps).not.toBe(apps);
    expect(draft.draftExcludedApps).not.toBe(next.privacy!.excludedApps);
  });

  // No-clobber by construction: unrelated draft fields are untouched.
  test("leaves unrelated draft fields untouched", () => {
    const draft = target({
      draftOcrEnabled: true,
      draftTranscriptionEnabled: true,
      draftCaptureSystemAudio: true,
    } as Partial<PrivacyDraftTarget> as PrivacyDraftTarget) as PrivacyDraftTarget & {
      draftOcrEnabled: boolean;
      draftTranscriptionEnabled: boolean;
      draftCaptureSystemAudio: boolean;
    };
    syncPrivacyDraftInto(draft, settingsWith([app("com.x")]));
    expect(draft.draftOcrEnabled).toBe(true);
    expect(draft.draftTranscriptionEnabled).toBe(true);
    expect(draft.draftCaptureSystemAudio).toBe(true);
    expect(draft.privacyEnabled).toBe(true);
  });
});
