#!/usr/bin/env pwsh
# Windows counterpart to scripts/dev-app.sh: launch the mnema desktop app in an
# isolated "dev profile" sandbox so a dev build and an installed production build
# can run side by side.
#
#   - Separate bundle identifier / product name (com.shaikzeeshan.mnema.dev),
#     from apps/desktop/src-tauri/tauri.dev.conf.json
#   - Separate data root:   %USERPROFILE%\.mnema-dev   (DB, recordings, OCR models)
#   - Separate config root: %APPDATA%\com.shaikzeeshan.mnema.dev
#       (matches dirs::config_dir() on Windows -> Roaming AppData, the non-macOS
#        default in crates/app-infra/src/brokered_access.rs)
#   - Separate deep-link scheme: mnema-dev://
#   - API keys are intentionally shared with prod via the OS credential store.
#
# Invoked via `bun run dev:sandbox` (scripts/dev-app.mjs dispatches here on
# Windows). Run it from a shell that already has the native build toolchain on
# PATH (MSVC vcvars + Strawberry Perl/NASM) -- see scripts/build-windows-local.ps1
# -- otherwise the first compile of the native crates (openssl-sys via SQLCipher,
# tesseract-rs) will fail. There is no gfortran/LIBRARY_PATH step here: on Windows
# openblas-src does not take the from-source gfortran path that dev-app.sh guards.

$ErrorActionPreference = 'Stop'

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Split-Path -Parent $scriptDir

$env:MNEMA_SAVE_DIRECTORY = Join-Path $HOME '.mnema-dev'
$env:MNEMA_APP_CONFIG_DIR = Join-Path $env:APPDATA 'com.shaikzeeshan.mnema.dev'

New-Item -ItemType Directory -Force -Path $env:MNEMA_SAVE_DIRECTORY | Out-Null
New-Item -ItemType Directory -Force -Path $env:MNEMA_APP_CONFIG_DIR | Out-Null

Write-Host 'mnema dev sandbox'
Write-Host "  save dir:   $($env:MNEMA_SAVE_DIRECTORY)"
Write-Host "  config dir: $($env:MNEMA_APP_CONFIG_DIR)"

if (-not $env:VSINSTALLDIR) {
    Write-Warning ('MSVC dev environment not detected (VSINSTALLDIR unset). If the ' +
        'native crates need recompiling this run will fail to link; launch from a ' +
        'shell prepared per scripts/build-windows-local.ps1.')
}

Push-Location $repoRoot
try {
    & bun run tauri -- dev -c src-tauri/tauri.dev.conf.json @args
}
finally {
    Pop-Location
}
exit $LASTEXITCODE
