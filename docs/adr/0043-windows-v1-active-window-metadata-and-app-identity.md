# Windows v1 active-window metadata and app identity

## Status

Accepted.

## Context

On macOS, active-app/window metadata (NSWorkspace active app + CoreGraphics
window list) is collected into `FrameMetadataSnapshot.app_bundle_id` /
`app_name`, persisted into `search_documents.app_bundle_id`, and drives timeline
app grouping, the `app:` search refinement, and app icons. The *privacy-rule*
identity (`ExcludedAppEntry.bundle_id`) is a **separate** field.

Two facts reshape what Windows v1 actually needs:

- The persisted metadata identity is a **grouping/label key**, not the
  privacy-matching key. App Privacy Exclusion matches the *live* foreground app at
  capture time (`evaluate_privacy` → the ScreenCaptureKit content filter) and is
  deferred on Windows (ADR 0025), so historical frame metadata never feeds a
  privacy decision. v1 therefore needs a good grouping key + icon source, not a
  bulletproof identity.
- The macOS metadata-refresh engine (`native_capture/privacy.rs`) — the
  NSWorkspace notifications, the generation/token coalescer, and the segment-loop
  filter application — exists almost entirely to keep the **live content filter**
  tight. Windows has no live filter in v1, so the only output that matters is the
  `latest_snapshot` that labels frames.

The research doc (`docs/windows/permissions-privacy-metadata-research.md`)
proposed a full identity precedence (AUMID/PFN → executable path) and a
structured `AppIdentity` DTO. Those are needed for correct per-app matching and
per-kind icon resolution — neither is a v1 concern.

## Decision

Windows v1 collects active-window metadata and records app identity as follows:

- **Fields:** `GetForegroundWindow` → PID (`GetWindowThreadProcessId`) →
  canonical executable path (`QueryFullProcessImageNameW`, normalized); window
  title (`GetWindowTextW`, gated by existing Metadata Settings); display name from
  the executable's version-info `FileDescription`, falling back to the file stem.
  `app_name` is **always** populated so a raw path never surfaces as a UI label
  (`timelineFrameAppLabel` falls back to the raw identity only when `app_name` is
  null).
- **Identity value:** the canonical executable path is stored **opaquely** in the
  existing `app_bundle_id` / `bundle_id` field. Trim-only `canonicalize_app_bundle_id`
  is path-safe, and the `app:` refinement index already lowercases
  (`LOWER(TRIM(...))`), giving case-insensitive matching that is correct for
  case-insensitive Windows paths. **No schema rename in v1** — the `bundleId`→`appId`
  rename and a structured `AppIdentity` DTO are deferred to when App Privacy
  Exclusion lands (that is when kind-disambiguation and per-kind icon resolution
  become necessary). macOS persisted settings are untouched.
- **Refresh mechanism:** a foreground-change listener
  (`SetWinEventHook(EVENT_SYSTEM_FOREGROUND)`) whose callback only *signals* a
  refresh (no heavy work in the callback), driving a prompt metadata-only snapshot
  update, backed by the existing cross-platform 1s segment-loop poll as a fallback.
  macOS's token/generation coalescer is **not** ported: it guards a stale async
  collector from clobbering the live content filter across a session reset, a
  hazard that does not exist without a live filter.

Explicitly deferred for v1: AUMID/PFN identity precedence; correct grouping of
UWP/Store apps hosted by `ApplicationFrameHost` (they may collapse under one host
name — documented limitation); and **browser-URL metadata** (reported
unsupported; adding any browser extension / native-host plumbing, or a URL source
that needs an OS-level permission the way macOS Gecko URLs need the Accessibility
permission, requires its own ADR and a matching permission-grant UX, per ADR 0013
and the SUPPORTS.md work item). "Exclude Current App" is hidden on the Windows
tray, applying ADR 0025.

## Alternatives Rejected

- **Structured `AppIdentity` DTO + full AUMID/PFN precedence now.** Correct UWP
  grouping and a future-proof privacy identity, but it builds the privacy-identity
  machinery — and threads `kind`/`platform` through the snapshot, search schema,
  and UI — ahead of the deferred feature that needs it.
- **Rename `bundleId`→`appId` in v1.** Removes the misnomer up front but requires
  a data-copy migration and index/FTS rebuild across shipped macOS installs for
  zero v1 benefit; the field is an opaque grouping key either way.
- **Plain 1s poll with no foreground hook.** Simpler, but frames captured during a
  sub-second focus switch would carry the previous app's label; the hook keeps
  per-frame app attribution accurate. (Chosen deliberately over the simpler poll.)

## Consequences

`app_bundle_id` is a cross-platform misnomer on Windows (it holds an executable
path); it is treated as an opaque grouping key, and the rename is a single atomic
migration deferred to the `AppIdentity` DTO work. UWP/Store apps may group under a
single host process name in v1. A future Windows privacy filter or real per-app
identity revisits the deferred structured identity, the foreground listener's
coalescing needs, and this ADR.
