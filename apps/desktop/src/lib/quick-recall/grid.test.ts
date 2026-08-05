// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, test } from "bun:test";
import { moveInGrid, buildGridSections, dayLabel, cellDuration } from "./grid";
import type { GridItem } from "./grid";

// Two sections: 6 items (2 full rows) + 5 items (row of 3 + ragged row of 2).
const SIZES = [6, 5];

describe("moveInGrid", () => {
  test("left/right walk the flattened order and clamp at the ends", () => {
    expect(moveInGrid(0, "left", SIZES)).toBe(0);
    expect(moveInGrid(0, "right", SIZES)).toBe(1);
    expect(moveInGrid(5, "right", SIZES)).toBe(6); // crosses the section seam
    expect(moveInGrid(10, "right", SIZES)).toBe(10);
  });

  test("down moves by row inside a section", () => {
    expect(moveInGrid(1, "down", SIZES)).toBe(4);
  });

  test("down hops to the next section's first row, same column", () => {
    expect(moveInGrid(4, "down", SIZES)).toBe(7); // col 1 → section 2 col 1
  });

  test("down clamps into a ragged next-section row", () => {
    // sizes [3, 1]: from col 2 of row 0 down into a 1-cell section.
    expect(moveInGrid(2, "down", [3, 1])).toBe(3);
  });

  test("down onto a ragged last row clamps to the section's last cell", () => {
    // Section of 5: index 2 (row 0 col 2) has no cell below (last row holds 2).
    expect(moveInGrid(8, "down", SIZES)).toBe(10);
  });

  test("down on the last row of the last section stays", () => {
    expect(moveInGrid(9, "down", SIZES)).toBe(9);
  });

  test("up moves by row and hops to the previous section's last row", () => {
    expect(moveInGrid(4, "up", SIZES)).toBe(1);
    expect(moveInGrid(7, "up", SIZES)).toBe(4); // col 1 → last row col 1
    expect(moveInGrid(1, "up", SIZES)).toBe(1); // top row of first section
  });

  test("empty grid yields -1; unset selection snaps to 0", () => {
    expect(moveInGrid(3, "down", [])).toBe(-1);
    expect(moveInGrid(-1, "down", SIZES)).toBe(0);
  });
});

describe("buildGridSections", () => {
  const frame = (groupStartAt: string): GridItem =>
    ({ kind: "frame", frame: { groupStartAt } }) as unknown as GridItem;

  test("groups consecutive same-day items into one section", () => {
    const now = new Date(2026, 7, 4, 12); // Aug 4 2026
    const sections = buildGridSections(
      [
        frame("2026-08-04 10:00:00"),
        frame("2026-08-04 09:00:00"),
        frame("2026-08-01 16:00:00"),
      ],
      now,
    );
    expect(sections.length).toBe(2);
    expect(sections[0]).toMatchObject({ start: 0, count: 2 });
    expect(sections[0].label.startsWith("Today — ")).toBe(true);
    expect(sections[1]).toMatchObject({ start: 2, count: 1 });
  });
});

describe("labels", () => {
  test("dayLabel distinguishes today / yesterday / a plain day", () => {
    const now = new Date(2026, 7, 4);
    expect(dayLabel(new Date(2026, 7, 4), now).startsWith("Today — ")).toBe(true);
    expect(dayLabel(new Date(2026, 7, 3), now).startsWith("Yesterday — ")).toBe(
      true,
    );
    expect(dayLabel(new Date(2026, 6, 31), now).includes("July")).toBe(true);
  });

  test("cellDuration formats", () => {
    expect(cellDuration(246_000)).toBe("4m 06s");
    expect(cellDuration(58_000)).toBe("58s");
  });
});
