// @ts-nocheck
import { describe, expect, test } from "bun:test";
import { buildPickerRows } from "./model-picker-rows";

const e = (provider, model) => ({ provider, model });
const base = (over = {}) => ({
  query: "",
  mode: "quick",
  groupOpen: {},
  providerOrder: ["anthropic", "openrouter"],
  engines: [e("anthropic", "claude-opus-5-5"), e("anthropic", "claude-haiku-5-5"), ...Array.from({ length: 40 }, (_, i) => e("openrouter", `vendor/m${i}`))],
  contextWindow: () => null,
  providerLabel: (p) => (p === "anthropic" ? "Anthropic" : "OpenRouter"),
  letter: (p) => p[0].toUpperCase(),
  connected: ["anthropic", "openrouter"],
  sentinel: { label: "Default", detail: "claude-opus-5-5", title: "Default", selected: true, glyph: "A" },
  selectedProvider: null,
  selectedModel: null,
  favorites: [e("openrouter", "vendor/m3"), e("anthropic", "claude-haiku-5-5")],
  recents: [e("anthropic", "claude-haiku-5-5"), e("anthropic", "claude-opus-5-5")],
  canStar: true,
  makeDefaultFor: { current: e("anthropic", "claude-opus-5-5") },
  loading: false,
  failures: [],
  allowExactId: true,
  exactIdPerProvider: true,
  ...over,
});
const options = (rows) => rows.filter((r) => r.type === "option");

describe("buildPickerRows", () => {
  test("quick view: default, numbered favorites, unstarred recents, browse row — no long lists", () => {
    const rows = buildPickerRows(base());
    const opts = options(rows);
    expect(opts.map((o) => (o.option.kind === "sentinel" ? "default" : o.option.model))).toEqual([
      "default",
      "vendor/m3",
      "claude-haiku-5-5",
      "claude-opus-5-5", // recent; haiku is starred so it isn't repeated
    ]);
    expect(opts[1].num).toBe(1);
    expect(opts[2].num).toBe(2);
    expect(opts[1].starred).toBe(true);
    // "Make default" never on the current default.
    expect(opts[3].canMakeDefault).toBe(false);
    expect(opts[2].canMakeDefault).toBe(true);
    expect(rows.at(-1)).toMatchObject({ type: "action", action: "browse", label: "Browse all 42 models" });
    // Every keyboard row has a distinct, sequential index.
    const idx = rows.filter((r) => "index" in r).map((r) => r.index);
    expect(idx).toEqual(idx.map((_, i) => i));
  });

  test("browse view: folded groups with counts; the default's provider starts open", () => {
    const rows = buildPickerRows(base({ mode: "browse" }));
    const groups = rows.filter((r) => r.type === "group");
    expect(groups.map((g) => [g.provider, g.count, g.open])).toEqual([
      ["anthropic", 2, true],
      ["openrouter", 40, false],
    ]);
    expect(options(rows)).toHaveLength(2);
    expect(rows[0]).toMatchObject({ type: "action", action: "back" });
    const opened = buildPickerRows(base({ mode: "browse", groupOpen: { openrouter: true, anthropic: false } }));
    expect(options(opened)).toHaveLength(40);
  });

  test("nothing starred or recent: browse leads with Default instead of a back row", () => {
    const rows = buildPickerRows(base({ mode: "browse", favorites: [], recents: [] }));
    expect(rows[0]).toMatchObject({ type: "option", option: { kind: "sentinel" } });
  });

  test("search spans every provider and marks the match; unknown id offers per-provider rows", () => {
    const hits = options(buildPickerRows(base({ query: "m3" }))).filter((o) => o.option.kind === "pool");
    expect(hits.map((h) => h.option.model)).toEqual(["vendor/m3", "vendor/m30", "vendor/m31", "vendor/m32", "vendor/m33", "vendor/m34", "vendor/m35", "vendor/m36", "vendor/m37", "vendor/m38", "vendor/m39"]);
    // Rows show the friendly name (vendor prefix dropped), match marked.
    expect(hits[1].main).toEqual({ before: "", match: "m3", after: "0" });
    const typed = buildPickerRows(base({ query: "my-model" }));
    expect(typed.some((r) => r.type === "note")).toBe(true);
    expect(options(typed).map((o) => [o.option.kind, o.option.provider])).toEqual([
      ["exact", "anthropic"],
      ["exact", "openrouter"],
    ]);
  });

  test("a provider that failed to list starts open with its failure, no count", () => {
    const rows = buildPickerRows(
      base({ mode: "browse", engines: [e("anthropic", "claude-opus-5-5")], failures: [{ provider: "openrouter", label: "OpenRouter", reason: "offline" }] }),
    );
    expect(rows.find((r) => r.type === "group" && r.provider === "openrouter")).toMatchObject({ open: true, count: null });
    expect(rows.some((r) => r.type === "failure")).toBe(true);
  });
});
