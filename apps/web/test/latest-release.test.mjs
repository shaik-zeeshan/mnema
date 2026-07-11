// Regression tests for the footer version-chip loader in Base.astro.
//
// Downloads are static links to the fixed R2 object (stable/Mnema.dmg); the
// only live behavior left is upgrading the footer chip's "latest" text from
// the stable update feed (release.mnema.day/stable/latest.json). The chip must
// keep its static fallback when the feed is unreachable or malformed.
//
// These tests run the REAL inline <script> extracted from Base.astro inside a
// vm sandbox with mocked DOM/fetch, so they exercise the shipped code rather
// than a copy. Runs under `bun test`; assertions use node:assert.
import { test } from "bun:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";

const baseAstro = readFileSync(new URL("../src/layouts/Base.astro", import.meta.url), "utf8");
const scripts = [...baseAstro.matchAll(/<script\b[^>]*>([\s\S]*?)<\/script>/g)].map((m) => m[1]);
const loaderSrc = scripts.find((s) => s.includes("mnema-latest-release-feed"));
assert.ok(loaderSrc, "version-chip loader script not found in Base.astro");

// Drive the real loader against a feed response and report what the chip shows.
async function resolve({ response }) {
  const versionEl = { textContent: "latest" }; // static fallback in Footer.astro

  const sandbox = {
    console, JSON, Array,
    setTimeout,
    document: {
      readyState: "complete",
      querySelector: (sel) =>
        sel.includes("mnema-latest-release-feed") ? { content: "https://feed.test/stable/latest.json" } : null,
      querySelectorAll: (sel) => (sel === "[data-latest-release-version]" ? [versionEl] : []),
      addEventListener() {},
    },
    fetch: async () => {
      if (response instanceof Error) throw response;
      return response;
    },
  };
  vm.createContext(sandbox);
  vm.runInContext(loaderSrc, sandbox);
  await new Promise((r) => setTimeout(r, 30));
  return { version: versionEl.textContent };
}

test("upgrades the chip from the stable feed's version", async () => {
  const r = await resolve({
    response: { ok: true, json: async () => ({ version: "0.1.12", platforms: {} }) },
  });
  assert.equal(r.version, "v0.1.12");
});

test("keeps the static fallback when the feed request fails (e.g. CORS, offline)", async () => {
  const r = await resolve({ response: new TypeError("Failed to fetch") });
  assert.equal(r.version, "latest");
});

test("keeps the static fallback on a non-200 response", async () => {
  const r = await resolve({ response: { ok: false, json: async () => ({}) } });
  assert.equal(r.version, "latest");
});

test("keeps the static fallback on a malformed feed", async () => {
  const r = await resolve({ response: { ok: true, json: async () => ({ version: 42 }) } });
  assert.equal(r.version, "latest");
});

test("keeps the static fallback on an empty-string version", async () => {
  // A string, but falsy — exercises the `!feed.version` arm of the guard;
  // without it the chip would render as a bare "v".
  const r = await resolve({ response: { ok: true, json: async () => ({ version: "" }) } });
  assert.equal(r.version, "latest");
});
