// Pure view logic for the dedicated update window (`/update`).
//
// It lives here rather than inline in `routes/update/+page.svelte` so it can be
// tested: the frontend suite (`bun --cwd=apps/desktop test`, run in CI) imports
// modules, and a Svelte component's `$derived` expressions are unreachable from
// it. The window's whole job is deciding what to say and which button to arm,
// so that decision is the part worth a test.

import type { LicenseStatus } from "$lib/licensing";
import type { AppUpdateStatus } from "$lib/types";

/** The backend is mid-flight and owns the outcome; nothing here can cancel it. */
export function isUpdateBusy(status: AppUpdateStatus | null): boolean {
  return status?.state === "downloading" || status?.state === "installing";
}

/**
 * Install stays armed on `failed`: a failed download or bundle swap keeps the
 * pending update, so it is retryable. Matches Settings → About and the tray's
 * "Retry Update" row.
 */
export function canInstallUpdate(status: AppUpdateStatus | null, acting: boolean): boolean {
  if (!status?.update || acting || isUpdateBusy(status)) return false;
  return status.state === "available" || status.state === "failed";
}

export function canRestartUpdate(status: AppUpdateStatus | null, acting: boolean): boolean {
  return status?.state === "restartRequired" && !acting;
}

/**
 * The window's headline.
 *
 * Keyed on `state`, never on the presence of a version: the backend clears
 * `update` at the START of every check and does not restore it when the check
 * fails, so a version-keyed fallback made the window that exists to announce an
 * update assert "mnema is up to date" — permanently, whenever the machine was
 * offline.
 */
export function appUpdateHeading(status: AppUpdateStatus | null): string {
  const version = status?.update?.version;
  const state = status?.state;
  // No `default` arm on the state switch: every AppUpdateState is spelled out so
  // adding one fails the exhaustiveness check below rather than silently
  // inheriting a catch-all. `incompatible` earned that the hard way — it landed
  // in a "Checking for updates" fallback and sat there forever.
  switch (state) {
    case "downloading":
      return version ? `Downloading ${version}` : "Downloading update";
    case "installing":
      return version ? `Installing ${version}` : "Installing update";
    case "restartRequired":
      return version ? `${version} is ready to run` : "Update is ready to run";
    case "checking":
      return "Checking for updates";
    case "failed":
      return version ? `Couldn’t update to ${version}` : "Couldn’t check for updates";
    case "upToDate":
      return "Mnema is up to date";
    case "availableOutOfWindow":
      return version ? `${version} is past your update window` : "Update window has lapsed";
    case "incompatible":
      // Reachable only from a failed check, which cleared `update` first — so
      // in practice this always renders the version-less form.
      return version
        ? `Mnema ${version} isn’t compatible with this Mac`
        : "No compatible update for this Mac";
    case "available":
      return version ? `Mnema ${version} is available` : "An update is available";
    case "idle":
      // The backend has not checked in this session (also the channel-switch
      // reset). The window opens on an available update, so this is transient.
      return version ? `Mnema ${version} is available` : "Checking for updates";
    case undefined:
      // Cold open: the initial `get_app_update_status` invoke has not resolved.
      return "Checking for updates";
    default: {
      const unreachable: never = state;
      return unreachable;
    }
  }
}

/** Stable only while Stable is the only channel a user would realistically be on. */
export function updateChannelLine(status: AppUpdateStatus | null): string | null {
  return status && status.channel !== "stable" ? "Preview channel" : null;
}

/**
 * The window's eleven drawn states (`docs/mockups/redesign/update.html`
 * `?state=` keys). `failed` splits three ways: a failed check clears `update`
 * before it can fail, while a failed download or install keeps it — and
 * `install` is the one error kind the bundle swap reports.
 */
export type UpdateViewKey =
  | "available"
  | "checking"
  | "uptodate"
  | "check-failed"
  | "downloading"
  | "download-failed"
  | "installing"
  | "install-failed"
  | "ready"
  | "incompatible"
  | "expired";

export function updateViewKey(status: AppUpdateStatus | null): UpdateViewKey {
  const state = status?.state;
  switch (state) {
    case undefined:
    case "checking":
      return "checking";
    case "idle":
      return status?.update ? "available" : "checking";
    case "available":
      return "available";
    case "upToDate":
      return "uptodate";
    case "failed":
      if (!status?.update) return "check-failed";
      return status.error?.kind === "install" ? "install-failed" : "download-failed";
    case "downloading":
      return "downloading";
    case "installing":
      return "installing";
    case "restartRequired":
      return "ready";
    case "incompatible":
      return "incompatible";
    case "availableOutOfWindow":
      return "expired";
    default: {
      const unreachable: never = state;
      return unreachable;
    }
  }
}

export const UPDATE_KICKER: Record<UpdateViewKey, string> = {
  available: "Update available",
  checking: "Software update",
  uptodate: "Software update",
  "check-failed": "Software update",
  downloading: "Updating",
  "download-failed": "Update failed",
  installing: "Updating",
  "install-failed": "Update failed",
  ready: "Ready",
  incompatible: "Software update",
  expired: "Outside your update window",
};

/**
 * Body copy for the states with no release notes. Never a restatement of the
 * headline — the drawn "Mnema is up to date" over "Mnema is up to date." said
 * the same thing twice.
 */
export function updateWindowMessage(key: UpdateViewKey): string | null {
  switch (key) {
    case "checking":
      return "Asking the update feed for a newer build. The version you have keeps working.";
    case "uptodate":
      return "There’s nothing new to install — you have the newest build.";
    case "check-failed":
      return "The check didn’t reach the update feed, so there’s nothing to install yet. The version you have keeps working.";
    case "incompatible":
      return "The newest build has no download for this Mac, so there’s nothing to install.";
    default:
      return null;
  }
}

export interface UpdateLine {
  tone: "ok" | "warn" | "danger" | null;
  icon: "power" | "check" | "spin" | "cloud-off" | "alert" | null;
  text: string;
}

const CONSEQUENCE: UpdateLine = {
  tone: null,
  icon: "power",
  text: "Installing stops recording and relaunches Mnema.",
};

/**
 * The controls band's status lines, above the buttons. A failure line extends
 * the backend's own user-facing message (`app_updates::user_facing_error_message`)
 * with what it means here. `install_app_update` is `-> AppUpdateStatus`, not
 * `Result`, so a failed install RESOLVES the invoke: `status.error` is the only
 * report of it and must always reach a line. Downloading / installing draw a
 * progress bar instead.
 */
export function updateWindowLines(status: AppUpdateStatus | null): UpdateLine[] {
  const failure = status?.error?.message?.trim() || "Update failed.";
  switch (updateViewKey(status)) {
    case "available":
      return [CONSEQUENCE];
    case "checking":
      return [{ tone: null, icon: "spin", text: "Checking…" }];
    case "uptodate":
      return [{ tone: null, icon: "check", text: "Mnema checks again every 2 hours." }];
    case "check-failed":
      return [{ tone: "warn", icon: "cloud-off", text: `${failure} Mnema tries again on its own.` }];
    case "download-failed":
      return [CONSEQUENCE, { tone: "danger", icon: "alert", text: `${failure} Nothing was installed.` }];
    case "install-failed":
      return [
        CONSEQUENCE,
        { tone: "danger", icon: "alert", text: `${failure} ${status?.app.version ?? "Your version"} is still running.` },
      ];
    case "ready":
      return [
        { tone: "ok", icon: "check", text: "Downloaded and verified. Restarting stops recording and relaunches Mnema." },
      ];
    case "incompatible":
      return [{ tone: null, icon: "alert", text: "Your current version keeps working." }];
    case "expired":
      return [{ tone: null, icon: null, text: "Already renewed? Mnema checks your license again — no new key needed." }];
    case "downloading":
    case "installing":
      return [];
  }
}

/** "just now" inside a minute, a clock time today, else a short date and time. */
export function formatCheckedClock(atMs: number | null | undefined, nowMs: number): string | null {
  if (!atMs) return null;
  if (nowMs - atMs < 60_000) return "just now";
  const at = new Date(atMs);
  const sameDay = at.toDateString() === new Date(nowMs).toDateString();
  return at.toLocaleString(
    undefined,
    sameDay
      ? { hour: "numeric", minute: "2-digit" }
      : { month: "short", day: "numeric", hour: "numeric", minute: "2-digit" },
  );
}

/**
 * Who the license is for and when its update window ended, for the
 * out-of-window state. `name` is "" when the key carries none, so the email
 * stands in.
 */
export function outOfWindowLicense(
  license: LicenseStatus | null,
): { licensee: string; windowEnd: string } | null {
  if (license?.kind !== "licensed") return null;
  return {
    licensee: license.name.trim() || license.email,
    windowEnd: new Date(license.updateThroughMs).toLocaleDateString(undefined, {
      year: "numeric",
      month: "short",
      day: "numeric",
    }),
  };
}

export const UPDATE_WINDOW_MIN_HEIGHT = 360;
export const UPDATE_WINDOW_MAX_HEIGHT = 640;

/**
 * The height the window should become to fit its notes, or `null` to leave it
 * alone. `overflow` is the content's natural height minus the height it was
 * given — positive means clipped, negative means dead space.
 */
export function fitUpdateWindowHeight(currentHeight: number, overflow: number): number | null {
  if (Math.abs(overflow) < 2) return null;
  const target = Math.round(
    Math.min(
      UPDATE_WINDOW_MAX_HEIGHT,
      Math.max(UPDATE_WINDOW_MIN_HEIGHT, currentHeight + overflow),
    ),
  );
  return target === Math.round(currentHeight) ? null : target;
}
