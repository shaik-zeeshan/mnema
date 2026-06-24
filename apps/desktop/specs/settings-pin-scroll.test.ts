import { describe, expect, test } from "bun:test";
import { shouldSnapBack } from "../src/lib/components/pin-scroll-on-open";

// The real condition in the pin's scroll listener:
//   !released && ancestor.scrollTop !== top
// Snap back only while the pin is NOT released and the scroll drifted off `top`.
describe("shouldSnapBack", () => {
  test("snaps back when not released and the scroll drifted off the pin", () => {
    // A programmatic scrollIntoView pushed scrollTop from the pinned 120 to 0.
    expect(shouldSnapBack(false, 0, 120)).toBe(true);
    // Drift in either direction off the pin counts.
    expect(shouldSnapBack(false, 200, 120)).toBe(true);
  });

  test("never snaps back once the pin is released", () => {
    // A real user gesture released the pin first — intentional scrolling wins.
    expect(shouldSnapBack(true, 0, 120)).toBe(false);
    expect(shouldSnapBack(true, 999, 120)).toBe(false);
    // Even if the position happens to equal the pin, released stays false-y.
    expect(shouldSnapBack(true, 120, 120)).toBe(false);
  });

  test("does not fight a position already sitting at the pin", () => {
    // scrollTop === top (e.g. our own corrective write) -> nothing to do.
    expect(shouldSnapBack(false, 120, 120)).toBe(false);
    expect(shouldSnapBack(false, 0, 0)).toBe(false);
  });
});
