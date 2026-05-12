import {
  matchShortcut,
  type KeyboardPlatform,
  type ShortcutDefinition,
} from "$lib/keyboard";

export type SourceShortcutKey = "screen" | "microphone" | "systemAudio";

export type GlobalShortcutId =
  | "toggleRecording"
  | "openSettings"
  | "openDebug"
  | "toggleSourceScreen"
  | "toggleSourceMicrophone"
  | "toggleSourceSystemAudio"
  | "toggleShortcutsHelp"
  | "closeShortcutsHelp";

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
  isIdle: boolean;
  isMainRoute: boolean;
  isMainWindow: boolean;
  isShortcutSuppressedTarget: boolean;
  shortcutsHelpOpen: boolean;
};

export const GLOBAL_SHORTCUTS: Record<GlobalShortcutId, ShortcutDefinition> = {
  toggleRecording: {
    id: "toggleRecording",
    label: "Start or stop recording",
    bindings: [{ key: "R", primary: true }],
    kind: "command",
    scope: "global",
  },
  openSettings: {
    id: "openSettings",
    label: "Open settings",
    bindings: [{ key: ",", primary: true }],
    kind: "command",
    scope: "global",
  },
  openDebug: {
    id: "openDebug",
    label: "Open debug",
    bindings: [{ key: "D", primary: true }],
    kind: "command",
    scope: "global",
  },
  toggleSourceScreen: {
    id: "toggleSourceScreen",
    label: "Toggle screen for the next recording",
    bindings: [{ key: "1" }],
    kind: "command",
    scope: "global",
  },
  toggleSourceMicrophone: {
    id: "toggleSourceMicrophone",
    label: "Toggle microphone for the next recording",
    bindings: [{ key: "2" }],
    kind: "command",
    scope: "global",
  },
  toggleSourceSystemAudio: {
    id: "toggleSourceSystemAudio",
    label: "Toggle system audio for the next recording",
    bindings: [{ key: "3" }],
    kind: "command",
    scope: "global",
  },
  toggleShortcutsHelp: {
    id: "toggleShortcutsHelp",
    label: "Show keyboard shortcuts",
    bindings: [{ key: "/" }, { key: "?", shift: true }],
    kind: "command",
    scope: "global",
  },
  closeShortcutsHelp: {
    id: "closeShortcutsHelp",
    label: "Close keyboard shortcuts",
    bindings: [{ key: "Escape" }],
    kind: "behavior",
    scope: "global",
  },
};

export function getGlobalShortcutAction(
  event: GlobalShortcutKeyEvent,
  context: GlobalShortcutContext,
  platform: KeyboardPlatform,
): GlobalShortcutAction | null {
  if (event.repeat) return null;

  if (
    context.shortcutsHelpOpen &&
    matchShortcut(event, GLOBAL_SHORTCUTS.closeShortcutsHelp, platform)
  ) {
    return { type: "closeShortcutsHelp" };
  }
  if (context.shortcutsHelpOpen) return null;

  if (!context.isMainWindow || !context.isMainRoute) return null;

  if (
    !context.isShortcutSuppressedTarget &&
    matchShortcut(event, GLOBAL_SHORTCUTS.toggleShortcutsHelp, platform)
  ) {
    return { type: "toggleShortcutsHelp" };
  }

  if (context.isShortcutSuppressedTarget) return null;

  if (matchShortcut(event, GLOBAL_SHORTCUTS.toggleRecording, platform)) {
    return { type: "toggleRecording" };
  }

  if (matchShortcut(event, GLOBAL_SHORTCUTS.openSettings, platform)) {
    return { type: "openSettings" };
  }

  if (
    context.devEnabled &&
    matchShortcut(event, GLOBAL_SHORTCUTS.openDebug, platform)
  ) {
    return { type: "openDebug" };
  }

  if (!context.isIdle) return null;

  if (matchShortcut(event, GLOBAL_SHORTCUTS.toggleSourceScreen, platform)) {
    return { type: "toggleSource", source: "screen" };
  }
  if (matchShortcut(event, GLOBAL_SHORTCUTS.toggleSourceMicrophone, platform)) {
    return { type: "toggleSource", source: "microphone" };
  }
  if (matchShortcut(event, GLOBAL_SHORTCUTS.toggleSourceSystemAudio, platform)) {
    return { type: "toggleSource", source: "systemAudio" };
  }

  return null;
}
