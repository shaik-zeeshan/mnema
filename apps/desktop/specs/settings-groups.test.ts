import { describe, expect, test } from "bun:test";
import {
  SETTINGS_GROUPS,
  groupForSection,
  resolveTabDeeplink,
  resolveFocusDeeplink,
  sectionForFocus,
  sectionAnchor,
  type SettingsGroupId,
  type SettingsSectionId,
} from "../src/lib/settings/groups";

describe("settings rail: 5-group structure", () => {
  test("exactly five groups in rail order", () => {
    expect(SETTINGS_GROUPS.map((g) => g.id)).toEqual([
      "general",
      "capture",
      "intelligence",
      "data",
      "about",
    ]);
  });

  test("each group stacks its sections in the prescribed order", () => {
    const byId = Object.fromEntries(SETTINGS_GROUPS.map((g) => [g.id, g.sections.map((s) => s.id)]));
    expect(byId.general).toEqual(["appearance", "startup", "shortcuts"]);
    expect(byId.capture).toEqual(["capture", "video", "audio", "privacy"]);
    expect(byId.intelligence).toEqual([
      "intelligence",
      "askAi",
      "userContext",
      "ocr",
      "transcription",
      "speakers",
      "semanticSearch",
    ]);
    expect(byId.data).toEqual(["storage", "access"]);
    expect(byId.about).toEqual(["about", "developer"]);
  });

  test("every section maps back to its owning group", () => {
    const expected: Record<SettingsSectionId, SettingsGroupId> = {
      appearance: "general",
      startup: "general",
      shortcuts: "general",
      capture: "capture",
      video: "capture",
      audio: "capture",
      privacy: "capture",
      intelligence: "intelligence",
      askAi: "intelligence",
      userContext: "intelligence",
      ocr: "intelligence",
      transcription: "intelligence",
      speakers: "intelligence",
      semanticSearch: "intelligence",
      storage: "data",
      access: "data",
      about: "about",
      developer: "about",
    };
    for (const [section, group] of Object.entries(expected)) {
      expect(groupForSection(section as SettingsSectionId)).toBe(group);
    }
  });

  test("section anchors are unique", () => {
    const anchors = SETTINGS_GROUPS.flatMap((g) => g.sections.map((s) => sectionAnchor(s.id)));
    expect(new Set(anchors).size).toBe(anchors.length);
  });
});

describe("settings deeplink resolution (verified gate expectations)", () => {
  // The prompt's explicit deeplink gate:
  //   ?tab=ocr|transcription|speakers → Intelligence group, scrolled to section
  //   ?tab=behavior                   → Capture group
  //   ?tab=keyboard                   → General/Shortcuts
  //   ?focus=cliAccess                → Data/Access
  test("?tab=ocr → Intelligence/ocr", () => {
    const section = resolveTabDeeplink("ocr");
    expect(section).toBe("ocr");
    expect(groupForSection(section!)).toBe("intelligence");
  });

  test("?tab=transcription → Intelligence/transcription", () => {
    const section = resolveTabDeeplink("transcription");
    expect(section).toBe("transcription");
    expect(groupForSection(section!)).toBe("intelligence");
  });

  test("?tab=speakers → Intelligence/speakers", () => {
    const section = resolveTabDeeplink("speakers");
    expect(section).toBe("speakers");
    expect(groupForSection(section!)).toBe("intelligence");
  });

  test("?tab=behavior → Capture/capture", () => {
    const section = resolveTabDeeplink("behavior");
    expect(section).toBe("capture");
    expect(groupForSection(section!)).toBe("capture");
  });

  test("?tab=keyboard → General/shortcuts", () => {
    const section = resolveTabDeeplink("keyboard");
    expect(section).toBe("shortcuts");
    expect(groupForSection(section!)).toBe("general");
  });

  test("?focus=cliAccess → Data/access", () => {
    const focus = resolveFocusDeeplink("cliAccess");
    expect(focus).toBe("cliAccess");
    const section = sectionForFocus(focus!);
    expect(section).toBe("access");
    expect(groupForSection(section)).toBe("data");
  });

  test("legacy aliases resolve to the same sections as before", () => {
    expect(resolveTabDeeplink("metadata")).toBe("privacy");
    expect(resolveTabDeeplink("microphone")).toBe("audio");
    expect(resolveTabDeeplink("reasoning-engine")).toBe("intelligence");
    expect(resolveTabDeeplink("userContext")).toBe("userContext");
    expect(resolveTabDeeplink("cli-access")).toBe("access");
    expect(resolveFocusDeeplink("agent-access")).toBe("cliAccess");
  });

  test("unknown tab/focus resolves to null (legacy no-op)", () => {
    expect(resolveTabDeeplink("nonsense")).toBeNull();
    expect(resolveTabDeeplink(null)).toBeNull();
    expect(resolveFocusDeeplink("nonsense")).toBeNull();
  });
});
