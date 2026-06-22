// Thin frontend seam over the Tauri commands that own dedicated reusable
// windows for auxiliary surfaces. Rust remains the single owner of window
// creation, focus, and close semantics so future windows can diverge in one
// place without duplicating webview logic in the frontend.
//
// Settings is no longer a dedicated window: it renders as the `/settings` route
// inside the Main window. `openSettings` therefore navigates in-window when the
// caller already lives in Main, and otherwise asks Rust to focus the Main window
// and emit the `open_settings_tab` deeplink (which the Main layout turns into a
// `/settings` navigation).

import { invoke } from "@tauri-apps/api/core";
import { goto } from "$app/navigation";
import { getCurrentWindow } from "@tauri-apps/api/window";

export type SurfaceWindowLabel = "main" | "onboarding" | "cli-access-request" | "debug" | "quick-recall";

export type SettingsWindowTab =
  | "about"
  | "capture"
  | "video"
  | "access"
  | "intelligence"
  | "privacy"
  | "shortcuts"
  | "audio"
  | "processing"
  | "storage"
  | "appearance"
  | "developer"
  | "behavior"
  | "keyboard"
  | "keyboard-shortcuts"
  | "microphone"
  | "ocr"
  | "transcription"
  | "speakers";

export type SettingsWindowFocus = "agentAccess" | "cliAccess";

// Canonical settings tabs the `/settings` route understands. The deeplink
// contract (aliases → canonical) is mirrored by `normalize_settings_tab` in
// Rust and `normalizeSettingsTab` in the settings page; this normalizer only
// needs to map the inbound aliases callers actually use so the URL query stays
// on a canonical tab. Unknown values fall through (route shows its default tab).
export function normalizeSettingsTab(tab?: SettingsWindowTab | string | null): string | null {
  switch (tab) {
    case "about":
      return "about";
    case "capture":
    case "behavior":
      return "capture";
    case "access":
    case "cliAccess":
    case "cli-access":
      return "access";
    case "intelligence":
    case "reasoning":
    case "ai":
    case "user-context":
      return "intelligence";
    case "privacy":
    case "metadata":
      return "privacy";
    case "shortcuts":
    case "keyboard":
    case "keyboard-shortcuts":
    case "keyboard_bindings":
      return "shortcuts";
    case "video":
      return "video";
    case "audio":
    case "microphone":
      return "audio";
    case "processing":
    case "ocr":
    case "transcription":
    case "speakers":
      return "processing";
    case "storage":
      return "storage";
    case "appearance":
      return "appearance";
    case "developer":
      return "developer";
    default:
      return null;
  }
}

export function normalizeSettingsFocus(focus?: SettingsWindowFocus | string | null): string | null {
  switch (focus) {
    case "agentAccess":
    case "agent-access":
    case "cliAccess":
    case "cli-access":
      return "cliAccess";
    default:
      return null;
  }
}

/** Build a `/settings` URL with a normalized `?tab`/`?focus` query. */
export function settingsRoutePath(tab?: SettingsWindowTab, focus?: SettingsWindowFocus): string {
  const params = new URLSearchParams();
  const normalizedTab = normalizeSettingsTab(tab);
  if (normalizedTab) params.set("tab", normalizedTab);
  const normalizedFocus = normalizeSettingsFocus(focus);
  if (normalizedFocus) params.set("focus", normalizedFocus);
  const query = params.toString();
  return query ? `/settings?${query}` : "/settings";
}

/**
 * Open the Settings surface. From the Main window this is an in-window route
 * navigation; from any other window (e.g. Quick Recall) it asks Rust to focus
 * the Main window and emit the `open_settings_tab` deeplink, which the Main
 * layout turns into the same `/settings` navigation.
 */
export async function openSettings(
  tab?: SettingsWindowTab,
  focus?: SettingsWindowFocus,
): Promise<void> {
  if (currentWindowLabel() === "main") {
    await goto(settingsRoutePath(tab, focus));
    return;
  }
  await invoke("focus_main_and_open_settings", { tab, focus });
}

export async function openDebugWindow(): Promise<void> {
  await invoke("open_debug_window");
}

/**
 * Returns the label of the window the calling code is running inside. Used
 * by the shared desktop chrome to decide whether it should render dedicated
 * window controls or main-window actions.
 */
export function currentWindowLabel(): SurfaceWindowLabel | string {
  try {
    return getCurrentWindow().label;
  } catch {
    return "main";
  }
}

export function isDedicatedSurfaceWindow(): boolean {
  const label = currentWindowLabel();
  return label === "onboarding" || label === "cli-access-request" || label === "debug";
}

export function isQuickRecallWindow(): boolean {
  return currentWindowLabel() === "quick-recall";
}

export async function closeCurrentWindow(): Promise<void> {
  await invoke("close_current_window");
}
