import { describe, expect, test } from "bun:test";
import { shouldOpenUpward } from "../src/lib/components/popover-direction";

// The real predicate: spaceBelow < needed && spaceAbove > spaceBelow.
// `needed` is each component's content max-height (Select 220, Combobox 260).
describe("shouldOpenUpward", () => {
  test("stays down when there is enough room below", () => {
    // spaceBelow (300) >= needed (220) -> never flips, regardless of above.
    expect(shouldOpenUpward(300, 50, 220)).toBe(false);
    expect(shouldOpenUpward(300, 800, 220)).toBe(false);
    // Exactly `needed` below is still "enough" (strict <).
    expect(shouldOpenUpward(220, 800, 220)).toBe(false);
  });

  test("stays down when below is tight but above is no roomier", () => {
    // Below is tight (100 < 220) but above (80) is not greater than below.
    expect(shouldOpenUpward(100, 80, 220)).toBe(false);
    // Equal room above and below -> not strictly greater, stays down.
    expect(shouldOpenUpward(100, 100, 220)).toBe(false);
  });

  test("flips up only when below is tight and above is strictly roomier", () => {
    expect(shouldOpenUpward(100, 101, 220)).toBe(true);
    expect(shouldOpenUpward(50, 400, 220)).toBe(true);
  });

  test("uses the caller's own `needed` constant", () => {
    // 240 of room below: tight for the Combobox (needed 260) but fine for the
    // Select (needed 220) — so the same geometry flips one and not the other.
    expect(shouldOpenUpward(240, 800, 260)).toBe(true);
    expect(shouldOpenUpward(240, 800, 220)).toBe(false);
  });
});
