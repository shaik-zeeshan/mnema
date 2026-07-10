// Tests for the /activate license landing page's inline script (activate.astro).
//
// The key arrives in the URL *hash* (never sent to the server); the script must
// wire the `mnema://license/activate` deep link + reveal the card when a key is
// present, and bounce to "/" when someone reaches /activate without one.
//
// Same pattern as latest-release.test.mjs: extract and run the REAL inline
// <script> inside a vm sandbox with a mocked DOM, so the shipped code is what's
// exercised, not a copy.
import { test } from "bun:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";

const activateAstro = readFileSync(new URL("../src/pages/activate.astro", import.meta.url), "utf8");
const scripts = [...activateAstro.matchAll(/<script\b[^>]*>([\s\S]*?)<\/script>/g)].map((m) => m[1]);
const inlineSrc = scripts.find((s) => s.includes("mnema://license/activate"));
assert.ok(inlineSrc, "activate inline script not found in activate.astro");

// Drive the real script against a location hash and report the resulting DOM.
function run(hash) {
  const el = () => ({ textContent: "…", href: "#", hidden: true, listeners: {}, classList: { add() {}, remove() {} }, addEventListener(name, fn) { this.listeners[name] = fn; } });
  const keyEl = el();
  const openEl = el();
  const copyEl = el();
  const cardEl = el();
  let replacedWith = null;

  const sandbox = {
    console, URLSearchParams, encodeURIComponent, setTimeout,
    location: {
      hash,
      replace: (url) => { replacedWith = url; },
    },
    document: {
      getElementById: (id) => ({ key: keyEl, open: openEl, copy: copyEl })[id] ?? null,
      querySelector: (sel) => (sel === ".card" ? cardEl : null),
    },
    navigator: { clipboard: { writeText: async () => {} } },
    window: { getSelection: () => null },
  };
  vm.createContext(sandbox);
  vm.runInContext(inlineSrc, sandbox);
  return { keyEl, openEl, cardEl, replacedWith };
}

test("hash key wires the deep link (url-encoded) and reveals the card", () => {
  const key = "eyJwYXlsb2FkIjoi+/=?&\".c2ln";
  const r = run("#key=" + encodeURIComponent(key));
  assert.equal(r.replacedWith, null, "must not bounce when a key is present");
  assert.equal(r.keyEl.textContent, key);
  assert.equal(r.openEl.href, "mnema://license/activate?key=" + encodeURIComponent(key));
  assert.equal(r.cardEl.hidden, false, "card must be revealed");
});

test("empty hash bounces to / and never shows the card", () => {
  const r = run("");
  assert.equal(r.replacedWith, "/");
  assert.equal(r.cardEl.hidden, true, "card must stay hidden");
});

test("hash without a key param also bounces", () => {
  const r = run("#foo=bar");
  assert.equal(r.replacedWith, "/");
  assert.equal(r.cardEl.hidden, true);
});
