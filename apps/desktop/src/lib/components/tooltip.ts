// Svelte action `use:tip={text}` — a themed replacement for the native `title`
// attribute. One shared popover node is portaled to <body> and reused by every
// trigger (no per-trigger DOM). Styling lives in the `.app-tooltip` global rule
// in routes/+layout.svelte so it reads the same `--app-*` tokens as the app.
// Not interactive (pointer-events: none) — parity with `title`.
//
// ponytail: hand-rolled positioning instead of the installed bits-ui Tooltip —
// bits-ui is a wrapper-tree component and would force every `title=` site into a
// <Root><Trigger><Content> tree; an action is a 1:1 attribute swap. Move to
// bits-ui only if tooltips ever need to be interactive/rich.

import type { Action } from "svelte/action";

const GAP = 6; // px between trigger and tooltip
const MARGIN = 4; // min px kept from every viewport edge
const SHOW_DELAY = 350; // ms hover dwell before showing (immediate on focus)
const TIP_ID = "app-tooltip";

let el: HTMLDivElement | null = null;
let owner: HTMLElement | null = null; // trigger the tooltip currently describes
let showTimer: ReturnType<typeof setTimeout> | undefined;

function ensureEl(): HTMLDivElement {
  if (el) return el;
  const node = document.createElement("div");
  node.id = TIP_ID;
  node.className = "app-tooltip";
  node.setAttribute("role", "tooltip");
  node.dataset.show = "false";
  document.body.appendChild(node);
  // A fixed-position tooltip can't follow its trigger, so dismiss on anything
  // that scrolls or obscures it. Capture phase catches scrolls in any subtree.
  addEventListener("scroll", () => hide(), true);
  addEventListener("pointerdown", () => hide(), true);
  addEventListener(
    "keydown",
    (e) => {
      if (e.key === "Escape") hide();
    },
    true,
  );
  el = node;
  return node;
}

// Pure placement math (unit-tested): prefer above the trigger, flip below when
// there's no room above, center horizontally then clamp inside the viewport.
export function computeTipPosition(
  trigger: { top: number; bottom: number; left: number; width: number },
  tipW: number,
  tipH: number,
  viewportW: number,
): { left: number; top: number } {
  let top = trigger.top - tipH - GAP;
  if (top < MARGIN) top = trigger.bottom + GAP;
  let left = trigger.left + trigger.width / 2 - tipW / 2;
  left = Math.max(MARGIN, Math.min(left, viewportW - tipW - MARGIN));
  return { left: Math.round(left), top: Math.round(top) };
}

function place(trigger: HTMLElement, node: HTMLDivElement) {
  const { left, top } = computeTipPosition(
    trigger.getBoundingClientRect(),
    node.offsetWidth,
    node.offsetHeight,
    innerWidth,
  );
  node.style.left = `${left}px`;
  node.style.top = `${top}px`;
}

function reveal(trigger: HTMLElement, text: string) {
  const node = ensureEl();
  // Moving to a new trigger: drop describedby from the old one first, so a stale
  // aria-describedby never lingers on an element the tip no longer covers (hide's
  // owner guard would otherwise skip cleanup once owner has moved on).
  if (owner && owner !== trigger) owner.removeAttribute("aria-describedby");
  node.textContent = text;
  owner = trigger;
  trigger.setAttribute("aria-describedby", TIP_ID);
  node.dataset.show = "false"; // measure while hidden
  place(trigger, node);
  node.dataset.show = "true";
}

function hide(trigger?: HTMLElement) {
  clearTimeout(showTimer);
  if (trigger && owner !== trigger) return; // a newer trigger owns it now
  if (el) el.dataset.show = "false";
  owner?.removeAttribute("aria-describedby");
  owner = null;
}

// A real `disabled` form control suppresses mouse/focus events in WebKit, so the
// JS tooltip can never fire on one. The native `title` attribute still renders on
// a disabled element, so mirror the text into `title` only while disabled — and
// strip it otherwise so enabled controls don't get a doubled OS bubble. Keeps
// "why is this disabled?" tooltips working (parity with the `title=` we replaced).
// aria-disabled is intentionally NOT included: those elements still fire events,
// so the JS tip works and a native fallback would double up.
function eventsSuppressed(node: HTMLElement): boolean {
  return (node as Partial<HTMLButtonElement>).disabled === true;
}

export const tip: Action<HTMLElement, string | null | undefined> = (
  node,
  text,
) => {
  let current = text ?? "";

  const syncNativeFallback = () => {
    if (current && eventsSuppressed(node)) node.setAttribute("title", current);
    else node.removeAttribute("title");
  };

  const onEnter = () => {
    if (!current) return;
    clearTimeout(showTimer);
    showTimer = setTimeout(() => reveal(node, current), SHOW_DELAY);
  };
  const onFocus = () => {
    if (!current) return;
    clearTimeout(showTimer);
    reveal(node, current); // immediate for keyboard users
  };
  const onLeave = () => hide(node);

  node.addEventListener("mouseenter", onEnter);
  node.addEventListener("mouseleave", onLeave);
  node.addEventListener("focus", onFocus);
  node.addEventListener("blur", onLeave);

  // Re-sync the native fallback whenever `disabled` toggles (Svelte flips the
  // attribute reactively, often without the tip text changing).
  const observer = new MutationObserver(syncNativeFallback);
  observer.observe(node, { attributes: true, attributeFilter: ["disabled"] });
  syncNativeFallback();

  return {
    update(next: string | null | undefined) {
      current = next ?? "";
      syncNativeFallback();
      if (!current) {
        hide(node);
      } else if (owner === node && el) {
        el.textContent = current; // live-update while visible
        place(node, el);
      }
    },
    destroy() {
      observer.disconnect();
      node.removeEventListener("mouseenter", onEnter);
      node.removeEventListener("mouseleave", onLeave);
      node.removeEventListener("focus", onFocus);
      node.removeEventListener("blur", onLeave);
      node.removeAttribute("title");
      hide(node);
    },
  };
};
