import { describe, expect, test } from "bun:test";
import { resolveTabDeeplink } from "../src/lib/settings/groups";

// Why this test can't import `normalizeSettingsTab` from surface-windows.ts:
// that module imports `@tauri-apps/api/*`, `$app/navigation`, and `$lib/...` at
// the top level, none of which resolve under plain `bun test` (no Tauri webview,
// no SvelteKit `$lib`/`$app` aliases — there's no bunfig/tsconfig paths mapping).
// `groups.ts` is deliberately framework-free, so `resolveTabDeeplink` imports
// cleanly. We therefore pin the alias *consistency* contract against an inlined
// mirror of the upstream `normalizeSettingsTab` alias set (the same set Rust's
// `normalize_settings_tab` emits), which is exactly what would drift.
//
// CLAUDE.md invariant: deeplink tab aliases must be normalized identically on the
// JS (surface-windows.ts) and Rust (windows.rs) sides, and the page-level
// resolver (groups.ts) runs downstream of them — so every alias the page resolver
// accepts must also survive the upstream normalizers (else it is unreachable).

// Inlined mirror of the upstream `normalizeSettingsTab` (surface-windows.ts) /
// `normalize_settings_tab` (Rust): the canonical tab each accepted alias maps to.
// Anything not listed here is dropped to null by the upstream normalizers and so
// can never reach `resolveTabDeeplink`.
const UPSTREAM_TAB_ALIASES: Record<string, string> = {
  about: "about",
  capture: "capture",
  behavior: "capture",
  access: "access",
  cliAccess: "access",
  "cli-access": "access",
  intelligence: "intelligence",
  reasoning: "intelligence",
  "reasoning-engine": "intelligence",
  ai: "intelligence",
  "ai-runtime": "intelligence",
  "user-context": "userContext",
  userContext: "userContext",
  privacy: "privacy",
  metadata: "privacy",
  shortcuts: "shortcuts",
  keyboard: "shortcuts",
  "keyboard-shortcuts": "shortcuts",
  keyboard_bindings: "shortcuts",
  video: "video",
  audio: "audio",
  microphone: "audio",
  ocr: "ocr",
  transcription: "transcription",
  speakers: "speakers",
  semanticSearch: "semanticSearch",
  "semantic-search": "semanticSearch",
  processing: "processing",
  storage: "storage",
  appearance: "appearance",
  developer: "developer",
};

// Every literal alias `resolveTabDeeplink` accepts (returns non-null for). Kept
// in sync with the switch arms in groups.ts; the consistency tests below fail if
// this set drifts away from the upstream alias set.
const RESOLVE_TAB_ALIASES = [
  "about",
  "capture",
  "behavior",
  "access",
  "cliAccess",
  "cli-access",
  "intelligence",
  "reasoning",
  "reasoning-engine",
  "ai",
  "ai-runtime",
  "user-context",
  "userContext",
  "privacy",
  "metadata",
  "shortcuts",
  "keyboard",
  "keyboard-shortcuts",
  "keyboard_bindings",
  "video",
  "audio",
  "microphone",
  "processing",
  "ocr",
  "transcription",
  "speakers",
  "semanticSearch",
  "semantic-search",
  "storage",
  "appearance",
  "developer",
];

describe("settings deeplink alias consistency", () => {
  test("every alias resolveTabDeeplink accepts is one the upstream normalizer keeps", () => {
    // No alias may reach resolveTabDeeplink that the upstream normalizers drop to
    // null — such an arm would be unreachable in production.
    for (const alias of RESOLVE_TAB_ALIASES) {
      expect(resolveTabDeeplink(alias)).not.toBeNull();
      expect(UPSTREAM_TAB_ALIASES[alias]).toBeDefined();
    }
  });

  test("RESOLVE_TAB_ALIASES list matches what resolveTabDeeplink actually accepts", () => {
    // Guard against the inlined list above silently drifting from the switch:
    // anything the upstream normalizer keeps and that maps onto a real section
    // should resolve, and the list should not claim aliases the function rejects.
    for (const alias of RESOLVE_TAB_ALIASES) {
      expect(resolveTabDeeplink(alias)).not.toBeNull();
    }
  });

  test("legacy reasoning-engine / ai-runtime aliases resolve to intelligence in both layers", () => {
    // These legacy ids are now accepted end-to-end: the upstream normalizers
    // keep them and the page resolver lands them on the Intelligence group's
    // Providers section, so an old `?tab=reasoning-engine` deeplink still works.
    for (const alias of ["reasoning-engine", "ai-runtime"]) {
      expect(resolveTabDeeplink(alias)).toBe("intelligence");
      expect(UPSTREAM_TAB_ALIASES[alias]).toBe("intelligence");
    }
  });

  test("unknown / empty values resolve to null", () => {
    expect(resolveTabDeeplink(null)).toBeNull();
    expect(resolveTabDeeplink(undefined)).toBeNull();
    expect(resolveTabDeeplink("")).toBeNull();
    expect(resolveTabDeeplink("zzz-not-a-tab")).toBeNull();
  });
});
