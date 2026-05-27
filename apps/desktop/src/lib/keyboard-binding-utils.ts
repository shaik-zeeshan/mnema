import type { KeyBinding, KeyboardPlatform, ShortcutDefinition } from "$lib/keyboard";

export function normalizeShortcutBinding(value: string): string {
  const parsed = parseShortcutBinding(value);
  if (!parsed) return "";
  const parts: string[] = [];
  if (parsed.primary) parts.push("CommandOrControl");
  if (parsed.alt) parts.push("Alt");
  if (parsed.shift) parts.push("Shift");
  parts.push(parsed.key);
  return parts.join("+");
}

function keyBindingsEqual(left: KeyBinding, right: KeyBinding): boolean {
  return left.key.toLowerCase() === right.key.toLowerCase()
    && (left.primary === true) === (right.primary === true)
    && (left.alt === true) === (right.alt === true)
    && (left.shift === true) === (right.shift === true);
}

export function shortcutDefinitionWithBinding(
  definition: ShortcutDefinition,
  binding: string,
): ShortcutDefinition {
  const parsed = parseShortcutBinding(binding);
  if (!parsed) {
    return {
      ...definition,
      bindings: [],
    };
  }

  const matchesBuiltInBinding = definition.bindings.some((builtIn) => keyBindingsEqual(builtIn, parsed));
  if (!matchesBuiltInBinding) {
    return {
      ...definition,
      bindings: [parsed],
    };
  }

  return {
    ...definition,
    bindings: [
      parsed,
      ...definition.bindings.filter((builtIn) => !keyBindingsEqual(builtIn, parsed)),
    ],
  };
}

export function parseShortcutBinding(value: string | null | undefined): KeyBinding | null {
  const trimmed = value?.trim();
  if (!trimmed) return null;
  let primary = false;
  let alt = false;
  let shift = false;
  let key: string | null = null;

  for (const rawPart of trimmed.split("+")) {
    const part = rawPart.trim();
    if (!part) return null;
    const normalized = part.toLowerCase().replace(/-/g, "");
    if (["commandorcontrol", "cmdorctrl", "primary", "command", "cmd", "meta", "control", "ctrl"].includes(normalized)) {
      if (key || primary) return null;
      primary = true;
      continue;
    }
    if (["alt", "option", "opt"].includes(normalized)) {
      if (key || alt) return null;
      alt = true;
      continue;
    }
    if (normalized === "shift") {
      if (key || shift) return null;
      shift = true;
      continue;
    }
    if (key) return null;
    key = normalizeKey(part);
  }

  if (!key) return null;
  return { key, primary, alt, shift };
}

function normalizeKey(key: string): string | null {
  const lower = key.toLowerCase();
  switch (lower) {
    case "esc":
    case "escape":
      return "Escape";
    case " ":
    case "space":
    case "spacebar":
      return "Space";
    case "left":
    case "arrowleft":
      return "ArrowLeft";
    case "right":
    case "arrowright":
      return "ArrowRight";
    case "up":
    case "arrowup":
      return "ArrowUp";
    case "down":
    case "arrowdown":
      return "ArrowDown";
    case "tab":
      return "Tab";
    case "enter":
    case "return":
      return "Enter";
    case "backspace":
      return "Backspace";
    case "delete":
      return "Delete";
    default: {
      if (key.length === 1) return key.toUpperCase();
      if (/^f\d+$/i.test(key)) {
        const number = Number(key.slice(1));
        if (Number.isInteger(number) && number >= 0 && number <= 255) {
          return key.toUpperCase();
        }
      }
      return null;
    }
  }
}

function keyFromPhysicalCode(code: string): string | null {
  if (/^Key[A-Z]$/.test(code)) return code.slice(3);
  if (/^Digit\d$/.test(code)) return code.slice(5);
  switch (code) {
    case "Backquote": return "`";
    case "Minus": return "-";
    case "Equal": return "=";
    case "BracketLeft": return "[";
    case "BracketRight": return "]";
    case "Backslash": return "\\";
    case "Semicolon": return ";";
    case "Quote": return "'";
    case "Comma": return ",";
    case "Period": return ".";
    case "Slash": return "/";
    case "Space": return "Space";
    default: return null;
  }
}

function shortcutCaptureKey(event: KeyboardEvent, platform: KeyboardPlatform): string {
  if (platform === "macos" && event.altKey) {
    return keyFromPhysicalCode(event.code) ?? event.key;
  }
  return event.key;
}

export function shortcutBindingFromKeyboardEvent(
  event: KeyboardEvent,
  platform: KeyboardPlatform = "other",
): string | null {
  if (event.key === "Meta" || event.key === "Control" || event.key === "Alt" || event.key === "Shift") {
    return null;
  }
  if (platform === "macos" && event.ctrlKey) {
    return null;
  }
  const parts: string[] = [];
  if (platform === "macos") {
    if (event.metaKey) parts.push("CommandOrControl");
  } else if (event.metaKey || event.ctrlKey) {
    parts.push("CommandOrControl");
  }
  if (event.altKey) parts.push("Alt");
  if (event.shiftKey) parts.push("Shift");
  const key = normalizeKey(shortcutCaptureKey(event, platform));
  if (!key) return null;
  parts.push(key);
  return parts.join("+");
}
