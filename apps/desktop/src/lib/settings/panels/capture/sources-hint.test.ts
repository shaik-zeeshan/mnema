// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { expect, test } from "bun:test";
import { sourcesHint } from "./sources-hint";

const idle = { running: false, loadingStart: false, loadingStop: false };

test("sources hint says what is happening while starting or stopping", () => {
  expect(sourcesHint({ ...idle, loadingStart: true })).toBe("Starting…");
  expect(sourcesHint({ ...idle, running: true, loadingStop: true })).toBe("Stopping…");
});

test("sources hint asks to stop only while recording", () => {
  expect(sourcesHint({ ...idle, running: true })).toBe("Stop recording to change sources.");
  expect(sourcesHint(idle)).toBe("Three independent families — any can run alone.");
});
