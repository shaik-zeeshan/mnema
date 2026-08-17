// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { afterEach, describe, expect, test } from "bun:test";
import { trapTabKey } from "./keyboard";

// No DOM in `bun test`, and none is needed: `trapTabKey` only calls
// `querySelectorAll` / `contains` / `focus` on its container, plus
// `document.activeElement` and `window.getComputedStyle`. Fakes for exactly those.
function focusable(name: string, onFocus: (name: string) => void) {
  return {
    name,
    hasAttribute: () => false,
    getAttribute: () => null,
    getClientRects: () => [{}],
    focus: () => onFocus(name),
  };
}

function dialog(names: string[], onFocus: (name: string) => void) {
  const children = names.map((name) => focusable(name, onFocus));
  const el = {
    name: "dialog",
    querySelectorAll: () => children,
    contains: (node: unknown) => node === el || children.includes(node),
    focus: () => onFocus("dialog"),
  };
  return { el, children };
}

function tabEvent(shiftKey = false) {
  let prevented = false;
  return {
    event: {
      key: "Tab",
      shiftKey,
      preventDefault: () => (prevented = true),
    },
    wasPrevented: () => prevented,
  };
}

afterEach(() => {
  delete globalThis.document;
  delete globalThis.window;
});

describe("trapTabKey", () => {
  // The CLI Access approval window is `aria-modal` and lands focus on Deny, but
  // under WebKit's default macOS keyboard mode plain <button>s are OUTSIDE the tab
  // ring — so focus never reaches the trap's `last` element and Tab walks straight
  // out of the dialog into the window chrome. Containment has to key off "focus
  // has left the container", not off landing on the last focusable.
  test("Tab re-enters the dialog when focus has already left it", () => {
    let focused: string | null = null;
    const { el, children } = dialog(["deny", "allow"], (name) => (focused = name));
    globalThis.window = { getComputedStyle: () => ({ display: "block", visibility: "visible" }) };
    // Focus is on something outside the dialog entirely (the window chrome).
    globalThis.document = { activeElement: { name: "outside" } };

    const { event, wasPrevented } = tabEvent();
    expect(trapTabKey(event, el)).toBe(true);
    expect(wasPrevented()).toBe(true);
    expect(focused).toBe(children[0].name);
  });

  test("Shift+Tab from outside re-enters at the last control", () => {
    let focused: string | null = null;
    const { el, children } = dialog(["deny", "allow"], (name) => (focused = name));
    globalThis.window = { getComputedStyle: () => ({ display: "block", visibility: "visible" }) };
    globalThis.document = { activeElement: { name: "outside" } };

    const { event, wasPrevented } = tabEvent(true);
    expect(trapTabKey(event, el)).toBe(true);
    expect(wasPrevented()).toBe(true);
    expect(focused).toBe(children[children.length - 1].name);
  });

  // Not vacuous: a trap that fires on EVERY Tab would make the dialog's own
  // Deny → Allow move impossible.
  test("Tab inside the dialog still moves between its controls", () => {
    let focused: string | null = null;
    const { el, children } = dialog(["deny", "allow"], (name) => (focused = name));
    globalThis.window = { getComputedStyle: () => ({ display: "block", visibility: "visible" }) };
    globalThis.document = { activeElement: children[0] };

    const { event, wasPrevented } = tabEvent();
    expect(trapTabKey(event, el)).toBe(false);
    expect(wasPrevented()).toBe(false);
    expect(focused).toBeNull();
  });

  test("a non-Tab key is never intercepted", () => {
    const { el } = dialog(["deny"], () => {});
    expect(trapTabKey({ key: "Enter", shiftKey: false, preventDefault: () => {} }, el)).toBe(false);
  });
});

