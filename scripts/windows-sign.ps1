<#
.SYNOPSIS
  Authenticode-sign a Windows binary via Azure Artifact Signing (formerly
  Azure Trusted Signing), or no-op cleanly when no credentials are provisioned.

.DESCRIPTION
  Invoked by the Tauri v2 bundler through `bundle.windows.signCommand`
  (apps/desktop/src-tauri/tauri.windows.conf.json) once per binary it
  produces: the app exe, any sidecar exe, and the NSIS `-setup.exe`
  installer. Because the bundler calls this DURING bundling, Authenticode
  signing lands BEFORE tauri emits the minisign updater signature
  (`-setup.exe.sig`), so the `.sig` covers the signed bytes.

  Env-var convention (gate + credentials):
    AZURE_SIGNING_ENABLED   "true" => sign; anything else/absent => no-op exit 0.
                            CI derives this from the presence of the
                            AZURE_TENANT_ID secret (see release.yml).
    When enabled, ALL of the following are required (hard failure if any is
    missing — never silently ship unsigned when signing was requested):
    AZURE_TENANT_ID         service principal tenant     \
    AZURE_CLIENT_ID         service principal app id      > read by Azure
    AZURE_CLIENT_SECRET     service principal secret     /  EnvironmentCredential
    AZURE_SIGNING_ENDPOINT  region endpoint, e.g. https://eus.codesigning.azure.net
    AZURE_SIGNING_ACCOUNT   Artifact Signing account name
    AZURE_SIGNING_PROFILE   certificate profile name

  Tooling: the `TrustedSigning` PowerShell module (Invoke-TrustedSigning) is
  still the current "PowerShell for Authenticode" integration for Azure
  Artifact Signing — the service was renamed Jan 2026 but the module kept its
  name (per learn.microsoft.com/azure/artifact-signing/how-to-signing-integrations,
  updated 2026-05). The separately named `Az.ArtifactSigning` module is only
  for App Control CI-policy signing, not Authenticode. Alternative
  integrations if this module ever disappears: signtool.exe + the
  Microsoft.ArtifactSigning.Client dlib, or `azuresigntool`.

.PARAMETER FilePath
  Path of the binary to sign. Tauri substitutes `%1`.

.EXAMPLE
  powershell -NoProfile -ExecutionPolicy Bypass -File scripts/windows-sign.ps1 path\to\Mnema-setup.exe
#>
[CmdletBinding()]
param(
  [Parameter(Mandatory = $true, Position = 0)]
  [string]$FilePath
)

$ErrorActionPreference = 'Stop'

if ($env:AZURE_SIGNING_ENABLED -ne 'true') {
  Write-Host "unsigned preview: no Azure Artifact Signing credentials, skipping $FilePath"
  exit 0
}

try {
  # Signing requested: from here on, every failure must be fatal (non-zero
  # exit) so an unsigned binary can never slip into a release that asked to
  # be signed.
  $required = @(
    'AZURE_TENANT_ID',
    'AZURE_CLIENT_ID',
    'AZURE_CLIENT_SECRET',
    'AZURE_SIGNING_ENDPOINT',
    'AZURE_SIGNING_ACCOUNT',
    'AZURE_SIGNING_PROFILE'
  )
  $missing = @($required | Where-Object {
      [string]::IsNullOrWhiteSpace([Environment]::GetEnvironmentVariable($_))
    })
  if ($missing.Count -gt 0) {
    throw "AZURE_SIGNING_ENABLED=true but required env vars are missing: $($missing -join ', ')"
  }

  if (-not (Test-Path -LiteralPath $FilePath)) {
    throw "file to sign does not exist: $FilePath"
  }

  # Module install only happens on the enabled path, so unsigned preview
  # builds never touch the network or the PSGallery.
  if (-not (Get-Module -ListAvailable -Name TrustedSigning)) {
    Write-Host "TrustedSigning module not found -> installing from PSGallery (CurrentUser)."
    # Windows PowerShell 5.1 defaults to TLS 1.0 and PSGallery requires 1.2.
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
    Install-Module -Name TrustedSigning -Force -Scope CurrentUser -Repository PSGallery
  }

  Write-Host "Azure Artifact Signing: signing $FilePath"
  Invoke-TrustedSigning `
    -Endpoint $env:AZURE_SIGNING_ENDPOINT `
    -CodeSigningAccountName $env:AZURE_SIGNING_ACCOUNT `
    -CertificateProfileName $env:AZURE_SIGNING_PROFILE `
    -Files $FilePath `
    -FileDigest SHA256 `
    -TimestampRfc3161 'http://timestamp.acs.microsoft.com' `
    -TimestampDigest SHA256

  # Belt-and-braces: Invoke-TrustedSigning throwing is the primary failure
  # signal, but verify the result anyway so a silent tool regression cannot
  # ship an unsigned binary.
  $sig = Get-AuthenticodeSignature -LiteralPath $FilePath
  if ($sig.Status -ne 'Valid') {
    throw "post-sign verification failed for ${FilePath}: Authenticode status is '$($sig.Status)' ($($sig.StatusMessage))"
  }
  Write-Host "signed OK: $FilePath ($($sig.SignerCertificate.Subject))"
  exit 0
}
catch {
  Write-Host "ERROR: Azure Artifact Signing failed: $($_.Exception.Message)"
  exit 1
}
