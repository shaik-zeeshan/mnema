#!/usr/bin/env bun
// Platform-neutral entry point for the mnema dev sandbox (`bun run dev:sandbox`).
//
// This shim only dispatches to the shell-specific launcher for the host OS:
//   - macOS / Linux -> scripts/dev-app.sh   (bash)
//   - Windows       -> scripts/dev-app.ps1  (PowerShell)
//
// Both launchers do the same job: point MNEMA_SAVE_DIRECTORY / MNEMA_APP_CONFIG_DIR
// at an isolated dev profile, then run `tauri dev` with tauri.dev.conf.json. The
// build-environment setup they need is irreducibly platform-specific (gfortran /
// LIBRARY_PATH on macOS; MSVC + Strawberry Perl on Windows), which is why each
// keeps its own script instead of sharing one.

import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const scriptsDir = dirname(fileURLToPath(import.meta.url));
const forwarded = process.argv.slice(2);

function launch(command, args) {
  const result = spawnSync(command, args, { stdio: "inherit" });
  if (result.error) {
    console.error(
      `dev:sandbox: failed to launch ${command}: ${result.error.message}`,
    );
    process.exit(127);
  }
  process.exit(result.status ?? 1);
}

function isAvailable(command) {
  // A missing executable surfaces as spawnSync().error (ENOENT).
  return !spawnSync(command, ["-NoProfile", "-Command", "exit 0"], {
    stdio: "ignore",
  }).error;
}

if (process.platform === "win32") {
  // Prefer PowerShell 7 (pwsh); fall back to Windows PowerShell 5.1, always
  // present on Win10/11. dev-app.ps1 is written to run under both.
  const shell = isAvailable("pwsh") ? "pwsh" : "powershell";
  const ps1 = join(scriptsDir, "dev-app.ps1");
  launch(shell, [
    "-NoProfile",
    "-ExecutionPolicy",
    "Bypass",
    "-File",
    ps1,
    ...forwarded,
  ]);
} else {
  launch("bash", [join(scriptsDir, "dev-app.sh"), ...forwarded]);
}
