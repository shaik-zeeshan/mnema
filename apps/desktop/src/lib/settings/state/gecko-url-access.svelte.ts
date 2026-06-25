// Optional Gecko (Firefox/Zen) browser-URL access state.
//
// Gecko browsers expose no scriptable active-tab URL like Chrome/Safari, so
// reading their page address needs the macOS Accessibility permission, which the
// user grants outside the app in System Settings. This store owns its own
// non-draft reactive state (the probe result + in-flight latches) and the
// probe/request/open-settings/recheck invokes. Surfaced in the capture Privacy
// panel only when a Gecko browser is installed; a failed probe leaves the status
// null so the row simply hides — it never gates anything.

import { invoke } from "@tauri-apps/api/core";
import type { BrowserUrlAccessibilityStatus } from "$lib/types";
import { errorText } from "./format";

export function createGeckoUrlAccessStore() {
  let status = $state<BrowserUrlAccessibilityStatus | null>(null);
  let requesting = $state(false);
  let rechecking = $state(false);
  let error = $state<string | null>(null);

  // Probe whether a Gecko browser is installed and whether Mnema is trusted for
  // the Accessibility API. Non-fatal: a failure leaves the status null (row hides).
  async function load() {
    try {
      status = await invoke<BrowserUrlAccessibilityStatus>("get_browser_url_accessibility_status");
    } catch {
      status = null;
    }
  }

  // Raises the macOS Accessibility prompt (and adds Mnema to the list). The grant
  // is completed by the user in System Settings, so `trusted` usually stays false
  // here until they enable Mnema and we re-poll via recheck.
  async function request() {
    if (requesting) return;
    error = null;
    requesting = true;
    try {
      status = await invoke<BrowserUrlAccessibilityStatus>("request_browser_url_accessibility");
    } catch (err) {
      error = errorText(err);
    } finally {
      requesting = false;
    }
  }

  async function openSettings() {
    error = null;
    try {
      await invoke("open_browser_url_accessibility_settings");
    } catch (err) {
      error = errorText(err);
    }
  }

  async function recheck() {
    if (rechecking) return;
    error = null;
    rechecking = true;
    try {
      status = await invoke<BrowserUrlAccessibilityStatus>("get_browser_url_accessibility_status");
    } catch (err) {
      error = errorText(err);
    } finally {
      rechecking = false;
    }
  }

  return {
    get installed() { return (status?.geckoBrowsers ?? []).some((b) => b.installed); },
    get trusted() { return status?.trusted ?? false; },
    get installedNames() {
      return (status?.geckoBrowsers ?? []).filter((b) => b.installed).map((b) => b.displayName);
    },
    get requesting() { return requesting; },
    get rechecking() { return rechecking; },
    get error() { return error; },
    load,
    request,
    openSettings,
    recheck,
  };
}

export type GeckoUrlAccessStore = ReturnType<typeof createGeckoUrlAccessStore>;
