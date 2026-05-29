#!/usr/bin/env bun
import { spawnSync, type SpawnSyncOptions } from "node:child_process";
import { chmodSync, copyFileSync, mkdirSync } from "node:fs";
import { platform } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

type Profile = "debug" | "release";

const SCRIPT_NAME = "prepare-mnema-cli-sidecar.ts";

function die(message: string, code = 1): never {
  process.stderr.write(`${message}\n`);
  process.exit(code);
}

function usage(): never {
  die(`usage: ${SCRIPT_NAME} [debug|release] [--locked]`, 2);
}

function spawnCapture(
  cmd: string,
  args: string[],
  opts: SpawnSyncOptions = {},
): { stdout: string; status: number | null } {
  const result = spawnSync(cmd, args, {
    stdio: ["ignore", "pipe", "inherit"],
    encoding: "utf8",
    ...opts,
  });
  if (result.error) die(`${cmd} failed to start: ${result.error.message}`);
  return { stdout: (result.stdout as string) ?? "", status: result.status };
}

function spawnInherit(cmd: string, args: string[], opts: SpawnSyncOptions = {}): number {
  const result = spawnSync(cmd, args, { stdio: "inherit", ...opts });
  if (result.error) die(`${cmd} failed to start: ${result.error.message}`);
  return result.status ?? 1;
}

const argv = process.argv.slice(2);
const profile = (argv[0] ?? "debug") as Profile;
if (profile !== "debug" && profile !== "release") usage();

let cargoLocked = false;
for (const arg of argv.slice(1)) {
  if (arg === "--locked") cargoLocked = true;
  else usage();
}

const scriptDir = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(scriptDir, "..");

let targetTriple =
  process.env.CARGO_BUILD_TARGET ??
  process.env.TAURI_ENV_TARGET_TRIPLE ??
  process.env.TARGET ??
  "";
if (!targetTriple) {
  const { stdout, status } = spawnCapture("rustc", ["-vV"]);
  if (status !== 0) die("rustc -vV failed");
  const match = stdout.match(/^host:\s*(.+)$/m);
  if (!match) die("failed to resolve Rust target triple from rustc -vV");
  targetTriple = match[1].trim();
}
if (!targetTriple) die("failed to resolve Rust target triple");

const exeSuffix = targetTriple.includes("windows") ? ".exe" : "";

const outputDir = join(repoRoot, "apps", "desktop", "src-tauri", "binaries");
const outputPath = join(outputDir, `mnema-cli-${targetTriple}${exeSuffix}`);

mkdirSync(outputDir, { recursive: true });

function sidecarOutputPath(rustTarget: string): string {
  return join(outputDir, `mnema-cli-${rustTarget}${exeSuffix}`);
}

function buildTarget(rustTarget: string): void {
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
  if (cargoLocked) cargoArgs.push("--locked");
  if (profile === "release") cargoArgs.push("--release");
  const status = spawnInherit("cargo", cargoArgs);
  if (status !== 0) process.exit(status);
}

function chmodIfPosix(path: string): void {
  if (platform() !== "win32") chmodSync(path, 0o755);
}

if (targetTriple === "universal-apple-darwin") {
  if (platform() !== "darwin") die("universal-apple-darwin sidecar builds require macOS");
  const lipoCheck = spawnSync("lipo", ["-version"], { stdio: "ignore" });
  if (lipoCheck.error || lipoCheck.status !== 0) {
    die("universal-apple-darwin sidecar builds require lipo in PATH");
  }

  buildTarget("aarch64-apple-darwin");
  buildTarget("x86_64-apple-darwin");

  const armSource = join(repoRoot, "target", "aarch64-apple-darwin", profile, "mnema-cli");
  const intelSource = join(repoRoot, "target", "x86_64-apple-darwin", profile, "mnema-cli");
  const armOutput = sidecarOutputPath("aarch64-apple-darwin");
  const intelOutput = sidecarOutputPath("x86_64-apple-darwin");

  copyFileSync(armSource, armOutput);
  copyFileSync(intelSource, intelOutput);

  const lipoStatus = spawnInherit("lipo", [
    "-create",
    "-output",
    outputPath,
    armSource,
    intelSource,
  ]);
  if (lipoStatus !== 0) process.exit(lipoStatus);

  chmodIfPosix(armOutput);
  chmodIfPosix(intelOutput);
} else {
  buildTarget(targetTriple);
  const sourcePath = join(
    repoRoot,
    "target",
    targetTriple,
    profile,
    `mnema-cli${exeSuffix}`,
  );
  copyFileSync(sourcePath, outputPath);
}

chmodIfPosix(outputPath);

process.stdout.write(`prepared ${outputPath}\n`);
