import test from "node:test";
import assert from "node:assert/strict";

import { isMainAppRoute, normalizeAppPathname } from "../src/lib/route-path.ts";

test("treats the static SPA entry file as the root route", () => {
  assert.equal(normalizeAppPathname("/index.html"), "/");
  assert.equal(isMainAppRoute("/index.html"), true);
});

test("normalizes trailing slashes on nested routes", () => {
  assert.equal(normalizeAppPathname("/settings/"), "/settings");
  assert.equal(normalizeAppPathname("/debug/"), "/debug");
});

test("leaves normal routes unchanged", () => {
  assert.equal(normalizeAppPathname("/"), "/");
  assert.equal(normalizeAppPathname("/settings"), "/settings");
  assert.equal(normalizeAppPathname("/debug/tools"), "/debug/tools");
});
