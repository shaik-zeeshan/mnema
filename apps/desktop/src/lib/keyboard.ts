export type KeyboardPlatform = "macos" | "windows" | "other";

export type KeyBinding = {
  key: string;
  primary?: boolean;
  shift?: boolean;
  alt?: boolean;
};

export type ShortcutKind = "command" | "behavior";

export type KeyboardScope =
  | "global"
  | "dashboard"
  | "audioDrawer"
  | "settings"
  | "onboarding"
  | "debug";

export type ShortcutDefinition = {
  id: string;
  label: string;
  bindings: KeyBinding[];
  kind: ShortcutKind;
  scope: KeyboardScope;
};

type ShortcutKeyboardEvent = Pick<
  KeyboardEvent,
  "altKey" | "ctrlKey" | "key" | "metaKey" | "shiftKey"
>;

const FOCUSABLE_SELECTOR = [
  "a[href]",
  "button:not([disabled])",
  "textarea:not([disabled])",
  "input:not([disabled])",
  "select:not([disabled])",
  "details > summary",
  '[contenteditable="true"]',
  '[tabindex]:not([tabindex="-1"])',
].join(", ");

const DEFAULT_SUPPRESSED_SELECTORS = [
  "input",
  "textarea",
  "select",
  "button",
  "audio",
  "video",
  '[contenteditable="true"]',
  '[role="textbox"]',
  '[role="searchbox"]',
  '[role="spinbutton"]',
  '[role="slider"]',
  '[role="combobox"]',
  '[role="switch"]',
  '[role="menuitem"]',
  "[data-shortcuts-ignore]",
];

function normalizedKey(key: string): string {
  if (key === "Esc") return "escape";
  if (key === " ") return "space";
  return key.length === 1 ? key.toLowerCase() : key.toLowerCase();
}

function eventMatchesPrimary(
  event: ShortcutKeyboardEvent,
  platform: KeyboardPlatform,
  required: boolean,
): boolean {
  if (!required) return !event.metaKey && !event.ctrlKey;
  if (platform === "macos") return event.metaKey && !event.ctrlKey;
  if (platform === "windows") return event.ctrlKey && !event.metaKey;
  return event.metaKey !== event.ctrlKey;
}

export function matchShortcut(
  event: ShortcutKeyboardEvent,
  definition: ShortcutDefinition,
  platform: KeyboardPlatform,
): boolean {
  return definition.bindings.some((binding) => {
    if (!eventMatchesPrimary(event, platform, binding.primary === true)) return false;
    if (event.altKey !== (binding.alt === true)) return false;
    if (event.shiftKey !== (binding.shift === true)) return false;
    return normalizedKey(event.key) === normalizedKey(binding.key);
  });
}

export function formatShortcut(
  binding: KeyBinding,
  platform: KeyboardPlatform,
): string[] {
  const tokens: string[] = [];
  if (binding.primary) tokens.push(platform === "macos" ? "⌘" : "Ctrl");
  if (binding.alt) tokens.push(platform === "macos" ? "⌥" : "Alt");
  if (binding.shift) tokens.push(platform === "macos" ? "⇧" : "Shift");
  tokens.push(formatKey(binding.key));
  return tokens;
}

function formatKey(key: string): string {
  switch (normalizedKey(key)) {
    case "escape":
      return "Esc";
    case "space":
      return "Space";
    case "arrowleft":
      return "←";
    case "arrowright":
      return "→";
    case "arrowup":
      return "↑";
    case "arrowdown":
      return "↓";
    default:
      return key.length === 1 ? key.toUpperCase() : key;
  }
}

export function isShortcutSuppressedTarget(
  target: EventTarget | null,
  extraSelectors: string[] = [],
): boolean {
  if (!(target instanceof Element)) return false;
  return Boolean(
    target.closest([...DEFAULT_SUPPRESSED_SELECTORS, ...extraSelectors].join(", ")),
  );
}

function isElementFocusable(el: HTMLElement): boolean {
  if (el.hasAttribute("disabled")) return false;
  if (el.getAttribute("aria-hidden") === "true") return false;
  const style = window.getComputedStyle(el);
  if (style.display === "none" || style.visibility === "hidden") return false;
  const rects = el.getClientRects();
  return rects.length > 0 || el === document.activeElement;
}

export function getFocusableElements(container: HTMLElement | null): HTMLElement[] {
  if (!container) return [];
  return Array.from(container.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR)).filter(
    isElementFocusable,
  );
}

export function trapTabKey(event: KeyboardEvent, container: HTMLElement | null): boolean {
  if (event.key !== "Tab") return false;
  const focusable = getFocusableElements(container);
  if (focusable.length === 0) {
    event.preventDefault();
    container?.focus();
    return true;
  }

  const first = focusable[0];
  const last = focusable[focusable.length - 1];
  const active = document.activeElement as HTMLElement | null;
  if (event.shiftKey) {
    if (active === first || !container?.contains(active)) {
      event.preventDefault();
      last.focus();
      return true;
    }
  } else if (active === last) {
    event.preventDefault();
    first.focus();
    return true;
  }
  return false;
}
