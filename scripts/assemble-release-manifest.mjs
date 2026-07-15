#!/usr/bin/env node
// assemble-release-manifest.mjs
//
// Hand-assembles the Tauri v2 updater `latest.json` manifest for a Mnema
// release, emitting it to stdout (or to --out <path>).
//
// Why this exists: a malformed manifest silently breaks the auto-updater for
// every installed client. The traps this script guards against are:
//   - `signature` MUST be the full text content of the platform's `.sig` file
//     (a multi-line minisign blob), never an inline string.
//   - macOS `url` MUST point at the `.app.tar.gz` updater archive, not the .dmg.
//   - Windows `url` MUST point at the NSIS `-setup.exe`.
//   - `version` MUST NOT have a leading `v`.
//   - `pub_date` MUST be RFC3339 UTC (passed in, not generated here).
//   - Both platforms MUST be present or the script fails loudly.
//
// CLI flags:
//   --version <v>          release version WITHOUT leading v, e.g. 0.1.9
//   --pub-date <rfc3339>   e.g. 2026-07-01T12:00:00Z (must end in Z)
//   --macos-sig <file>     path to the .app.tar.gz.sig file
//   --windows-sig <file>   path to the NSIS -setup.exe.sig file
//   --notes <str>          optional; default "See the release notes on GitHub."
//   --out <path>           optional; write here instead of stdout
//
//   URL: either pass explicit download URLs...
//   --macos-url <url> --windows-url <url>
//   ...or let them be built from --repo + asset filenames:
//   --repo <owner/name> --macos-asset <file> --windows-asset <file>
//     (URL = https://github.com/<repo>/releases/download/v<version>/<asset>)
//
// Self-test:
//   node scripts/assemble-release-manifest.mjs --self-test
//
// Example:
//   node scripts/assemble-release-manifest.mjs \
//     --version 0.1.9 --pub-date 2026-07-01T12:00:00Z \
//     --repo shaik-zeeshan/mnema \
//     --macos-sig ./mnema_0.1.9_aarch64.app.tar.gz.sig \
//     --macos-asset mnema_0.1.9_aarch64.app.tar.gz \
//     --windows-sig ./mnema_0.1.9_x64-setup.exe.sig \
//     --windows-asset mnema_0.1.9_x64-setup.exe

import { readFileSync, writeFileSync } from "node:fs";
import assert from "node:assert";

const DEFAULT_NOTES = "See the release notes on GitHub.";

// --- pure builder ----------------------------------------------------------

/**
 * Build the Tauri v2 updater manifest object.
 * @param {object} o
 * @param {string} o.version   version WITHOUT leading v
 * @param {string} o.pubDate   RFC3339 UTC timestamp ending in Z
 * @param {string} o.notes
 * @param {{ "darwin-aarch64": {signature: string, url: string},
 *           "windows-x86_64": {signature: string, url: string} }} o.platforms
 */
export function buildManifest({ version, pubDate, notes, platforms }) {
  if (!version) throw new Error("version is required");
  if (/^v/i.test(version)) throw new Error(`version must not have a leading v: ${version}`);
  if (!pubDate) throw new Error("pub_date is required");
  if (!/Z$/.test(pubDate)) throw new Error(`pub_date must be RFC3339 UTC (end in Z): ${pubDate}`);

  const required = ["darwin-aarch64", "windows-x86_64"];
  const out = {};
  for (const key of required) {
    const p = platforms?.[key];
    if (!p) throw new Error(`missing platform entry: ${key}`);
    if (!p.signature) throw new Error(`missing/empty signature for ${key}`);
    if (!p.url) throw new Error(`missing url for ${key}`);
    out[key] = { signature: p.signature, url: p.url };
  }

  return {
    version,
    notes: notes || DEFAULT_NOTES,
    pub_date: pubDate,
    platforms: out,
  };
}

// --- IO helpers ------------------------------------------------------------

function readSig(file, label) {
  if (!file) throw new Error(`missing ${label} .sig file path`);
  let text;
  try {
    text = readFileSync(file, "utf8");
  } catch (e) {
    throw new Error(`could not read ${label} .sig file "${file}": ${e.message}`);
  }
  // Trim a single trailing newline only (minisign blobs are multi-line).
  const sig = text.replace(/\r?\n$/, "");
  if (!sig) throw new Error(`${label} .sig file "${file}" is empty`);
  return sig;
}

function parseArgs(argv) {
  const args = {};
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a.startsWith("--")) {
      const key = a.slice(2);
      const next = argv[i + 1];
      if (next === undefined || next.startsWith("--")) {
        args[key] = true;
      } else {
        args[key] = next;
        i++;
      }
    }
  }
  return args;
}

function ghUrl(repo, version, asset) {
  return `https://github.com/${repo}/releases/download/v${version}/${asset}`;
}

function resolveUrl({ explicit, repo, version, asset, label }) {
  if (explicit) return explicit;
  if (repo && asset) return ghUrl(repo, version, asset);
  throw new Error(`need either --${label}-url, or --repo + --${label}-asset`);
}

// --- self-test -------------------------------------------------------------

function selfTest() {
  const m = buildManifest({
    version: "0.1.9",
    pubDate: "2026-07-01T12:00:00Z",
    notes: undefined,
    platforms: {
      "darwin-aarch64": { signature: "SIG-MAC\nline2", url: "https://example/mac.app.tar.gz" },
      "windows-x86_64": { signature: "SIG-WIN\nline2", url: "https://example/win-setup.exe" },
    },
  });

  assert.deepStrictEqual(
    Object.keys(m).sort(),
    ["notes", "platforms", "pub_date", "version"],
    "top-level keys must be exactly {version, notes, pub_date, platforms}",
  );
  assert.deepStrictEqual(
    Object.keys(m.platforms).sort(),
    ["darwin-aarch64", "windows-x86_64"],
    "platforms must have exactly the two expected keys",
  );
  for (const key of ["darwin-aarch64", "windows-x86_64"]) {
    assert.deepStrictEqual(
      Object.keys(m.platforms[key]).sort(),
      ["signature", "url"],
      `${key} must have exactly {signature, url}`,
    );
  }
  assert.ok(!/^v/i.test(m.version), "version must not have a leading v");
  assert.strictEqual(m.notes, DEFAULT_NOTES, "notes should default when omitted");

  // leading-v version rejected
  assert.throws(
    () => buildManifest({ version: "v0.1.9", pubDate: "2026-07-01T12:00:00Z", platforms: m.platforms }),
    /leading v/,
  );
  // non-UTC pub_date rejected
  assert.throws(
    () => buildManifest({ version: "0.1.9", pubDate: "2026-07-01T12:00:00", platforms: m.platforms }),
    /RFC3339 UTC/,
  );
  // missing signature throws
  assert.throws(
    () =>
      buildManifest({
        version: "0.1.9",
        pubDate: "2026-07-01T12:00:00Z",
        platforms: {
          "darwin-aarch64": { signature: "", url: "https://x/mac" },
          "windows-x86_64": { signature: "S", url: "https://x/win" },
        },
      }),
    /missing\/empty signature/,
  );
  // missing platform throws
  assert.throws(
    () =>
      buildManifest({
        version: "0.1.9",
        pubDate: "2026-07-01T12:00:00Z",
        platforms: { "darwin-aarch64": { signature: "S", url: "https://x/mac" } },
      }),
    /missing platform entry/,
  );

  console.log("assemble-release-manifest self-test: OK");
}

// --- main ------------------------------------------------------------------

function main() {
  const argv = process.argv.slice(2);

  if (argv[0] === "--self-test") {
    selfTest();
    return;
  }

  const args = parseArgs(argv);
  const version = args.version;
  const pubDate = args["pub-date"];
  const repo = args.repo;

  if (typeof version !== "string") throw new Error("--version <v> is required");

  const platforms = {
    "darwin-aarch64": {
      signature: readSig(args["macos-sig"], "macOS"),
      url: resolveUrl({
        explicit: args["macos-url"],
        repo,
        version,
        asset: args["macos-asset"],
        label: "macos",
      }),
    },
    "windows-x86_64": {
      signature: readSig(args["windows-sig"], "Windows"),
      url: resolveUrl({
        explicit: args["windows-url"],
        repo,
        version,
        asset: args["windows-asset"],
        label: "windows",
      }),
    },
  };

  const manifest = buildManifest({
    version,
    pubDate,
    notes: typeof args.notes === "string" ? args.notes : undefined,
    platforms,
  });

  const json = JSON.stringify(manifest, null, 2) + "\n";
  if (typeof args.out === "string") {
    writeFileSync(args.out, json);
  } else {
    process.stdout.write(json);
  }
}

try {
  main();
} catch (e) {
  console.error(`error: ${e.message}`);
  process.exit(1);
}
