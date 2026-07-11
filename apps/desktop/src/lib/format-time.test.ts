// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, test } from "bun:test";
import { formatRelativeTime } from "./format-time";

// Fixed "now" so bucket boundaries are deterministic. Inputs use the SQLite
// space form to exercise parseCapturedAt normalization on the way in.
const NOW = new Date("2026-07-07T12:00:00");

describe("formatRelativeTime", () => {
  test("buckets ages into m/h/d/w ago", () => {
    expect(formatRelativeTime("2026-07-07 11:59:40", NOW)).toBe("just now");
    expect(formatRelativeTime("2026-07-07 11:48:00", NOW)).toBe("12m ago");
    expect(formatRelativeTime("2026-07-07 08:00:00", NOW)).toBe("4h ago");
    expect(formatRelativeTime("2026-07-04 12:00:00", NOW)).toBe("3d ago");
    expect(formatRelativeTime("2026-06-20 12:00:00", NOW)).toBe("2w ago");
    expect(formatRelativeTime("2026-04-01 12:00:00", NOW)).toBe("3mo ago");
    expect(formatRelativeTime("2024-06-01 12:00:00", NOW)).toBe("2y ago");
  });

  test("future timestamps (clock skew) read as just now", () => {
    expect(formatRelativeTime("2026-07-07 12:05:00", NOW)).toBe("just now");
  });

  test("unparseable input falls back to the raw string", () => {
    expect(formatRelativeTime("not a time", NOW)).toBe("not a time");
  });
});
