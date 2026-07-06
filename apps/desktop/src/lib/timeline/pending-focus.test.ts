// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, it } from "bun:test";
import { setPendingTimelineFocus, takePendingTimelineFocus } from "./pending-focus";

describe("pending timeline focus", () => {
  it("round-trips a frame focus and consumes it once", () => {
    setPendingTimelineFocus({ frameId: 7 });
    expect(takePendingTimelineFocus()).toEqual({ frameId: 7 });
    expect(takePendingTimelineFocus()).toBeNull();
  });

  it("round-trips an audio-segment focus and consumes it once", () => {
    setPendingTimelineFocus({ audioSegmentId: 42 });
    expect(takePendingTimelineFocus()).toEqual({ audioSegmentId: 42 });
    expect(takePendingTimelineFocus()).toBeNull();
  });
});
