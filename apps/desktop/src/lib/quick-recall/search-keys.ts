// Quick Recall search-mode keyboard routing (extracted from the quick-recall
// +page.svelte in the search-mode extraction slice — no behavior change). The
// page-level listeners stay wired exactly as before: the search input's
// onkeydown calls `handleSearchKeydown`, and a window CAPTURE-phase listener
// calls `handleLauncherCaptureKeydown` (guarded by mode in the page). Both
// operate on the search store singleton passed in.
import type { SearchStore } from "./searchStore.svelte";

// The search input's keydown handler. `askAvailable` + `onAskAi` are the Ask
// AI pivot's inputs (they belong to the page's ask mode, so they're passed in
// rather than owned here).
export function handleSearchKeydown(
  search: SearchStore,
  event: KeyboardEvent,
  askAvailable: boolean,
  onAskAi: () => void,
): void {
  if (event.isComposing) {
    return;
  }
  const filters = search.filters;

  // While the syntax-help popover is open, a plain Escape just closes it and
  // is stopped here (preventDefault + stopPropagation) so it never reaches the
  // layout's window-close handler. This runs FIRST so the popover is the
  // innermost Escape target; when it's closed the branch is skipped and Escape
  // falls through to the normal search→close-window behavior. The picker and
  // ask-mode never coexist with this (the trigger only renders in search mode
  // and the popover is a transient overlay), so their Escape paths are
  // unaffected.
  if (
    search.syntaxHelpOpen &&
    event.key === "Escape" &&
    !event.metaKey &&
    !event.ctrlKey &&
    !event.altKey &&
    !event.shiftKey
  ) {
    event.preventDefault();
    event.stopPropagation();
    search.closeSyntaxHelp();
    return;
  }

  // While the picker is open it fully owns Arrow / Enter / Escape / Tab. Hand
  // the event to the picker FIRST; if it consumes it, none of the results
  // navigation / Ask AI pivot / ghost-accept below runs (exactly one list owns
  // the arrows at any instant).
  if (filters.pickerOpen) {
    if (filters.handlePickerKeydown(event)) {
      return;
    }
    // An unconsumed key while the picker is open (e.g. plain typing) falls
    // through to normal input handling, but the results-navigation switch at
    // the bottom is gated on !pickerOpen so it never double-drives selection.
  }

  // Ctrl+F / Cmd+F opens the Filter Picker from anywhere (empty or not) — a
  // launcher-native summon that doesn't depend on the input contents.
  if (
    (event.metaKey || event.ctrlKey) &&
    !event.altKey &&
    !event.shiftKey &&
    (event.key === "f" || event.key === "F")
  ) {
    event.preventDefault();
    if (!filters.pickerOpen) {
      filters.openPicker();
    }
    return;
  }

  // `/` on an EMPTY input opens the picker. preventDefault so no `/` is
  // inserted (the `/` was the trigger). Empty-input-only by design: once the
  // input is non-empty, `/` is a literal character (so `/usr/local/bin` stays
  // typeable), which is also why "Escape leaves a literal slash" holds — the
  // literal-slash path is exactly the non-empty case, where `/` never
  // triggers, so the picker can't have eaten a slash the user meant literally.
  if (
    event.key === "/" &&
    !event.metaKey &&
    !event.ctrlKey &&
    !event.altKey &&
    !event.shiftKey &&
    search.query.trim().length === 0 &&
    !filters.pickerOpen
  ) {
    event.preventDefault();
    filters.openPicker();
    return;
  }

  // While the Filter Value List is up it fully owns ↑/↓/Enter/Escape so exactly
  // one list consumes the arrows. It sits ABOVE the Ask AI pivot so Ctrl+Enter
  // is suppressed while the list is up (Esc out first). Tab and → fall through
  // to ghost-accept below (a value-accept), so they are intentionally NOT here.
  if (filters.valueListActive) {
    switch (event.key) {
      case "ArrowDown":
        event.preventDefault();
        filters.moveValueListSelection(1);
        return;
      case "ArrowUp":
        event.preventDefault();
        filters.moveValueListSelection(-1);
        return;
      case "Enter":
        event.preventDefault();
        // Plain Enter commits the highlighted enabled row; Ctrl/Cmd+Enter is
        // suppressed (no Ask AI pivot while the list is up) and is a no-op.
        if (!event.metaKey && !event.ctrlKey) {
          filters.commitHighlightedValueListRow();
        }
        return;
      case "Escape":
        if (!event.metaKey && !event.ctrlKey && !event.altKey && !event.shiftKey) {
          event.preventDefault();
          event.stopPropagation();
          filters.abandonOperator();
          return;
        }
        break;
    }
  }

  // The Ask AI pivot is Ctrl/Cmd+Enter ONLY (ADR 0025). Tab is reserved for
  // ghost-text accept (handled below), never the pivot. The Filter Value List
  // block above suppresses this pivot while it's up (Enter is consumed there).
  if (askAvailable && event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
    event.preventDefault();
    onAskAi();
    return;
  }

  // Tab accepts the inline ghost completion at ANY caret position (the `→`
  // accept below additionally works, but only at end-of-input, fish-style).
  // Tab no longer pivots to Ask AI. When no ghost is showing, Tab is a no-op
  // that keeps focus in the launcher rather than escaping via native focus
  // traversal. Shift+Tab is left to native focus traversal.
  if (
    event.key === "Tab" &&
    !event.shiftKey &&
    !event.metaKey &&
    !event.ctrlKey &&
    !event.altKey
  ) {
    event.preventDefault();
    if (filters.hasGhost) {
      filters.acceptGhost();
    }
    return;
  }

  // ⌘/Ctrl+1–9 jumps SELECTION to the Nth visible result (select = preview;
  // it no longer opens — Enter is the open action).
  if ((event.metaKey || event.ctrlKey) && /^[1-9]$/.test(event.key)) {
    const index = Number(event.key) - 1;
    if (index < search.resultCount) {
      event.preventDefault();
      search.selectResultAt(index);
    }
    return;
  }

  // ⌘/Ctrl+O opens the selected frame result's captured page in the browser —
  // the keyboard path to each card's hover-only "open in browser" chip, which
  // can't be a tab stop inside this aria-activedescendant listbox. A no-op when
  // nothing is selected or the selection has no openable URL (audio / no link).
  if (
    (event.metaKey || event.ctrlKey) &&
    !event.altKey &&
    !event.shiftKey &&
    (event.key === "o" || event.key === "O")
  ) {
    event.preventDefault();
    search.openSelectedResultUrl();
    return;
  }

  // Backspace with a collapsed caret at position 0 removes the LAST chip
  // (rightmost, nearest the caret) instead of deleting text — the chip row
  // reads as an extension of the query, so a leading Backspace peels off the
  // most-recently-applied scope. Any non-zero caret or selection falls through
  // to native Backspace. Guarded to plain Backspace so it never collides with
  // the Tab / ⌘Enter Ask AI pivot or the arrow-navigation switch below.
  if (
    event.key === "Backspace" &&
    !event.metaKey &&
    !event.ctrlKey &&
    !event.altKey &&
    search.activeFilterChips.length > 0 &&
    search.inputEl !== null &&
    search.inputEl.selectionStart === 0 &&
    search.inputEl.selectionEnd === 0
  ) {
    event.preventDefault();
    search.removeChip(search.activeFilterChips[search.activeFilterChips.length - 1]);
    return;
  }

  // ArrowRight (→) also accepts the inline ghost — but ONLY at end-of-input
  // (so → isn't used to move through existing text). Tab accepts at any caret
  // position (above). Only as a plain keypress: Enter/ArrowUp/ArrowDown above
  // keep their meaning, so ghost-text never fights results navigation or the
  // Ask AI pivot. When the gate fails we DON'T preventDefault, letting → do
  // its native cursor move. Handled before the switch so it never
  // preventDefaults unconditionally.
  if (
    event.key === "ArrowRight" &&
    !event.metaKey &&
    !event.ctrlKey &&
    !event.altKey &&
    !event.shiftKey &&
    filters.hasGhost &&
    search.caretAtEnd
  ) {
    event.preventDefault();
    filters.acceptGhost();
    return;
  }

  // The picker owns navigation while open (handled + returned above), so the
  // roving results-list selection below must NOT also run. The Filter Value
  // List likewise owns the arrows while up, so Home/End and any stray key must
  // not drive the results list underneath it. This guard backstops the early
  // returns in case an unconsumed key reaches here.
  if (filters.pickerOpen || filters.valueListActive) {
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
    case "Home":
      if (search.resultCount > 0) {
        event.preventDefault();
        search.selectedIndex = 0;
      }
      break;
    case "End":
      if (search.resultCount > 0) {
        event.preventDefault();
        search.selectedIndex = search.resultCount - 1;
      }
      break;
    case "Enter":
      // Enter = open the SELECTED result in the main-window timeline + close
      // Quick Recall (selection itself only previews in the detail pane).
      if (search.selectedIndex >= 0) {
        event.preventDefault();
        search.openResultAt(search.selectedIndex);
      }
      break;
  }
}

// Launcher sub-surface keys via a WINDOW CAPTURE listener (focus-independent).
//
// DOM focus is unreliable in this WKWebView (the same reason the app uses
// window capture-phase keydown listeners elsewhere instead of element
// onkeydown). So while the Filter Picker or Filter Value List is up we must
// NOT rely on the search input keeping focus to own Escape/Arrow/Enter. If
// focus drifts off the input, a plain Escape would otherwise reach the
// layout's bubble-phase `dismissQuickRecallOnEscape` and close the ENTIRE
// Quick Recall window rather than just the open sub-surface.
//
// This runs in the CAPTURE phase (before the layout's bubble handler and
// regardless of focus). While a sub-surface is open it owns Escape/Arrow/
// Enter, calling the same helpers the input-level handlers use, and stops
// propagation so neither handleSearchKeydown nor the layout window-close runs
// for those keys. Every other key (typing, Tab/ghost, Ctrl+Enter when no value
// list) is left untouched so the focused input still handles it normally. When
// nothing is open this does nothing, so a plain-search Escape still closes the
// window. (The page's wrapper additionally gates on search mode + isComposing.)
export function handleLauncherCaptureKeydown(
  search: SearchStore,
  event: KeyboardEvent,
): void {
  const filters = search.filters;
  const plain =
    !event.metaKey && !event.ctrlKey && !event.altKey && !event.shiftKey;

  // Syntax-help popover closes on a plain Escape.
  if (search.syntaxHelpOpen && event.key === "Escape" && plain) {
    event.preventDefault();
    event.stopPropagation();
    search.closeSyntaxHelp();
    return;
  }

  if (filters.pickerOpen) {
    switch (event.key) {
      case "Escape":
        if (!plain) return;
        event.preventDefault();
        event.stopPropagation();
        filters.closePicker();
        return;
      case "ArrowDown":
        event.preventDefault();
        event.stopPropagation();
        filters.pickerMove(1);
        return;
      case "ArrowUp":
        event.preventDefault();
        event.stopPropagation();
        filters.pickerMove(-1);
        return;
      case "Enter":
        event.preventDefault();
        event.stopPropagation();
        filters.pickerSelectHighlighted();
        return;
    }
    return;
  }

  if (filters.valueListActive) {
    switch (event.key) {
      case "Escape":
        if (!plain) return;
        event.preventDefault();
        event.stopPropagation();
        filters.abandonOperator();
        return;
      case "ArrowDown":
        event.preventDefault();
        event.stopPropagation();
        filters.moveValueListSelection(1);
        return;
      case "ArrowUp":
        event.preventDefault();
        event.stopPropagation();
        filters.moveValueListSelection(-1);
        return;
      case "Enter":
        // Plain Enter commits the highlighted row; Ctrl/Cmd+Enter is suppressed
        // (no Ask AI pivot while the value list is up).
        event.preventDefault();
        event.stopPropagation();
        if (!event.metaKey && !event.ctrlKey) {
          filters.commitHighlightedValueListRow();
        }
        return;
    }
    return;
  }
}
