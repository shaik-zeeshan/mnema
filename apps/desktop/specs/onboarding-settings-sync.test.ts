// Onboarding full-save must not clobber Settings-page-owned slices.
//
// `buildSettingsRequestFrom` sends the WHOLE `RecordingSettings` and is
// authoritative on save, so any field onboarding doesn't surface must be
// round-tripped from `base` untouched. This is a documented regression class
// (the privacy sync once clobbered other in-progress slices); the two slices
// guarded here are the Ask AI web-fetch opt-in (`access.askAiWebFetchEnabled`)
// and the MCP connector list (`aiRuntime.mcpServers`) — both configured only
// on the Settings page, both reset-to-default if omitted from the payload.

import { describe, expect, mock, test } from "bun:test";
import type { McpServerConfig, RecordingSettings } from "../src/lib/types";

// `buildSettingsRequestFrom` reads `theme` at call time, and `theme.svelte`'s
// module body evaluates Svelte runes (`$state`) that bun-test can't run. Stub
// the module (onboarding-transcribe-rehydrate.test.ts pattern).
mock.module("$lib/theme.svelte", () => ({
  theme: { loaded: false, appearance: "system", resolved: "dark" },
}));

const { buildSettingsRequestFrom } = await import(
  "../src/routes/onboarding/onboarding-settings-sync"
);
type OnboardingDraftTarget =
  import("../src/routes/onboarding/onboarding-settings-sync").OnboardingDraftTarget;

const mcpServers: McpServerConfig[] = [
  {
    id: "github",
    label: "GitHub",
    enabled: true,
    transport: "http",
    command: null,
    args: [],
    env: [],
    url: "https://api.githubcopilot.com/mcp/",
    secretEnvName: null,
    enabledTools: ["mcp__github__search"],
  },
];

// A saved `RecordingSettings` carrying only the slices these assertions read;
// the cast keeps the fixture terse (sibling-spec precedent).
const base = {
  access: {
    askAiEnabled: false,
    askAiWebFetchEnabled: true,
    askAiMaxToolCalls: 12,
    askAiModel: null,
  },
  aiRuntime: {
    enabled: true,
    providers: [],
    defaultModel: null,
    mcpServers,
  },
} as unknown as RecordingSettings;

// A draft target with only the fields `buildSettingsRequestFrom` dereferences
// at runtime (strings it trims, arrays it maps); everything else may be absent.
const draft = {
  settings: base,
  draftSaveDirectory: "",
  draftOcrLanguage: "",
  draftTranscriptionLanguage: "",
  selectedSemanticSearchModel: null,
  ai: { draftAiProviders: [], draftAiDefaultModel: null },
} as unknown as OnboardingDraftTarget;

describe("buildSettingsRequestFrom preserves Settings-page-owned slices", () => {
  test("askAiWebFetchEnabled round-trips from base", () => {
    expect(buildSettingsRequestFrom(draft).access.askAiWebFetchEnabled).toBe(true);
  });

  test("mcpServers round-trip from base untouched", () => {
    expect(buildSettingsRequestFrom(draft).aiRuntime.mcpServers).toEqual(mcpServers);
  });
});

// ADR 0052: system audio is an independent capture family — no screen
// dependency, and audio-only sessions are allowed. Anding the saved flag with
// `draftCaptureScreen` silently persisted `captureSystemAudio: false` for a
// returning user whose saved `captureScreen` was off, making audio-only
// unreachable from onboarding.
describe("buildSettingsRequestFrom system audio has no screen dependency", () => {
  const withCapture = (over: Partial<OnboardingDraftTarget>): OnboardingDraftTarget =>
    ({ ...draft, ...over }) as unknown as OnboardingDraftTarget;

  test("sysaudio on + screen off -> persists sysaudio on", () => {
    const next = buildSettingsRequestFrom(
      withCapture({ draftCaptureScreen: false, draftCaptureSystemAudio: true } as Partial<OnboardingDraftTarget>),
    );
    expect(next.captureScreen).toBe(false);
    expect(next.captureSystemAudio).toBe(true);
  });

  test("sysaudio off -> persists sysaudio off regardless of screen", () => {
    expect(
      buildSettingsRequestFrom(
        withCapture({ draftCaptureScreen: true, draftCaptureSystemAudio: false } as Partial<OnboardingDraftTarget>),
      ).captureSystemAudio,
    ).toBe(false);
  });
});
