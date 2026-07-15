---
status: accepted
---

# Windows reads browser URLs through UI Automation, engine-keyed and permission-free

This ADR documents the decision as taken at design time, after an on-device probe validated the approach. It is the browser-URL ADR that [ADR 0043](0043-windows-v1-active-window-metadata-and-app-identity.md) required before adding any Windows URL source.

## Context

On macOS, `browser_url` is captured through a per-browser **Browser URL Strategy** ([ADR 0039](0039-gecko-browser-active-tab-url-via-accessibility-api.md)): AppleScript for the Chromium/WebKit families, the Accessibility API for Gecko. Windows had no URL source at all — ADR 0043 shipped active-window metadata with `browser_url` always empty and explicitly deferred the URL to its own ADR, warning that a source needing an OS permission would also need a permission-grant UX.

Windows offers no AppleScript equivalent, and the Gecko session-file and browser-extension alternatives were already rejected on macOS (stale; heavyweight distribution). The remaining candidate was **UI Automation** (UIA), Windows' accessibility API. A standalone probe (Rust, `windows` 0.61) was tested on-device against **Helium** (Chromium) and **Zen** (Gecko):

- **Chromium**: the foreground window's `Document` element exposes the full-fidelity URL (scheme, query, fragment) via ValuePattern, ~7 ms warm. The renderer's accessibility is **dormant** until the first UIA client connects — the first read finds no `Document`; the connection itself wakes it and a re-read milliseconds later succeeds (the macOS Gecko dormancy case, mirrored).
- **Gecko**: native UIA, no dormancy, but `FindFirst(Document)` returns a preloaded `about:blank` — the *correct* read climbs from `GetFocusedElement` to the enclosing `Document` (~1 ms), the direct analog of the macOS climb to the outermost `AXWebArea`, and correct under Zen split view. An `IsOffscreen`-filtered document scan can recover a URL when focus sits in browser chrome, but it is ambiguous under split view and only meaningful for a foreground window.
- The **address bar** is lossy on Chromium (strips scheme, elides the domain) and is not even a UIA `Edit` control on Gecko.
- **No permission**: same-integrity UIA reads need no consent prompt, manifest change, or elevation, and work without the browser being foreground.
- **Identity wrinkle**: the exe stem is not the brand — Helium ships as `chrome.exe` (`...\imput\Helium\Application\chrome.exe`).

## Decision

**UI Automation is the third Browser URL Strategy** — Windows-only, engine-dialected, permission-free. It joins AppleScript and Accessibility; the strategies are never unified across platforms.

- **Engine-dialected reads.** Chromium: `ElementFromHandle` on the foreground window → `FindFirst` for the `Document` control → ValuePattern value. Gecko: `GetFocusedElement` → climb the control-view ancestors to the enclosing `Document` (bounded hops) → ValuePattern value. Both return the full URL, then flow through the unchanged `sanitize_url`.
- **The no-guess invariant carries over.** No `IsOffscreen` document scan, no address-bar read: when the focused element resolves to no document (e.g. the user is typing in the urlbar), Gecko yields no URL for that tick — exactly the macOS behavior. Preferring no URL over a guessed one is now a cross-platform invariant of every Browser URL Strategy.
- **Brand-less recognition: exe stem → engine family.** A pure allowlist in `crates/capture-metadata` maps a lowercased executable stem to `Chromium` or `Gecko` (v1: `chrome`, `msedge`, `brave`, `vivaldi`, `opera`, `opera_gx`, `chromium`, `arc` / `firefox`, `zen`, `librewolf`, `waterfox`, `floorp`). The strategy needs only the engine; the display name stays version-info-driven (ADR 0043), so Helium-as-`chrome.exe` is covered for free and there is deliberately no Windows notion of "Helium" in the URL pipeline. Unrecognized executables — including every Electron app — are never probed. In code this is a **parallel type** (`BrowserEngine` + stem resolver), not a new variant of the macOS `BrowserUrlStrategy` enum: the platforms share no dispatch site, and a shared variant would force dead arms into every macOS match.
- **Bounded-cost contract, mirroring the AX reader.** `IUIAutomation2` connection/transaction timeouts (~500 ms, the analog of `AXUIElementSetMessagingTimeout`) bound any single cross-process call; a wall-clock budget (~400 ms) checked before every UIA call bounds one read attempt; traversal shapes are chosen so page weight cannot matter (the climb costs one call per ancestor hop, `FindFirst(Document)` early-exits at a shallow fixed position, and nothing enumerates tabs or page content). A blown budget yields no URL for that tick. Dormant Chromium is handled with the macOS cold-poll shape (≤500 ms, 50 ms steps) inside the read, so even the first frame after focusing a fresh browser gets a URL; Gecko never triggers it.
- **Live reads, no probe cache.** Windows does not adopt the title-gated `BrowserUrlProbeCache`: the cache is a cost workaround for macOS probes (an `osascript` spawn, or an AX read up to ~1.4 s), and UIA reads cost ~1–7 ms. Reading live on every metadata refresh (the 1 s poll and the debounced foreground refresh, both off every capture lock) is strictly fresher — SPA navigations that change the URL without the title are caught every tick, and the cache's known dynamic-title desync window never occurs. The cache seam stays in place cross-platform if a pathological browser ever surfaces.
- **Zero new UI.** No permission means no Windows analog of the macOS Accessibility grant row or onboarding item (ADR 0043's permission-UX requirement is satisfied vacuously). The platform-neutral metadata settings — frame-context toggle and browser-URL mode — fully govern the feature, and a UIA-sourced URL inherits the ADR 0038 read-time broker guard unchanged.
- **On-device smoke.** A `--windows-browser-url-smoke` flag (the `windows_transient_liveness_smoke` pattern) exercises the production reader and stem resolver against installed browsers, so browser-update regressions are caught by one command rather than an ad-hoc debugging session.

## Considered Options

- **Browser extension / native-host plumbing.** Accurate but heavyweight distribution and per-browser maintenance; already rejected as the macOS default and held as a last resort (ADR 0039).
- **Gecko session file.** Already rejected on macOS: checkpointed minutes behind the live tab, ambiguous active-tab recovery.
- **Address-bar read.** Rejected by probe evidence: lossy on Chromium (no scheme, elided domain), not a UIA `Edit` on Gecko, and reflects user-typed text.
- **`FindAll(Document)` + `IsOffscreen` scan as a Gecko fallback.** Rejected: it is the "window scan" ADR 0039 refused — ambiguous under Zen split view, foreground-only semantics, and cost that scales with tab count — papering over a moment (urlbar focused) where macOS also, deliberately, yields no URL.
- **Verifying brand via version-info before probing.** Rejected: recognition needs only the engine; a brand table adds maintenance and a version-info read for nothing — a non-browser named `chrome.exe` merely gets a bounded UIA read that finds no document.
- **Reusing `BrowserUrlProbeCache` on Windows.** Rejected: it would import the cache's staleness trade-offs (5 s same-title backstop, 1.5 s re-probe floor) to defend against a cost class that does not exist on Windows.
- **Extending the shared `BrowserUrlStrategy` enum with a `UiAutomation` variant.** Rejected: symmetry in name only — every macOS dispatch would gain an unreachable arm while Windows still needed its own stem-keyed table.

## Consequences

- Windows gains `browser_url` for the Chromium and Gecko families with no new permission, no new UI, and no schema change (`browser_url` already rides the snapshot JSON and the ADR 0038 guard).
- Recognition is engine-granular: all ships-as-`chrome.exe` forks (Chrome, Helium, …) are one Chromium hit, indistinguishable in the URL pipeline by design; per-brand treatment would require the deferred structured `AppIdentity` work (ADR 0043).
- Reading UIA wakes Chromium's renderer accessibility, which stays resident for that browser process's lifetime — the same residency cost class macOS Gecko users pay, here paid by Chromium users on Windows (Gecko's tree is always on).
- New browsers ship in v1 only if their stem is in the allowlist; unlisted forks silently capture no URL until added (one-line change plus smoke run).
- `apps/desktop/src-tauri` takes its first dependency on the full COM-capable `windows` crate (`Win32_UI_Accessibility`, `Win32_System_Com`, `Win32_System_Ole`), alongside the existing `windows-sys`.
- WebKit has no Windows presence and Windows-only exotic engines are out of scope; SUPPORTS.md documents the supported set.

Extends [ADR 0039](0039-gecko-browser-active-tab-url-via-accessibility-api.md) (the strategy model and no-guess invariant) and discharges the browser-URL deferral in [ADR 0043](0043-windows-v1-active-window-metadata-and-app-identity.md); URLs inherit [ADR 0038](0038-brokered-browser-urls-are-read-time-guarded-host-path-raw-url-opens-app-mediated.md) unchanged.
