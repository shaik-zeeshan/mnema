// Pure view logic for the dedicated update window (`$lib/update-window`), which
// decides what the window says and which button is armed. Three of these cases
// are regressions the window shipped with:
//   - the headline asserted "Mnema is up to date" during and after every check
//   - a failed install produced no visible error anywhere
//   - the fit pass' clamp had no coverage at all
//
// The module imports only types, so unlike about.svelte.ts it needs no Tauri
// stubs.

import { describe, expect, test } from "bun:test";
import type { AppUpdateState, AppUpdateStatus } from "../src/lib/types/app-updates";
import {
  appUpdateHeading,
  canInstallUpdate,
  canRestartUpdate,
  fitUpdateWindowHeight,
  isUpdateBusy,
  UPDATE_WINDOW_MAX_HEIGHT,
  UPDATE_WINDOW_MIN_HEIGHT,
  formatCheckedClock,
  outOfWindowLicense,
  updateChannelLine,
  updateViewKey,
  updateWindowLines,
  updateWindowMessage,
} from "../src/lib/update-window";

function status(over: Partial<AppUpdateStatus> = {}): AppUpdateStatus {
  return {
    app: {
      productName: "mnema",
      version: "0.1.20",
      identifier: "day.mnema",
      platform: "macos",
      arch: "aarch64",
    },
    channel: "stable",
    state: "idle",
    ...over,
  };
}

const pending = { version: "0.1.21", channel: "stable" as const };

describe("appUpdateHeading", () => {
  test("never claims the app is up to date while an update is pending", () => {
    expect(appUpdateHeading(status({ state: "available", update: pending }))).toBe(
      "Mnema 0.1.21 is available",
    );
    expect(appUpdateHeading(status({ state: "downloading", update: pending }))).toBe(
      "Downloading 0.1.21",
    );
    expect(appUpdateHeading(status({ state: "installing", update: pending }))).toBe(
      "Installing 0.1.21",
    );
    expect(appUpdateHeading(status({ state: "restartRequired", update: pending }))).toBe(
      "0.1.21 is ready to run",
    );
  });

  test("a check in flight does not flip the headline to 'up to date'", () => {
    // `run_update_check` sets state=Checking AND clears `update` before it hits
    // the network, then emits — so an open window sees this exact shape. Keying
    // the headline on the presence of a version made it read "mnema is up to
    // date" for the whole round trip.
    expect(appUpdateHeading(status({ state: "checking", update: null }))).toBe(
      "Checking for updates",
    );
  });

  test("a failed check does not leave the window asserting it is up to date", () => {
    // `set_runtime_error` restores state/error/progress but NOT `update`, so an
    // offline check lands here and STAYS here until the next successful one.
    expect(appUpdateHeading(status({ state: "failed", update: null }))).toBe(
      "Couldn’t check for updates",
    );
  });

  test("a failed install names the version it failed on", () => {
    // A failed install keeps `pending_update`, so `update` survives here.
    expect(appUpdateHeading(status({ state: "failed", update: pending }))).toBe(
      "Couldn’t update to 0.1.21",
    );
  });

  test("only upToDate says up to date", () => {
    expect(appUpdateHeading(status({ state: "upToDate" }))).toBe("Mnema is up to date");
  });

  test("every remaining state has its own exact headline", () => {
    // Exact strings, not a `not.toContain` sweep: deleting a whole switch arm
    // left the suite green, because the fallback happened to avoid the one
    // phrase the sweep looked for.
    const withVersion: Record<string, string> = {
      idle: "Mnema 0.1.21 is available",
      available: "Mnema 0.1.21 is available",
      availableOutOfWindow: "0.1.21 is past your update window",
      incompatible: "Mnema 0.1.21 isn’t compatible with this Mac",
    };
    for (const [state, expected] of Object.entries(withVersion)) {
      expect(appUpdateHeading(status({ state: state as AppUpdateState, update: pending }))).toBe(
        expected,
      );
    }

    // The version-less shapes the backend actually produces: every failure path
    // clears `update` before it can fail.
    expect(appUpdateHeading(status({ state: "availableOutOfWindow", update: null }))).toBe(
      "Update window has lapsed",
    );
    expect(appUpdateHeading(status({ state: "incompatible", update: null }))).toBe(
      "No compatible update for this Mac",
    );
    expect(appUpdateHeading(status({ state: "available", update: null }))).toBe(
      "An update is available",
    );
  });

  test("a cold window says it is checking, not that it is up to date", () => {
    // `status` is null until the initial get_app_update_status invoke resolves.
    expect(appUpdateHeading(null)).toBe("Checking for updates");
  });
});

describe("updateViewKey", () => {
  test("splits `failed` into check, download and install failures", () => {
    // A failed check clears `update` before it can fail; a failed download or
    // install keeps it, and only the bundle swap reports kind `install`.
    expect(updateViewKey(status({ state: "failed", update: null, error: { kind: "network", message: "x" } }))).toBe("check-failed");
    expect(updateViewKey(status({ state: "failed", update: pending, error: { kind: "verification", message: "x" } }))).toBe("download-failed");
    expect(updateViewKey(status({ state: "failed", update: pending, error: { kind: "install", message: "x" } }))).toBe("install-failed");
  });

  test("maps the rest one to one, and a cold or unchecked window to checking", () => {
    expect(updateViewKey(null)).toBe("checking");
    expect(updateViewKey(status({ state: "idle" }))).toBe("checking");
    expect(updateViewKey(status({ state: "idle", update: pending }))).toBe("available");
    expect(updateViewKey(status({ state: "upToDate" }))).toBe("uptodate");
    expect(updateViewKey(status({ state: "restartRequired", update: pending }))).toBe("ready");
    expect(updateViewKey(status({ state: "availableOutOfWindow", update: pending }))).toBe("expired");
  });
});

describe("updateWindowLines", () => {
  test("surfaces a backend install failure that never rejected the invoke", () => {
    // `install_app_update` is `-> AppUpdateStatus`, not `Result`: the invoke
    // RESOLVES on failure, so `status.error` is the only report of it.
    const lines = updateWindowLines(
      status({ state: "failed", update: pending, error: { kind: "install", message: "Update could not be installed." } }),
    );
    expect(lines.at(-1)).toEqual({
      tone: "danger",
      icon: "alert",
      text: "Update could not be installed. 0.1.20 is still running.",
    });
  });

  test("a retryable failure keeps the install consequence above the error", () => {
    const lines = updateWindowLines(
      status({ state: "failed", update: pending, error: { kind: "verification", message: "Update could not be verified." } }),
    );
    expect(lines.map((l) => l.text)).toEqual([
      "Installing stops recording and relaunches Mnema.",
      "Update could not be verified. Nothing was installed.",
    ]);
  });

  test("progress states carry no line — the bar is the status", () => {
    expect(updateWindowLines(status({ state: "downloading", update: pending }))).toEqual([]);
    expect(updateWindowLines(status({ state: "installing", update: pending }))).toEqual([]);
  });
});

describe("updateWindowMessage", () => {
  test("never restates the headline", () => {
    for (const s of ["checking", "upToDate", "incompatible"] as const) {
      const st = status({ state: s });
      expect(updateWindowMessage(updateViewKey(st))?.replace(/\.$/, "")).not.toBe(appUpdateHeading(st));
    }
  });

  test("note-carrying states have no message", () => {
    expect(updateWindowMessage("available")).toBeNull();
    expect(updateWindowMessage("expired")).toBeNull();
  });
});

describe("formatCheckedClock", () => {
  const now = new Date(2026, 9, 11, 15, 0).getTime();
  test("never checked is null; inside a minute is 'just now'", () => {
    expect(formatCheckedClock(null, now)).toBeNull();
    expect(formatCheckedClock(now - 30_000, now)).toBe("just now");
  });

  test("today is a clock time; an older check carries its date", () => {
    expect(formatCheckedClock(now - 3 * 3_600_000, now)).not.toMatch(/Oct/);
    expect(formatCheckedClock(now - 2 * 86_400_000, now)).toMatch(/Oct/);
  });
});

describe("outOfWindowLicense", () => {
  const licensed = {
    kind: "licensed" as const,
    updateThroughMs: new Date(2026, 8, 12).getTime(),
    inWindow: false,
    email: "a@b.c",
    name: "Shaik Zeeshan",
    activation: { state: "activated" as const },
  };

  test("names the licensee, falling back to the email for a nameless key", () => {
    expect(outOfWindowLicense(licensed)?.licensee).toBe("Shaik Zeeshan");
    expect(outOfWindowLicense({ ...licensed, name: "" })?.licensee).toBe("a@b.c");
    expect(outOfWindowLicense(licensed)?.windowEnd).toMatch(/2026/);
  });

  test("anything but a license is null", () => {
    expect(outOfWindowLicense(null)).toBeNull();
    expect(outOfWindowLicense({ kind: "readOnly" })).toBeNull();
  });
});

describe("install / restart gates", () => {
  test("install is armed on available and on a retryable failure", () => {
    expect(canInstallUpdate(status({ state: "available", update: pending }), false)).toBe(true);
    expect(canInstallUpdate(status({ state: "failed", update: pending }), false)).toBe(true);
  });

  test("install is disarmed with no pending update, mid-flight, or while acting", () => {
    expect(canInstallUpdate(status({ state: "available", update: null }), false)).toBe(false);
    expect(canInstallUpdate(status({ state: "available", update: pending }), true)).toBe(false);
    expect(canInstallUpdate(status({ state: "downloading", update: pending }), false)).toBe(false);
    expect(canInstallUpdate(status({ state: "installing", update: pending }), false)).toBe(false);
  });

  test("an out-of-window build is never installable from here", () => {
    // Not installable by design — Settings does the renew directing, and the
    // tray deliberately carries no row for it either.
    expect(
      canInstallUpdate(status({ state: "availableOutOfWindow", update: pending }), false),
    ).toBe(false);
  });

  test("restart is armed only once the bundle swap is staged", () => {
    expect(canRestartUpdate(status({ state: "restartRequired", update: pending }), false)).toBe(true);
    expect(canRestartUpdate(status({ state: "restartRequired", update: pending }), true)).toBe(false);
    expect(canRestartUpdate(status({ state: "available", update: pending }), false)).toBe(false);
  });

  test("busy tracks exactly the two states the backend owns", () => {
    expect(isUpdateBusy(status({ state: "downloading" }))).toBe(true);
    expect(isUpdateBusy(status({ state: "installing" }))).toBe(true);
    expect(isUpdateBusy(status({ state: "available" }))).toBe(false);
    expect(isUpdateBusy(null)).toBe(false);
  });
});

describe("updateChannelLine", () => {
  test("stays silent on stable and names a preview feed", () => {
    expect(updateChannelLine(status({ channel: "stable" }))).toBeNull();
    expect(updateChannelLine(status({ channel: "preview" }))).toBe("Preview channel");
    expect(updateChannelLine(null)).toBeNull();
  });
});

describe("fitUpdateWindowHeight", () => {
  test("ignores sub-pixel noise so the window cannot oscillate", () => {
    expect(fitUpdateWindowHeight(520, 0)).toBeNull();
    expect(fitUpdateWindowHeight(520, 1.5)).toBeNull();
    expect(fitUpdateWindowHeight(520, -1.5)).toBeNull();
  });

  test("grows for clipped notes and shrinks away dead space", () => {
    expect(fitUpdateWindowHeight(520, 60)).toBe(580);
    expect(fitUpdateWindowHeight(520, -100)).toBe(420);
  });

  test("clamps both ends", () => {
    // A 500-line changelog must not grow the panel without bound...
    expect(fitUpdateWindowHeight(520, 5000)).toBe(UPDATE_WINDOW_MAX_HEIGHT);
    // ...and a two-line one must not collapse it.
    expect(fitUpdateWindowHeight(520, -5000)).toBe(UPDATE_WINDOW_MIN_HEIGHT);
  });

  test("returns null once already clamped, so the fit converges", () => {
    // Without this the effect would re-issue the same setSize forever.
    expect(fitUpdateWindowHeight(UPDATE_WINDOW_MAX_HEIGHT, 5000)).toBeNull();
    expect(fitUpdateWindowHeight(UPDATE_WINDOW_MIN_HEIGHT, -5000)).toBeNull();
  });

  // The floor-vs-min_inner_size rule is asserted where it can actually fail —
  // windows.rs::tests::the_update_windows_fit_floor_clears_its_own_min_inner_size
  // reads this module's constant and compares it to the Rust window config.
  // Restating it here would just be two literals compared to each other.
});
