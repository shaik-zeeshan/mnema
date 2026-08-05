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
  | "toggleQuickRecall"
  | "openTimelineSurface"
  | "openOverviewSurface"
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
  | { type: "openSurface"; surface: MainSurface }
  | { type: "toggleSource"; source: SourceShortcutKey };

/** The two peer surfaces the Main window switches between (⌘1 / ⌘2). */
export type MainSurface = "timeline" | "overview";

export type GlobalShortcutKeyEvent = Pick<
  KeyboardEvent,
  "altKey" | "code" | "ctrlKey" | "key" | "metaKey" | "repeat" | "shiftKey"
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
  toggleQuickRecall: {
    id: "toggleQuickRecall",
    label: "Summon Quick Recall",
    bindings: [{ key: "Space", primary: true, alt: true }],
    kind: "command",
    scope: "global",
  },
  // Surface switching is structural (it names a place, like a tab), so unlike
  // the command shortcuts these two are not rebindable — same reason
  // `closeShortcutsHelp` isn't.
  openTimelineSurface: {
    id: "openTimelineSurface",
    label: "Go to Timeline",
    bindings: [{ key: "1", primary: true }],
    kind: "command",
    scope: "global",
  },
  openOverviewSurface: {
    id: "openOverviewSurface",
    label: "Go to Overview",
    bindings: [{ key: "2", primary: true }],
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
    label: "Toggle screen",
    bindings: [{ key: "1" }],
    kind: "command",
    scope: "global",
  },
  toggleSourceMicrophone: {
    id: "toggleSourceMicrophone",
    label: "Toggle microphone",
    bindings: [{ key: "2" }],
    kind: "command",
    scope: "global",
  },
  toggleSourceSystemAudio: {
    id: "toggleSourceSystemAudio",
    label: "Toggle system audio",
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
  if (
    id === "closeShortcutsHelp" ||
    id === "openTimelineSurface" ||
    id === "openOverviewSurface"
  ) {
    return GLOBAL_SHORTCUTS[id];
  }
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

  if (!context.isMainWindow) return null;

  // Surface switching is checked before the Timeline-only gate below: it has to
  // work *from* the other surfaces (⌘1 is how you get back, including from
  // Settings), and from inside text fields — ⌘/Ctrl+digit steals nothing from
  // text editing, unlike the bare 1/2/3 source toggles.
  if (matchShortcut(event, effectiveShortcut("openTimelineSurface"), platform)) {
    return { type: "openSurface", surface: "timeline" };
  }
  if (matchShortcut(event, effectiveShortcut("openOverviewSurface"), platform)) {
    return { type: "openSurface", surface: "overview" };
  }

  if (!context.isMainRoute) return null;

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
