// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, test } from "bun:test";

import { clampTarget, MIN_VISIBLE_PT, type Monitor } from "$lib/render-idle-clamp";

const monitor = (x: number, y: number, width: number, height: number, scaleFactor = 1): Monitor => ({
  position: { x, y },
  size: { width, height },
  scaleFactor,
});

describe("clampTarget", () => {
  test("no monitors leaves the window where it is", () => {
    // Nothing to move it onto. Returning a position here would park the window at
    // an arbitrary origin during a transient all-displays-gone reconfiguration.
    expect(clampTarget([], { x: 3000, y: 200 }, { width: 800, height: 600 }, 1)).toBeNull();
  });

  test("a window already meaningfully visible is left alone", () => {
    // Exactly MIN_VISIBLE_PT of overlap on both axes — the boundary. The check is
    // `>=`, so this must NOT move; an off-by-one to `>` would yank a window the user
    // deliberately parked at the screen edge.
    const screens = [monitor(0, 0, 1920, 1080)];
    const size = { width: 1440, height: 900 };
    const pos = { x: 1920 - MIN_VISIBLE_PT, y: 1080 - MIN_VISIBLE_PT };
    expect(clampTarget(screens, pos, size, 1)).toBeNull();
  });

  test("one pixel short of the threshold is clamped", () => {
    const screens = [monitor(0, 0, 1920, 1080)];
    const size = { width: 1440, height: 900 };
    const pos = { x: 1920 - MIN_VISIBLE_PT + 1, y: 1080 - MIN_VISIBLE_PT + 1 };
    expect(clampTarget(screens, pos, size, 1)).not.toBeNull();
  });

  test("overlap on only one axis still counts as offscreen", () => {
    // Full width overlap but a 5px sliver of height. macOS reports this window as
    // visible, so it never occludes and never goes hidden — exactly the state whose
    // repaints strand IOSurfaces, and the reason the check is AND, not OR.
    const screens = [monitor(0, 0, 1920, 1080)];
    expect(
      clampTarget(screens, { x: 0, y: 1075 }, { width: 800, height: 600 }, 1),
    ).toEqual({ x: 560, y: 240 });
  });

  test("a window parked off a disconnected display is centred on the survivor", () => {
    const screens = [monitor(0, 0, 1920, 1080)];
    expect(
      clampTarget(screens, { x: 3000, y: 200 }, { width: 800, height: 600 }, 1),
    ).toEqual({ x: 560, y: 240 });
  });

  test("a monitor left of the primary has a negative origin", () => {
    // The case a naive Math.abs or an origin-ignoring centre silently breaks: the
    // only remaining display sits at x = -1920, so the target must be negative too.
    const screens = [monitor(-1920, 0, 1920, 1080)];
    expect(
      clampTarget(screens, { x: 500, y: 500 }, { width: 800, height: 600 }, 1),
    ).toEqual({ x: -1360, y: 240 });
  });

  test("a window larger than the monitor pins to the origin, never negative", () => {
    // (1440 - 1920) / 2 is negative; without the Math.max(0, …) the clamp would move
    // the window further offscreen than it started.
    const screens = [monitor(0, 0, 1440, 900)];
    expect(
      clampTarget(screens, { x: 5000, y: 5000 }, { width: 1920, height: 1080 }, 1),
    ).toEqual({ x: 0, y: 0 });
  });

  test("visibility is checked against every monitor, not just the first", () => {
    // Fully on the second display. Checking only monitors[0] would yank it across.
    const screens = [monitor(0, 0, 1920, 1080), monitor(1920, 0, 1920, 1080)];
    expect(
      clampTarget(screens, { x: 2200, y: 100 }, { width: 800, height: 600 }, 1),
    ).toBeNull();
  });

  test("monitors with different scale factors are compared in points", () => {
    // A Retina Air (3420×2224 px at 2x = 1710×1112 pt) at the origin and a 1x
    // 1080p monitor to its right at 1710 pt, whose physical origin is therefore
    // x = 1710, not 3420. A 2x-backed window fully on the 1x monitor at
    // (2000, 100) pt reports x = 4000 px — right of BOTH monitors' raw pixel
    // rects, so a pixel comparison would yank it across.
    const screens = [monitor(0, 0, 3420, 2224, 2), monitor(1710, 0, 1920, 1080, 1)];
    expect(clampTarget(screens, { x: 4000, y: 200 }, { width: 1600, height: 1200 }, 2)).toBeNull();
  });

  test("the clamp target is in points, centred on the first monitor's point rect", () => {
    // Parked far right; centred on the 2x Air: (1710 - 800) / 2, (1112 - 600) / 2.
    const screens = [monitor(0, 0, 3420, 2224, 2)];
    expect(
      clampTarget(screens, { x: 9000, y: 200 }, { width: 1600, height: 1200 }, 2),
    ).toEqual({ x: 455, y: 256 });
  });
});
