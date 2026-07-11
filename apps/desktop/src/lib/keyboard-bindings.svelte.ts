import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { KeyboardBindingsSettings } from "$lib/types";
import { humanizeError } from "$lib/format-error";
export {
  normalizeShortcutBinding,
  parseShortcutBinding,
  shortcutBindingFromKeyboardEvent,
  shortcutDefinitionWithBinding,
} from "$lib/keyboard-binding-utils";

const KEYBOARD_BINDINGS_CHANGED_EVENT = "keyboard_bindings_settings_changed";

export const DEFAULT_KEYBOARD_BINDINGS: KeyboardBindingsSettings = {
  schemaVersion: 1,
  globalShortcuts: {
    enabled: true,
    bindings: {
      toggleRecording: "CommandOrControl+Alt+R",
      pauseResumeRecording: "CommandOrControl+Alt+P",
      toggleMainWindow: "CommandOrControl+Alt+M",
      quickRecall: "CommandOrControl+Alt+Space",
    },
  },
  appShortcuts: {
    openSettings: "CommandOrControl+,",
    openDebug: "CommandOrControl+D",
    toggleSourceScreen: "1",
    toggleSourceMicrophone: "2",
    toggleSourceSystemAudio: "3",
    toggleShortcutsHelp: "/",
  },
  dashboardShortcuts: {
    openJumpPicker: "J",
    jumpLatest: "L",
    toggleOcr: "O",
    refreshTimeline: "R",
    copyFrame: "C",
    downloadFrame: "D",
  },
  audioDrawerShortcuts: {
    playPause: "Space",
    seekBack: "ArrowLeft",
    seekForward: "ArrowRight",
    seekBackFast: "Shift+ArrowLeft",
    seekForwardFast: "Shift+ArrowRight",
  },
};

export type EditableShortcutActionId =
  | "toggleRecording"
  | "pauseResumeRecording"
  | "toggleMainWindow"
  | "toggleQuickRecall"
  | "openSettings"
  | "openDebug"
  | "toggleSourceScreen"
  | "toggleSourceMicrophone"
  | "toggleSourceSystemAudio"
  | "toggleShortcutsHelp"
  | "dashboard.openJumpPicker"
  | "dashboard.jumpLatest"
  | "dashboard.toggleOcr"
  | "dashboard.refreshTimeline"
  | "dashboard.copyFrame"
  | "dashboard.downloadFrame"
  | "audioDrawer.playPause"
  | "audioDrawer.seekBack"
  | "audioDrawer.seekForward"
  | "audioDrawer.seekBackFast"
  | "audioDrawer.seekForwardFast";

export type ShortcutCategory = "global" | "app" | "dashboard" | "audioDrawer";

type ShortcutConflictScope = "global" | "foreground" | "dashboard" | "audioDrawer";
type ReservedShortcutScope = "foreground" | "dashboard";

export type ReservedShortcutBinding = {
  scope: ReservedShortcutScope;
  binding: string;
  label: string;
};

export type EditableShortcutAction = {
  id: EditableShortcutActionId;
  label: string;
  description: string;
  category: ShortcutCategory;
  nativeBackground: boolean;
};

export function shortcutConflictScope(action: EditableShortcutAction): ShortcutConflictScope {
  if (action.nativeBackground) return "global";
  if (action.category === "dashboard") return "dashboard";
  if (action.category === "audioDrawer") return "audioDrawer";
  return "foreground";
}

export function shortcutScopesConflict(
  left: ShortcutConflictScope,
  right: ShortcutConflictScope,
): boolean {
  return left === "global"
    || right === "global"
    || left === "foreground"
    || right === "foreground"
    || left === right;
}

export const RESERVED_SHORTCUT_BINDINGS: ReservedShortcutBinding[] = [
  { scope: "foreground", binding: "Escape", label: "close the active surface" },
  { scope: "foreground", binding: "Tab", label: "move keyboard focus" },
  { scope: "foreground", binding: "Shift+Tab", label: "move keyboard focus backward" },
  { scope: "dashboard", binding: "ArrowLeft", label: "move to an older frame" },
  { scope: "dashboard", binding: "ArrowRight", label: "move to a newer frame" },
  { scope: "dashboard", binding: "Shift+ArrowLeft", label: "move 10 frames older" },
  { scope: "dashboard", binding: "Shift+ArrowRight", label: "move 10 frames newer" },
];

export function reservedShortcutConflict(
  action: EditableShortcutAction,
  normalizedBinding: string,
): ReservedShortcutBinding | null {
  const key = normalizedBinding.toLowerCase();
  return RESERVED_SHORTCUT_BINDINGS.find((reserved) => {
    if (reserved.binding.toLowerCase() !== key) return false;
    if (reserved.scope === "foreground") return action.category !== "global";
    return reserved.scope === action.category;
  }) ?? null;
}

export const EDITABLE_SHORTCUT_ACTIONS: EditableShortcutAction[] = [
  { id: "toggleRecording", label: "Start or stop recording", description: "Works even when Mnema is in the background.", category: "global", nativeBackground: true },
  { id: "pauseResumeRecording", label: "Pause or resume recording", description: "Pauses or resumes User Capture Pause without stopping the session.", category: "global", nativeBackground: true },
  { id: "toggleMainWindow", label: "Show or hide Mnema", description: "Works even when Mnema is in the background.", category: "global", nativeBackground: true },
  { id: "toggleQuickRecall", label: "Summon Quick Recall", description: "Works even when Mnema is in the background.", category: "global", nativeBackground: true },
  { id: "openSettings", label: "Open settings", description: "Foreground app shortcut.", category: "app", nativeBackground: false },
  { id: "openDebug", label: "Open debug", description: "Visible when developer options are enabled.", category: "app", nativeBackground: false },
  { id: "toggleSourceScreen", label: "Toggle screen for the next recording", description: "Available while recording is stopped.", category: "app", nativeBackground: false },
  { id: "toggleSourceMicrophone", label: "Toggle microphone for the next recording", description: "Available while recording is stopped.", category: "app", nativeBackground: false },
  { id: "toggleSourceSystemAudio", label: "Toggle system audio for the next recording", description: "Available while recording is stopped.", category: "app", nativeBackground: false },
  { id: "toggleShortcutsHelp", label: "Show keyboard shortcuts", description: "Opens the shortcut help overlay.", category: "app", nativeBackground: false },
  { id: "dashboard.openJumpPicker", label: "Open jump picker", description: "Dashboard timeline navigation.", category: "dashboard", nativeBackground: false },
  { id: "dashboard.jumpLatest", label: "Jump to latest", description: "Move to the newest available frame.", category: "dashboard", nativeBackground: false },
  { id: "dashboard.toggleOcr", label: "Toggle OCR panel", description: "Show or hide OCR for the active frame.", category: "dashboard", nativeBackground: false },
  { id: "dashboard.refreshTimeline", label: "Refresh timeline", description: "Reload timeline and audio data.", category: "dashboard", nativeBackground: false },
  { id: "dashboard.copyFrame", label: "Copy active frame image", description: "Copies the current frame preview.", category: "dashboard", nativeBackground: false },
  { id: "dashboard.downloadFrame", label: "Download active frame image", description: "Downloads the current frame preview.", category: "dashboard", nativeBackground: false },
  { id: "audioDrawer.playPause", label: "Play or pause", description: "Audio drawer playback.", category: "audioDrawer", nativeBackground: false },
  { id: "audioDrawer.seekBack", label: "Seek back 5 seconds", description: "Audio drawer playback.", category: "audioDrawer", nativeBackground: false },
  { id: "audioDrawer.seekForward", label: "Seek forward 5 seconds", description: "Audio drawer playback.", category: "audioDrawer", nativeBackground: false },
  { id: "audioDrawer.seekBackFast", label: "Seek back 30 seconds", description: "Audio drawer playback.", category: "audioDrawer", nativeBackground: false },
  { id: "audioDrawer.seekForwardFast", label: "Seek forward 30 seconds", description: "Audio drawer playback.", category: "audioDrawer", nativeBackground: false },
];

const _state = $state<{
  settings: KeyboardBindingsSettings;
  loaded: boolean;
  loading: boolean;
  error: string | null;
}>({
  settings: structuredClone(DEFAULT_KEYBOARD_BINDINGS),
  loaded: false,
  loading: false,
  error: null,
});

let _initialized = false;

export const keyboardBindings = {
  get settings(): KeyboardBindingsSettings {
    return _state.settings;
  },
  get loaded(): boolean {
    return _state.loaded;
  },
  get loading(): boolean {
    return _state.loading;
  },
  get error(): string | null {
    return _state.error;
  },
};

function serializeError(err: unknown): string {
  return humanizeError(err);
}

export function initKeyboardBindings(): void {
  if (_initialized || typeof window === "undefined") return;
  _initialized = true;
  void listen<KeyboardBindingsSettings>(KEYBOARD_BINDINGS_CHANGED_EVENT, (event) => {
    _state.settings = withKeyboardBindingDefaults(event.payload);
    _state.loaded = true;
    _state.error = null;
  });
  void loadKeyboardBindings();
}

export async function loadKeyboardBindings(): Promise<KeyboardBindingsSettings> {
  _state.loading = true;
  try {
    const settings = await invoke<KeyboardBindingsSettings>("get_keyboard_bindings_settings");
    _state.settings = withKeyboardBindingDefaults(settings);
    _state.loaded = true;
    _state.error = null;
    return _state.settings;
  } catch (err) {
    _state.error = serializeError(err);
    throw err;
  } finally {
    _state.loading = false;
  }
}

export function withKeyboardBindingDefaults(settings: Partial<KeyboardBindingsSettings> | null | undefined): KeyboardBindingsSettings {
  return {
    schemaVersion: settings?.schemaVersion ?? DEFAULT_KEYBOARD_BINDINGS.schemaVersion,
    globalShortcuts: {
      enabled: settings?.globalShortcuts?.enabled ?? DEFAULT_KEYBOARD_BINDINGS.globalShortcuts.enabled,
      bindings: {
        ...DEFAULT_KEYBOARD_BINDINGS.globalShortcuts.bindings,
        ...(settings?.globalShortcuts?.bindings ?? {}),
      },
    },
    appShortcuts: {
      ...DEFAULT_KEYBOARD_BINDINGS.appShortcuts,
      ...(settings?.appShortcuts ?? {}),
    },
    dashboardShortcuts: {
      ...DEFAULT_KEYBOARD_BINDINGS.dashboardShortcuts,
      ...(settings?.dashboardShortcuts ?? {}),
    },
    audioDrawerShortcuts: {
      ...DEFAULT_KEYBOARD_BINDINGS.audioDrawerShortcuts,
      ...(settings?.audioDrawerShortcuts ?? {}),
    },
  };
}

export function getShortcutBinding(settings: KeyboardBindingsSettings, id: EditableShortcutActionId): string {
  switch (id) {
    case "toggleRecording": return settings.globalShortcuts.bindings.toggleRecording;
    case "pauseResumeRecording": return settings.globalShortcuts.bindings.pauseResumeRecording;
    case "toggleMainWindow": return settings.globalShortcuts.bindings.toggleMainWindow;
    case "toggleQuickRecall": return settings.globalShortcuts.bindings.quickRecall;
    case "openSettings": return settings.appShortcuts.openSettings;
    case "openDebug": return settings.appShortcuts.openDebug;
    case "toggleSourceScreen": return settings.appShortcuts.toggleSourceScreen;
    case "toggleSourceMicrophone": return settings.appShortcuts.toggleSourceMicrophone;
    case "toggleSourceSystemAudio": return settings.appShortcuts.toggleSourceSystemAudio;
    case "toggleShortcutsHelp": return settings.appShortcuts.toggleShortcutsHelp;
    case "dashboard.openJumpPicker": return settings.dashboardShortcuts.openJumpPicker;
    case "dashboard.jumpLatest": return settings.dashboardShortcuts.jumpLatest;
    case "dashboard.toggleOcr": return settings.dashboardShortcuts.toggleOcr;
    case "dashboard.refreshTimeline": return settings.dashboardShortcuts.refreshTimeline;
    case "dashboard.copyFrame": return settings.dashboardShortcuts.copyFrame;
    case "dashboard.downloadFrame": return settings.dashboardShortcuts.downloadFrame;
    case "audioDrawer.playPause": return settings.audioDrawerShortcuts.playPause;
    case "audioDrawer.seekBack": return settings.audioDrawerShortcuts.seekBack;
    case "audioDrawer.seekForward": return settings.audioDrawerShortcuts.seekForward;
    case "audioDrawer.seekBackFast": return settings.audioDrawerShortcuts.seekBackFast;
    case "audioDrawer.seekForwardFast": return settings.audioDrawerShortcuts.seekForwardFast;
  }
}

export function setShortcutBinding(settings: KeyboardBindingsSettings, id: EditableShortcutActionId, binding: string): KeyboardBindingsSettings {
  const next = structuredClone(settings);
  switch (id) {
    case "toggleRecording": next.globalShortcuts.bindings.toggleRecording = binding; break;
    case "pauseResumeRecording": next.globalShortcuts.bindings.pauseResumeRecording = binding; break;
    case "toggleMainWindow": next.globalShortcuts.bindings.toggleMainWindow = binding; break;
    case "toggleQuickRecall": next.globalShortcuts.bindings.quickRecall = binding; break;
    case "openSettings": next.appShortcuts.openSettings = binding; break;
    case "openDebug": next.appShortcuts.openDebug = binding; break;
    case "toggleSourceScreen": next.appShortcuts.toggleSourceScreen = binding; break;
    case "toggleSourceMicrophone": next.appShortcuts.toggleSourceMicrophone = binding; break;
    case "toggleSourceSystemAudio": next.appShortcuts.toggleSourceSystemAudio = binding; break;
    case "toggleShortcutsHelp": next.appShortcuts.toggleShortcutsHelp = binding; break;
    case "dashboard.openJumpPicker": next.dashboardShortcuts.openJumpPicker = binding; break;
    case "dashboard.jumpLatest": next.dashboardShortcuts.jumpLatest = binding; break;
    case "dashboard.toggleOcr": next.dashboardShortcuts.toggleOcr = binding; break;
    case "dashboard.refreshTimeline": next.dashboardShortcuts.refreshTimeline = binding; break;
    case "dashboard.copyFrame": next.dashboardShortcuts.copyFrame = binding; break;
    case "dashboard.downloadFrame": next.dashboardShortcuts.downloadFrame = binding; break;
    case "audioDrawer.playPause": next.audioDrawerShortcuts.playPause = binding; break;
    case "audioDrawer.seekBack": next.audioDrawerShortcuts.seekBack = binding; break;
    case "audioDrawer.seekForward": next.audioDrawerShortcuts.seekForward = binding; break;
    case "audioDrawer.seekBackFast": next.audioDrawerShortcuts.seekBackFast = binding; break;
    case "audioDrawer.seekForwardFast": next.audioDrawerShortcuts.seekForwardFast = binding; break;
  }
  return next;
}

