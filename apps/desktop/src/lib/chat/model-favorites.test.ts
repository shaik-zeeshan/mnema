// @ts-nocheck
import { describe, expect, test } from "bun:test";
import { askAiDefaultEngine, nextFavorite, pushRecent, quickRecents, toggleFavorite } from "./model-favorites";

const e = (provider, model) => ({ provider, model });

describe("model favorites", () => {
  test("toggleFavorite appends in star order and removes", () => {
    const a = e("anthropic", "opus"), b = e("ollama", "qwen");
    expect(toggleFavorite([a], b)).toEqual([a, b]);
    expect(toggleFavorite([a, b], e("anthropic", "opus"))).toEqual([b]);
  });

  test("pushRecent moves to front, de-dupes, caps at 5", () => {
    const list = ["1", "2", "3", "4", "5"].map((m) => e("p", m));
    expect(pushRecent(list, e("p", "3")).map((x) => x.model)).toEqual(["3", "1", "2", "4", "5"]);
    expect(pushRecent(list, e("p", "9")).map((x) => x.model)).toEqual(["9", "1", "2", "3", "4"]);
  });

  test("quickRecents skips starred models", () => {
    const recents = [e("p", "a"), e("p", "b"), e("p", "c")];
    expect(quickRecents(recents, [e("p", "a")]).map((x) => x.model)).toEqual(["b", "c"]);
  });

  test("nextFavorite wraps, and starts at the first when current isn't starred", () => {
    const favs = [e("p", "a"), e("p", "b")];
    expect(nextFavorite(favs, e("p", "a"))).toEqual(e("p", "b"));
    expect(nextFavorite(favs, e("p", "b"))).toEqual(e("p", "a"));
    expect(nextFavorite(favs, e("p", "z"))).toEqual(e("p", "a"));
    expect(nextFavorite(favs, null)).toEqual(e("p", "a"));
    expect(nextFavorite([], null)).toBeNull();
  });
});

describe("askAiDefaultEngine (mirrors ask_ai_resolver_inputs)", () => {
  const runtime = {
    providers: [{ id: "anthropic" }, { id: "ollama" }],
    defaultModel: e("anthropic", "claude-sonnet-4-5"),
  };
  test("Ask AI model with its provider runs on that provider", () => {
    expect(askAiDefaultEngine(runtime, { askAiModel: "qwen3:8b", askAiModelProvider: "ollama" })).toEqual(e("ollama", "qwen3:8b"));
  });
  test("its provider removed → the global default, never the id on another provider", () => {
    expect(askAiDefaultEngine(runtime, { askAiModel: "qwen3:8b", askAiModelProvider: "gone" })).toEqual(e("anthropic", "claude-sonnet-4-5"));
  });
  test("a bare model id rides on the default's provider", () => {
    expect(askAiDefaultEngine(runtime, { askAiModel: "claude-opus-5-5", askAiModelProvider: null })).toEqual(e("anthropic", "claude-opus-5-5"));
  });
  test("no Ask AI model → the global default; no default → null", () => {
    expect(askAiDefaultEngine(runtime, { askAiModel: "" })).toEqual(e("anthropic", "claude-sonnet-4-5"));
    expect(askAiDefaultEngine({ providers: [], defaultModel: null }, { askAiModel: "x" })).toBeNull();
  });
});
