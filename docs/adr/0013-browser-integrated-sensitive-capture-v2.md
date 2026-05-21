---
status: rejected
---

# Do not ship browser-integrated credential-entry suspension

Mnema will not ship browser-integrated credential-entry suspension in this branch. The extension, native messaging host, pairing flow, coverage UI, automatic suspension/resume logic, and durable safety-gap records were removed.

## Context

The branch explored automatic recording suspension while browser credential fields were focused. That design required a browser add-on, a native messaging host, pairing state, heartbeat and fail-closed behavior, extra migration state, onboarding, and Settings controls. It expanded the core recording path and made recording less predictable.

## Decision

Reliability and explicit user control are more important than an unstable automatic privacy feature. Mnema keeps explicit controls and recovery paths: Pause Capture, User Capture Pause, inactivity pause, App Privacy Exclusion, Exclude Current App, Browser Capture Disclosure, and Delete Recent Capture.

Browser metadata remains metadata-only through the native browser URL probe governed by Browser Metadata Collection settings. It does not drive live privacy decisions or automatic recording suspension.

Mnema keeps the safer downstream work from the branch: Secret Redaction Pipeline and Brokered Capture Access.

## Consequences

There is no browser add-on setup, native host install, pairing token, browser coverage panel, automatic credential-entry pause, or safety-gap timeline interval. Unsupported or sensitive browser content is handled by explicit app exclusion, user pause, and delete-recent recovery rather than silent automatic suspension.

ADR 0010 is rejected by this ADR.
