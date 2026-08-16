// Pure view logic for the dedicated update window (`/update`).
//
// It lives here rather than inline in `routes/update/+page.svelte` so it can be
// tested: the frontend suite (`bun --cwd=apps/desktop test`, run in CI) imports
// modules, and a Svelte component's `$derived` expressions are unreachable from
// it. The window's whole job is deciding what to say and which button to arm,
// so that decision is the part worth a test.

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
      return version ? `Couldn't update to ${version}` : "Couldn't check for updates";
    case "upToDate":
      return "mnema is up to date";
    case "availableOutOfWindow":
      return version ? `mnema ${version} is past your update window` : "Update window has lapsed";
    case "incompatible":
      // Reachable only from a failed check, which cleared `update` first — so
      // in practice this always renders the version-less form.
      return version
        ? `mnema ${version} isn't compatible with this Mac`
        : "No compatible update for this Mac";
    case "available":
      return version ? `mnema ${version} is available` : "An update is available";
    case "idle":
      // The backend has not checked in this session (also the channel-switch
      // reset). The window opens on an available update, so this is transient.
      return version ? `mnema ${version} is available` : "Checking for updates";
    case undefined:
      // Cold open: the initial `get_app_update_status` invoke has not resolved.
      return "Checking for updates";
    default: {
      const unreachable: never = state;
      return unreachable;
    }
  }
}

/**
 * The error to show, if any.
 *
 * `install_app_update` is declared `-> AppUpdateStatus`, not `Result`, so the
 * invoke RESOLVES on a failed install and the page's `catch` never runs. The
 * only report of that failure is `status.error`, so it has to be the fallback
 * or a failed install is completely silent.
 */
export function updateWindowError(
  actionError: string | null,
  status: AppUpdateStatus | null,
): string | null {
  return actionError ?? status?.error?.message ?? null;
}

/** Stable only while Stable is the only channel a user would realistically be on. */
export function updateChannelLine(status: AppUpdateStatus | null): string | null {
  return status && status.channel !== "stable" ? "Preview channel" : null;
}

/**
 * Whether a release-note href may be handed to the OS opener.
 *
 * `renderMarkdown` is NOT sufficient proof on its own: it strips the href of
 * any link carrying a disallowed scheme, but `isAllowedLinkHref` deliberately
 * keeps scheme-less relative hrefs (`#a`, `?q=1`, `/x`) and tags them
 * `data-external` too. The feed's `notes` are injected after signing (the
 * promote workflow says so), so they are untrusted input and this window
 * re-validates rather than relying on tauri-plugin-opener's URL scope to
 * silently reject what slips through.
 */
export function isOpenableNoteHref(href: string | null | undefined): boolean {
  if (!href) return false;
  let url: URL;
  try {
    // Parsed with NO base on purpose: the href is handed to the OS opener
    // verbatim, so anything that is not already an absolute URL is meaningless
    // there. This is what rejects `/etc/passwd`, `#anchor`, `?q=1` and the
    // protocol-relative `//host` — all of which throw without a base.
    url = new URL(href);
  } catch {
    return false;
  }
  return url.protocol === "http:" || url.protocol === "https:" || url.protocol === "mailto:";
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
