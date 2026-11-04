// @ts-nocheck
import { describe, expect, test } from "bun:test";
import { nextFavorite, pushRecent, quickRecents, toggleFavorite } from "./model-favorites";

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
