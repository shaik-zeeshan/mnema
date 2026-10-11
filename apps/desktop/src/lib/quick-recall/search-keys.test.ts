// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, test } from "bun:test";
import { handleLauncherCaptureKeydown, handleSearchKeydown } from "./search-keys";

// A stand-in for SearchStore: just the fields and calls the keymap touches.
function fakeStore({ query = "", pills = [], caret = query.length, ghost = false } = {}) {
  const calls: string[] = [];
  const filters = {
    pillIndex: null,
    popoverOpen: false,
    hasGhost: ghost,
    selectPill(i) {
      this.pillIndex = i;
      if (i === null) this.popoverOpen = false;
      calls.push(`selectPill:${i}`);
    },
    removePill: (i) => calls.push(`removePill:${i}`),
    openPopover: (i) => calls.push(`openPopover:${i}`),
    acceptGhost: () => calls.push("acceptGhost"),
    popoverMove: (d) => calls.push(`popoverMove:${d}`),
    applyOption: () => calls.push("applyOption"),
    closePopover: () => calls.push("closePopover"),
  };
  return {
    calls,
    query,
    pills,
    filters,
    caretAtEnd: caret === query.length,
    inputEl: { selectionStart: caret, selectionEnd: caret },
    selectedIndex: 0,
    commitPills: () => calls.push("commitPills"),
    clearQuery: () => calls.push("clearQuery"),
    moveSelection: (d) => calls.push(`moveSelection:${d}`),
    openResultAt: (i) => calls.push(`openResultAt:${i}`),
    openSelectedResultUrl: () => calls.push("openSelectedResultUrl"),
  };
}

function press(store, key, mods = {}) {
  const event = {
    key,
    isComposing: false,
    metaKey: false,
    ctrlKey: false,
    altKey: false,
    shiftKey: false,
    ...mods,
    defaultPrevented: false,
    propagationStopped: false,
    preventDefault() {
      this.defaultPrevented = true;
    },
    stopPropagation() {
      this.propagationStopped = true;
    },
  };
  let toggled = 0;
  handleSearchKeydown(store, event, () => (toggled += 1));
  return { event, toggled };
}

const PILL = { kind: "app", app: "Arc" };

describe("field keymap", () => {
  test("Tab flips search ⇄ ask (committing pills first) and never accepts the ghost", () => {
    const store = fakeStore({ query: "pricing ap", ghost: true });
    const { event, toggled } = press(store, "Tab");
    expect(toggled).toBe(1);
    expect(event.defaultPrevented).toBe(true);
    expect(store.calls).toEqual(["commitPills"]);
  });

  test("→ accepts the ghost only at the end of the text", () => {
    const atEnd = fakeStore({ query: "pricing ap", ghost: true });
    press(atEnd, "ArrowRight");
    expect(atEnd.calls).toEqual(["acceptGhost"]);

    const midText = fakeStore({ query: "pricing ap", ghost: true, caret: 3 });
    const { event } = press(midText, "ArrowRight");
    expect(midText.calls).toEqual([]);
    expect(event.defaultPrevented).toBe(false);
  });

  test("⌫ at the start walks into the pills, then removes the walked-into pill", () => {
    const store = fakeStore({ query: "pricing", pills: [PILL, PILL], caret: 0 });
    press(store, "Backspace");
    expect(store.filters.pillIndex).toBe(1);
    press(store, "Backspace");
    expect(store.calls).toEqual(["selectPill:1", "removePill:1"]);
  });

  test("⌫ inside the text is a normal delete", () => {
    const store = fakeStore({ query: "pricing", pills: [PILL], caret: 3 });
    const { event } = press(store, "Backspace");
    expect(event.defaultPrevented).toBe(false);
    expect(store.calls).toEqual([]);
  });

  test("↵ on a walked-into pill opens its popover; typing lets the pill go", () => {
    const store = fakeStore({ query: "", pills: [PILL], caret: 0 });
    store.filters.pillIndex = 0;
    press(store, "Enter");
    expect(store.calls).toEqual(["openPopover:0"]);
    press(store, "a");
    expect(store.filters.pillIndex).toBe(null);
  });

  test("Esc clears a non-empty field and is stopped; an empty field lets it close the window", () => {
    const full = fakeStore({ query: "pricing" });
    const a = press(full, "Escape");
    expect(full.calls).toEqual(["clearQuery"]);
    expect(a.event.propagationStopped).toBe(true);

    const pillsOnly = fakeStore({ pills: [PILL] });
    press(pillsOnly, "Escape");
    expect(pillsOnly.calls).toEqual(["clearQuery"]);

    const empty = fakeStore();
    const b = press(empty, "Escape");
    expect(empty.calls).toEqual([]);
    expect(b.event.defaultPrevented).toBe(false);
  });

  test("↵ opens the selected moment, ⌘↵ its page", () => {
    const store = fakeStore({ query: "pricing" });
    press(store, "Enter");
    press(store, "Enter", { metaKey: true });
    expect(store.calls).toEqual([
      "commitPills",
      "openResultAt:0",
      "commitPills",
      "openSelectedResultUrl",
    ]);
  });
});

describe("pill popover keys (capture phase)", () => {
  test("own ↑↓ ↵ ⌫ Esc while open, and nothing when closed", () => {
    const store = fakeStore({ pills: [PILL] });
    const ev = (key) => ({
      key,
      isComposing: false,
      preventDefault() {},
      stopPropagation() {},
    });
    handleLauncherCaptureKeydown(store, ev("ArrowDown"));
    expect(store.calls).toEqual([]);

    store.filters.popoverOpen = true;
    for (const key of ["ArrowDown", "ArrowUp", "Enter", "Backspace", "Escape"]) {
      handleLauncherCaptureKeydown(store, ev(key));
    }
    expect(store.calls).toEqual([
      "popoverMove:1",
      "popoverMove:-1",
      "applyOption",
      "removePill:undefined",
      "closePopover",
    ]);
  });
});
