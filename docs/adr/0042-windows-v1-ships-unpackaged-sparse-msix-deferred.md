# Windows v1 ships unpackaged; sparse MSIX deferred

## Status

Accepted.

## Context

Windows currently ships as an unsigned NSIS/MSI build with the Tauri/minisign
updater (`SUPPORTS.md`, `release.yml`). Adopting a **sparse MSIX** package
(package-with-external-location) would give Mnema its own **package identity**
(Package Family Name / AppUserModelID) — the only way on Windows to get (a) a
real *per-app* microphone grant through `DeviceAccessInformation`, which for an
unpackaged app degrades to the *global* privacy toggle, and (b) a stable,
OS-enforced app identity for future per-app privacy rules.

Both of those payoffs are, however, already off the v1 table:

- The per-app microphone grant was **deliberately declined** for v1. Mnema ships
  the honest-minimal model — capture-start `E_ACCESSDENIED` maps to a recoverable
  `microphone_access_denied` error plus a deep link to `ms-settings:privacy-microphone`,
  and permission is reported as `Unknown` (`SUPPORTS.md`, permissions checklist).
- **App Privacy Exclusion is deferred** on Windows (ADR 0025), so the OS-enforced
  per-app identity has no v1 consumer.

Meanwhile a sparse package's manifest must be **signed** with a certificate
chaining to a trusted root before Windows will grant package identity, and Mnema
ships **unsigned** today (Authenticode is a secret-gated no-op stub →
SmartScreen "unknown publisher"). MSIX distribution would also rework the
NSIS + minisign updater path.

## Decision

Windows v1 ships **unpackaged** (NSIS/MSI) with the existing Tauri/minisign
updater. Sparse MSIX is deferred to a future decision, gated on first acquiring a
code-signing certificate and reworking distribution/updater. Downstream, Windows
identifies captured apps by **canonical executable path** (see ADR 0043), with no
AUMID/PFN enforcement guarantees in v1.

## Alternatives Rejected

- **Adopt sparse MSIX now.** Unlocks a real per-app microphone grant and a path
  to an OS-enforced per-app privacy identity — but both are already deferred for
  v1 (mic ships honest-minimal; App Privacy Exclusion is deferred by ADR 0025),
  and it presupposes a signing certificate Mnema does not yet have plus an
  updater/distribution rework. Large scope for benefits v1 does not consume.

## Consequences

Microphone permission stays honest-minimal and there is no OS-enforced per-app
identity in v1. The eventual real per-app microphone or App Privacy Exclusion
work must revisit packaging **first**, and its ADR should revisit this one.
Distribution remains SmartScreen "unknown publisher" until code signing lands
(tracked separately from this decision).
