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
import { isMainAppRoute, normalizeAppPathname } from "$lib/route-path";

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
  | "license"
  | "appearance"
  | "developer"
  | "behavior"
  | "keyboard"
  | "keyboard-shortcuts"
  | "microphone"
  | "ocr"
  | "transcription"
  | "speakers"
  | "semanticSearch"
  | "userContext";

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
    case "reasoning-engine":
    case "ai":
    case "ai-runtime":
      return "intelligence";
    // User Context has its own Intelligence-group section, so it deep-links 1:1
    // (the page resolves "userContext" to that section) rather than collapsing
    // onto Providers.
    case "user-context":
    case "userContext":
      return "userContext";
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
    // Granular processing sub-tabs pass through so a notification targeting
    // (say) transcription lands on the transcription section rather than being
    // collapsed to the legacy "processing" tab (which the page resolves to OCR).
    case "ocr":
      return "ocr";
    case "transcription":
      return "transcription";
    case "speakers":
      return "speakers";
    case "semanticSearch":
    case "semantic-search":
      return "semanticSearch";
    // Legacy "processing" alias kept for back-compat; the page maps it to OCR.
    case "processing":
      return "processing";
    case "storage":
      return "storage";
    case "license":
      return "license";
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

// The last main surface (Timeline `/` or Insights `/insights`) the user was on
// before entering Settings, so the settings rail's "← Back to app" can return
// there instead of always landing on the home shell. Only known main surfaces
// are accepted; anything else (`/settings`, `/onboarding`, …) is rejected and we
// keep the `/insights` fallback (the story-first home). Survives navigations
// because it's module-level, not route state; resets on a cold load (e.g. tray
// deeplink straight into Settings), which is the desired fallback.
let lastMainSurfacePath = "/insights";

/** Is `pathname` one of the main app surfaces (Timeline, Insights, Triggers)? */
function isMainSurface(pathname: string): boolean {
  const normalized = normalizeAppPathname(pathname);
  return (
    isMainAppRoute(pathname) ||
    normalized.startsWith("/insights") ||
    normalized.startsWith("/triggers")
  );
}

/**
 * Record the main surface the user is leaving, so the settings rail knows where
 * to return. No-ops for any path that isn't a known main surface, leaving the
 * `/` fallback in place.
 */
export function recordMainSurface(path: string): void {
  if (isMainSurface(path)) lastMainSurfacePath = normalizeAppPathname(path);
}

/** The last recorded main surface path, falling back to the home shell
 *  (`/insights`). */
export function getLastMainSurface(): string {
  return lastMainSurfacePath;
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
    // Remember where we came from so "← Back to app" returns to it. The other
    // branch (focusing Main from a non-main window) has no meaningful previous
    // main surface, so we leave the stored fallback untouched.
    recordMainSurface(window.location.pathname);
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
