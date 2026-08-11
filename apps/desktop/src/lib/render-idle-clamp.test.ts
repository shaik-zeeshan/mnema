// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, test } from "bun:test";

import { clampTarget, MIN_VISIBLE_PX, type Rect } from "$lib/render-idle-clamp";

const monitor = (x: number, y: number, width: number, height: number): Rect => ({
  position: { x, y },
  size: { width, height },
});

describe("clampTarget", () => {
  test("no monitors leaves the window where it is", () => {
    // Nothing to move it onto. Returning a position here would park the window at
    // an arbitrary origin during a transient all-displays-gone reconfiguration.
    expect(clampTarget([], { x: 3000, y: 200 }, { width: 800, height: 600 })).toBeNull();
  });

  test("a window already meaningfully visible is left alone", () => {
    // Exactly MIN_VISIBLE_PX of overlap on both axes — the boundary. The check is
    // `>=`, so this must NOT move; an off-by-one to `>` would yank a window the user
    // deliberately parked at the screen edge.
    const screens = [monitor(0, 0, 1920, 1080)];
    const size = { width: 1440, height: 900 };
    const pos = { x: 1920 - MIN_VISIBLE_PX, y: 1080 - MIN_VISIBLE_PX };
    expect(clampTarget(screens, pos, size)).toBeNull();
  });

  test("one pixel short of the threshold is clamped", () => {
    const screens = [monitor(0, 0, 1920, 1080)];
    const size = { width: 1440, height: 900 };
    const pos = { x: 1920 - MIN_VISIBLE_PX + 1, y: 1080 - MIN_VISIBLE_PX + 1 };
    expect(clampTarget(screens, pos, size)).not.toBeNull();
  });

  test("overlap on only one axis still counts as offscreen", () => {
    // Full width overlap but a 5px sliver of height. macOS reports this window as
    // visible, so it never occludes and never goes hidden — exactly the state whose
    // repaints strand IOSurfaces, and the reason the check is AND, not OR.
    const screens = [monitor(0, 0, 1920, 1080)];
    expect(
      clampTarget(screens, { x: 0, y: 1075 }, { width: 800, height: 600 }),
    ).toEqual({ x: 560, y: 240 });
  });

  test("a window parked off a disconnected display is centred on the survivor", () => {
    const screens = [monitor(0, 0, 1920, 1080)];
    expect(
      clampTarget(screens, { x: 3000, y: 200 }, { width: 800, height: 600 }),
    ).toEqual({ x: 560, y: 240 });
  });

  test("a monitor left of the primary has a negative origin", () => {
    // The case a naive Math.abs or an origin-ignoring centre silently breaks: the
    // only remaining display sits at x = -1920, so the target must be negative too.
    const screens = [monitor(-1920, 0, 1920, 1080)];
    expect(
      clampTarget(screens, { x: 500, y: 500 }, { width: 800, height: 600 }),
    ).toEqual({ x: -1360, y: 240 });
  });

  test("a window larger than the monitor pins to the origin, never negative", () => {
    // (1440 - 1920) / 2 is negative; without the Math.max(0, …) the clamp would move
    // the window further offscreen than it started.
    const screens = [monitor(0, 0, 1440, 900)];
    expect(
      clampTarget(screens, { x: 5000, y: 5000 }, { width: 1920, height: 1080 }),
    ).toEqual({ x: 0, y: 0 });
  });

  test("visibility is checked against every monitor, not just the first", () => {
    // Fully on the second display. Checking only monitors[0] would yank it across.
    const screens = [monitor(0, 0, 1920, 1080), monitor(1920, 0, 1920, 1080)];
    expect(
      clampTarget(screens, { x: 2200, y: 100 }, { width: 800, height: 600 }),
    ).toBeNull();
  });
});
