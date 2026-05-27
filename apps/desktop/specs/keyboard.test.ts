import { describe, expect, test } from "bun:test";
import {
  keyboardPlatformFromUserAgent,
  matchShortcut,
  type ShortcutDefinition,
} from "../src/lib/keyboard";
import {
  shortcutBindingFromKeyboardEvent,
  shortcutDefinitionWithBinding,
} from "../src/lib/keyboard-binding-utils";

const searchShortcut: ShortcutDefinition = {
  id: "dashboard.search",
  label: "Search captured content",
  bindings: [{ key: "K", primary: true }],
  kind: "command",
  scope: "dashboard",
};

function keyEvent(
  overrides: Partial<
    Pick<KeyboardEvent, "altKey" | "ctrlKey" | "key" | "metaKey" | "shiftKey">
  >,
): Pick<KeyboardEvent, "altKey" | "ctrlKey" | "key" | "metaKey" | "shiftKey"> {
  return {
    altKey: false,
    ctrlKey: false,
    key: "k",
    metaKey: false,
    shiftKey: false,
    ...overrides,
  };
}

describe("keyboard helpers", () => {
  test("detects keyboard platform from browser user agents", () => {
    expect(
      keyboardPlatformFromUserAgent("Mozilla/5.0 (Macintosh; Intel Mac OS X 14_5)"),
    ).toBe("macos");
    expect(
      keyboardPlatformFromUserAgent("Mozilla/5.0 (Windows NT 10.0; Win64; x64)"),
    ).toBe("windows");
    expect(keyboardPlatformFromUserAgent("Mozilla/5.0 (X11; Linux x86_64)")).toBe("other");
  });

  test("matches primary modifier shortcuts exactly for macOS", () => {
    expect(matchShortcut(keyEvent({ metaKey: true }), searchShortcut, "macos")).toBe(true);
    expect(matchShortcut(keyEvent({ ctrlKey: true }), searchShortcut, "macos")).toBe(false);
    expect(
      matchShortcut(keyEvent({ metaKey: true, ctrlKey: true }), searchShortcut, "macos"),
    ).toBe(false);
    expect(
      matchShortcut(keyEvent({ metaKey: true, shiftKey: true }), searchShortcut, "macos"),
    ).toBe(false);
    expect(
      matchShortcut(keyEvent({ metaKey: true, altKey: true }), searchShortcut, "macos"),
    ).toBe(false);
  });

  test("matches primary modifier shortcuts exactly for Windows", () => {
    expect(matchShortcut(keyEvent({ ctrlKey: true }), searchShortcut, "windows")).toBe(true);
    expect(matchShortcut(keyEvent({ metaKey: true }), searchShortcut, "windows")).toBe(false);
    expect(
      matchShortcut(keyEvent({ metaKey: true, ctrlKey: true }), searchShortcut, "windows"),
    ).toBe(false);
  });

  test("does not capture macOS Control as CommandOrControl", () => {
    expect(
      shortcutBindingFromKeyboardEvent(
        keyEvent({ ctrlKey: true, key: "p" }) as KeyboardEvent,
        "macos",
      ),
    ).toBeNull();
    expect(
      shortcutBindingFromKeyboardEvent(
        keyEvent({ metaKey: true, key: "p" }) as KeyboardEvent,
        "macos",
      ),
    ).toBe("CommandOrControl+P");
  });

  test("captures Windows Control as CommandOrControl", () => {
    expect(
      shortcutBindingFromKeyboardEvent(
        keyEvent({ ctrlKey: true, key: "p" }) as KeyboardEvent,
        "windows",
      ),
    ).toBe("CommandOrControl+P");
  });

  test("preserves built-in alternate bindings when the editable binding is built in", () => {
    const shortcutHelp: ShortcutDefinition = {
      id: "toggleShortcutsHelp",
      label: "Show keyboard shortcuts",
      bindings: [{ key: "/" }, { key: "?", shift: true }],
      kind: "command",
      scope: "global",
    };

    expect(shortcutDefinitionWithBinding(shortcutHelp, "/").bindings).toEqual([
      { key: "/", primary: false, alt: false, shift: false },
      { key: "?", shift: true },
    ]);
    expect(shortcutDefinitionWithBinding(shortcutHelp, "H").bindings).toEqual([
      { key: "H", primary: false, alt: false, shift: false },
    ]);
  });
});
