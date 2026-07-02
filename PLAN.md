# Plan: Windows browser-URL capture via UI Automation

Design authority: [ADR 0044](docs/adr/0044-windows-browser-url-via-ui-automation.md) (accepted), extending ADR 0039's strategy model and discharging ADR 0043's browser-URL deferral. Approach validated on-device 2026-07-02 with a standalone UIA probe against Helium (Chromium, ships as `chrome.exe`) and Zen (Gecko).

## Problem

Windows frames carry app identity and window title but never a `browser_url` — macOS's strongest "what was the user doing" signal. Windows users get no URL context in the timeline, no URL-backed search results, no domain usage charts, and no Open Captured URL. The Browser URL mode setting is visible on Windows but inert.

## Solution

Add **UI Automation** as the third Browser URL Strategy (Windows-only, permission-free, engine-dialected): recognize the foreground browser by executable stem mapped to an engine family, read the active-tab URL via UIA (Chromium: window's Document element value; Gecko: focused-element climb to the enclosing Document), sanitize with the unchanged `sanitize_url`, and populate `FrameMetadataSnapshot.browser_url` in the existing Windows metadata refresh. Zero new UI; URLs inherit the ADR 0038 broker guard automatically.

## User Stories

1. As a Windows user, I want captured frames to carry the active tab's URL for my browser, so that timeline and search context match what macOS users get.
2. As a Windows user, I want URL capture governed by the existing frame-context toggle and Browser URL mode (Off / Sanitized / Full), so that my privacy posture needs no new controls or permissions.
3. As a Helium or Zen user, I want my non-mainstream browser covered, so that using a fork doesn't silently degrade my capture context.
4. As a maintainer, I want a one-command on-device smoke, so that a browser update breaking UIA URL reads is caught by rerunning it, not by ad-hoc debugging.

## Implementation Decisions

All settled in the 2026-07-02 grill session (see ADR 0044 for rationale and rejected alternatives):

- **No-guess invariant is cross-platform**: Gecko reads via focus-climb only; no `IsOffscreen` document scan, no address-bar read. Focus in browser chrome ⇒ no URL that tick.
- **Brand-less recognition**: pure `known_browser_engine_for_exe_stem(stem) -> Option<BrowserEngine>` allowlist in `crates/capture-metadata` (unit-testable, no Win32). v1 stems — Chromium: `chrome`, `msedge`, `brave`, `vivaldi`, `opera`, `opera_gx`, `chromium`, `arc`; Gecko: `firefox`, `zen`, `librewolf`, `waterfox`, `floorp`. Unrecognized exes (all Electron apps) are never probed.
- **Parallel type, not a shared-enum variant**: new `BrowserEngine` enum; the macOS `BrowserUrlStrategy` enum and bundle-id registry are untouched.
- **Reader**: new `apps/desktop/src-tauri/src/native_capture_browser_url_uia.rs` (`#[cfg(target_os = "windows")]`, declared in `native_capture.rs` beside the macOS `browser_url_ax`), mirroring the AX module's shape including `ReadOutcome::{Url, Dormant, NoWeb}`.
- **Bounded cost** (heavy pages cannot slow a tick): `IUIAutomation2` connection/transaction timeouts ~500ms per cross-process call; ~400ms wall-clock attempt budget checked before every UIA call; climb capped by ancestor hops; `FindFirst(Document)` early-exits shallow. Blown budget ⇒ no URL this tick.
- **Dormant Chromium**: macOS cold-poll shape inside the read (≤500ms total, 50ms steps); Gecko never triggers it.
- **Live reads, no `BrowserUrlProbeCache`**: read on every Windows metadata refresh (1s segment-loop poll + debounced foreground refresh, both off every capture lock). No `BrowserUrlReadMode::Cached` plumbing on Windows.
- **Wiring**: `collect_windows_active_window_snapshot()` (`native_capture_metadata.rs`) threads the foreground `hwnd` + pid to the reader when the exe stem resolves to an engine and `metadata_collection_plan` wants URLs; sanitize via `sanitize_url(raw, metadata.browser_url_mode)` exactly like macOS.
- **COM**: `CoInitializeEx(COINIT_MULTITHREADED)` per reading thread (poll thread and foreground-listener thread), init-once-per-thread; probe confirmed MTA works from a plain worker and inside the `WM_TIMER` pump.
- **Dependency**: add full `windows` crate 0.61 to `apps/desktop/src-tauri` `[target.'cfg(windows)']` with `Win32_UI_Accessibility`, `Win32_System_Com`, `Win32_System_Ole` (needed for `CreatePropertyCondition`/`VARIANT`; note `BOOL` lives in `windows::core` in 0.61).
- Multi-window browsers: always read the specific foreground `hwnd`, never "the process".
- Assumption: stems are best-effort and cheap to extend (one line + smoke run); unlisted forks silently capture no URL (ADR 0044 consequence).

## Testing Decisions

- **Unit tests (pure, cross-platform)**: stem→engine resolver in `capture-metadata` (case-insensitivity, `chrome` ⇒ Chromium, `zen` ⇒ Gecko, unknown/Electron stems ⇒ None, stem extraction from full paths via existing `app_display_name_from_exe_path` behavior).
- **Unit tests (Windows collector)**: snapshot wiring keeps `browser_url: None` when metadata disabled / mode Off / unrecognized exe — factor the decision ("should we probe, with which engine") into a pure function so it tests without Win32.
- **Not unit-testable**: the live UIA read (needs a running browser) — covered by the smoke flag on-device, mirroring how the macOS AX reader is verified manually.
- **On-device smoke** (`--windows-browser-url-smoke`): exercises the production reader + resolver against installed browsers; PASS = recognized browser yielded a well-formed URL within budget; prints engine, URL, timing.
- Run `cargo test -p capture-metadata` and `cargo check` locally (test gotchas: `ORT_DYLIB_PATH` needed for mnema-package tests; run cargo test in the foreground or the link step hits LNK1104; 23 pre-existing Windows failures are known-baseline, not ours).
- Do not test: macOS paths (untouched), sanitize_url internals (already covered), UIA tree shapes per browser version (that's the smoke's job).

## Slices

1. **Stem→engine resolver** (`crates/capture-metadata`)
   - Goal: `BrowserEngine` enum + `known_browser_engine_for_exe_stem` + v1 stem table + unit tests.
   - Areas: `crates/capture-metadata/src/lib.rs`.
   - Acceptance: unit tests green incl. case-insensitivity and unknown-stem None.
   - Depends on: none. Parallel: yes (with 2).
2. **UIA reader module** (`apps/desktop/src-tauri`)
   - Goal: `native_capture_browser_url_uia.rs` with `read_active_tab_url(hwnd, pid, engine) -> Option<String>`, ReadOutcome model, cold-poll, timeouts + wall-clock budget, per-thread COM init; Cargo dep addition.
   - Areas: new module, `native_capture.rs` declaration, `apps/desktop/src-tauri/Cargo.toml`.
   - Acceptance: `cargo check` green on Windows; module doc explains the bounded-cost contract (mirror the AX module's header).
   - Depends on: none (engine enum arrives from slice 1 at merge). Parallel: yes (with 1).
3. **Snapshot wiring**
   - Goal: populate `browser_url` in `collect_windows_active_window_snapshot` — thread `hwnd`/pid, gate on `metadata_collection_plan` + stem resolution, sanitize, keep collection off-lock.
   - Areas: `native_capture_metadata.rs` (Windows block).
   - Acceptance: pure gating tests green; manual in-app check shows `browserUrl` in the capture-privacy debug surface with Helium and Zen.
   - Depends on: 1 + 2.
4. **Smoke flag**
   - Goal: `--windows-browser-url-smoke` following the `windows_transient_liveness_smoke` pattern (`main.rs` guard, `lib.rs` wrapper with off-Windows exit-2 stub, smoke module with `--exe <stem>` targeting and PASS/FAIL).
   - Areas: `main.rs`, `lib.rs`, `native_capture/windows_browser_url_smoke.rs`.
   - Acceptance: smoke PASSes on-device against Helium and Zen; FAILs cleanly with no browser running.
   - Depends on: 2 (reader), 1 (resolver); ideally after 3 so it exercises the same gating.
5. **Docs + verification**
   - Goal: flip the SUPPORTS.md Windows browser-URL row to supported-via-UIA (note brand-less stems / Helium-as-chrome.exe), add the smoke to `docs/windows/on-device-capture-smoke-runbook.md`, record the on-device result.
   - Areas: `SUPPORTS.md`, `docs/windows/`.
   - Acceptance: docs match shipped behavior; smoke run recorded.
   - Depends on: 3 + 4 verified on-device.

Parallel groups: [1, 2], then [3], then [4], then [5].

## Out of Scope

- No `BrowserUrlProbeCache` adoption on Windows (revisit only if a pathological browser surfaces).
- No `IsOffscreen`/document-scan fallback, no address-bar reads.
- No brand-level identity, no structured `AppIdentity` (stays deferred per ADR 0043).
- No browser extension / native-host plumbing.
- No new UI, settings, onboarding, or permission surfaces.
- No macOS changes (registry, AX reader, AppleScript paths untouched).
- No schema/DB changes (`browser_url` already rides `snapshot_json` and the ADR 0038 guard).
- WebKit on Windows; exotic engines.

## Further Notes

- **Risks**: browser updates changing UIA tree shapes (mitigated by the smoke); Chromium accessibility residency cost inside the browser once woken (documented in ADR 0044, same class macOS Gecko pays); `opera_gx` stem needs on-device confirmation (Opera GX may ship as `opera.exe`) — harmless either way, verify during slice 5.
- **Reference**: probe findings and timing numbers live in ADR 0044's Context; the scratchpad probe source was throwaway per the grill (findings preserved, code superseded by the production reader + smoke).
- Build env: `scripts/build-windows-local.ps1` for full builds; the capture-metadata slice builds standalone and fast.
