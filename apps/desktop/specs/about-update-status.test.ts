// About-panel update status copy (about.svelte.ts pure helpers): the
// `availableOutOfWindow` label + its two-trigger message fork — a newer remote
// build past the window (update present) vs the running build itself past the
// window (fresh install after lapse, `update` absent).
//
// about.svelte.ts imports Tauri plugins at module top (none exist under
// bun-test), so stub them the same way surface-windows.test.ts does; the pure
// helpers under test never touch the stubs.

import { describe, expect, mock, test } from "bun:test";
import type { AppUpdateStatus } from "../src/lib/types/app-updates";

mock.module("@tauri-apps/api/core", () => ({
  invoke: async () => {},
  convertFileSrc: (p: string) => p,
}));
mock.module("@tauri-apps/plugin-clipboard-manager", () => ({
  writeText: async () => {},
}));
mock.module("@tauri-apps/plugin-opener", () => ({
  openUrl: async () => {},
}));

const { appUpdateStateLabel, appUpdateStatusMessage } = await import(
  "../src/lib/settings/state/about.svelte"
);

function status(over: Partial<AppUpdateStatus> = {}): AppUpdateStatus {
  return {
    app: { productName: "mnema", version: "0.2.0", identifier: "day.mnema", platform: "macos", arch: "aarch64" },
    channel: "stable",
    state: "idle",
    recordingActive: false,
    ...over,
  };
}

describe("availableOutOfWindow", () => {
  test("label reads 'Outside update window'", () => {
    expect(appUpdateStateLabel(status({ state: "availableOutOfWindow" }))).toBe(
      "Outside update window",
    );
  });

  test("with an update present: names the version, renew pitch, keeps-working reassurance", () => {
    const message = appUpdateStatusMessage(
      status({
        state: "availableOutOfWindow",
        update: { version: "0.3.0", channel: "stable" },
      }),
    );
    expect(message).toContain("Version 0.3.0 is past your update window.");
    expect(message).toContain("Renew");
    expect(message).toContain("keeps working forever");
  });

  test("without an update (running build past the window): the fresh-install-after-lapse copy", () => {
    const message = appUpdateStatusMessage(status({ state: "availableOutOfWindow", update: null }));
    expect(message).toContain("newer than your update window");
    expect(message).toContain("renew to receive new builds");
    expect(message).toContain("Your recordings are untouched");
  });
});

describe("neighboring states stay distinct", () => {
  test("available (in-window) offers the install copy, not the renew pitch", () => {
    const message = appUpdateStatusMessage(
      status({ state: "available", update: { version: "0.3.0", channel: "stable" } }),
    );
    expect(appUpdateStateLabel(status({ state: "available" }))).toBe("Update available");
    expect(message).toBe("Version 0.3.0 is ready to download and install.");
    expect(message).not.toContain("Renew");
  });

  test("upToDate / idle labels", () => {
    expect(appUpdateStateLabel(status({ state: "upToDate" }))).toBe("Up to date");
    expect(appUpdateStateLabel(status({ state: "idle" }))).toBe("Not checked");
    expect(appUpdateStateLabel(null)).toBe("Loading");
  });

  test("recording-active override wins for available, but NOT for availableOutOfWindow", () => {
    expect(
      appUpdateStateLabel(status({ state: "available", recordingActive: true })),
    ).toBe("Recording active");
    // Out-of-window is a licensing condition, not an install action — recording
    // doesn't mask it.
    expect(
      appUpdateStateLabel(status({ state: "availableOutOfWindow", recordingActive: true })),
    ).toBe("Outside update window");
  });
});
