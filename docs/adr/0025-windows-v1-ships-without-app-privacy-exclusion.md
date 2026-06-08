# Windows v1 ships without App Privacy Exclusion

## Status

Accepted.

## Context

On macOS, **App Privacy Exclusion** is applied through the **Live Privacy Filter**: ScreenCaptureKit's content filter removes excluded apps' windows from the capture stream before frames reach Mnema (ADR 0006, ADR 0008). Windows Graphics Capture has no equivalent — monitor capture cannot exclude an individual app's windows from the captured surface. A Windows exclusion mechanism would have to be architecturally different, most plausibly suspension-shaped (pause or blank screen capture while an excluded app is relevant, reusing the existing privacy-suspension machinery) rather than pixel-filtering-shaped.

ADR 0013 already rejected automatic credential-entry suspension on every platform, so that is not a Windows gap. The rest of the sensitive-capture protection stack — explicit pause, app exclusions where supported, delete-recent recovery, secret redaction of derived text (ADR 0011/0015/0016), and brokered access (ADR 0012) — is platform-neutral and remains available on Windows.

## Decision

Windows v1 ships screen capture without App Privacy Exclusion.

- The settings and onboarding surfaces must not offer Excluded Apps on Windows (already enforced: the Privacy tab exposes frame metadata only, and the exclusion UI is platform-gated).
- The Recording Lifecycle on Windows carries no privacy-suspension state for app exclusion; nothing pretends a filter was applied.
- A future Windows exclusion mechanism is expected to be capture-suspension-shaped, not a WGC pixel filter, and requires its own ADR before being promised in any UI.

## Alternatives Rejected

- **Block Windows capture on designing a suspension-based exclusion mechanism.** Honors platform parity but holds the entire Windows port hostage to a design effort with real UX questions (how "excluded app is on screen" is detected; what suspension does to multi-source recordings). The platform-neutral protections plus an honest, absent UI were judged sufficient for v1.
- **Approximate exclusion by post-capture frame redaction.** Contradicts ADR 0006/0008: protection must happen live, before frames are persisted; post-capture filtering is exactly what those decisions ruled out.

## Consequences

Windows users have no per-app capture exclusion in v1; their protections are explicit pause, delete-recent, secret redaction of derived text, and brokered access. Any surface that conditionally shows exclusion features must gate on platform capability (consistent with the capability-driven-UI principle in ADR 0022), not assume exclusions exist everywhere. When a Windows mechanism is designed, it starts from the privacy-suspension machinery, and its ADR should revisit this one.
