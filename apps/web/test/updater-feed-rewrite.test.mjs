// Regression test for the updater-feed URL rewrite in the promote workflow
// (.github/workflows/macos-release-promote.yml). The rewrite is the
// load-bearing transform of the auto-updater contract: it must repoint every
// .platforms[].url at the immutable R2 copy while leaving version, notes,
// pub_date, and each platform's signature untouched — a regression here bricks
// auto-update for every installed client.
//
// Same pattern as latest-release.test.mjs: extract the REAL jq program from
// the workflow YAML and run it with real jq, so the test exercises the shipped
// expression rather than a copy. jq ships on macOS and GitHub runners.
import { test } from "bun:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const workflow = readFileSync(
  new URL("../../../.github/workflows/macos-release-promote.yml", import.meta.url),
  "utf8",
);
const jqProgram = workflow.match(/'(\.platforms \|= with_entries\([^']*\))'/)?.[1];
assert.ok(jqProgram, "feed-rewrite jq program not found in macos-release-promote.yml");

const feed = {
  version: "0.1.12",
  notes: "See the release notes.",
  pub_date: "2026-07-11T00:00:00Z",
  platforms: {
    "darwin-aarch64": {
      signature: "SIG-AARCH64",
      url: "https://github.com/shaik-zeeshan/mnema/releases/download/v0.1.12/mnema_0.1.12_aarch64.app.tar.gz",
    },
    // Second platform proves with_entries rewrites every entry, not just one.
    "darwin-x86_64": {
      signature: "SIG-X64",
      url: "https://github.com/shaik-zeeshan/mnema/releases/download/v0.1.12/mnema_0.1.12_x64.app.tar.gz",
    },
  },
};

test("rewrites every platform url to the R2 release path, preserving signature and version", () => {
  const proc = Bun.spawnSync(
    ["jq", "--arg", "base", "https://release.mnema.day/releases/v0.1.12", jqProgram],
    { stdin: Buffer.from(JSON.stringify(feed)) },
  );
  assert.equal(proc.exitCode, 0, proc.stderr.toString());
  const out = JSON.parse(proc.stdout.toString());

  assert.equal(
    out.platforms["darwin-aarch64"].url,
    "https://release.mnema.day/releases/v0.1.12/mnema_0.1.12_aarch64.app.tar.gz",
  );
  assert.equal(
    out.platforms["darwin-x86_64"].url,
    "https://release.mnema.day/releases/v0.1.12/mnema_0.1.12_x64.app.tar.gz",
  );
  assert.equal(out.platforms["darwin-aarch64"].signature, "SIG-AARCH64");
  assert.equal(out.platforms["darwin-x86_64"].signature, "SIG-X64");
  assert.equal(out.version, "0.1.12");
  assert.equal(out.notes, "See the release notes.");
  assert.equal(out.pub_date, "2026-07-11T00:00:00Z");
});
