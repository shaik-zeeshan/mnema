#!/usr/bin/env pwsh
# Windows counterpart to scripts/build-macos-local-sign.sh.
#
# Sets up the native build environment the desktop app needs on Windows, then
# runs `tauri build` and surfaces the produced installer. Two pieces of setup
# are mandatory on this platform:
#
#   1. MSVC developer environment (vcvars64). Provides cl/link/nmake AND sets
#      VSINSTALLDIR, without which tesseract-rs's build.rs picks the multi-config
#      Visual Studio generator and emits debug-suffixed leptonica/tesseract libs
#      it then can't find ("could not find native static library leptonica").
#   2. Strawberry Perl + NASM on PATH. openssl-sys (via libsqlite3-sys/SQLCipher)
#      vendor-builds OpenSSL from source; the Git/MSYS perl on PATH lacks the
#      Locale::Maketext::Simple modules its Configure needs.
#
# It also caps cargo's parallelism (CARGO_BUILD_JOBS): with ~20 logical CPUs the
# default fan-out of rustc + native cmake jobs exhausts committable memory and
# the build dies with "memory allocation of N bytes failed". Override the cap
# with $env:CARGO_BUILD_JOBS before running, or with -Jobs.

[CmdletBinding()]
param(
    # Max parallel cargo/rustc jobs. Lower if you still hit OOM, raise if you
    # have headroom and want a faster build. Defaults to CARGO_BUILD_JOBS if set.
    [int]$Jobs = $(if ($env:CARGO_BUILD_JOBS) { [int]$env:CARGO_BUILD_JOBS } else { 4 }),

    # Run `cargo clean` for the release target first. Use this after an
    # interrupted/OOM-killed build: a half-written .rlib or partial vendored
    # OpenSSL build dir surfaces later as "can't find crate for std" /
    # "only metadata stub found for rlib dependency core" / OpenSSL Configure
    # exit 22, none of which a re-run fixes on its own.
    [switch]$Clean
)

$ErrorActionPreference = 'Stop'

if (-not $IsWindows -and $env:OS -ne 'Windows_NT') {
    Write-Error 'This script only runs on Windows.'
    exit 1
}

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Split-Path -Parent $scriptDir

function Resolve-FirstExisting {
    param([string[]]$Candidates)
    foreach ($c in $Candidates) {
        if ($c -and (Test-Path $c)) { return $c }
    }
    return $null
}

# --- 1. Locate vcvars64 (prefer vswhere, fall back to well-known paths) -------
$vcvars = $null
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
if (Test-Path $vswhere) {
    $installPath = & $vswhere -latest -products * `
        -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 `
        -property installationPath 2>$null | Select-Object -First 1
    if ($installPath) {
        $candidate = Join-Path $installPath 'VC\Auxiliary\Build\vcvars64.bat'
        if (Test-Path $candidate) { $vcvars = $candidate }
    }
}
if (-not $vcvars) {
    $vcvars = Resolve-FirstExisting @(
        "${env:ProgramFiles(x86)}\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat",
        "$env:ProgramFiles\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat",
        "$env:ProgramFiles\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat",
        "$env:ProgramFiles\Microsoft Visual Studio\2022\Professional\VC\Auxiliary\Build\vcvars64.bat",
        "$env:ProgramFiles\Microsoft Visual Studio\2022\Enterprise\VC\Auxiliary\Build\vcvars64.bat"
    )
}
if (-not $vcvars) {
    Write-Error 'Could not find vcvars64.bat. Install Visual Studio 2022 (or Build Tools) with the "Desktop development with C++" workload.'
    exit 1
}
Write-Host "Using MSVC environment: $vcvars"

# --- 2. Import the vcvars64 environment into this session ---------------------
# Run the batch in cmd, dump the resulting environment, and apply it here so
# VSINSTALLDIR/INCLUDE/LIB/PATH (cl, link, nmake) are visible to cargo.
$envDump = cmd.exe /c "`"$vcvars`" >nul 2>&1 && set"
foreach ($line in $envDump) {
    if ($line -match '^(.*?)=(.*)$') {
        Set-Item -Path "Env:\$($matches[1])" -Value $matches[2]
    }
}
if (-not $env:VSINSTALLDIR) {
    Write-Error 'vcvars64 did not set VSINSTALLDIR; the tesseract/leptonica link will fail. Aborting.'
    exit 1
}

# --- 3. Strawberry Perl + NASM for the vendored OpenSSL build -----------------
$perlBin = Resolve-FirstExisting @("$env:STRAWBERRY_PERL_BIN", 'C:\Strawberry\perl\bin')
$nasmBin = Resolve-FirstExisting @("$env:STRAWBERRY_C_BIN", 'C:\Strawberry\c\bin')
if (-not $perlBin -or -not (Test-Path (Join-Path $perlBin 'perl.exe'))) {
    Write-Error 'Strawberry Perl not found (expected C:\Strawberry\perl\bin\perl.exe). Install it (it ships the OpenSSL Configure modules) or set $env:STRAWBERRY_PERL_BIN.'
    exit 1
}
if (-not $nasmBin -or -not (Test-Path (Join-Path $nasmBin 'nasm.exe'))) {
    Write-Error 'NASM not found (expected C:\Strawberry\c\bin\nasm.exe). Install Strawberry Perl (bundles NASM) or set $env:STRAWBERRY_C_BIN.'
    exit 1
}
$env:PATH = "$perlBin;$nasmBin;$env:PATH"
Write-Host "Perl: $(Join-Path $perlBin 'perl.exe')"
Write-Host "NASM: $(Join-Path $nasmBin 'nasm.exe')"

# --- 4. Cap parallelism to avoid commit-memory exhaustion --------------------
if ($Jobs -lt 1) { $Jobs = 1 }
$env:CARGO_BUILD_JOBS = "$Jobs"
Write-Host "CARGO_BUILD_JOBS=$($env:CARGO_BUILD_JOBS)"

# --- 5. Optional clean -------------------------------------------------------
if ($Clean) {
    Write-Host 'Cleaning release artifacts (cargo clean --release)...'
    cargo clean --release --manifest-path (Join-Path $repoRoot 'Cargo.toml')
    if ($LASTEXITCODE -ne 0) {
        Write-Error "cargo clean failed with exit code $LASTEXITCODE"
        exit $LASTEXITCODE
    }
}

# --- 6. Build ----------------------------------------------------------------
# tauri.conf.json sets bundle.createUpdaterArtifacts=true with an updater pubkey,
# so `tauri build` tries to sign the updater artifacts and exits 1 unless
# TAURI_SIGNING_PRIVATE_KEY is set — even though the .msi/.exe installers are
# already produced. For a local build without that key, override the config to
# skip updater artifacts so the build completes; if the key IS set (e.g. for a
# release), let it sign normally.
$tauriArgs = @('run', 'tauri', '--', 'build')
if (-not $env:TAURI_SIGNING_PRIVATE_KEY) {
    Write-Warning 'TAURI_SIGNING_PRIVATE_KEY not set - building installers WITHOUT signed updater artifacts. Set it (and TAURI_SIGNING_PRIVATE_KEY_PASSWORD) to produce updater .sig files.'
    $noUpdaterConfig = Join-Path ([System.IO.Path]::GetTempPath()) 'mnema-no-updater.json'
    [System.IO.File]::WriteAllText($noUpdaterConfig, '{"bundle":{"createUpdaterArtifacts":false}}')
    $tauriArgs += @('--config', $noUpdaterConfig)
}

Push-Location (Join-Path $repoRoot 'apps\desktop')
try {
    $env:CI = 'true'
    bun @tauriArgs
    if ($LASTEXITCODE -ne 0) {
        Write-Error "tauri build failed with exit code $LASTEXITCODE"
        exit $LASTEXITCODE
    }
}
finally {
    Pop-Location
}

# --- 7. Surface the installer ------------------------------------------------
$nsisDir = Join-Path $repoRoot 'target\release\bundle\nsis'
$installer = $null
if (Test-Path $nsisDir) {
    $installer = Get-ChildItem -Path $nsisDir -Filter '*.exe' |
        Sort-Object LastWriteTime -Descending | Select-Object -First 1
}
if ($installer) {
    Write-Host "Built installer: $($installer.FullName)"
    Start-Process explorer.exe "/select,`"$($installer.FullName)`""
}
else {
    Write-Warning "Build succeeded, but no NSIS installer was found in $nsisDir."
}
