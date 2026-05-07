export type SourceShortcutKey = "screen" | "microphone" | "systemAudio";

export type GlobalShortcutAction =
  | { type: "closeShortcutsHelp" }
  | { type: "toggleShortcutsHelp" }
  | { type: "toggleRecording" }
  | { type: "openSettings" }
  | { type: "openDebug" }
  | { type: "toggleSource"; source: SourceShortcutKey };

export type GlobalShortcutKeyEvent = Pick<
  KeyboardEvent,
  "altKey" | "ctrlKey" | "key" | "metaKey" | "repeat" | "shiftKey"
>;

export type GlobalShortcutContext = {
  devEnabled: boolean;
  isShortcutSuppressedTarget: boolean;
  shortcutsHelpOpen: boolean;
};

export function getGlobalShortcutAction(
  event: GlobalShortcutKeyEvent,
  context: GlobalShortcutContext,
): GlobalShortcutAction | null {
  if (event.repeat) return null;

  if (context.shortcutsHelpOpen && event.key === "Escape") {
    return { type: "closeShortcutsHelp" };
  }

  const key = event.key.toLowerCase();
  const primary = event.metaKey || event.ctrlKey;

  if (
    context.shortcutsHelpOpen &&
    !primary &&
    !event.altKey &&
    event.shiftKey &&
    event.key === "?"
  ) {
    return { type: "toggleShortcutsHelp" };
  }

  if (context.isShortcutSuppressedTarget) return null;

  if (primary && !event.altKey && !event.shiftKey && key === "r") {
    return { type: "toggleRecording" };
  }

  if (primary && !event.altKey && !event.shiftKey && event.key === ",") {
    return { type: "openSettings" };
  }

  if (primary && !event.altKey && !event.shiftKey && key === "d") {
    if (!context.devEnabled) return null;
    return { type: "openDebug" };
  }

  if (!primary && !event.altKey && !event.shiftKey) {
    if (event.key === "1") return { type: "toggleSource", source: "screen" };
    if (event.key === "2") return { type: "toggleSource", source: "microphone" };
    if (event.key === "3") return { type: "toggleSource", source: "systemAudio" };
    if (event.key === "/") return { type: "toggleShortcutsHelp" };
  }

  if (!primary && !event.altKey && event.shiftKey && event.key === "?") {
    return { type: "toggleShortcutsHelp" };
  }

  return null;
}
