// Thin frontend seam over the Tauri commands that own dedicated reusable
// windows for auxiliary surfaces. Rust remains the single owner of window
// creation, focus, and close semantics so future windows can diverge in one
// place without duplicating webview logic in the frontend.

import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

export type SurfaceWindowLabel = "main" | "onboarding" | "settings" | "debug";

export type SettingsWindowTab =
  | "capture"
  | "video"
  | "audio"
  | "processing"
  | "storage"
  | "appearance"
  | "developer"
  | "behavior"
  | "microphone"
  | "ocr"
  | "transcription"
  | "speakers";

export async function openSettingsWindow(tab?: SettingsWindowTab): Promise<void> {
  if (tab) {
    await invoke("open_settings_window_to_tab", { tab });
    return;
  }
  await invoke("open_settings_window");
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
  return label === "onboarding" || label === "settings" || label === "debug";
}

export async function closeCurrentWindow(): Promise<void> {
  await invoke("close_current_window");
}
