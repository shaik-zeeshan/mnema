# `fetch_url`: opaque-id-keyed live page fetch with a two-boundary secret rule

## Status

Proposed.

## Context

Ask AI (Quick Recall + Chat) can only *read* capture data through the Brokered
Capture Access tools ([ADR 0033](0033-ask-ai-migrates-onto-shared-reasoning-engine.md)):
it sees what a page looked like *at capture time*. It cannot answer "what's the
status of that PR **now**?" — a live fetch of a page the user actually visited.

The obvious shape (a tool that takes a URL and GETs it) is an open-web-fetch /
SSRF surface, and it would hand a raw URL — the exact credential-bearing string
[ADR 0038](0038-brokered-browser-urls-are-read-time-guarded-host-path-raw-url-opens-app-mediated.md)
keeps off the cloud boundary — to the model. It also looks like the brokered
"open captured URL" path 0038 **removed** as a CSRF/replay primitive. `fetch_url`
must give the model *current* page state without reopening either hole.

## Decision

A single opt-in tool, `fetch_url`, gated on `ask_ai_web_fetch_enabled`
(`AccessSettings`, **default off**). When off the tool is absent from the
toolset. Both doors share one toolset (ADR 0031), so gating is settings-based,
never per-door.

1. **The model never supplies a URL — it passes an opaque capture id.** The
   schema is `{ opaqueId: string }`; the id comes from a prior `search` result's
   `context.url`. Resolution runs desktop-side:
   `authorize_active_opaque_capture_reference` → `frame_id` →
   `AppInfra::get_frame` → `metadata_snapshot.browser_url`. **Timeline intervals
   carry no opaque ids in v1** — to fetch a page seen in the timeline, the model
   searches for it first (the search-first recipe, taught in the tool
   description). Audio / no-URL captures resolve to a readable error, not a fetch.

2. **The two-boundary rule** — a captured URL is treated **two ways by two
   consumers**, extending 0038's split to the fetch case:
   - **Model-facing text** (`url`, `finalUrl` in the result JSON): the full
     `guard_url` form — query stripped, path secrets redacted. This is 0038's
     unchanged cloud boundary; the model still sees only a guarded host+path.
   - **Fetch target** (the string that reaches the origin, which already knows
     its own URL): the stored URL **minus secrets only**. A new `pub` scrubber
     sits next to `guard_url` in `app-infra/src/brokered_access/url_guard.rs`
     and reuses the deterministic secret detector
     ([ADR 0016](0016-secret-redaction-v2-deterministic-gate.md)): drop query
     params with credential-shaped **names** (`token`, `access_token`, `code`,
     `auth`, `key`, `api_key`, `secret`, `password`, `session`, `sig`, `otp`, …);
     drop params whose **value** matches the secret detector (JWT, `gh_`, `sk-`,
     …); **keep innocent params** (`?v=`, `?id=`, `?tab=` survive so the origin
     returns the right resource); **path-secret redaction stays** (a redacted
     path 404s — fail closed); the `https` scheme is forced.

   With the default `browser_url_mode` (Sanitized) queries are already stripped
   at capture, so param preservation only matters for Full-mode users; Sanitized
   users still get a host+path fetch.

3. **A cookie-less, first-party GET.** A shared static `reqwest::Client`
   (rustls, 15 s timeout, explicit UA, **no cookie jar** — so it carries no
   ambient session and is not a CSRF primitive). Redirects capped at 5 and
   **https-only on every hop**. The body is streamed with a **2 MB cap** behind a
   content-type gate (`text/html`, `text/plain`, `application/json`,
   `application/xhtml+xml`). `htmd` converts HTML→Markdown (pruning
   `script`/`style`/`noscript`/`nav`/`header`/`footer`/`svg`), capped at ~24k
   chars with `truncated: true`.

Result JSON: `{ url, finalUrl, status, title, content, truncated }` — both URLs
in guarded form.

## Considered options

- **A tool that takes a URL from the model.** Rejected — an open-web-fetch /
  SSRF surface, and it hands a raw credential-bearing URL to the cloud model.
  Keying off a prior `search` result confines every fetch to a page the user
  actually visited and keeps the raw URL out of the model's hands entirely.
- **Fetch the guarded form** (send the origin the query-stripped, path-redacted
  URL). Rejected (supersedes the earlier "fetch the guarded form" sketch): the
  guarded URL routinely 404s or returns the wrong resource because the origin
  needs its own real params (`?id=`, pagination, `?tab=`). The origin already
  knows its own address; withholding innocent params buys no privacy and breaks
  the fetch. We send the origin the real URL **minus secrets** and keep the
  guarded form only for the model.
- **A per-domain query-param allowlist.** Rejected — per-site maintenance for
  the same outcome the deterministic, domain-agnostic secret detector already
  gives. The scrubber is the allowlist's inverse (deny credential-shaped, keep
  the rest) with no site list to curate.
- **Route the fetch through the broker / revive `OpenCapturedUrl`.** Rejected —
  0038 removed brokered opening as a CSRF/replay primitive and set the invariant
  "the broker is never an opener." `fetch_url` is a desktop-side tool, not a
  broker request.

## Consequences

- **The "broker is never an opener" invariant (0038) holds.** `fetch_url` is
  desktop-side and **cookie-less**: it never navigates the user's authenticated
  browser and never replays a session — it is not the brokered captured-URL open
  that 0038 killed. The model still never sees a raw URL (guarded text only,
  0038's unchanged cloud boundary) and now never even *supplies* one. The single
  new egress is a first-party, cookie-less GET to a page the user already
  visited.
- **Read-time secret scrubbing, no migration.** Like 0038's guard, the scrubber
  runs read-time over the stored URL and reuses the 0016 detector, so it covers
  all history with no index column or backfill.
- **Accepted residual — a side-effectful GET with innocent param names** (the
  one-click-unsubscribe class): a URL like `…/unsubscribe?id=123` carries no
  credential-shaped param, so the scrubber keeps it and the GET can have a side
  effect. Accepted because it is bounded to pages the user actually visited, the
  request is cookie-less (no session to act under), and the dominant token vector
  (credential params + path tokens) is scrubbed. Revisit if it proves real.
- **Opt-in, off by default.** No behavior change until the user flips
  `ask_ai_web_fetch_enabled`; the toggle aside states plainly that the feature
  "re-requests the page address (minus secrets) over the network."

Extends [ADR 0038](0038-brokered-browser-urls-are-read-time-guarded-host-path-raw-url-opens-app-mediated.md)
(the two-boundary URL treatment and the broker-is-never-an-opener invariant) and
builds on [ADR 0033](0033-ask-ai-migrates-onto-shared-reasoning-engine.md)
(the in-process agent loop these tools plug into).
