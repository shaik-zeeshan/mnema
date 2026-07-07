// Quick Recall filter-input surfaces: the Filter Picker (ADR 0025 path B), the
// inline ghost-text autocomplete (path A), and the Filter Value List (typed
// path). Extracted from the quick-recall +page.svelte in the search-mode
// extraction slice — no behavior change. All three cluster around the raw
// query text / caret / app catalog owned by the SearchStore, so this class is
// composed INTO the store (`searchStore.filters`) rather than being a second
// standalone singleton; components and key handlers reach it via the store.
import { tick } from "svelte";
import { trailingToken, appOperatorToken } from "./query-tokens";
import type { SearchStore } from "./searchStore.svelte";

// The three categories, in display order. `level` is the operator family the
// category commits (its stub is written and the value list takes over).
export const PICKER_CATEGORIES = [
  { id: "app", label: "App", hint: "Narrow to one captured app", level: "app" as const },
  {
    id: "source",
    label: "Source",
    hint: "Microphone, system audio, or screen",
    level: "source" as const,
  },
  {
    id: "date",
    label: "Date range",
    hint: "A day, a preset, or a typed range",
    level: "date" as const,
  },
];

// The three fixed Source values and the operator each commits.
export const PICKER_SOURCES = [
  { id: "mic", label: "Microphone audio", token: "source:mic" },
  { id: "system", label: "System audio", token: "source:system" },
  { id: "screen", label: "Screen", token: "source:screen" },
];

// Date presets. The backend parser (crates/app-infra/src/search.rs) accepts
// `date:today` / `date:yesterday` as named-period day spans, and relative
// point tokens `Nd` for after:/before: — so "Last 7/30 days" commit `after:7d`
// / `after:30d` (an open-ended "since N days ago" window) rather than computing
// absolute dates. Verified against resolve_day_or_period / resolve_point_date.
export const PICKER_DATE_PRESETS = [
  { id: "today", label: "Today", token: "date:today" },
  { id: "yesterday", label: "Yesterday", token: "date:yesterday" },
  { id: "last7", label: "Last 7 days", token: "after:7d" },
  { id: "last30", label: "Last 30 days", token: "after:30d" },
];

// The canonical operator names the ghost completes. Order matters only for
// first match.
const GHOST_OPERATORS = ["app:", "source:", "date:", "after:", "before:"] as const;
// The canonical source: values we complete (short spellings the parser accepts).
const GHOST_SOURCE_VALUES = ["mic", "system", "screen"] as const;

export const PICKER_OPT_PREFIX = "qr-picker-opt-";
export const VALUE_LIST_OPT_PREFIX = "qr-vl-opt-";

export class FilterSurfaces {
  #store: SearchStore;

  constructor(store: SearchStore) {
    this.#store = store;
  }

  // ── Filter Picker (category door) ──────────────────────────────────────────
  // A launcher-native overlay that REPLACES the results region and, while open,
  // FULLY OWNS Arrow / Enter / Escape so exactly one list consumes arrows at
  // any instant. It is a CATEGORY DOOR ONLY: selecting App / Source / Date
  // writes that operator's stub (`app:` / `source:` / `date:`) into the raw
  // query and immediately hands off to the Filter Value List — there is no
  // in-picker drilling and no second navigable list. DOM focus stays on the
  // search input (aria-activedescendant pattern, like the results listbox).

  // Whether the picker overlay is open (replacing the results region).
  pickerOpen = $state(false);
  // The highlighted category index within the root category list.
  pickerIndex = $state(0);
  // Bound to the picker overlay element.
  pickerEl = $state<HTMLDivElement | null>(null);

  // The App value list: captured-app names from `searchableApps`, in recency
  // order. The backend collapses by app IDENTITY (bundle id, else name), so two
  // distinct bundle ids that share a display name can return two rows with the
  // same name. We dedupe by name (case-insensitive, first/most-recent wins) so
  // the value list never renders two identical rows: a duplicate key in the
  // `{#each}` would reconcile to a stale node and swallow the first
  // click/enter. The value commits `app:<name>`, so collapsing same-name rows
  // is also semantically correct.
  pickerAppNames = $derived.by<string[]>(() => {
    const apps = this.#store.searchableApps;
    if (apps === null) {
      return [];
    }
    const seen = new Set<string>();
    const names: string[] = [];
    for (const app of apps) {
      const name = (app.name ?? "").trim();
      if (name.length === 0) {
        continue;
      }
      const key = name.toLowerCase();
      if (seen.has(key)) {
        continue;
      }
      seen.add(key);
      names.push(name);
    }
    return names;
  });

  // The number of arrow-navigable categories, used to clamp pickerIndex on
  // ArrowUp/ArrowDown. The picker is category-only, so this is constant.
  pickerItemCount = $derived(PICKER_CATEGORIES.length);

  // The active option id for aria-activedescendant on the search input while
  // the picker is open, mirroring the results listbox's activeOptionId pattern.
  pickerActiveOptionId = $derived(
    this.pickerOpen && this.pickerItemCount > 0
      ? `${PICKER_OPT_PREFIX}${this.pickerIndex}`
      : undefined,
  );

  // Open the picker fresh at the category list, resetting the highlight. DOM
  // focus stays on the search input (its keydown owns picker navigation while
  // open), so we refocus it rather than the overlay. The captured-app catalog
  // is warmed here so the App value list has selectable rows the instant the
  // App category is chosen.
  openPicker(): void {
    this.pickerOpen = true;
    this.pickerIndex = 0;
    void this.#store.ensureSearchableAppsLoaded();
    void tick().then(() => this.#store.inputEl?.focus());
  }

  // Close the picker and return DOM focus to the search input so the search
  // (now carrying any just-appended operator) reruns and the chip appears.
  closePicker(): void {
    this.pickerOpen = false;
    this.pickerIndex = 0;
    void tick().then(() => this.#store.inputEl?.focus());
  }

  // Selecting a category writes its operator STUB (`app:` / `source:` /
  // `date:`) into the query and closes the picker. The caret then sits in an
  // un-committed operator value, so the Filter Value List opens and shows that
  // operator's values — the SAME surface the typed path reaches. No trailing
  // space is added so the empty value keeps the full list up.
  commitOperatorStub(level: "app" | "source" | "date"): void {
    const stub = level === "app" ? "app:" : level === "source" ? "source:" : "date:";
    const base = this.#store.query.trimEnd();
    this.#store.query = base.length > 0 ? `${base} ${stub}` : stub;
    // Close the category door and open the value list in the SAME reactive
    // flush: `caretAtEnd` is set synchronously (we know the caret lands at end)
    // so `activeOperatorContext` becomes non-null the instant `pickerOpen`
    // flips false — the surface swaps in one step with no empty flash and no
    // dependence on focus/tick timing (WebKit focus restore is unreliable in
    // this webview).
    this.pickerOpen = false;
    this.pickerIndex = 0;
    this.#store.caretAtEnd = true;
    // Warm the app catalog so the App value list has selectable rows immediately.
    if (level === "app") {
      void this.#store.ensureSearchableAppsLoaded();
    }
    void tick().then(() => {
      const el = this.#store.inputEl;
      if (el !== null) {
        el.focus();
        const end = el.value.length;
        el.setSelectionRange(end, end);
        this.#store.caretAtEnd = true;
      }
    });
  }

  // Commit the highlighted category: write its operator stub into the query
  // and hand off to the Filter Value List. The picker is a category door only —
  // it never selects values itself, so this is its single commit path.
  pickerSelectHighlighted(): void {
    const category = PICKER_CATEGORIES[this.pickerIndex];
    if (category) {
      this.commitOperatorStub(category.level);
    }
  }

  // Move the category highlight, wrapping at the ends.
  pickerMove(delta: number): void {
    const count = this.pickerItemCount;
    if (count === 0) {
      return;
    }
    this.pickerIndex = (this.pickerIndex + delta + count) % count;
  }

  // Picker key ownership: while the picker is open, it consumes Arrow/Enter/
  // Escape/Tab before the normal results navigation runs (handleSearchKeydown
  // calls this first and returns early when it handles the event). Returns true
  // when the event was consumed. Tab is ALWAYS suppressed while open so the Ask
  // AI pivot can't fire mid-picker (no ambiguous owner of the keys).
  handlePickerKeydown(event: KeyboardEvent): boolean {
    if (!this.pickerOpen || event.isComposing) {
      return false;
    }

    // Tab never pivots to Ask AI while the picker owns the keys.
    if (event.key === "Tab") {
      event.preventDefault();
      return true;
    }

    switch (event.key) {
      case "Escape":
        event.preventDefault();
        event.stopPropagation();
        this.closePicker();
        return true;
      case "Enter":
      case "ArrowRight":
        event.preventDefault();
        this.pickerSelectHighlighted();
        return true;
      case "ArrowDown":
        event.preventDefault();
        this.pickerMove(1);
        return true;
      case "ArrowUp":
        event.preventDefault();
        this.pickerMove(-1);
        return true;
    }

    return false;
  }

  // ── Inline ghost-text autocomplete (ADR 0025, path A) ──────────────────────
  // Ambient dimmed completion of known Field Operator names and their two
  // enumerable value vocabularies, shown trailing the caret. It NEVER consumes
  // navigation keys — the accept is bound to ArrowRight at end-of-input ONLY
  // (plus Tab at any caret position), so Enter/ArrowUp/ArrowDown keep their
  // existing meaning. The ghost is a pure $derived off `query` + caretAtEnd +
  // the lazily-loaded app catalog.
  //
  // Completes:
  //   - operator NAMES: app: / source: / date: / after: / before:
  //   - source: VALUES: mic / system / screen (canonical short spellings)
  //   - app: VALUES: from list_searchable_apps (case-insensitive on name);
  //     a completion containing a space inserts the quoted form (app:"Name").
  //   - date:/after:/before: VALUES are NOT completed (free-form, no vocab).

  // Compute the ghost SUFFIX for a trailing operator-value partial, or null.
  // The partial is everything after the first ":" of the trailing token. For
  // app: values we may need quoting, so we return the suffix that, appended to
  // the already-typed partial, yields a valid token — quoting the WHOLE value
  // when the completion contains a space (sentinel NUL form).
  #ghostForValue(operator: string, typedValue: string): string | null {
    if (operator === "source:") {
      const lower = typedValue.toLowerCase();
      if (lower.length === 0) {
        return null;
      }
      for (const value of GHOST_SOURCE_VALUES) {
        if (value.startsWith(lower) && value.length > lower.length) {
          // Source values never contain spaces; plain suffix.
          return value.slice(typedValue.length);
        }
      }
      return null;
    }

    if (operator === "app:") {
      // Don't try to complete an already-quoted partial (the user is steering
      // the quoting themselves); keep it simple per the plan.
      if (typedValue.startsWith('"')) {
        return null;
      }
      const apps = this.#store.searchableApps;
      if (apps === null) {
        // Kick off the lazy load so the next partial can complete; no ghost yet.
        void this.#store.ensureSearchableAppsLoaded();
        return null;
      }
      const lower = typedValue.toLowerCase();
      if (lower.length === 0) {
        return null;
      }
      for (const app of apps) {
        const name = (app.name ?? "").trim();
        if (name.length === 0) {
          continue;
        }
        if (name.toLowerCase().startsWith(lower) && name.length > typedValue.length) {
          if (name.includes(" ")) {
            // Completing to a name with a space: re-emit the whole value quoted,
            // so the trailing token becomes app:"Full Name". The suffix replaces
            // the unquoted partial entirely (the accept handler swaps the token).
            return `"${name}"\0`;
          }
          return name.slice(typedValue.length);
        }
      }
      return null;
    }

    return null;
  }

  // The active ghost completion (the dimmed SUFFIX to append), or null. Only
  // shown when the caret is at end-of-input. Derives the operator/value tier
  // from the trailing token of `query`. A trailing app-value completion that
  // requires quoting is encoded with a sentinel NUL (see #ghostForValue) and is
  // resolved separately at accept time; for DISPLAY we strip the sentinel.
  ghostRaw = $derived.by<string | null>(() => {
    if (!this.#store.caretAtEnd) {
      return null;
    }
    const token = trailingToken(this.#store.query);
    if (token.length === 0) {
      return null;
    }

    const colon = token.indexOf(":");
    if (colon === -1) {
      // No colon yet → completing an operator NAME from a partial at the token
      // start. Only complete a bare alphabetic partial (not e.g. "-app").
      if (!/^[a-z]+$/i.test(token)) {
        return null;
      }
      const lower = token.toLowerCase();
      for (const op of GHOST_OPERATORS) {
        if (op.startsWith(lower) && op.length > lower.length) {
          return op.slice(token.length);
        }
      }
      return null;
    }

    // Has a colon → completing an operator VALUE. Only the enumerable vocabs.
    const operator = token.slice(0, colon + 1).toLowerCase();
    const typedValue = token.slice(colon + 1);
    return this.#ghostForValue(operator, typedValue);
  });

  // Whether the active ghost is a quoted app-value replacement (sentinel form).
  ghostIsQuotedAppValue = $derived(
    this.ghostRaw !== null && this.ghostRaw.endsWith("\0"),
  );

  // The display suffix (dimmed text shown after the typed text). For the quoted
  // app-value case we still want the overlay to read sensibly: the typed
  // partial gets visually replaced, so we show the remaining characters of the
  // quoted name (closing-quote-stripped suffix relative to what's typed).
  ghostCompletion = $derived.by<string | null>(() => {
    if (this.ghostRaw === null) {
      return null;
    }
    if (this.ghostIsQuotedAppValue) {
      // Sentinel-encoded: the replacement is `"Full Name"` for the whole value.
      // For display, show the tail beyond the already-typed partial characters.
      const token = trailingToken(this.#store.query);
      const colon = token.indexOf(":");
      const typedValue = colon === -1 ? "" : token.slice(colon + 1);
      const quoted = this.ghostRaw.slice(0, -1); // strip sentinel → `"Full Name"`
      // The opening quote sits before the typed partial; the visible ghost is
      // the remainder of the name + closing `"`.
      const inner = quoted.slice(1, -1); // Full Name
      if (!inner.toLowerCase().startsWith(typedValue.toLowerCase())) {
        return null;
      }
      return `${inner.slice(typedValue.length)}"`;
    }
    return this.ghostRaw;
  });

  // True when a ghost completion is currently shown (drives the accept gate).
  // Suppressed while the Filter Picker is open so the dimmed completion doesn't
  // paint behind the overlay and ArrowRight stays owned by the picker.
  hasGhost = $derived(
    !this.pickerOpen &&
      this.ghostCompletion !== null &&
      this.ghostCompletion.length > 0,
  );

  // Accept the active ghost: mutate `query` to include the completion, move the
  // caret to the end, and clear the ghost (it re-derives empty). Operator-NAME
  // accepts leave the caret ready for a value (no trailing space). Value
  // accepts complete a full operator token and add a trailing space so typing
  // continues. Returns true when something was accepted.
  acceptGhost(): boolean {
    if (this.ghostRaw === null) {
      return false;
    }
    const token = trailingToken(this.#store.query);
    const colon = token.indexOf(":");

    if (this.ghostIsQuotedAppValue) {
      // Replace the trailing unquoted `app:partial` token with `app:"Full Name"`.
      const operator = token.slice(0, colon + 1); // preserves typed case of key
      const quoted = this.ghostRaw.slice(0, -1); // `"Full Name"`
      const replaced =
        this.#store.query.slice(0, this.#store.query.length - token.length) +
        operator +
        quoted;
      this.#store.query = `${replaced} `;
    } else if (colon === -1) {
      // Operator NAME accept: append the suffix; no trailing space (value next).
      this.#store.query = this.#store.query + this.ghostRaw;
    } else {
      // Operator VALUE accept (source: or unquoted app:): append + trailing space.
      this.#store.query = this.#store.query + this.ghostRaw + " ";
    }

    // Move the caret to the very end after the value commits.
    void tick().then(() => {
      const el = this.#store.inputEl;
      if (el !== null) {
        const end = el.value.length;
        el.setSelectionRange(end, end);
        this.#store.caretAtEnd = true;
      }
    });
    return true;
  }

  // ── Filter Value List (typed path) ─────────────────────────────────────────
  // When the caret sits in an UN-COMMITTED field-operator value — the trailing
  // token of `query` is `app:…`/`source:…`/`date:…`/`after:…`/`before:…`, the
  // caret is at end-of-input, and the picker is closed — the results region is
  // REPLACED by a value list for that operator. It fully owns ↑/↓/Enter/Escape
  // while up. This is the keyboard-native sibling of the Filter Picker: the
  // same value vocabularies, the same operator-token commit seam, reusing the
  // picker's CSS. Mutually exclusive with the picker by construction:
  // activeOperatorContext returns null while `pickerOpen`.

  // The active operator + the value typed so far, or null when the value list
  // shouldn't be up. Null while the picker is open or the caret isn't at end
  // (the ghost/value affordances only ever act at end-of-input). The operator
  // key is lowercased and colon-suffixed.
  activeOperatorContext = $derived.by<
    | {
        operator: "app:" | "source:" | "date:" | "after:" | "before:";
        typedValue: string;
      }
    | null
  >(() => {
    if (this.pickerOpen || !this.#store.caretAtEnd) {
      return null;
    }
    const match = trailingToken(this.#store.query).match(
      /^(app|source|date|after|before):(.*)$/i,
    );
    if (match === null) {
      return null;
    }
    const operator = `${match[1].toLowerCase()}:` as
      | "app:"
      | "source:"
      | "date:"
      | "after:"
      | "before:";
    return { operator, typedValue: match[2] };
  });

  // Whether the Filter Value List currently owns the results region + arrows.
  valueListActive = $derived(this.activeOperatorContext !== null);

  // The rows for the active operator's value list. App names filter as a
  // SUBSTRING match on the typed value; source values filter on canonical
  // value/label; date operators always show the four presets (date values are
  // free-form, so we never filter them by the typed text). `disabled` marks
  // rows that would create a structural conflict with an already-active chip
  // (app+audio source are mutually exclusive at the operator level).
  valueListRows = $derived.by<
    Array<{ id: string; label: string; token: string; disabled: boolean }>
  >(() => {
    const context = this.activeOperatorContext;
    if (context === null) {
      return [];
    }
    const hasAppChip = this.#store.activeFilterChips.some((c) => c.kind === "app");
    const hasAudioSourceChip = this.#store.activeFilterChips.some(
      (c) => c.kind === "source" && c.data.source !== "screen",
    );
    const typed = context.typedValue.toLowerCase();

    if (context.operator === "app:") {
      return this.pickerAppNames
        .filter((name) => typed.length === 0 || name.toLowerCase().includes(typed))
        .map((name) => ({
          id: `app:${name}`,
          label: name,
          token: appOperatorToken(name),
          disabled: hasAudioSourceChip,
        }));
    }

    if (context.operator === "source:") {
      return PICKER_SOURCES.filter((source) => {
        if (typed.length === 0) {
          return true;
        }
        // The canonical value is the token's text after `source:` (e.g. `mic`).
        const value = source.token.slice("source:".length).toLowerCase();
        return value.includes(typed) || source.label.toLowerCase().includes(typed);
      }).map((source) => ({
        id: `source:${source.id}`,
        label: source.label,
        token: source.token,
        // The Screen row is never disabled; mic/system clash with an app chip.
        disabled: source.id !== "screen" && hasAppChip,
      }));
    }

    // date: / after: / before: — always the four presets, never filtered.
    return PICKER_DATE_PRESETS.map((preset) => ({
      id: `date:${preset.id}`,
      label: preset.label,
      token: preset.token,
      disabled: false,
    }));
  });

  // A single one-line note shown below the value list when the active operator
  // structurally conflicts with an already-active chip (mirrors the backend's
  // app_source_conflict). Null when there's no conflict.
  valueListConflictReason = $derived.by<string | null>(() =>
    this.activeOperatorContext?.operator === "source:" &&
    this.#store.activeFilterChips.some((c) => c.kind === "app")
      ? "Audio has no app — remove the app filter to search audio"
      : this.activeOperatorContext?.operator === "app:" &&
          this.#store.activeFilterChips.some(
            (c) => c.kind === "source" && c.data.source !== "screen",
          )
        ? "Audio has no app — remove the audio filter to scope by app"
        : null,
  );

  // The empty-state line for the `app:` operator only (source/date always have
  // rows). Distinguishes "still loading", "nothing captured", and "no match" so
  // the surface never renders a blank list.
  valueListEmptyMessage = $derived.by<string | null>(() => {
    if (this.activeOperatorContext?.operator !== "app:") {
      return null;
    }
    if (this.#store.searchableApps === null && this.#store.searchableAppsLoading) {
      return "Loading apps…";
    }
    if (this.pickerAppNames.length === 0) {
      return "No apps captured yet";
    }
    if (this.valueListRows.length === 0) {
      return "No matching app";
    }
    return null;
  });

  // The highlighted row index within the value list. Kept pointed at an ENABLED
  // row by the page-level $effect; -1 means no enabled row exists (so Enter is
  // a no-op).
  valueListIndex = $state(0);

  // Move the highlight among ENABLED rows only, wrapping at the ends. A no-op
  // when no row is selectable (e.g. every row conflicts with an active chip).
  moveValueListSelection(delta: number): void {
    const rows = this.valueListRows;
    const enabled = rows
      .map((row, index) => ({ row, index }))
      .filter((entry) => !entry.row.disabled);
    if (enabled.length === 0) {
      return;
    }
    const currentPos = enabled.findIndex(
      (entry) => entry.index === this.valueListIndex,
    );
    // From an unset/disabled position a forward move lands on the first enabled
    // row, a backward move on the last — same wrap idiom as moveSelection.
    const base = currentPos < 0 ? (delta > 0 ? -1 : 0) : currentPos;
    const nextPos = (base + delta + enabled.length) % enabled.length;
    this.valueListIndex = enabled[nextPos].index;
  }

  // Commit a value: REPLACE the trailing partial operator token in `query` with
  // the full operator token + a trailing space. The trailing space empties the
  // trailing token → valueListActive flips false → the reactive search effect
  // runs with the committed operator → the chip derives from the backend
  // response. Mirrors how acceptGhost finalizes the caret at end-of-input.
  async commitValueListRow(token: string): Promise<void> {
    const t = trailingToken(this.#store.query);
    const base = this.#store.query.slice(0, this.#store.query.length - t.length);
    this.#store.query = `${base}${token} `;
    await tick();
    const el = this.#store.inputEl;
    if (el !== null) {
      const end = el.value.length;
      el.setSelectionRange(end, end);
      this.#store.caretAtEnd = true;
    }
    this.#store.inputEl?.focus();
  }

  // Commit the highlighted enabled row (Enter). Real-values-only: if nothing is
  // highlighted or the highlighted row is disabled, this is a NO-OP — there's
  // no phantom commit of a half-typed value the parser wouldn't accept.
  commitHighlightedValueListRow(): void {
    if (this.valueListIndex < 0) {
      return;
    }
    const row = this.valueListRows[this.valueListIndex];
    if (row === undefined || row.disabled) {
      return;
    }
    void this.commitValueListRow(row.token);
  }

  // Escape out of an un-committed operator: strip the trailing partial operator
  // token from `query` (and any trailing whitespace it leaves), then refocus.
  // The search reruns on the residual, so the prior results return cleanly.
  abandonOperator(): void {
    const t = trailingToken(this.#store.query);
    this.#store.query = this.#store.query
      .slice(0, this.#store.query.length - t.length)
      .replace(/\s+$/, "");
    this.#store.inputEl?.focus();
  }

  // The active option id for aria-activedescendant on the search input while
  // the value list is up, mirroring the results/picker activeOptionId pattern.
  valueListActiveOptionId = $derived(
    this.valueListActive && this.valueListIndex >= 0
      ? `${VALUE_LIST_OPT_PREFIX}${this.valueListIndex}`
      : undefined,
  );
}
