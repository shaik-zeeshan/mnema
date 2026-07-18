// Pins the license bounce page's deep-link producer strings against the Rust
// consumer (deep-link parsers in apps/desktop/src-tauri/src/lib.rs, which are
// test-pinned on their side). The strings are hand-mirrored across artifacts;
// silent drift here means a buyer completes checkout and the app never learns.
//
// Like latest-release.test.mjs, these run the REAL inline <script> extracted
// from the .astro page inside a vm sandbox with a mocked DOM/location, so
// they exercise the shipped code rather than a copy.
import { test } from "bun:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";

function inlineScript() {
  const source = readFileSync(new URL("../src/pages/license/open.astro", import.meta.url), "utf8");
  const scripts = [...source.matchAll(/<script\b[^>]*>([\s\S]*?)<\/script>/g)].map((m) => m[1]);
  assert.equal(scripts.length, 1, "expected exactly one inline script in open.astro");
  return scripts[0];
}

// Run the bounce script against a query string; report where the browser went
// and which per-flow copy variants were unhidden.
function bounce(search) {
  const openEl = { href: "#" };
  const card = { hidden: true };
  const variants = {};
  const location = { search, href: null, replaced: null, replace: (url) => (location.replaced = url) };
  const sandbox = {
    URLSearchParams,
    encodeURIComponent,
    location,
    document: {
      getElementById: (id) => (id === "open" ? openEl : null),
      querySelector: (sel) => (sel === ".card" ? card : (variants[sel] ??= { hidden: true })),
    },
  };
  vm.createContext(sandbox);
  vm.runInContext(inlineScript(), sandbox);
  return { openEl, card, location, variants };
}

test("default flow fires mnema://license/claim with the encoded checkout id", () => {
  const { openEl, card, location, variants } = bounce("?checkout_id=co_abc%2F123 x");
  const expected = "mnema://license/claim?checkout_id=co_abc%2F123%20x";
  assert.equal(location.href, expected);
  assert.equal(openEl.href, expected);
  assert.equal(card.hidden, false);
  assert.equal(variants['.copy[data-flow="claim"]'].hidden, false);
  assert.equal(variants['.hint[data-flow="claim"]'].hidden, false);
  assert.equal(location.replaced, null);
});

// licensegate substitutes its email template name into {EVENT}; an unknown or
// explicit license_delivery flow must degrade to the claim path.
for (const search of ["?flow=license_delivery&checkout_id=co_abc", "?flow=surprise&checkout_id=co_abc"]) {
  test(`${search} falls back to the claim deep link`, () => {
    const { location, variants } = bounce(search);
    assert.equal(location.href, "mnema://license/claim?checkout_id=co_abc");
    assert.equal(variants['.copy[data-flow="claim"]'].hidden, false);
  });
}

test("flow=renewal fires the payload-free mnema://license/renewed link", () => {
  const { openEl, card, location, variants } = bounce("?flow=renewal&checkout_id=co_abc");
  assert.equal(location.href, "mnema://license/renewed");
  assert.equal(openEl.href, "mnema://license/renewed");
  assert.equal(card.hidden, false);
  assert.equal(variants['.copy[data-flow="renewal"]'].hidden, false);
  assert.equal(variants['.hint[data-flow="renewal"]'].hidden, false);
  assert.equal(location.replaced, null);
});

for (const search of ["", "?flow=renewal"]) {
  test(`"${search}" without a checkout_id bounces home and never fires the deep link`, () => {
    const { card, location } = bounce(search);
    assert.equal(location.replaced, "/");
    assert.equal(location.href, null);
    assert.equal(card.hidden, true);
  });
}
