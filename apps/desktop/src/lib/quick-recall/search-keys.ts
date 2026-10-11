// Quick Recall field keymap (IMPLEMENTATION.md §Quick Recall "Final keymap",
// Field section). The search input's onkeydown calls `handleSearchKeydown`; a
// window CAPTURE-phase listener calls `handleLauncherCaptureKeydown` so the
// pill popover owns its keys whatever DOM focus does in this WKWebView.
//
//   typing edits · → at the end accepts the ghost · Tab = search ⇄ ask ·
//   ↑/↓ move · ↵ opens the selected moment · ⌘↵ opens its page ·
//   ⌫ (or ←) at the start walks into the pills · Esc clears, then closes.
//
// Esc on an empty field is left alone: the layout's bubble handler closes
// the window. (Stopping an answer stream is ask mode's, QR5.)
//
// Quick Look (`handleQuickLookKeydown`, same capture listener): Space on an
// arrowed-into selection opens it; then ←/→ scrub · ↑/↓ other results ·
// ↵ opens the scrubbed frame · ⌘↵ page · ⌘C copy · Esc/Space back; typing
// closes it and edits the query.
import type { SearchStore } from "./searchStore.svelte";
import type { QuickLookStore } from "./detailStore.svelte";

const plain = (e: KeyboardEvent) => !e.metaKey && !e.ctrlKey && !e.altKey && !e.shiftKey;

function consume(event: KeyboardEvent): void {
  event.preventDefault();
  event.stopPropagation();
}

export function handleSearchKeydown(
  search: SearchStore,
  event: KeyboardEvent,
  onToggleMode: () => void,
): void {
  if (event.isComposing) return;
  const filters = search.filters;
  const el = search.inputEl;
  const atStart = el !== null && el.selectionStart === 0 && el.selectionEnd === 0;

  // A walked-into pill owns ⌫ / ← / → / ↵ / Esc; any other key lets it go.
  if (filters.pillIndex !== null) {
    const i = filters.pillIndex;
    switch (event.key) {
      case "Backspace":
      case "Delete":
        consume(event);
        filters.removePill(i);
        return;
      case "ArrowLeft":
        consume(event);
        filters.selectPill(Math.max(0, i - 1));
        return;
      case "ArrowRight":
        consume(event);
        filters.selectPill(i + 1 < search.pills.length ? i + 1 : null);
        return;
      case "Enter":
        consume(event);
        filters.openPopover(i);
        return;
      case "Escape":
        if (!plain(event)) return;
        consume(event);
        filters.selectPill(null);
        return;
    }
    filters.selectPill(null);
  }

  if (event.key === "Tab" && plain(event)) {
    event.preventDefault();
    search.commitPills();
    onToggleMode();
    return;
  }

  // → at the very end accepts the ghost; anywhere else it moves the caret.
  if (event.key === "ArrowRight" && plain(event) && filters.hasGhost && search.caretAtEnd) {
    event.preventDefault();
    filters.acceptGhost();
    return;
  }

  if (
    (event.key === "Backspace" || event.key === "ArrowLeft") &&
    !event.metaKey &&
    !event.ctrlKey &&
    !event.altKey &&
    atStart &&
    search.pills.length > 0
  ) {
    event.preventDefault();
    filters.selectPill(search.pills.length - 1);
    return;
  }

  if (event.key === "Escape" && plain(event)) {
    if (search.query.length > 0 || search.pills.length > 0) {
      consume(event);
      search.clearQuery();
    }
    return;
  }

  switch (event.key) {
    case "ArrowDown":
      event.preventDefault();
      search.moveSelection(1);
      break;
    case "ArrowUp":
      event.preventDefault();
      search.moveSelection(-1);
      break;
    case "Enter":
      event.preventDefault();
      search.commitPills();
      if (event.metaKey || event.ctrlKey) {
        if (search.selectedIndex >= 0) search.openSelectedResultUrl();
      } else if (search.selectedIndex >= 0) {
        search.openResultAt(search.selectedIndex);
      }
      break;
  }
}

// The pill popover's keys, focus-independent: ↑/↓ move, ↵ applies, ⌫ removes
// the pill, Esc/Tab close it. Any other key closes it and types on.
export function handleLauncherCaptureKeydown(
  search: SearchStore,
  event: KeyboardEvent,
  look?: QuickLookStore,
): void {
  const filters = search.filters;
  if (event.isComposing) return;
  if (!filters.popoverOpen) {
    if (look) handleQuickLookKeydown(search, look, event);
    return;
  }
  switch (event.key) {
    case "ArrowDown":
      consume(event);
      filters.popoverMove(1);
      return;
    case "ArrowUp":
      consume(event);
      filters.popoverMove(-1);
      return;
    case "Enter":
      consume(event);
      filters.applyOption();
      return;
    case "Backspace":
    case "Delete":
      consume(event);
      filters.removePill();
      return;
    case "Escape":
    case "Tab":
      consume(event);
      filters.closePopover();
      return;
  }
  if (!event.metaKey && !event.ctrlKey) filters.selectPill(null);
}

const typesText = (e: KeyboardEvent) =>
  (e.key.length === 1 && !e.metaKey && !e.ctrlKey) || e.key === "Backspace";

export function handleQuickLookKeydown(
  search: SearchStore,
  look: QuickLookStore,
  event: KeyboardEvent,
): void {
  const copyKey = event.metaKey && event.key.toLowerCase() === "c";
  if (look.result) {
    if (event.key === "Enter") {
      consume(event);
      if (event.metaKey) look.openPage();
      else void look.openHere();
      return;
    }
    if (copyKey) {
      consume(event);
      void look.copy();
      return;
    }
    const keys: Record<string, () => void> = {
      ArrowLeft: () => look.scrub(-1),
      ArrowRight: () => look.scrub(1),
      ArrowUp: () => look.step(-1),
      ArrowDown: () => look.step(1),
      Escape: () => look.close(),
      " ": () => look.close(),
      Tab: () => look.close(),
    };
    if (keys[event.key] && plain(event)) {
      consume(event);
      keys[event.key]();
    } else if (typesText(event)) {
      look.close(); // and let the key edit the query
      look.nav = false;
    }
    return;
  }
  const selected = search.selectedResult;
  if ((event.key === "ArrowDown" || event.key === "ArrowUp") && plain(event)) {
    look.nav = true; // the field's handler moves the selection
  } else if (event.key === " " && plain(event) && look.nav && selected) {
    consume(event);
    look.show(selected);
  } else if (copyKey && selected && search.inputEl?.selectionStart === search.inputEl?.selectionEnd) {
    consume(event);
    void look.copyResult(selected);
  } else if (typesText(event)) {
    look.nav = false;
  }
}
