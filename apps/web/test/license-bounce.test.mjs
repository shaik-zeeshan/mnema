// Pins the license bounce pages' deep-link producer strings against the Rust
// consumer (deep-link parsers in apps/desktop/src-tauri/src/lib.rs, which are
// test-pinned on their side). The strings are hand-mirrored across artifacts;
// silent drift here means a buyer completes checkout and the app never learns.
//
// Like latest-release.test.mjs, these run the REAL inline <script> extracted
// from each .astro page inside a vm sandbox with a mocked DOM/location, so
// they exercise the shipped code rather than a copy.
import { test } from "bun:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";

function inlineScript(page) {
  const source = readFileSync(new URL(`../src/pages/license/${page}`, import.meta.url), "utf8");
  const scripts = [...source.matchAll(/<script\b[^>]*>([\s\S]*?)<\/script>/g)].map((m) => m[1]);
  assert.equal(scripts.length, 1, `expected exactly one inline script in ${page}`);
  return scripts[0];
}

// Run a bounce script against a query string; report where the browser went.
function bounce(page, search) {
  const openEl = { href: "#" };
  const card = { hidden: true };
  const location = { search, href: null, replaced: null, replace: (url) => (location.replaced = url) };
  const sandbox = {
    URLSearchParams,
    encodeURIComponent,
    location,
    document: { getElementById: (id) => (id === "open" ? openEl : null), querySelector: (sel) => (sel === ".card" ? card : null) },
  };
  vm.createContext(sandbox);
  vm.runInContext(inlineScript(page), sandbox);
  return { openEl, card, location };
}

test("claim page fires mnema://license/claim with the encoded checkout id", () => {
  const { openEl, card, location } = bounce("claim.astro", "?checkout_id=co_abc%2F123 x");
  const expected = "mnema://license/claim?checkout_id=co_abc%2F123%20x";
  assert.equal(location.href, expected);
  assert.equal(openEl.href, expected);
  assert.equal(card.hidden, false);
  assert.equal(location.replaced, null);
});

test("renewed page fires the payload-free mnema://license/renewed link", () => {
  const { openEl, card, location } = bounce("renewed.astro", "?checkout_id=co_abc");
  assert.equal(location.href, "mnema://license/renewed");
  assert.equal(openEl.href, "mnema://license/renewed");
  assert.equal(card.hidden, false);
  assert.equal(location.replaced, null);
});

for (const page of ["claim.astro", "renewed.astro"]) {
  test(`${page} without a checkout_id bounces home and never fires the deep link`, () => {
    const { card, location } = bounce(page, "");
    assert.equal(location.replaced, "/");
    assert.equal(location.href, null);
    assert.equal(card.hidden, true);
  });
}
