// Quick Recall field surfaces: the inline ghost completion (→ accepts it at
// the end of the text) and the pills — which one ⌫/← walked into, and the
// popover that edits it (recall.html `.rc-tok` + `.rc-pop`). Composed INTO the
// search store (`search.filters`) because both work on the store's query,
// caret and pills.
import { tick } from "svelte";
import { trailingToken } from "./query-tokens";
import { pillEditText, pillLabel, pillOperators } from "./filter-chips";
import type { PartOfDay, Pill } from "./natural-tokens";
import type { SearchStore } from "./searchStore.svelte";

// The canonical operator names the ghost completes. Order matters only for
// first match.
const GHOST_OPERATORS = ["app:", "source:", "date:", "after:", "before:"] as const;
// The enumerable operator values the ghost completes (short spellings the
// parser accepts).
const GHOST_VALUES: Record<string, readonly string[]> = {
  "source:": ["mic", "system", "screen"],
  "date:": ["today", "yesterday", "this-week", "last-week"],
};

const PARTS: PartOfDay[] = ["morning", "afternoon", "evening"];

// One row of a pill's popover: the pill it would become (null = drop the
// filter). `op` keys the facet counts (`search.facetCounts`).
export type PillOption = { label: string; pill: Pill | null; op: string };

const option = (pill: Pill | null, label = pill ? pillLabel(pill) : ""): PillOption => ({
  label,
  pill,
  op: pill ? pillOperators(pill) : "",
});

// The alternatives offered for one pill (recall.html `showPop`).
export function pillOptions(pill: Pill, appNames: readonly string[]): PillOption[] {
  if (pill.kind === "app") {
    return [...appNames.map((app) => option({ kind: "app", app })), option(null, "Any app")];
  }
  if (pill.kind === "source") {
    return [
      option({ kind: "source", source: "screen" }),
      option({ kind: "source", source: "mic" }, "On a call (mic)"),
      option({ kind: "source", source: "system" }),
      option(null, "Any source"),
    ];
  }
  // Parts of the day only narrow one day (or stand alone).
  const oneDay = pill.value === null || !/-(week|month)$/.test(pill.value);
  return [
    ...(oneDay ? PARTS.map((part) => option({ ...pill, part })) : []),
    ...(oneDay && pill.value !== null
      ? [option({ ...pill, part: null }, `${pill.label} · all day`)]
      : []),
    option({ kind: "date", value: "this-week", label: "This week", part: null }),
    option({ kind: "date", value: "last-week", label: "Last week", part: null }),
    option(null, "Any time"),
  ];
}

export class FilterSurfaces {
  #store: SearchStore;

  constructor(store: SearchStore) {
    this.#store = store;
  }

  // ── Pills ──────────────────────────────────────────────────────────────────
  // ⌫ (or ←) at the start of the text walks into the pills; the walked-into
  // pill reads pressed. Its popover (click, or ↵ on it) offers alternatives,
  // Edit as text, and Remove. DOM focus stays on the input throughout.

  pillIndex = $state<number | null>(null);
  popoverOpen = $state(false);
  popoverIndex = $state(0);

  popoverOptions = $derived.by<PillOption[]>(() => {
    const pill = this.pillIndex === null ? undefined : this.#store.pills[this.pillIndex];
    return this.popoverOpen && pill ? pillOptions(pill, this.#store.appNames) : [];
  });

  selectPill(index: number | null): void {
    this.pillIndex = index;
    if (index === null) this.popoverOpen = false;
  }

  openPopover(index: number): void {
    const pill = this.#store.pills[index];
    if (!pill) return;
    this.pillIndex = index;
    const current = pillOperators(pill);
    this.popoverIndex = Math.max(
      0,
      pillOptions(pill, this.#store.appNames).findIndex((o) => o.op === current),
    );
    this.popoverOpen = true;
    if (pill.kind === "app") void this.#store.ensureSearchableAppsLoaded();
  }

  closePopover(): void {
    this.popoverOpen = false;
  }

  popoverMove(delta: number): void {
    const n = this.popoverOptions.length;
    if (n > 0) this.popoverIndex = (this.popoverIndex + delta + n) % n;
  }

  // Swap the pressed pill for an option (null drops it).
  applyOption(choice: PillOption | undefined = this.popoverOptions[this.popoverIndex]): void {
    if (!choice || this.pillIndex === null) return;
    const i = this.pillIndex;
    this.#store.pills = choice.pill
      ? this.#store.pills.map((p, k) => (k === i ? (choice.pill as Pill) : p))
      : this.#store.pills.filter((_, k) => k !== i);
    this.#done();
  }

  removePill(index: number | null = this.pillIndex): void {
    if (index === null) return;
    this.#store.pills = this.#store.pills.filter((_, k) => k !== index);
    this.#done();
  }

  // Put the pill back into the text as the phrase it came from.
  editAsText(): void {
    const pill = this.pillIndex === null ? undefined : this.#store.pills[this.pillIndex];
    if (!pill) return;
    this.removePill();
    this.#store.query = `${this.#store.query.trim()} ${pillEditText(pill)}`.trim();
    this.#store.scheduleCommit();
  }

  #done(): void {
    this.pillIndex = null;
    this.popoverOpen = false;
    void this.#caretToEnd();
  }

  async #caretToEnd(): Promise<void> {
    await tick();
    const el = this.#store.inputEl;
    if (el === null) return;
    el.focus();
    el.setSelectionRange(el.value.length, el.value.length);
    this.#store.caretAtEnd = true;
  }

  // ── Inline ghost completion ────────────────────────────────────────────────
  // A dimmed completion trailing the caret, accepted with → at the end of the
  // text (Tab flips search ⇄ ask). Completes operator names, the enumerable
  // source:/date: values, app: values, and an app name after in/on/from/using.

  #completeApp(typed: string): string | null {
    if (typed.length === 0 || typed.startsWith('"')) return null;
    const apps = this.#store.appNames;
    if (apps.length === 0) {
      // Kick off the lazy load so the next partial can complete; no ghost yet.
      void this.#store.ensureSearchableAppsLoaded();
      return null;
    }
    const lower = typed.toLowerCase();
    return apps.find((a) => a.toLowerCase().startsWith(lower) && a.length > typed.length) ?? null;
  }

  // The ghost SUFFIX (or a NUL-terminated quoted app-value replacement), or null.
  ghostRaw = $derived.by<string | null>(() => {
    if (!this.#store.caretAtEnd || this.pillIndex !== null) return null;
    const query = this.#store.query;
    const token = trailingToken(query);
    if (token.length === 0) return null;
    const colon = token.indexOf(":");
    if (colon === -1) {
      if (!/^[a-z]+$/i.test(token)) return null;
      if (/\b(?:in|on|from|using)\s+\S+$/i.test(query)) {
        const app = this.#completeApp(token);
        if (app) return app.slice(token.length);
      }
      if (token.length < 2) return null;
      const lower = token.toLowerCase();
      const op = GHOST_OPERATORS.find((o) => o.startsWith(lower) && o.length > lower.length);
      return op ? op.slice(token.length) : null;
    }
    const operator = token.slice(0, colon + 1).toLowerCase();
    const typed = token.slice(colon + 1);
    if (operator === "app:") {
      const app = this.#completeApp(typed);
      // A name with a space is re-emitted quoted (sentinel NUL; see acceptGhost).
      return app === null ? null : app.includes(" ") ? `"${app}"\0` : app.slice(typed.length);
    }
    const lower = typed.toLowerCase();
    const value = lower
      ? GHOST_VALUES[operator]?.find((v) => v.startsWith(lower) && v.length > lower.length)
      : undefined;
    return value ? value.slice(typed.length) : null;
  });

  ghostIsQuotedAppValue = $derived(this.ghostRaw !== null && this.ghostRaw.endsWith("\0"));

  // The dimmed text drawn after the typed text.
  ghostCompletion = $derived.by<string | null>(() => {
    if (this.ghostRaw === null) return null;
    if (!this.ghostIsQuotedAppValue) return this.ghostRaw;
    const token = trailingToken(this.#store.query);
    const typed = token.slice(token.indexOf(":") + 1);
    const name = this.ghostRaw.slice(1, -2); // `"Full Name"\0` → Full Name
    return `${name.slice(typed.length)}"`;
  });

  hasGhost = $derived(this.ghostCompletion !== null && this.ghostCompletion.length > 0);

  // Accept the ghost. An operator NAME leaves the caret ready for its value;
  // a completed value gets a trailing space so typing continues.
  acceptGhost(): boolean {
    const raw = this.ghostRaw;
    if (raw === null) return false;
    const query = this.#store.query;
    const token = trailingToken(query);
    if (this.ghostIsQuotedAppValue) {
      const operator = token.slice(0, token.indexOf(":") + 1);
      this.#store.query = `${query.slice(0, query.length - token.length)}${operator}${raw.slice(0, -1)} `;
    } else {
      this.#store.query = query + raw + (raw.endsWith(":") ? "" : " ");
    }
    void this.#caretToEnd();
    return true;
  }
}
