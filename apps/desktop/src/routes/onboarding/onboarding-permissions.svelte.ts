// Onboarding permissions + Gecko browser-URL access subsystem.
//
// The capture-permission probing/requesting and the optional macOS
// Accessibility (Gecko/Firefox/Zen URL) access flow live here so the main
// `onboarding.svelte.ts` controller stays under the 800-line file cap. Like the
// model stores in `onboarding-models.svelte.ts`, this is a behavior-preserving
// re-home: a FACTORY whose only outward coupling is a `setError` callback (the
// controller's single shared `errorMessage` field). The controller composes one
// instance and re-exposes its members flat so body components keep their
// verbatim `controller.requestPermission(...)` / `controller.permissions` sites.
import { invoke } from "@tauri-apps/api/core";
import type {
  BrowserUrlAccessibilityStatus,
  GetPermissionsResponse,
} from "$lib/types";
import { serializeError } from "./onboarding-mapping";
import {
  permissionActionFor,
  permissionLabelFor,
  permissionToneFor,
} from "./onboarding-attention";
import type { PermissionKey, PermissionValue } from "./onboarding-attention";

interface OnboardingPermissionsDeps {
  /** Write the controller's shared error banner (null clears it). */
  setError: (message: string | null) => void;
}

export function createOnboardingPermissionsStore(deps: OnboardingPermissionsDeps) {
  let permissions = $state<Record<PermissionKey, PermissionValue> | null>(null);
  let requestingPerm = $state<PermissionKey | null>(null);
  let refreshingPerms = $state(false);

  // Optional Gecko (Firefox/Zen) browser-URL access via the macOS Accessibility
  // API. Shown only when a Gecko browser is installed; the status is non-fatal (a
  // null probe simply hides the row) and never gates onboarding progression.
  let geckoUrlAccess = $state<BrowserUrlAccessibilityStatus | null>(null);
  let requestingGeckoAccess = $state(false);
  let recheckingGeckoAccess = $state(false);

  // "assumed_working" is system audio's granted (ADR 0052): a tap that has
  // delivered sound is as good as a read grant, and it is the strongest answer
  // that permission can ever give.
  const grantedCount = $derived(
    permissions === null
      ? 0
      : (["screen", "microphone", "systemAudio"] as const).filter(
          (k) => permissions?.[k] === "granted" || permissions?.[k] === "assumed_working",
        ).length,
  );
  const geckoInstalled = $derived(
    (geckoUrlAccess?.geckoBrowsers ?? []).some((b) => b.installed),
  );
  const geckoTrusted = $derived(geckoUrlAccess?.trusted ?? false);
  const geckoInstalledNames = $derived(
    (geckoUrlAccess?.geckoBrowsers ?? []).filter((b) => b.installed).map((b) => b.displayName),
  );

  async function refreshPermissions(): Promise<void> {
    deps.setError(null);
    refreshingPerms = true;
    try {
      const response = await invoke<GetPermissionsResponse>("get_capture_permissions");
      permissions = response.permissions as Record<PermissionKey, PermissionValue>;
    } catch (err) {
      deps.setError(serializeError(err));
    } finally {
      refreshingPerms = false;
    }
  }

  // Granted/unsupported need no action. macOS won't re-prompt once denied, so
  // those rows deep-link to System Settings instead of re-requesting.
  function permissionAction(
    value: PermissionValue | undefined,
  ): { label: string; mode: "request" | "settings" } | null {
    return permissionActionFor(value);
  }

  async function requestPermission(key: PermissionKey): Promise<void> {
    if (requestingPerm) return;
    deps.setError(null);
    requestingPerm = key;
    try {
      const action = permissionAction(permissions?.[key]);
      if (action?.mode === "settings") {
        await invoke("open_capture_privacy_settings", { kind: key });
      } else {
        const response = await invoke<GetPermissionsResponse>("request_capture_permission", { kind: key });
        permissions = response.permissions as Record<PermissionKey, PermissionValue>;
      }
    } catch (err) {
      deps.setError(serializeError(err));
    } finally {
      requestingPerm = null;
    }
  }

  function permissionLabel(value: PermissionValue | undefined): string {
    return permissionLabelFor(value);
  }

  function permissionTone(value: PermissionValue | undefined): "ok" | "pending" | "blocked" {
    return permissionToneFor(value);
  }

  // Probe whether a Gecko browser (Firefox/Zen) is installed and whether Mnema is
  // trusted for the macOS Accessibility API used to read its active-tab URL.
  // Non-fatal: a failure leaves the status null so the optional row simply hides.
  async function loadGeckoUrlAccess(): Promise<void> {
    try {
      geckoUrlAccess = await invoke<BrowserUrlAccessibilityStatus>("get_browser_url_accessibility_status");
    } catch {
      geckoUrlAccess = null;
    }
  }

  // Raises the macOS Accessibility prompt (and adds Mnema to the list). The grant
  // is completed by the user in System Settings, so `trusted` usually stays false
  // here until they enable Mnema and we re-poll via recheck.
  async function requestGeckoAccess(): Promise<void> {
    if (requestingGeckoAccess) return;
    deps.setError(null);
    requestingGeckoAccess = true;
    try {
      geckoUrlAccess = await invoke<BrowserUrlAccessibilityStatus>("request_browser_url_accessibility");
    } catch (err) {
      deps.setError(serializeError(err));
    } finally {
      requestingGeckoAccess = false;
    }
  }

  async function openGeckoAccessSettings(): Promise<void> {
    deps.setError(null);
    try {
      await invoke("open_browser_url_accessibility_settings");
    } catch (err) {
      deps.setError(serializeError(err));
    }
  }

  async function recheckGeckoAccess(): Promise<void> {
    if (recheckingGeckoAccess) return;
    deps.setError(null);
    recheckingGeckoAccess = true;
    try {
      geckoUrlAccess = await invoke<BrowserUrlAccessibilityStatus>("get_browser_url_accessibility_status");
    } catch (err) {
      deps.setError(serializeError(err));
    } finally {
      recheckingGeckoAccess = false;
    }
  }

  return {
    get permissions() {
      return permissions;
    },
    set permissions(value: Record<PermissionKey, PermissionValue> | null) {
      permissions = value;
    },
    get requestingPerm() {
      return requestingPerm;
    },
    get refreshingPerms() {
      return refreshingPerms;
    },
    get geckoUrlAccess() {
      return geckoUrlAccess;
    },
    get requestingGeckoAccess() {
      return requestingGeckoAccess;
    },
    get recheckingGeckoAccess() {
      return recheckingGeckoAccess;
    },
    get grantedCount() {
      return grantedCount;
    },
    get geckoInstalled() {
      return geckoInstalled;
    },
    get geckoTrusted() {
      return geckoTrusted;
    },
    get geckoInstalledNames() {
      return geckoInstalledNames;
    },
    refreshPermissions,
    permissionAction,
    requestPermission,
    permissionLabel,
    permissionTone,
    loadGeckoUrlAccess,
    requestGeckoAccess,
    openGeckoAccessSettings,
    recheckGeckoAccess,
  };
}
