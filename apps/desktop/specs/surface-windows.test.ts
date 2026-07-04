// Pure exports of the surface-windows seam: the deeplink alias normalizers
// (`normalizeSettingsTab`/`normalizeSettingsFocus`), the `/settings` route-path
// query assembly (`settingsRoutePath`), and the module-level "last main surface"
// memory (`recordMainSurface`/`getLastMainSurface`).
//
// Importing the module pulls in Tauri (`@tauri-apps/api/core`,
// `@tauri-apps/api/window`) and SvelteKit (`$app/navigation`) at module top —
// none of which exist under bun-test — so stub them the same way
// `onboarding-transcribe-rehydrate.test.ts` stubs `$lib/theme.svelte`. The pure
// functions under test never touch these stubs; they're only here so the import
// resolves. `$lib/route-path` is a plain (rune-free) module and is imported for
// real, so `recordMainSurface`'s main-surface check exercises real logic.

import { describe, expect, mock, test } from "bun:test";

mock.module("@tauri-apps/api/core", () => ({
  invoke: async () => {},
  // bun module mocks fix the export-name set process-wide; later test files
  // transitively import convertFileSrc (frame-preview), so it must exist here.
  convertFileSrc: (p: string) => p,
}));
mock.module("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ label: "main" }),
}));
mock.module("$app/navigation", () => ({
  goto: async () => {},
}));

const {
  normalizeSettingsTab,
  normalizeSettingsFocus,
  settingsRoutePath,
  recordMainSurface,
  getLastMainSurface,
} = await import("../src/lib/surface-windows");

describe("normalizeSettingsTab", () => {
  // Every (alias -> canonical) pair from the switch in `surface-windows.ts`.
  // Enumerated from source, not invented.
  const aliases: Array<[string, string]> = [
    ["about", "about"],
    ["capture", "capture"],
    ["behavior", "capture"],
    ["access", "access"],
    ["cliAccess", "access"],
    ["cli-access", "access"],
    ["intelligence", "intelligence"],
    ["reasoning", "intelligence"],
    ["reasoning-engine", "intelligence"],
    ["ai", "intelligence"],
    ["ai-runtime", "intelligence"],
    ["user-context", "userContext"],
    ["userContext", "userContext"],
    ["privacy", "privacy"],
    ["metadata", "privacy"],
    ["shortcuts", "shortcuts"],
    ["keyboard", "shortcuts"],
    ["keyboard-shortcuts", "shortcuts"],
    ["keyboard_bindings", "shortcuts"],
    ["video", "video"],
    ["audio", "audio"],
    ["microphone", "audio"],
    ["ocr", "ocr"],
    ["transcription", "transcription"],
    ["speakers", "speakers"],
    ["semanticSearch", "semanticSearch"],
    ["semantic-search", "semanticSearch"],
    ["processing", "processing"],
    ["storage", "storage"],
    ["appearance", "appearance"],
    ["developer", "developer"],
  ];

  for (const [alias, canonical] of aliases) {
    test(`"${alias}" -> "${canonical}"`, () => {
      expect(normalizeSettingsTab(alias)).toBe(canonical);
    });
  }

  test("unknown value -> null", () => {
    expect(normalizeSettingsTab("not-a-tab")).toBe(null);
    expect(normalizeSettingsTab("")).toBe(null);
  });

  test("null/undefined -> null", () => {
    expect(normalizeSettingsTab(null)).toBe(null);
    expect(normalizeSettingsTab(undefined)).toBe(null);
    expect(normalizeSettingsTab()).toBe(null);
  });
});

describe("normalizeSettingsFocus", () => {
  // The four accepted focus aliases all collapse onto "cliAccess".
  for (const alias of ["agentAccess", "agent-access", "cliAccess", "cli-access"]) {
    test(`"${alias}" -> "cliAccess"`, () => {
      expect(normalizeSettingsFocus(alias)).toBe("cliAccess");
    });
  }

  test("unknown -> null", () => {
    expect(normalizeSettingsFocus("somethingElse")).toBe(null);
    expect(normalizeSettingsFocus("")).toBe(null);
  });

  test("null/undefined -> null", () => {
    expect(normalizeSettingsFocus(null)).toBe(null);
    expect(normalizeSettingsFocus(undefined)).toBe(null);
    expect(normalizeSettingsFocus()).toBe(null);
  });
});

describe("settingsRoutePath", () => {
  test("no args -> bare /settings", () => {
    expect(settingsRoutePath()).toBe("/settings");
  });

  test("tab only -> ?tab= with canonical value", () => {
    // "behavior" normalizes to "capture", proving the query carries the
    // canonical tab rather than the raw alias.
    expect(settingsRoutePath("behavior" as never)).toBe("/settings?tab=capture");
    expect(settingsRoutePath("about")).toBe("/settings?tab=about");
  });

  test("tab + focus -> ?tab=&focus=", () => {
    expect(settingsRoutePath("access", "agentAccess")).toBe(
      "/settings?tab=access&focus=cliAccess",
    );
  });

  test("focus only -> ?focus= (no tab)", () => {
    expect(settingsRoutePath(undefined, "cliAccess")).toBe(
      "/settings?focus=cliAccess",
    );
  });

  test("unknown tab drops to bare /settings", () => {
    // An unrecognized tab normalizes to null, so no `tab` param is set and the
    // query stays empty.
    expect(settingsRoutePath("not-a-tab" as never)).toBe("/settings");
  });

  test("unknown tab + unknown focus -> bare /settings", () => {
    expect(settingsRoutePath("not-a-tab" as never, "nope" as never)).toBe(
      "/settings",
    );
  });
});

describe("recordMainSurface / getLastMainSurface", () => {
  // `lastMainSurfacePath` is module-level state that persists across tests, so
  // each test first records a known accepted value, then asserts relative to
  // what it just recorded — no reliance on default `/` or prior-test ordering.

  test("recording Timeline (/) is returned", () => {
    recordMainSurface("/insights"); // set a non-default baseline
    recordMainSurface("/");
    expect(getLastMainSurface()).toBe("/");
  });

  test("recording Insights (/insights) is returned", () => {
    recordMainSurface("/");
    recordMainSurface("/insights");
    expect(getLastMainSurface()).toBe("/insights");
  });

  test("trailing-slash paths normalize before storing", () => {
    recordMainSurface("/");
    recordMainSurface("/insights/");
    // normalizeAppPathname strips the trailing slash.
    expect(getLastMainSurface()).toBe("/insights");
  });

  test("/index.html normalizes to / and is accepted", () => {
    recordMainSurface("/insights");
    recordMainSurface("/index.html");
    expect(getLastMainSurface()).toBe("/");
  });

  test("rejected path (/settings) keeps the prior accepted value", () => {
    recordMainSurface("/insights");
    recordMainSurface("/settings");
    expect(getLastMainSurface()).toBe("/insights");
  });

  test("rejected path (/onboarding) keeps the prior accepted value", () => {
    recordMainSurface("/");
    recordMainSurface("/onboarding");
    expect(getLastMainSurface()).toBe("/");
  });

  test("rejected arbitrary path keeps the prior accepted value", () => {
    recordMainSurface("/insights");
    recordMainSurface("/some/other/route");
    expect(getLastMainSurface()).toBe("/insights");
  });
});
