#!/usr/bin/env node
// Cross-platform port of prepare-mnema-cli-sidecar.sh.
// Builds the `mnema-cli` binary and copies it into the Tauri sidecar
// directory (`apps/desktop/src-tauri/binaries`) named with the Rust target
// triple that Tauri's `externalBin` resolution expects.
//
// Usage: node scripts/prepare-mnema-cli-sidecar.mjs [debug|release] [--locked]

import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, copyFileSync, chmodSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

function fail(message, code = 1) {
  console.error(message);
  process.exit(code);
}

const args = process.argv.slice(2);
const profile = args.length > 0 ? args[0] : "debug";
if (profile !== "debug" && profile !== "release") {
  fail("usage: prepare-mnema-cli-sidecar [debug|release] [--locked]", 2);
}

let cargoLocked = false;
for (const arg of args.slice(1)) {
  if (arg === "--locked") {
    cargoLocked = true;
  } else {
    fail("usage: prepare-mnema-cli-sidecar [debug|release] [--locked]", 2);
  }
}

const scriptDir = dirname(fileURLToPath(import.meta.url));
const repoRoot = dirname(scriptDir);

function resolveTargetTriple() {
  const fromEnv =
    process.env.CARGO_BUILD_TARGET ||
    process.env.TAURI_ENV_TARGET_TRIPLE ||
    process.env.TARGET;
  if (fromEnv) {
    return fromEnv;
  }
  const result = spawnSync("rustc", ["-vV"], { encoding: "utf8" });
  if (result.status !== 0 || !result.stdout) {
    fail("failed to resolve Rust target triple");
  }
  const hostLine = result.stdout
    .split(/\r?\n/)
    .find((line) => line.startsWith("host:"));
  if (!hostLine) {
    fail("failed to resolve Rust target triple");
  }
  return hostLine.slice("host:".length).trim();
}

const targetTriple = resolveTargetTriple();
const exeSuffix = targetTriple.includes("windows") ? ".exe" : "";

const outputDir = join(repoRoot, "apps", "desktop", "src-tauri", "binaries");
const outputPath = join(outputDir, `mnema-cli-${targetTriple}${exeSuffix}`);

mkdirSync(outputDir, { recursive: true });

function sidecarOutputPath(rustTarget) {
  return join(outputDir, `mnema-cli-${rustTarget}${exeSuffix}`);
}

// The original shell script ran `chmod 755` on every copied/lipo output so the
// sidecar keeps its executable bit. The Unix permission bit is meaningless on
// Windows (NTFS does not model it), so only apply it off-Windows.
function ensureExecutable(targetPath) {
  if (process.platform !== "win32") {
    chmodSync(targetPath, 0o755);
  }
}

function buildTarget(rustTarget) {
  const cargoArgs = [
    "build",
    "--manifest-path",
    join(repoRoot, "Cargo.toml"),
    "-p",
    "cli",
    "--bin",
    "mnema-cli",
    "--target",
    rustTarget,
  ];
  if (cargoLocked) {
    cargoArgs.push("--locked");
  }
  if (profile === "release") {
    cargoArgs.push("--release");
  }

  const result = spawnSync("cargo", cargoArgs, { stdio: "inherit" });
  if (result.status !== 0) {
    fail(`cargo build failed for ${rustTarget}`, result.status ?? 1);
  }
}

if (targetTriple === "universal-apple-darwin") {
  if (process.platform !== "darwin") {
    fail("universal-apple-darwin sidecar builds require macOS");
  }
  if (spawnSync("lipo", ["-help"], { stdio: "ignore" }).error) {
    fail("universal-apple-darwin sidecar builds require lipo in PATH");
  }

  buildTarget("aarch64-apple-darwin");
  buildTarget("x86_64-apple-darwin");

  const armSourcePath = join(
    repoRoot,
    "target",
    "aarch64-apple-darwin",
    profile,
    "mnema-cli",
  );
  const intelSourcePath = join(
    repoRoot,
    "target",
    "x86_64-apple-darwin",
    profile,
    "mnema-cli",
  );

  const armOutputPath = sidecarOutputPath("aarch64-apple-darwin");
  const intelOutputPath = sidecarOutputPath("x86_64-apple-darwin");
  copyFileSync(armSourcePath, armOutputPath);
  copyFileSync(intelSourcePath, intelOutputPath);
  ensureExecutable(armOutputPath);
  ensureExecutable(intelOutputPath);

  const lipo = spawnSync(
    "lipo",
    ["-create", "-output", outputPath, armSourcePath, intelSourcePath],
    { stdio: "inherit" },
  );
  if (lipo.status !== 0) {
    fail("lipo failed to create universal binary", lipo.status ?? 1);
  }
  ensureExecutable(outputPath);
} else {
  buildTarget(targetTriple);

  const sourcePath = join(
    repoRoot,
    "target",
    targetTriple,
    profile,
    `mnema-cli${exeSuffix}`,
  );
  if (!existsSync(sourcePath)) {
    fail(`expected build output not found at ${sourcePath}`);
  }
  copyFileSync(sourcePath, outputPath);
  ensureExecutable(outputPath);
}

console.log(`prepared ${outputPath}`);
