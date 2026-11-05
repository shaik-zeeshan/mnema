// ModelPrefs (Chat + Settings pickers' starred/recent models): saves ONLY the
// changed list (so it never races Settings' providers/default autosave), shows
// the new list at once, settles on what the backend kept, and rolls back to
// the snapshot when the save fails. Runes precompiled by _reactivity/build.mjs.
import { beforeAll, expect, mock, test } from "bun:test";
import { spawnSync } from "child_process";
import { flushSync } from "svelte";
import { resolve } from "path";

const here = resolve(import.meta.dir, "_reactivity");
type Call = { cmd: string; args: any; resolve: (v: unknown) => void; reject: (e: unknown) => void };
const calls: Call[] = [];
mock.module("@tauri-apps/api/core", () => ({
  invoke: (cmd: string, args: unknown) =>
    new Promise((res, rej) => calls.push({ cmd, args, resolve: res, reject: rej })),
  convertFileSrc: (p: string) => p,
}));

let makeModelPrefsDriver: (initial: unknown) => any;
beforeAll(async () => {
  const built = spawnSync("node", [resolve(here, "build.mjs")], { encoding: "utf8" });
  if (built.status !== 0) throw new Error(`rune precompile failed: ${built.stderr || built.stdout}`);
  ({ makeModelPrefsDriver } = await import(resolve(here, "gen/model-prefs-driver.js")));
});

const e = (provider: string, model: string) => ({ provider, model });
const runtime = (favoriteModels: unknown[], recentModels: unknown[] = []) => ({ favoriteModels, recentModels });
const saved = (aiRuntime: unknown) => ({ settings: { aiRuntime } });
const tick = () => new Promise((r) => setTimeout(r, 0)).then(() => flushSync());

test("a star sends only favoriteModels and shows at once; a later save wins over an earlier late reply", async () => {
  calls.length = 0;
  const d = makeModelPrefsDriver(runtime([e("a", "1")]));
  d.prefs.toggleFavorite(e("b", "2"));
  d.prefs.toggleFavorite(e("c", "3"));
  flushSync();
  expect(d.prefs.favorites).toEqual([e("a", "1"), e("b", "2"), e("c", "3")]);
  expect(calls.map((c) => [c.cmd, Object.keys(c.args.request)])).toEqual([
    ["update_ai_runtime_settings", ["favoriteModels"]],
    ["update_ai_runtime_settings", ["favoriteModels"]],
  ]);
  // Second reply first, then the stale first reply: the list must not regress.
  calls[1].resolve(saved(runtime([e("a", "1"), e("b", "2"), e("c", "3")])));
  await tick();
  calls[0].resolve(saved(runtime([e("a", "1"), e("b", "2")])));
  await tick();
  expect(d.prefs.favorites).toEqual([e("a", "1"), e("b", "2"), e("c", "3")]);
  d.stop();
});

test("a failed save falls back to the snapshot; a recent already first is not re-saved", async () => {
  calls.length = 0;
  const d = makeModelPrefsDriver(runtime([e("a", "1")], [e("r", "1")]));
  d.prefs.toggleFavorite(e("a", "1"));
  flushSync();
  expect(d.prefs.favorites).toEqual([]);
  calls[0].reject(new Error("offline"));
  await tick();
  expect(d.prefs.favorites).toEqual([e("a", "1")]);
  d.prefs.pushRecent(e("r", "1"));
  expect(calls).toHaveLength(1);
  d.prefs.pushRecent(e("s", "2"));
  expect(calls[1].args.request).toEqual({ recentModels: [e("s", "2"), e("r", "1")] });
  d.stop();
});

test("a settled star gives way to a newer snapshot, even one that skipped past it", async () => {
  calls.length = 0;
  const [a, b, c] = [e("p", "a"), e("p", "b"), e("p", "c")];
  const d = makeModelPrefsDriver(runtime([]));
  d.prefs.toggleFavorite(a);
  // Another window's star lands before this window re-reads: the snapshot jumps
  // straight to [a, b] and never equals this save's [a].
  d.setSnapshot(runtime([a, b]));
  flushSync();
  calls[0].resolve(saved(runtime([a])));
  await tick();
  expect(d.prefs.favorites).toEqual([a, b]);
  d.prefs.toggleFavorite(c);
  expect(calls[1].args.request.favoriteModels).toEqual([a, b, c]);
  d.stop();
});
