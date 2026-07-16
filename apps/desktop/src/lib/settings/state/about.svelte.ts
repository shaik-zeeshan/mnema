// About-panel state: app-update status/actions, third-party notices, and the
// about-details copy flow. Owns its own non-draft reactive state plus the
// invokes; pure label/format helpers are exported standalone for the panel.

import { invoke } from "@tauri-apps/api/core";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { openUrl } from "@tauri-apps/plugin-opener";
import type {
  AppUpdateChannel,
  AppUpdateStatus,
  ThirdPartyNotices,
} from "$lib/types";
import { describeError, formatBytes } from "./format";

// ── Pure label/format helpers ───────────────────────────────────────────────

export function appUpdateStateLabel(status: AppUpdateStatus | null): string {
  if (!status) return "Loading";
  if (status.recordingActive && (status.state === "available" || status.state === "recordingBlocked")) {
    return "Recording active";
  }
  switch (status.state) {
    case "idle": return "Not checked";
    case "checking": return "Checking";
    case "upToDate": return "Up to date";
    case "available": return "Update available";
    case "availableOutOfWindow": return "Outside update window";
    case "downloading": return "Downloading";
    case "installing": return "Installing";
    case "restartRequired": return "Restart required";
    case "recordingBlocked": return "Recording active";
    case "incompatible": return "Incompatible";
    case "failed": return "Failed";
    default: return "Unknown";
  }
}

export function appUpdateStatusMessage(status: AppUpdateStatus | null): string {
  if (!status) return "Loading update status.";
  if (status.recordingActive && (status.state === "available" || status.state === "recordingBlocked")) {
    return "Stop recording to install this update.";
  }
  if (status.error?.message) return status.error.message;
  switch (status.state) {
    case "idle": return "Mnema has not checked for updates in this app session.";
    case "checking": return "Checking the selected update channel.";
    case "upToDate": return "Mnema is current on the selected channel.";
    case "available": return `Version ${status.update?.version ?? "newer"} is ready to download and install.`;
    case "availableOutOfWindow":
      // Two triggers, one state: a newer remote build past the window (update present),
      // or the running build itself past the window (fresh install after lapse).
      return status.update
        ? `Version ${status.update.version} is past your update window. Renew to receive new builds — your current version keeps working forever.`
        : "You're on a build newer than your update window. Get the newest build your license covers, or renew to receive new builds. Your recordings are untouched.";
    case "downloading": return "Downloading the update package.";
    case "installing": return "Installing the update. Keep Mnema open until this finishes.";
    case "restartRequired": return "Restart Mnema when you are ready to finish updating.";
    case "incompatible": return "No compatible update is available for this Mac.";
    case "failed": return "The last update operation failed. You can retry.";
    default: return "Update status is unavailable.";
  }
}

export function updateChannelLabel(channel: AppUpdateChannel | null | undefined): string {
  return channel === "preview" ? "Preview" : "Stable";
}

export function platformLabel(status: AppUpdateStatus | null): string {
  if (!status) return "Unknown";
  const os = status.app.platform === "macos" ? "macOS" : status.app.platform;
  const arch = status.app.arch === "aarch64" ? "Apple Silicon" : status.app.arch;
  return `${os} · ${arch}`;
}

// A single-line, paste-ready summary for bug reports: product, version, build
// target, and bundle identifier.
export function aboutDetailsText(status: AppUpdateStatus | null): string {
  const app = status?.app;
  const product = app?.productName ?? "mnema";
  const version = app?.version ?? "unknown";
  const target = app ? `${app.platform}/${app.arch}` : "unknown";
  const identifier = app?.identifier ?? "unknown";
  return `${product} ${version} (${target}) ${identifier}`;
}

export function formatUpdateDate(value: string | null | undefined): string | null {
  if (!value) return null;
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleDateString(undefined, { year: "numeric", month: "short", day: "numeric" });
}

export function formatCheckedAt(value: number | null | undefined): string {
  if (!value) return "Not checked yet";
  return new Date(value).toLocaleString(undefined, {
    month: "short", day: "numeric", hour: "numeric", minute: "2-digit",
  });
}

export function appUpdateProgressText(status: AppUpdateStatus | null): string {
  const progress = status?.progress;
  if (!progress) return "";
  if (progress.contentLengthBytes) {
    return `${formatBytes(progress.downloadedBytes)} of ${formatBytes(progress.contentLengthBytes)}`;
  }
  return `${formatBytes(progress.downloadedBytes)} downloaded`;
}

export function appUpdateProgressPercent(status: AppUpdateStatus | null): number {
  const progress = status?.progress;
  if (!progress?.contentLengthBytes) return 8;
  const percent = (progress.downloadedBytes / progress.contentLengthBytes) * 100;
  return Math.max(4, Math.min(100, percent));
}

// ── Reactive store ──────────────────────────────────────────────────────────

export function createAboutStore() {
  let appUpdateStatus = $state<AppUpdateStatus | null>(null);
  let checkingAppUpdate = $state(false);
  let switchingAppUpdateChannel = $state(false);
  let installingAppUpdate = $state(false);
  let restartingAfterUpdate = $state(false);
  let appUpdateActionError = $state<string | null>(null);
  let previewConfirmationVisible = $state(false);

  // About panel: transient "Copied" confirmation plus a local error slot for the
  // copy/open-link actions (kept separate from update-action errors).
  let aboutDetailsCopied = $state(false);
  let aboutActionError = $state<string | null>(null);
  let aboutDetailsCopiedTimer: ReturnType<typeof setTimeout> | null = null;

  // Acknowledgements: third-party model attribution assembled by the backend.
  let thirdPartyNotices = $state<ThirdPartyNotices | null>(null);
  let loadingThirdPartyNotices = $state(false);
  let thirdPartyNoticesError = $state<string | null>(null);
  let thirdPartyNoticesCopied = $state(false);
  let thirdPartyNoticesCopiedTimer: ReturnType<typeof setTimeout> | null = null;

  // Entries grouped by `kind`, preserving the backend's category order as kinds
  // are first seen.
  const thirdPartyNoticeGroups = $derived.by(() => {
    const groups: { kind: string; entries: ThirdPartyNotices["entries"] }[] = [];
    for (const entry of thirdPartyNotices?.entries ?? []) {
      let group = groups.find((g) => g.kind === entry.kind);
      if (!group) {
        group = { kind: entry.kind, entries: [] };
        groups.push(group);
      }
      group.entries.push(entry);
    }
    return groups;
  });

  // Depends only on reactive state, so these read live values.
  const canInstallAppUpdate = $derived(
    !!appUpdateStatus?.update
      && (appUpdateStatus.state === "available"
        || appUpdateStatus.state === "recordingBlocked"
        || appUpdateStatus.state === "failed")
      && !appUpdateStatus.recordingActive
      && !installingAppUpdate
      && !checkingAppUpdate
      && !switchingAppUpdateChannel,
  );

  const canRestartAfterUpdate = $derived(
    appUpdateStatus?.state === "restartRequired"
      && !appUpdateStatus.recordingActive
      && !restartingAfterUpdate,
  );

  async function loadAppUpdateStatus() {
    appUpdateActionError = null;
    try {
      appUpdateStatus = await invoke<AppUpdateStatus>("get_app_update_status");
    } catch (err) {
      appUpdateActionError = describeError(err);
    }
  }

  async function checkForAppUpdate() {
    checkingAppUpdate = true;
    appUpdateActionError = null;
    try {
      appUpdateStatus = await invoke<AppUpdateStatus>("check_for_app_update");
    } catch (err) {
      appUpdateActionError = describeError(err);
    } finally {
      checkingAppUpdate = false;
    }
  }

  async function useAppUpdateChannel(channel: AppUpdateChannel) {
    if (appUpdateStatus?.channel === channel && !previewConfirmationVisible) return;
    switchingAppUpdateChannel = true;
    appUpdateActionError = null;
    try {
      appUpdateStatus = await invoke<AppUpdateStatus>("set_app_update_channel", { channel });
      previewConfirmationVisible = false;
    } catch (err) {
      appUpdateActionError = describeError(err);
    } finally {
      switchingAppUpdateChannel = false;
    }
  }

  function chooseAppUpdateChannel(channel: AppUpdateChannel) {
    if (channel === "preview" && appUpdateStatus?.channel !== "preview") {
      previewConfirmationVisible = true;
      return;
    }
    void useAppUpdateChannel(channel);
  }

  async function installAppUpdate() {
    installingAppUpdate = true;
    appUpdateActionError = null;
    try {
      appUpdateStatus = await invoke<AppUpdateStatus>("install_app_update");
    } catch (err) {
      appUpdateActionError = describeError(err);
    } finally {
      installingAppUpdate = false;
    }
  }

  async function restartAfterAppUpdate() {
    restartingAfterUpdate = true;
    appUpdateActionError = null;
    try {
      await invoke("restart_after_app_update");
    } catch (err) {
      appUpdateActionError = describeError(err);
      await loadAppUpdateStatus();
    } finally {
      restartingAfterUpdate = false;
    }
  }

  async function copyAboutDetails() {
    aboutActionError = null;
    try {
      await writeText(aboutDetailsText(appUpdateStatus));
      aboutDetailsCopied = true;
      if (aboutDetailsCopiedTimer !== null) clearTimeout(aboutDetailsCopiedTimer);
      aboutDetailsCopiedTimer = setTimeout(() => {
        aboutDetailsCopied = false;
        aboutDetailsCopiedTimer = null;
      }, 2000);
    } catch (err) {
      aboutActionError = describeError(err);
    }
  }

  async function openExternalUrl(url: string) {
    aboutActionError = null;
    try {
      await openUrl(url);
    } catch (err) {
      aboutActionError = describeError(err);
    }
  }

  async function loadThirdPartyNotices() {
    loadingThirdPartyNotices = true;
    thirdPartyNoticesError = null;
    try {
      thirdPartyNotices = await invoke<ThirdPartyNotices>("get_third_party_notices");
    } catch (err) {
      thirdPartyNoticesError = describeError(err);
    } finally {
      loadingThirdPartyNotices = false;
    }
  }

  async function copyThirdPartyNotices() {
    if (!thirdPartyNotices) return;
    thirdPartyNoticesError = null;
    try {
      await writeText(thirdPartyNotices.plainText);
      thirdPartyNoticesCopied = true;
      if (thirdPartyNoticesCopiedTimer !== null) clearTimeout(thirdPartyNoticesCopiedTimer);
      thirdPartyNoticesCopiedTimer = setTimeout(() => {
        thirdPartyNoticesCopied = false;
        thirdPartyNoticesCopiedTimer = null;
      }, 2000);
    } catch (err) {
      thirdPartyNoticesError = describeError(err);
    }
  }

  return {
    get appUpdateStatus() { return appUpdateStatus; },
    get checkingAppUpdate() { return checkingAppUpdate; },
    get switchingAppUpdateChannel() { return switchingAppUpdateChannel; },
    get installingAppUpdate() { return installingAppUpdate; },
    get restartingAfterUpdate() { return restartingAfterUpdate; },
    get appUpdateActionError() { return appUpdateActionError; },
    set appUpdateActionError(v: string | null) { appUpdateActionError = v; },
    get previewConfirmationVisible() { return previewConfirmationVisible; },
    set previewConfirmationVisible(value: boolean) { previewConfirmationVisible = value; },
    get aboutDetailsCopied() { return aboutDetailsCopied; },
    get aboutActionError() { return aboutActionError; },
    get thirdPartyNotices() { return thirdPartyNotices; },
    get loadingThirdPartyNotices() { return loadingThirdPartyNotices; },
    get thirdPartyNoticesError() { return thirdPartyNoticesError; },
    get thirdPartyNoticesCopied() { return thirdPartyNoticesCopied; },
    get thirdPartyNoticeGroups() { return thirdPartyNoticeGroups; },
    get canInstallAppUpdate() { return canInstallAppUpdate; },
    get canRestartAfterUpdate() { return canRestartAfterUpdate; },
    // Allow the backend event listener to push a fresh status + clear errors.
    setAppUpdateStatus(status: AppUpdateStatus) {
      appUpdateStatus = status;
      appUpdateActionError = null;
    },
    loadAppUpdateStatus,
    checkForAppUpdate,
    useAppUpdateChannel,
    chooseAppUpdateChannel,
    installAppUpdate,
    restartAfterAppUpdate,
    copyAboutDetails,
    openExternalUrl,
    loadThirdPartyNotices,
    copyThirdPartyNotices,
  };
}

export type AboutStore = ReturnType<typeof createAboutStore>;
