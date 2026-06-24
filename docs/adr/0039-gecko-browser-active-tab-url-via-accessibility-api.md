---
status: accepted
---

# Gecko browsers expose their active-tab URL through the macOS Accessibility API, opt-in beside AppleScript

This ADR documents the decision as taken at implementation time.

## Context

Mnema captures a per-frame `browser_url` as timeline and search context — a strong "what was the user doing" signal that also anchors Open Captured URL ([ADR 0038](0038-brokered-browser-urls-are-read-time-guarded-host-path-raw-url-opens-app-mediated.md)). On macOS that URL came from one mechanism: an `osascript` AppleScript probe of the frontmost browser, which the Chromium and WebKit families answer (per-browser dialect). Gecko browsers — Firefox (`org.mozilla.firefox`) and Zen (`app.zen-browser.zen`) — expose **no scriptable URL surface**, so Mnema captured no URL for them at all. Gecko is a meaningful share of frames; the gap was a real hole in timeline/search context.

Two non-AppleScript sources were investigated before this decision:

- **The on-disk session file (`recovery.jsonlz4`).** Firefox/Zen persist the active tab into a compressed session store. Probes showed it lags the live tab by minutes (it is checkpointed, not written on every tab switch), and disambiguating the active tab from it is unreliable (e.g. Zen's `selected` flag does not track the focused split pane). It reports a stale, often wrong tab — unacceptable for capture that is supposed to reflect the on-screen moment.
- **A browser extension.** Could read the live URL accurately, but requires shipping and installing a per-browser extension — heavyweight distribution and an ongoing maintenance surface. Held as an explicit last resort.

Probing the macOS Accessibility (AX) tree settled it: the focused web area carries an `AXURL` attribute that returns the **true active tab, live and correct**, including in Zen's split view (the focused pane's document, not a background one). AX needs the macOS Accessibility permission (TCC) and wakes the browser's own accessibility engine, but it reads the live state with no extension and no stale on-disk file.

## Decision

Active-tab URL reading is modeled as a per-browser **Browser URL Strategy**, and a second strategy is added beside AppleScript:

- **`BrowserUrlStrategy::AppleScript(dialect)`** stays the Chromium/WebKit path — `osascript`, no extra permission, no side effects.
- **`BrowserUrlStrategy::Accessibility`** is the new Gecko (Firefox/Zen) path. The reader (`native_capture_browser_url_ax.rs`, macOS-only) takes `AXFocusedUIElement` and climbs the `AXParent` chain to the **outermost `AXWebArea`** (the top document, not an iframe), then reads `AXURL`. It never scans windows or the address bar; if it cannot resolve a focused web area it returns no URL rather than guess.

The strategy is resolved per bundle id (`browser_url_strategy(bundle_id)`), and the two strategies are **never unified** — Chromium/WebKit stay on AppleScript.

The Accessibility strategy is **opt-in and Gecko-only**, gated by a bare `AXIsProcessTrusted()` check in the reader:

- An optional, **non-blocking onboarding item** to grant the permission is rendered only when a Gecko browser is installed.
- For users who skip onboarding, a **one-time-per-process first-sighting prompt** (`maybe_prompt_on_gecko_frontmost`, via `AXIsProcessTrustedWithOptions` with `kAXTrustedCheckOptionPrompt`) fires the first time a Gecko browser is frontmost while browser-URL capture is on and trust is missing.
- If the permission is never granted, Gecko browsers yield no URL — exactly the prior behavior.

The Tauri command surface — `get_browser_url_accessibility_status`, `request_browser_url_accessibility`, `open_browser_url_accessibility_settings` — is registered in `lib.rs`; status reports `{ trusted, geckoBrowsers: [{ bundleId, displayName, installed }] }`.

The reader is built on hand-rolled `extern "C"` FFI against the `ApplicationServices` framework (no new crate dependency). It bounds a hung browser with `AXUIElementSetMessagingTimeout` (0.5s) and polls the first read (≤500ms, 50ms steps) to wake a dormant a11y engine — measured cold→live latency is ~100–150ms.

## Considered Options

- **`recovery.jsonlz4` session file.** Rejected: stale (checkpointed, lags live tab by minutes) and the active tab is ambiguous to recover from it (Zen `selected` does not track the focused pane). Capture must reflect the on-screen moment, not a stale checkpoint.
- **A browser extension.** Rejected as the default and held as an explicit last resort: accurate but requires shipping and installing a per-browser extension, with heavyweight distribution and ongoing maintenance.
- **Address-bar text or a window scan as a fallback URL.** Rejected: the address bar is not in the AX tree unless it is focused, and a window scan would guess at which document is active. Preferring no URL over a guessed one keeps captured context truthful.
- **Unifying Chromium/WebKit onto AX too (one strategy).** Rejected: AppleScript already answers those browsers with no permission and no a11y-engine wake. Forcing them onto AX would impose the heavier Accessibility permission and the a11y-engine cost on users who pay neither today, for no new capability.

## Consequences

- **A new, heavier permission.** Mnema now asks for the macOS Accessibility (TCC) grant — broader than the screen/microphone permissions it already holds. It is opt-in and Gecko-only, so users without a Gecko browser are never prompted, and a user who declines keeps today's behavior (no Gecko URL).
- **A11y-engine residency cost inside the browser.** Reading AX wakes Firefox/Zen's own accessibility engine, which then stays resident for that browser process's lifetime, costing some CPU/memory **inside the browser** (not in Mnema). This is the price of a live, accurate read; it only happens for users who grant the permission and run a Gecko browser.
- **Graceful degradation.** With the permission not granted (or the read failing), Gecko browsers degrade to today's blank-URL behavior. The 0.5s messaging timeout and ≤500ms first-read poll bound the cost of a hung or dormant browser so a stuck read cannot stall capture.
- **Privacy parity — no Gecko-special handling.** Gecko URLs flow through the unchanged `sanitize_url` at capture and the read-time broker guard from [ADR 0038](0038-brokered-browser-urls-are-read-time-guarded-host-path-raw-url-opens-app-mediated.md): only the guarded host+path **Broker URL Context** crosses the broker boundary, and the raw URL opens only via app-mediated Open Captured URL. An AX-sourced URL is just a captured `browser_url` like any other; ADR 0038's guard already covers it.
- A second native source area for browser URLs (`native_capture_browser_url_ax.rs`) now exists alongside the AppleScript probe in `native_capture_metadata.rs`; the per-browser strategy resolution lives in `crates/capture-metadata`.

Extends [ADR 0038](0038-brokered-browser-urls-are-read-time-guarded-host-path-raw-url-opens-app-mediated.md) (the broker guard and app-mediated open already cover AX-sourced URLs unchanged).
