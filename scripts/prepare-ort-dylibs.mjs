#!/usr/bin/env node
// Stage the ONNX Runtime DLLs for the Windows dynamic-ORT build (#137, Slice 1).
//
// ─────────────────────────────────────────────────────────────────────────────
// WHY this script exists (the ort-sys investigation)
// ─────────────────────────────────────────────────────────────────────────────
// On Windows the binary is built with `ort/load-dynamic` (see
// `crates/audio-transcription/Cargo.toml`). `load-dynamic` implies
// `ort-sys/disable-linking`, and the `ort-sys` 2.0.0-rc.12 build script
// (`build/main.rs`) early-returns the instant `disable-linking` is set:
//
//     if env::var("DOCS_RS").is_ok() || cfg!(feature = "disable-linking") { return; }
//
// i.e. under `load-dynamic` ort-sys does NOT download, extract, link, or copy any
// `onnxruntime.dll` — even though `download-binaries` is still (inertly) unioned
// on. The DLL is loaded at RUNTIME via `libloading`: `ort` reads `ORT_DYLIB_PATH`
// (or defaults to the bare name `onnxruntime.dll`, resolved first against
// `current_exe().parent()`), then refuses any runtime whose MINOR version is
// < 24 (it targets ONNX Runtime 1.24.x).
//
// So WE must supply the DLL. This script stages a version-locked, MIT-licensed
// `onnxruntime.dll` (>= 1.24.x, to match the `ort = =2.0.0-rc.12` pin) into
// `apps/desktop/src-tauri/resources/ort/`, which `tauri.windows.conf.json` then
// bundles flat next to the exe. At runtime `src/ort_dylib.rs` points
// `ORT_DYLIB_PATH` at that exe-adjacent copy (in both the main app and the
// re-invoked speaker-analysis helper subprocess).
//
// macOS/Linux: NO-OP. macOS keeps `download-binaries` (static ONNX Runtime linked
// into the binary; zero DLL) and macOS speakrs is native CoreML (no `ort`), so
// there is nothing to stage off-Windows.
//
// ─────────────────────────────────────────────────────────────────────────────
// DLL source precedence (first that resolves wins)
// ─────────────────────────────────────────────────────────────────────────────
//   1. $MNEMA_ORT_DYLIB_DIR  — a directory already containing onnxruntime.dll
//      (+ optional provider DLLs). Operator/CI override; zero network.
//   2. $MNEMA_ORT_REDIST_ZIP — a local Microsoft ONNX Runtime Windows-x64 redist
//      .zip (e.g. onnxruntime-win-x64-gpu-1.24.x.zip). Extracted with bsdtar
//      (`tar -xf`, present on Win10+); whichever of the three DLLs it contains
//      are copied. Use this for offline/air-gapped builds or a warmed CI cache.
//   3. AUTO-DOWNLOAD (zero-config default): fetch the pinned, sha256-verified
//      `onnxruntime-win-x64-gpu-<ver>.zip` from Microsoft's GitHub releases and
//      cache it under `target/ort-cache/` (gitignored). This is the `-gpu-` build,
//      so it ships ALL THREE DLLs — including the 263 MB CUDA execution provider —
//      which is exactly what `tauri.windows.conf.json` -> `bundle.resources` now
//      declares. The pin (URL + sha256) lives in a constant below, coupled to the
//      `ort = =2.0.0-rc.12` / ONNX Runtime 1.24.x requirement; bump them together.
//
// There is intentionally NO in-repo committed-DLL fallback. A committed CPU-only
// `onnxruntime.dll` was a rebuild footgun: with no env override it reverted the
// staged runtime to CPU while a stale GPU `onnxruntime_providers_cuda.dll` lingered
// in `resources/ort/` (version mismatch -> load crash), and it could not satisfy
// the now-3-DLL bundle. Committing the 263 MB CUDA provider instead is a non-starter
// (git bloat). Auto-download keeps the staged set internally consistent, GPU-capable,
// and reproducible without committing any binary.
//
// This stages all three DLLs the source contains: `onnxruntime.dll` (the runtime
// `ort` loads via ORT_DYLIB_PATH), `onnxruntime_providers_shared.dll`, and
// `onnxruntime_providers_cuda.dll` (the GPU execution provider, loaded from the
// same dir when CUDA is selected at runtime).
//
// Usage: node scripts/prepare-ort-dylibs.mjs [debug|release]

import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  copyFileSync,
  createReadStream,
  createWriteStream,
  readdirSync,
  renameSync,
  rmSync,
  statSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, basename } from "node:path";
import { Readable } from "node:stream";
import { pipeline } from "node:stream/promises";
import { fileURLToPath } from "node:url";

// The three ONNX Runtime DLLs that travel with the binary, version-locked to the
// `ort` pin. `onnxruntime.dll` is the runtime ort loads via ORT_DYLIB_PATH; the
// provider DLLs sit in the SAME dir so ONNX Runtime finds them (the CUDA provider
// is what enables GPU execution).
const ORT_DLL_NAMES = [
  "onnxruntime.dll",
  "onnxruntime_providers_shared.dll",
  "onnxruntime_providers_cuda.dll",
];
// The bundler only requires this one to exist (the providers are optional and
// only present in a GPU-capable source).
const REQUIRED_DLL = "onnxruntime.dll";

// ─────────────────────────────────────────────────────────────────────────────
// Pinned GPU ONNX Runtime redistributable (the auto-download default source).
// ─────────────────────────────────────────────────────────────────────────────
// Coupled to `ort = =2.0.0-rc.12` (which targets ONNX Runtime 1.24.x and rejects
// any loaded runtime with MINOR < 24). This is the `-gpu-` build: it carries the
// CUDA execution provider alongside the base runtime, so a default build is
// GPU-capable and the staged DLL set is internally version-consistent.
//
// To bump the pin, change VERSION + SHA256 + SIZE together (the URL is derived):
// download the new asset and recompute, e.g. `certutil -hashfile <zip> SHA256`
// on Windows or `sha256sum <zip>`.
const ORT_GPU_VERSION = "1.24.4";
const ORT_GPU_ZIP_NAME = `onnxruntime-win-x64-gpu-${ORT_GPU_VERSION}.zip`;
const ORT_GPU_ZIP_URL = `https://github.com/microsoft/onnxruntime/releases/download/v${ORT_GPU_VERSION}/${ORT_GPU_ZIP_NAME}`;
const ORT_GPU_ZIP_SHA256 =
  "ef3337a0b8184eb8beec310f7c83bd50376b3eefc43aab84ac8e452f6987df0a";
const ORT_GPU_ZIP_SIZE = 280958859; // bytes (~268 MB); cheap pre-hash sanity check.

function fail(message, code = 1) {
  console.error(`prepare-ort-dylibs: ${message}`);
  process.exit(code);
}

const args = process.argv.slice(2);
const profile = args.length > 0 ? args[0] : "debug";
if (profile !== "debug" && profile !== "release") {
  fail("usage: prepare-ort-dylibs [debug|release]", 2);
}

// macOS/Linux: nothing to stage (see header). Exit 0 so the `&&` chain in
// beforeBuildCommand/beforeDevCommand keeps going on every non-Windows build.
if (process.platform !== "win32") {
  console.log(
    "prepare-ort-dylibs: non-Windows host, skipping (ORT is statically linked off-Windows).",
  );
  process.exit(0);
}

const scriptDir = dirname(fileURLToPath(import.meta.url));
const repoRoot = dirname(scriptDir);
const srcTauriDir = join(repoRoot, "apps", "desktop", "src-tauri");
const stagingDir = join(srcTauriDir, "resources", "ort");

mkdirSync(stagingDir, { recursive: true });

// Recursively collect the target ORT DLLs found anywhere under `root`, keyed by
// basename (ONNX Runtime redist zips nest them under e.g. `*/lib/`).
function collectDlls(root) {
  const found = new Map();
  const walk = (dir) => {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const full = join(dir, entry.name);
      if (entry.isDirectory()) {
        walk(full);
      } else if (ORT_DLL_NAMES.includes(entry.name) && !found.has(entry.name)) {
        found.set(entry.name, full);
      }
    }
  };
  walk(root);
  return found;
}

// Copy only when missing or size-changed, so repeat builds are cheap and don't
// needlessly rewrite a 16 MB DLL.
function copyIfChanged(src, dest) {
  if (existsSync(dest) && statSync(dest).size === statSync(src).size) {
    return false;
  }
  copyFileSync(src, dest);
  return true;
}

// Streaming sha256 of a file (the GPU zip is ~268 MB — don't slurp it into RAM).
function sha256File(filePath) {
  return new Promise((resolve, reject) => {
    const hash = createHash("sha256");
    const stream = createReadStream(filePath);
    stream.on("error", reject);
    stream.on("data", (chunk) => hash.update(chunk));
    stream.on("end", () => resolve(hash.digest("hex")));
  });
}

// True iff `zipPath` exists and matches the pinned size AND sha256. The size
// check is a cheap guard so a half-written / wrong file skips the hash entirely.
async function verifiedZip(zipPath) {
  if (!existsSync(zipPath)) {
    return false;
  }
  const size = statSync(zipPath).size;
  if (size !== ORT_GPU_ZIP_SIZE) {
    console.log(
      `prepare-ort-dylibs: '${zipPath}' size ${size} != expected ${ORT_GPU_ZIP_SIZE}`,
    );
    return false;
  }
  const actual = await sha256File(zipPath);
  if (actual !== ORT_GPU_ZIP_SHA256) {
    console.log(
      `prepare-ort-dylibs: '${zipPath}' sha256 ${actual} != expected ${ORT_GPU_ZIP_SHA256}`,
    );
    return false;
  }
  return true;
}

// Stream a URL to disk via global fetch (Bun/Node 18+), following redirects
// (GitHub release assets 302 to a storage CDN).
async function downloadFile(url, destPath) {
  const response = await fetch(url, { redirect: "follow" });
  if (!response.ok) {
    throw new Error(`HTTP ${response.status} ${response.statusText}`);
  }
  if (!response.body) {
    throw new Error("empty response body");
  }
  await pipeline(Readable.fromWeb(response.body), createWriteStream(destPath));
}

// Zero-config default source: provision the pinned GPU ONNX Runtime ourselves.
// Reuses a verified cached zip when present; otherwise downloads + verifies once
// into `target/ort-cache/` (gitignored), then extracts the DLLs to a temp dir
// the caller cleans up. Fails loudly on download error or sha256 mismatch rather
// than ever staging an unverified runtime.
async function resolveViaAutoDownload() {
  const cacheDir = join(repoRoot, "target", "ort-cache");
  mkdirSync(cacheDir, { recursive: true });
  const cachedZip = join(cacheDir, ORT_GPU_ZIP_NAME);

  if (await verifiedZip(cachedZip)) {
    console.log(`prepare-ort-dylibs: using cached GPU ORT redist '${cachedZip}'`);
  } else {
    rmSync(cachedZip, { force: true });
    console.log(
      `prepare-ort-dylibs: downloading pinned GPU ORT redist (~268 MB, one-time;\n` +
        `  cached in target/ort-cache/): ${ORT_GPU_ZIP_URL}`,
    );
    const tmpZip = `${cachedZip}.${process.pid}.partial`;
    try {
      await downloadFile(ORT_GPU_ZIP_URL, tmpZip);
    } catch (error) {
      rmSync(tmpZip, { force: true });
      fail(
        `failed to download GPU ORT redist from ${ORT_GPU_ZIP_URL}: ${error.message}\n` +
          `(set MNEMA_ORT_REDIST_ZIP or MNEMA_ORT_DYLIB_DIR to build offline).`,
      );
    }
    if (!(await verifiedZip(tmpZip))) {
      rmSync(tmpZip, { force: true });
      fail(
        `downloaded GPU ORT redist failed verification (size/sha256 mismatch vs the\n` +
          `pin in this script). Refusing to stage an unverified runtime; if the pin was\n` +
          `just bumped, recompute ORT_GPU_ZIP_SHA256/SIZE for the new asset.`,
      );
    }
    renameSync(tmpZip, cachedZip);
    console.log(`prepare-ort-dylibs: downloaded + verified -> '${cachedZip}'`);
  }

  const extractDir = mkdtempSync(join(tmpdir(), "mnema-ort-gpu-"));
  // bsdtar (Windows 10+ `tar.exe`) reads .zip natively.
  const result = spawnSync("tar", ["-xf", cachedZip, "-C", extractDir], {
    stdio: "inherit",
  });
  if (result.status !== 0) {
    rmSync(extractDir, { recursive: true, force: true });
    fail(
      `failed to extract '${cachedZip}' with tar (need bsdtar / Win10+)`,
      result.status ?? 1,
    );
  }
  return { dlls: collectDlls(extractDir), cleanup: extractDir };
}

// Resolve the DLL set from the configured source (see precedence in the header).
async function resolveSourceDlls() {
  const dirOverride = process.env.MNEMA_ORT_DYLIB_DIR;
  if (dirOverride) {
    if (!existsSync(join(dirOverride, REQUIRED_DLL))) {
      fail(
        `MNEMA_ORT_DYLIB_DIR='${dirOverride}' does not contain ${REQUIRED_DLL}`,
      );
    }
    console.log(`prepare-ort-dylibs: sourcing DLLs from MNEMA_ORT_DYLIB_DIR='${dirOverride}'`);
    return { dlls: collectDlls(dirOverride), cleanup: null };
  }

  const zipOverride = process.env.MNEMA_ORT_REDIST_ZIP;
  if (zipOverride) {
    if (!existsSync(zipOverride)) {
      fail(`MNEMA_ORT_REDIST_ZIP='${zipOverride}' does not exist`);
    }
    const extractDir = mkdtempSync(join(tmpdir(), "mnema-ort-redist-"));
    // bsdtar (Windows 10+ `tar.exe`) reads .zip natively.
    const result = spawnSync("tar", ["-xf", zipOverride, "-C", extractDir], {
      stdio: "inherit",
    });
    if (result.status !== 0) {
      rmSync(extractDir, { recursive: true, force: true });
      fail(
        `failed to extract MNEMA_ORT_REDIST_ZIP='${zipOverride}' with tar (need bsdtar / Win10+)`,
        result.status ?? 1,
      );
    }
    console.log(`prepare-ort-dylibs: sourcing DLLs from MNEMA_ORT_REDIST_ZIP='${zipOverride}'`);
    return { dlls: collectDlls(extractDir), cleanup: extractDir };
  }

  // Zero-config default: auto-download the pinned, sha256-verified GPU redist.
  return resolveViaAutoDownload();
}

// Best-effort: also drop the DLLs next to the non-bundled dev/run exe so
// `tauri dev` (and `cargo run`) find them. In a packaged build Tauri places the
// bundled resource next to the exe itself, so this only matters off-bundle.
function copyNextToProfileExe(stagedFiles) {
  const candidates = [join(repoRoot, "target", profile)];
  const triple =
    process.env.CARGO_BUILD_TARGET ||
    process.env.TAURI_ENV_TARGET_TRIPLE ||
    process.env.TARGET;
  if (triple) {
    candidates.push(join(repoRoot, "target", triple, profile));
  }
  for (const dir of candidates) {
    try {
      mkdirSync(dir, { recursive: true });
      for (const src of stagedFiles) {
        copyIfChanged(src, join(dir, basename(src)));
      }
    } catch (error) {
      // Non-fatal: the bundled path does not depend on this.
      console.warn(
        `prepare-ort-dylibs: could not stage next to ${dir} (non-fatal): ${error.message}`,
      );
    }
  }
}

const { dlls, cleanup } = await resolveSourceDlls();
try {
  if (!dlls.has(REQUIRED_DLL)) {
    fail(`source did not contain ${REQUIRED_DLL}`);
  }
  const stagedFiles = [];
  for (const name of ORT_DLL_NAMES) {
    const src = dlls.get(name);
    if (!src) {
      continue; // provider DLLs optional (a CPU-only env-override source lacks them)
    }
    const dest = join(stagingDir, name);
    const changed = copyIfChanged(src, dest);
    stagedFiles.push(dest);
    console.log(
      `prepare-ort-dylibs: ${changed ? "staged" : "up-to-date"} ${name} -> ${dest}`,
    );
  }
  copyNextToProfileExe(stagedFiles);
} finally {
  if (cleanup) {
    rmSync(cleanup, { recursive: true, force: true });
  }
}

console.log("prepare-ort-dylibs: done.");
