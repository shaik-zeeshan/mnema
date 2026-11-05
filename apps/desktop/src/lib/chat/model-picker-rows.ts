// The model picker's row list, as pure data (rendered by ModelPickerMenu).
// Three views over one model pool:
//   quick  — "Default", the starred models (numbered 1-9) and the last couple
//            picked, then a "Browse all N models" row. What the picker opens on.
//   browse — every provider as a folding group with its model count; only the
//            provider of the current model (and any that failed to list) start open.
//   search — any typed query: matches from every provider, grouped, the matching
//            text marked, plus "use as a model id" rows for an unlisted id.
// Every keyboard-reachable row carries a flat `index` for arrow-key navigation.
import { modelDisplayName } from "$lib/insights/conversation";
import type { AiEngineRef } from "$lib/types/recording";
import { isFavorite, quickRecents, sameEngine } from "./model-favorites";

export type PickerMode = "quick" | "browse";

export type PickerOption =
  | { kind: "sentinel" }
  | { kind: "pool"; provider: string; model: string }
  | { kind: "exact"; provider: string; model: string };

/** `main` split around the search match so the view can mark it. */
export type Marked = { before: string; match: string; after: string };

export type PickerRow =
  | { type: "header"; key: string; provider: string | null; label: string; icon?: "star" | "clock"; hint?: string }
  | { type: "group"; key: string; provider: string; label: string; count: number | null; open: boolean; index: number }
  | { type: "action"; key: string; action: "browse" | "back"; label: string; detail: string | null; index: number }
  | { type: "loading"; key: string; label: string }
  | { type: "failure"; key: string; text: string }
  | { type: "note"; key: string; text: string }
  | {
      type: "option";
      key: string;
      option: PickerOption;
      glyph: string;
      main: Marked;
      sub: string | null;
      title: string;
      selected: boolean;
      /** Starred state, or null when the row can't be starred (Default, typed id). */
      starred: boolean | null;
      /** 1-9 shortcut shown on a starred row in the quick view. */
      num: number | null;
      /** Offer "Make default" (only where the parent allows it, never on the current default). */
      canMakeDefault: boolean;
      index: number;
    };

export interface PickerRowsInput {
  query: string;
  mode: PickerMode;
  /** Explicit open/closed groups (browse view); unset ones use the default rule. */
  groupOpen: Record<string, boolean>;
  /** Provider ids in display order. */
  providerOrder: string[];
  /** Pool engines (provider-tagged model ids, de-duplicated). */
  engines: AiEngineRef[];
  contextWindow: (provider: string, model: string) => number | null;
  providerLabel: (provider: string) => string;
  letter: (provider: string) => string;
  /** Connected provider ids (for "use as a model id" rows). */
  connected: string[];
  sentinel: { label: string; detail: string | null; title: string; selected: boolean; glyph: string } | null;
  selectedProvider: string | null;
  selectedModel: string | null;
  favorites: AiEngineRef[];
  recents: AiEngineRef[];
  /** Whether rows can be starred at all (the parent wired `onfavorite`). */
  canStar: boolean;
  /** The current global default, when the parent wired `onmakedefault`; else null. */
  makeDefaultFor: { current: AiEngineRef | null } | null;
  loading: boolean;
  failures: { provider: string; label: string; reason: string }[];
  allowExactId: boolean;
  exactIdPerProvider: boolean;
}

/** Whether the quick view has anything beyond "Default" (else the picker opens on browse). */
export function hasQuickView(favorites: AiEngineRef[], recents: AiEngineRef[]): boolean {
  return favorites.length > 0 || recents.length > 0;
}

function mark(text: string, q: string): Marked {
  const at = q ? text.toLowerCase().indexOf(q) : -1;
  return at < 0
    ? { before: text, match: "", after: "" }
    : { before: text.slice(0, at), match: text.slice(at, at + q.length), after: text.slice(at + q.length) };
}

export function buildPickerRows(input: PickerRowsInput): PickerRow[] {
  const rows: PickerRow[] = [];
  let index = 0;
  const raw = input.query.trim();
  const q = raw.toLowerCase();
  const isSelected = (provider: string, model: string) =>
    model === input.selectedModel && (input.selectedProvider === null || provider === input.selectedProvider);

  const addEngine = (provider: string, model: string, opts: { num?: number | null; markQuery?: boolean; sub?: string | null } = {}) => {
    const ctx = input.contextWindow(provider, model);
    const engine = { provider, model };
    rows.push({
      type: "option",
      key: `opt-${index}`,
      option: { kind: "pool", provider, model },
      glyph: input.letter(provider),
      main: mark(modelDisplayName(model), opts.markQuery ? q : ""),
      sub: opts.sub !== undefined ? opts.sub : ctx ? `${Math.round(ctx / 1000)}k` : null,
      title: `${input.providerLabel(provider)} · ${model}`,
      selected: isSelected(provider, model),
      starred: input.canStar ? isFavorite(input.favorites, engine) : null,
      num: opts.num ?? null,
      canMakeDefault: input.makeDefaultFor !== null && !sameEngine(input.makeDefaultFor.current, engine),
      index: index++,
    });
  };
  const addSentinel = () => {
    const s = input.sentinel;
    if (!s) return;
    rows.push({
      type: "option",
      key: `opt-${index}`,
      option: { kind: "sentinel" },
      glyph: s.glyph,
      main: mark(s.label, ""),
      sub: null,
      title: s.title,
      selected: s.selected,
      starred: null,
      num: null,
      canMakeDefault: false,
      index: index++,
    });
  };
  const modelsOf = (provider: string) => input.engines.filter((e) => e.provider === provider).map((e) => e.model);

  // ── search ────────────────────────────────────────────────────────────────
  if (q.length > 0) {
    if (input.sentinel && input.sentinel.label.toLowerCase().includes(q)) addSentinel();
    for (const provider of input.providerOrder) {
      const label = input.providerLabel(provider);
      const matches = modelsOf(provider).filter(
        (m) => m.toLowerCase().includes(q) || modelDisplayName(m).toLowerCase().includes(q) || label.toLowerCase().includes(q),
      );
      if (matches.length === 0) continue;
      rows.push({ type: "header", key: `hdr-${provider}`, provider, label });
      for (const model of matches) addEngine(provider, model, { markQuery: true });
    }
    const listed = input.engines.some((e) => e.model === raw);
    if (!rows.some((r) => r.type === "option")) {
      rows.push({ type: "note", key: "no-match", text: `No models match “${raw}”.` });
    }
    if (input.allowExactId && !listed) {
      const targets = input.exactIdPerProvider ? input.connected : input.connected.slice(0, 1);
      if (targets.length > 0) rows.push({ type: "header", key: "hdr-exact", provider: null, label: "Use as a model id" });
      for (const provider of targets) {
        rows.push({
          type: "option",
          key: `opt-${index}`,
          option: { kind: "exact", provider, model: raw },
          glyph: input.letter(provider),
          main: mark(raw, ""),
          sub: input.exactIdPerProvider ? input.providerLabel(provider) : null,
          title: `${input.providerLabel(provider)} · ${raw}`,
          selected: false,
          starred: null,
          num: null,
          canMakeDefault: false,
          index: index++,
        });
      }
    }
    return rows;
  }

  const quick = hasQuickView(input.favorites, input.recents);

  // ── quick ─────────────────────────────────────────────────────────────────
  if (input.mode === "quick" && quick) {
    addSentinel();
    if (input.favorites.length > 0) {
      rows.push({ type: "header", key: "hdr-fav", provider: null, label: "Favorites", icon: "star", hint: input.favorites.length > 1 ? "press 1–9" : "press 1" });
      input.favorites.forEach((f, i) => addEngine(f.provider, f.model, { num: i < 9 ? i + 1 : null }));
    }
    const recents = quickRecents(input.recents, input.favorites);
    if (recents.length > 0) {
      rows.push({ type: "header", key: "hdr-recent", provider: null, label: "Recent", icon: "clock" });
      for (const r of recents) addEngine(r.provider, r.model);
    }
    const total = input.engines.length;
    rows.push({
      type: "action",
      key: "act-browse",
      action: "browse",
      label: total > 0 ? `Browse all ${total} models` : "Browse all models",
      detail: input.providerOrder.length > 1 ? `${input.providerOrder.length} providers` : null,
      index: index++,
    });
    return rows;
  }

  // ── browse ────────────────────────────────────────────────────────────────
  if (quick) rows.push({ type: "action", key: "act-back", action: "back", label: "Favorites", detail: null, index: index++ });
  else addSentinel();
  const openByDefault = input.selectedProvider ?? input.providerOrder[0] ?? null;
  for (const provider of input.providerOrder) {
    const label = input.providerLabel(provider);
    const failure = input.failures.find((f) => f.provider === provider);
    const models = modelsOf(provider);
    const waiting = input.loading && models.length === 0 && !failure;
    const open = input.groupOpen[provider] ?? (provider === openByDefault || !!failure);
    rows.push({ type: "group", key: `grp-${provider}`, provider, label, count: waiting || failure ? null : models.length, open, index: index++ });
    if (!open) continue;
    if (failure) rows.push({ type: "failure", key: `fail-${provider}`, text: `${failure.label} — ${failure.reason}` });
    else if (waiting) rows.push({ type: "loading", key: `load-${provider}`, label });
    else for (const model of models) addEngine(provider, model);
  }
  if (!input.loading && input.engines.length === 0 && input.failures.length === 0) {
    rows.push({ type: "note", key: "empty", text: "No models yet." });
  }
  return rows;
}
