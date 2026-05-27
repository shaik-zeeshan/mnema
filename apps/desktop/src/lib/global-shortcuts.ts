import {
  matchShortcut,
  type KeyboardPlatform,
  type ShortcutDefinition,
} from "$lib/keyboard";
import {
  getShortcutBinding,
  keyboardBindings,
  shortcutDefinitionWithBinding,
} from "$lib/keyboard-bindings.svelte";

export type SourceShortcutKey = "screen" | "microphone" | "systemAudio";

export type GlobalShortcutId =
  | "toggleRecording"
  | "pauseResumeRecording"
  | "openSettings"
  | "openDebug"
  | "toggleMainWindow"
  | "toggleSourceScreen"
  | "toggleSourceMicrophone"
  | "toggleSourceSystemAudio"
  | "toggleShortcutsHelp"
  | "closeShortcutsHelp";

export type GlobalShortcutAction =
  | { type: "closeShortcutsHelp" }
  | { type: "toggleShortcutsHelp" }
  | { type: "toggleRecording" }
  | { type: "pauseResumeRecording" }
  | { type: "toggleMainWindow" }
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
    bindings: [{ key: "R", primary: true, alt: true }],
    kind: "command",
    scope: "global",
  },
  pauseResumeRecording: {
    id: "pauseResumeRecording",
    label: "Pause or resume recording",
    bindings: [{ key: "P", primary: true, alt: true }],
    kind: "command",
    scope: "global",
  },
  toggleMainWindow: {
    id: "toggleMainWindow",
    label: "Show or hide Mnema",
    bindings: [{ key: "M", primary: true, alt: true }],
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

export function getEffectiveGlobalShortcut(id: GlobalShortcutId): ShortcutDefinition {
  if (id === "closeShortcutsHelp") return GLOBAL_SHORTCUTS[id];
  const binding = getShortcutBinding(keyboardBindings.settings, id);
  return shortcutDefinitionWithBinding(GLOBAL_SHORTCUTS[id], binding);
}

function effectiveShortcut(id: GlobalShortcutId): ShortcutDefinition {
  return getEffectiveGlobalShortcut(id);
}

export function getGlobalShortcutAction(
  event: GlobalShortcutKeyEvent,
  context: GlobalShortcutContext,
  platform: KeyboardPlatform,
): GlobalShortcutAction | null {
  if (event.repeat) return null;

  if (
    context.shortcutsHelpOpen &&
    matchShortcut(event, effectiveShortcut("closeShortcutsHelp"), platform)
  ) {
    return { type: "closeShortcutsHelp" };
  }
  if (context.shortcutsHelpOpen) return null;

  if (!context.isMainWindow || !context.isMainRoute) return null;

  if (
    !context.isShortcutSuppressedTarget &&
    matchShortcut(event, effectiveShortcut("toggleShortcutsHelp"), platform)
  ) {
    return { type: "toggleShortcutsHelp" };
  }

  if (context.isShortcutSuppressedTarget) return null;

  if (matchShortcut(event, effectiveShortcut("toggleRecording"), platform)) {
    return { type: "toggleRecording" };
  }

  if (matchShortcut(event, effectiveShortcut("pauseResumeRecording"), platform)) {
    return { type: "pauseResumeRecording" };
  }

  if (matchShortcut(event, effectiveShortcut("toggleMainWindow"), platform)) {
    return { type: "toggleMainWindow" };
  }

  if (matchShortcut(event, effectiveShortcut("openSettings"), platform)) {
    return { type: "openSettings" };
  }

  if (
    context.devEnabled &&
    matchShortcut(event, effectiveShortcut("openDebug"), platform)
  ) {
    return { type: "openDebug" };
  }

  if (!context.isIdle) return null;

  if (matchShortcut(event, effectiveShortcut("toggleSourceScreen"), platform)) {
    return { type: "toggleSource", source: "screen" };
  }
  if (matchShortcut(event, effectiveShortcut("toggleSourceMicrophone"), platform)) {
    return { type: "toggleSource", source: "microphone" };
  }
  if (matchShortcut(event, effectiveShortcut("toggleSourceSystemAudio"), platform)) {
    return { type: "toggleSource", source: "systemAudio" };
  }

  return null;
}
