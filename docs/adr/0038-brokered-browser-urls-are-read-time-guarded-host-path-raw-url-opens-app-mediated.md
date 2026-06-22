---
status: accepted
---

# Brokered browser URLs are a read-time guarded host+path; the raw URL opens app-mediated

## Context

Mnema already captures a per-frame `browser_url` (`FrameMetadataSnapshot`, default `BrowserUrlMode::Sanitized`, which strips query string and fragment at capture). The **Brokered Capture Access** layer did not surface it — search/timeline `context` carried only app bundle id, app name, and window title. We want URLs available to agents (they are strong "what was the user doing" signal) and a nicer in-app affordance, without leaking the credential material that lives in URLs (`?token=`, `#access_token=`, reset/invite/magic tokens in the path).

The per-result `context` channel is in fact **already open**: app name and window title already cross the broker boundary on every screen result (`brokered_access.rs` `map_search_response`, `broker_frame_timeline`). So this extends an existing channel rather than opening a new one.

## Decision

A captured URL is treated **two ways by two consumers**:

1. **Broker (cloud-facing): a guarded host+path text — the Broker URL Context.**
   - **Sanitize at the boundary:** strip query string and fragment *regardless of the capture-time setting*, so even a `Full`-mode stored URL yields only host+path to the broker. `http`/`https` only; `file://` collapses to `file://[local-file]`.
   - **URL Path Token Guard** over the host+path: reuse the deterministic secret detector for known key/token shapes (`gh_`, `sk-`, JWT, …), then redact a high-entropy path segment **only when it follows a credential-bearing predecessor** (`reset`/`reset-password`, `verify`, `confirm`, `activate`, `invite`, `magic`/`magic-link`, `otp`, `token`, `auth`). Auth/login *page* keywords stay visible; only token-shaped material is redacted.
   - **Read-time, from the representative frame, gated on data presence.** Computed at broker-return by joining the result's representative `Captured Frame` → metadata snapshot — no new index column, no backfill, covers history for free. Returned as `Search Context` on **both** search results and timeline intervals (an interval is page-granular via frame equivalence, so its representative frame's URL is accurate). Gated on the frame *carrying* a URL, not on the current `BrowserUrlMode`.

2. **App UI (local, trusted): Open Captured URL.**
   - An app-mediated "open in browser" action, the sibling of the CLI `open`/open-in-Mnema handoff: keyed off the **opaque result id**, resolved locally by the app, which opens the **raw stored URL** via `@tauri-apps/plugin-opener`. The raw URL is never returned as broker text and never reaches a cloud model — it materializes only on the user's click. Surfaced wherever a captured web moment shows (first-party views and AI answer cards), labeled by the host from the Broker URL Context. Not an agent tool.

## Considered options

- **Domain-only broker text.** Rejected: too little signal. Full path was chosen; the guard is the price.
- **Project a redacted `browser_url` column into the search index.** Rejected: persists an unguarded/pre-policy URL into the index (itself broker-visible text), needs a migration + backfill, and reopens persist-time redaction. The read-time join keeps the guard at the boundary and covers all history.
- **Blunt high-entropy path redaction.** Rejected: a reset token and a Google Doc id / commit SHA / UUID are character-indistinguishable, so entropy alone would gut the resource-id signal full-path was chosen for. Position (the predecessor) discriminates; raw entropy does not.
- **Retroactive setting-gate (flipping `Off` hides historical URLs).** Rejected: makes URL a special case *more* aggressive than window titles and OCR. Gating on per-frame data presence is consistent with every other captured field.

## Consequences

- The guard runs **read-time**, diverging from secret redaction's prospective persist-time gate ([ADR 0015](0015-secret-redaction-v2-is-prospective.md)/[0016](0016-secret-redaction-v2-deterministic-gate.md)) — by design, so it covers all historical URLs with no migration.
- **Accepted residual:** a bare, context-free path token with no credential-bearing predecessor (e.g. `app.com/AbC9x…`) can reach the broker as text. Accepted because query/fragment — the dominant token vector — is already stripped, so this is the long tail, and the alternative (entropy) destroys resource ids.
- Turning URL capture `Off` is **not retroactive**: frames captured while it was on keep their URLs (guarded at the boundary, openable in the UI) until **Delete Recent Capture** or **Retention Cleanup** removes them — same as every other captured field.
- This supersedes the V1 language in `crates/cli/CONTEXT.md` that said search/timeline return no app/window/browser metadata: app/window context already ships, and a guarded host+path now rides on it. Auth-channel and audit records still carry no URLs.

Extends [ADR 0012](0012-encrypted-capture-index-and-brokered-access.md) (brokered access returns redacted derived content + opaque ids) and [ADR 0024](0024-ask-ai-uses-pi-tool-shim-over-installed-runtime.md) (app-mediated open is not an agent tool).
