// Regression tests for the homepage "latest release" resolver in Base.astro.
//
// Bug: the resolver showed/linked a PRERELEASE (v0.1.0) as the latest version
// because it selected the first non-draft release without excluding
// prereleases — unlike GitHub's "Latest" badge. See git blame for context.
//
// These tests run the REAL inline <script> extracted from Base.astro inside a
// vm sandbox with mocked DOM/fetch/sessionStorage, so they exercise the
// shipped code rather than a copy. Runs under `bun test` (the repo's standard
// JS test runner); assertions use node:assert, which throws on failure.
import { test } from "bun:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";

const baseAstro = readFileSync(new URL("../src/layouts/Base.astro", import.meta.url), "utf8");
const scripts = [...baseAstro.matchAll(/<script\b[^>]*>([\s\S]*?)<\/script>/g)].map((m) => m[1]);
const loaderSrc = scripts.find((s) => s.includes("RELEASE_CACHE_KEY"));
assert.ok(loaderSrc, "release-loader script not found in Base.astro");

// Drive the real loader against a release list and report what the footer shows.
async function resolve({ list, seedCache = null }) {
  class HTMLAnchorElement {}
  const versionEl = Object.assign(Object.create(HTMLAnchorElement.prototype), {
    href: "FALLBACK",
    textContent: "latest", // static fallback in Footer.astro
  });
  const downloadEl = Object.assign(Object.create(HTMLAnchorElement.prototype), { href: "FALLBACK" });

  const store = new Map();
  if (seedCache)
    store.set(
      "mnema-latest-release-v3",
      JSON.stringify({ fetchedAt: Date.now(), release: seedCache }),
    );
  let fetchCalls = 0;

  const bySelector = {
    "[data-latest-download-link]": [downloadEl],
    "[data-latest-release-link]": [versionEl],
    "[data-latest-release-version]": [versionEl],
  };

  const sandbox = {
    console, Date, JSON, Array, setTimeout, HTMLAnchorElement,
    document: {
      readyState: "complete",
      querySelector: (sel) =>
        sel.includes("mnema-latest-release-api") ? { content: "https://api.test/releases" } : null,
      querySelectorAll: (sel) => bySelector[sel] || [],
      addEventListener() {},
    },
    sessionStorage: {
      getItem: (k) => (store.has(k) ? store.get(k) : null),
      setItem: (k, v) => store.set(k, v),
      removeItem: (k) => store.delete(k),
    },
    fetch: async () => {
      fetchCalls++;
      return { ok: true, json: async () => list };
    },
  };
  vm.createContext(sandbox);
  vm.runInContext(loaderSrc, sandbox);
  await new Promise((r) => setTimeout(r, 30));
  return { version: versionEl.textContent, downloadHref: downloadEl.href, fetchCalls };
}

const rel = (tag, prerelease, draft = false) => ({
  tag_name: tag,
  prerelease,
  draft,
  html_url: `https://gh/tag/${tag}`,
  assets: [{ name: `mnema_${tag.replace(/^v/, "")}_aarch64.dmg`, browser_download_url: `https://dl/${tag}.dmg` }],
});

test("selects the latest stable, skipping a more-recent prerelease (the bug)", async () => {
  const r = await resolve({ list: [rel("v0.2.0-beta.1", true), rel("v0.1.1", false)] });
  assert.equal(r.version, "v0.1.1");
});

test("live shape: v0.1.1 stable above v0.1.0 prerelease resolves to v0.1.1", async () => {
  const r = await resolve({ list: [rel("v0.1.1", false), rel("v0.1.0", true)] });
  assert.equal(r.version, "v0.1.1");
  assert.equal(r.downloadHref, "https://dl/v0.1.1.dmg");
});

test("only a prerelease exists: keeps the static fallback, never shows the prerelease", async () => {
  const r = await resolve({ list: [rel("v0.1.0", true)] });
  assert.equal(r.version, "latest"); // unchanged fallback, not "v0.1.0"
});

test("a non-prerelease stable release is still resolved normally", async () => {
  const r = await resolve({ list: [rel("v1.0.0", false)] });
  assert.equal(r.version, "v1.0.0");
  assert.equal(r.fetchCalls, 1);
});

test("revalidates even with a fresh cache, upgrading to a newer release (the stale-cache bug)", async () => {
  // A tab cached v0.1.2 while it was latest; v0.1.3 ships later. The loader
  // must still fetch and update rather than serving the cached version.
  const r = await resolve({
    seedCache: rel("v0.1.2", false),
    list: [rel("v0.1.3", false), rel("v0.1.2", false)],
  });
  assert.equal(r.fetchCalls, 1, "must revalidate against GitHub despite a cache hit");
  assert.equal(r.version, "v0.1.3");
  assert.equal(r.downloadHref, "https://dl/v0.1.3.dmg");
});
