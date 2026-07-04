// @ts-nocheck — run under `bun test`; bun:test types aren't in the svelte-check
// tsconfig, so skip static checking here (same as journal-view.test.ts).
import { describe, expect, it } from "bun:test";
import {
  audioDrawerPointerDownAction,
  audioDrawerWheelAction,
} from "./audio-drawer-dismiss";

const base = {
  drawerOpen: true,
  insideDrawer: false,
  insidePopover: false,
  onAudioBar: false,
  popoverOpen: false,
  insideTimelineSurface: false,
};

describe("audio drawer dismissal — browse-frames-while-listening contract", () => {
  it("keeps the drawer open when the user wheel-scrubs the timeline rail/stage", () => {
    expect(audioDrawerWheelAction({ ...base, insideTimelineSurface: true })).toBe(
      "ignore",
    );
  });
  it("keeps the drawer open when the user clicks a frame on the rail/stage", () => {
    expect(
      audioDrawerPointerDownAction({ ...base, insideTimelineSurface: true }),
    ).toBe("ignore");
  });
  it("still collapses an open popover when scrubbing the rail (before the carve-out applies)", () => {
    expect(
      audioDrawerWheelAction({
        ...base,
        insideTimelineSurface: true,
        popoverOpen: true,
      }),
    ).toBe("collapse-popover");
  });
});

describe("audio drawer dismissal — layering and switch", () => {
  it("switches (not closes) when another audio bar is clicked", () => {
    expect(audioDrawerPointerDownAction({ ...base, onAudioBar: true })).toBe(
      "switch",
    );
  });
  it("collapses the popover first when a bar is clicked with the popover open", () => {
    expect(
      audioDrawerPointerDownAction({
        ...base,
        onAudioBar: true,
        popoverOpen: true,
      }),
    ).toBe("collapse-popover");
  });
  it("ignores interactions inside the drawer", () => {
    expect(audioDrawerPointerDownAction({ ...base, insideDrawer: true })).toBe(
      "ignore",
    );
    expect(audioDrawerWheelAction({ ...base, insideDrawer: true })).toBe(
      "ignore",
    );
  });
  it("ignores wheel inside the popover itself", () => {
    expect(
      audioDrawerWheelAction({
        ...base,
        insidePopover: true,
        popoverOpen: true,
      }),
    ).toBe("ignore");
  });
  it("collapses only the popover on a wheel inside the drawer while it is open", () => {
    // Branch-ORDER invariant: popover collapse must precede the
    // inside-drawer ignore, or scrolling the transcript leaves the
    // viewport-fixed popover drifting away from its chip.
    expect(
      audioDrawerWheelAction({ ...base, insideDrawer: true, popoverOpen: true }),
    ).toBe("collapse-popover");
  });
  it("closes the drawer on a genuine outside pointerdown/wheel", () => {
    expect(audioDrawerPointerDownAction({ ...base })).toBe("close-drawer");
    expect(audioDrawerWheelAction({ ...base })).toBe("close-drawer");
  });
  it("does nothing when the drawer is closed", () => {
    expect(
      audioDrawerPointerDownAction({ ...base, drawerOpen: false }),
    ).toBe("ignore");
    expect(audioDrawerWheelAction({ ...base, drawerOpen: false })).toBe(
      "ignore",
    );
  });
});
