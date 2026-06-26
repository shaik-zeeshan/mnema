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
//      .zip (e.g. onnxruntime-win-x64-1.24.x.zip, or the `-gpu-` variant for the
//      Slice 2 provider DLLs). Extracted with bsdtar (`tar -xf`, present on
//      Win10+); whichever of the three DLLs it contains are copied.
//   3. The in-repo `apps/desktop/src-tauri/resources/windows/` dir — its committed
//      `onnxruntime.dll` is MIT ONNX Runtime 1.24.4 (minor 24 >= 24, satisfies the
//      `ort` gate) plus `onnxruntime_providers_shared.dll`. Zero-config default so
//      a plain `bun run tauri build` works on this repo today.
//   4. None found -> hard error with instructions (better a build-time failure
//      than shipping a bundle with no `onnxruntime.dll`, which would crash on the
//      first transcription/diarization).
//
// Slice 1 stages CPU-only (`onnxruntime.dll`, and `onnxruntime_providers_shared.dll`
// if present). The CUDA provider DLL (`onnxruntime_providers_cuda.dll`) only
// exists once Slice 2 enables `ort/cuda`; this script copies it too whenever the
// chosen source contains it, so Slice 2 just needs a GPU-capable source.
//
// Usage: node scripts/prepare-ort-dylibs.mjs [debug|release]

import { spawnSync } from "node:child_process";
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  copyFileSync,
  readdirSync,
  rmSync,
  statSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, basename } from "node:path";
import { fileURLToPath } from "node:url";

// The three ONNX Runtime DLLs that travel with the binary, version-locked to the
// `ort` pin. `onnxruntime.dll` is the runtime ort loads via ORT_DYLIB_PATH; the
// provider DLLs sit in the SAME dir so ONNX Runtime finds them (CUDA in Slice 2).
const ORT_DLL_NAMES = [
  "onnxruntime.dll",
  "onnxruntime_providers_shared.dll",
  "onnxruntime_providers_cuda.dll",
];
// The bundler only requires this one to exist (the providers are optional and
// only present in a GPU-capable source / Slice 2).
const REQUIRED_DLL = "onnxruntime.dll";

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

// Resolve the DLL set from the configured source (see precedence in the header).
function resolveSourceDlls() {
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

  // Zero-config default: the in-repo committed MIT ONNX Runtime DLLs.
  const inRepo = join(srcTauriDir, "resources", "windows");
  if (existsSync(join(inRepo, REQUIRED_DLL))) {
    console.log(`prepare-ort-dylibs: sourcing DLLs from in-repo '${inRepo}'`);
    return { dlls: collectDlls(inRepo), cleanup: null };
  }

  fail(
    [
      `could not locate ${REQUIRED_DLL}. Provide a version-locked (ONNX Runtime >= 1.24)`,
      "MIT onnxruntime.dll via one of:",
      "  - set MNEMA_ORT_DYLIB_DIR to a dir containing the ORT DLLs, OR",
      "  - set MNEMA_ORT_REDIST_ZIP to a Microsoft ONNX Runtime win-x64 .zip",
      "    (download onnxruntime-win-x64-<ver>.zip — or the -gpu- variant for the",
      "     CUDA provider DLLs — from",
      "     https://github.com/microsoft/onnxruntime/releases for a 1.24.x tag), OR",
      `  - restore apps/desktop/src-tauri/resources/windows/${REQUIRED_DLL}.`,
    ].join("\n"),
  );
  return { dlls: new Map(), cleanup: null }; // unreachable; keeps the type obvious
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

const { dlls, cleanup } = resolveSourceDlls();
try {
  if (!dlls.has(REQUIRED_DLL)) {
    fail(`source did not contain ${REQUIRED_DLL}`);
  }
  const stagedFiles = [];
  for (const name of ORT_DLL_NAMES) {
    const src = dlls.get(name);
    if (!src) {
      continue; // provider DLLs are optional (absent until Slice 2's GPU source)
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
