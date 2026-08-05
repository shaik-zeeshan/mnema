/**
 * Keyboard row navigation for the settings pane (direction 04 "Command Deck").
 *
 * Settings rows are list rows here, not a soup of controls: ↑↓ steps the
 * selection, the selected row takes full-row accent selection, and ␣ activates
 * its primary control. The mockup draws that selection, so it has to be real —
 * a selection style with no keyboard behind it would be a lie.
 *
 * ponytail: DOM-driven, not reactive. Selection is one class + real DOM focus
 * on the row (`tabindex="-1"`), which is ~15 lines against threading a
 * selected-index through ~100 <SettingRow> instances and their panels. Focus
 * is the source of truth, so blur/click clears it for free.
 */

const ROW = ".setting-row";
const KEY_CLASS = "setting-row--key";

/** Rows the user can actually reach: rendered, not filtered out, not disabled. */
function visibleRows(root: HTMLElement): HTMLElement[] {
	return Array.from(root.querySelectorAll<HTMLElement>(ROW)).filter(
		(el) =>
			!el.classList.contains("setting-row--miss") &&
			!el.classList.contains("setting-row--disabled") &&
			el.offsetParent !== null,
	);
}

/**
 * Is this keystroke ours? Only when focus is NOT inside a control — every
 * slider, combobox and text field owns its own arrow/space keys, and stealing
 * them would break the controls the rows exist to hold.
 */
function ownsKeys(event: KeyboardEvent, root: HTMLElement): boolean {
	if (event.metaKey || event.ctrlKey || event.altKey || event.shiftKey) return false;
	const target = event.target;
	if (!(target instanceof HTMLElement)) return true;
	if (target === document.body || target === root) return true;
	// A selected row is ours. So is a tab: clicking one leaves focus on it, and
	// "switch section, then arrow down the rows" is the direction's core motion.
	return target.classList.contains(KEY_CLASS) || target.classList.contains("stab");
}

/** The control ␣ activates — the row's switch, checkbox, or first button. */
function primaryControl(row: HTMLElement): HTMLElement | null {
	return row.querySelector<HTMLElement>(
		'[role="switch"], input[type="checkbox"], button:not([disabled])',
	);
}

function select(row: HTMLElement | undefined, previous: HTMLElement | null) {
	previous?.classList.remove(KEY_CLASS);
	if (!row) return;
	row.classList.add(KEY_CLASS);
	row.tabIndex = -1;
	row.focus({ preventScroll: true });
	row.scrollIntoView({ block: "nearest" });
}

/**
 * Attach ↑↓/␣ row navigation to a settings scroll region.
 * Returns the teardown, so it drops straight out of an `$effect`.
 */
export function attachRowNav(root: HTMLElement): () => void {
	let current: HTMLElement | null = null;

	function onKeydown(event: KeyboardEvent) {
		if (event.key !== "ArrowDown" && event.key !== "ArrowUp" && event.key !== " ") return;
		if (!ownsKeys(event, root)) return;

		if (event.key === " ") {
			if (!current) return;
			event.preventDefault();
			primaryControl(current)?.click();
			return;
		}

		const rows = visibleRows(root);
		if (rows.length === 0) return;
		event.preventDefault();
		const at = current ? rows.indexOf(current) : -1;
		const next =
			event.key === "ArrowDown"
				? rows[at + 1 >= rows.length ? 0 : at + 1]
				: rows[at <= 0 ? rows.length - 1 : at - 1];
		select(next, current);
		current = next ?? null;
	}

	// Losing focus (a click elsewhere, Tab into a control) drops the selection —
	// two "selected" rows would be two lies at once.
	function onFocusOut(event: FocusEvent) {
		if (!current) return;
		const next = event.relatedTarget;
		if (next instanceof Node && current.contains(next)) return;
		current.classList.remove(KEY_CLASS);
		current = null;
	}

	window.addEventListener("keydown", onKeydown);
	root.addEventListener("focusout", onFocusOut);
	return () => {
		window.removeEventListener("keydown", onKeydown);
		root.removeEventListener("focusout", onFocusOut);
		current?.classList.remove(KEY_CLASS);
	};
}
