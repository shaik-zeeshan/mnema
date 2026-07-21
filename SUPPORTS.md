# Platform Support

_Last reviewed: 2026-07-07_

This file tracks Mnema platform-specific implementation status. It is intentionally implementation-facing: it names the OS-owned capabilities that must exist behind Mnema's shared capture, processing, privacy, storage, and release seams.

## Legend

- `[x]` implemented / usable in the current app
- `[ ]` not implemented yet
- `[~]` partially implemented, stubbed, or supported only through a platform-specific fallback

## Current support summary

| Area | macOS | Windows | Linux | Notes |
| --- | --- | --- | --- | --- |
| Tauri desktop shell | [x] | [~] | [~] | Shell is mostly cross-platform, but window chrome/dock behavior has macOS-specific paths. |
| Native screen capture | [x] | [ ] | [ ] | macOS uses ScreenCaptureKit / AVFoundation fallback. |
| Native microphone capture | [x] | [ ] | [ ] | macOS uses AVFoundation. |
| Native system-audio capture | [x] | [ ] | [ ] | macOS uses Core Audio process taps (`CATapDescription` + aggregate device) in `crates/capture-system-audio` — an **independent capture family**, a sibling of the microphone, with no screen dependency: audio-only sessions are allowed, it records through display sleep/lock/disconnect, and it carries its own TCC category ("Screen & System Audio Recording", `NSAudioCaptureUsageDescription`) that no API can query. Recovery is a tap rebuild (device-change listeners + zero-watchdog with backoff). Runtime gate stays macOS 15.0. See [ADR 0052](docs/adr/0052-system-audio-is-an-independent-capture-family-on-core-audio-process-taps.md). |
| Capture segment lifecycle | [x] | [ ] | [ ] | Lifecycle is generic in shape but active runtime fields are macOS-gated. |
| Media writers/finalization | [x] | [ ] | [ ] | macOS uses AVAssetWriter, AVFoundation, `afconvert`, and some `ffmpeg` trim paths. |
| Screen frame export / frame index | [x] | [ ] | [ ] | Required for OCR, exact frame lookup, duplicate detection, and scrub previews. |
| Exact frame preview from video | [x] | [ ] | [ ] | macOS uses AVAssetImageGenerator. |
| Scrub preview generation | [x] | [ ] | [ ] | macOS-only today. |
| OCR: Apple Vision | [x] | [ ] | [ ] | Apple-only provider. |
| OCR: Tesseract/PaddleOCR | [x] | [~] | [~] | Cross-platform intent, but Windows/Linux packaging/runtime need verification. |
| Audio transcription: Apple Speech | [x] | [ ] | [ ] | Apple-only provider. |
| Audio transcription: Local Whisper/Parakeet | [x] | [~] | [~] | Models are cross-platform-ish, but audio decode is AVFoundation-only today. |
| Audio transcription: Deepgram (cloud) | [x] | [x] | [x] | First transcription provider whose availability is **not** platform-bound: a plain HTTPS upload of the finished `.m4a` segment to Deepgram's pre-recorded API — no local audio-decode path. Requires a user-supplied Deepgram API key (BYO-key, keychain account `transcription.deepgram`) and a blocking **consent gate** before any audio leaves the device ([ADR 0047](docs/adr/0047-cloud-transcription-is-a-provider-property-with-an-explicit-consent-gate.md)); connectivity/auth errors are transient liveness that never lose transcriptions ([ADR 0048](docs/adr/0048-cloud-transcription-errors-are-transient-liveness-not-job-failures.md)). |
| Speaker analysis | [x] | [ ] | [ ] | On-device diarization runs through the `speakrs` provider (pure-Rust pyannote-community-1 segmentation + WeSpeaker embedding + VBx clustering on CoreML), which links system OpenBLAS. CoreML ties it to Apple Silicon macOS; audio decode is AVFoundation-only too. |
| Semantic Search (local embeddings + vec0) | [x] | [~] | [~] | Built and tested on macOS only. Embeddings run on-device via **candle** behind a pluggable **Semantic Search Backend**: the Apple GPU (Metal, F16) on macOS or **candle-CPU** (F32) elsewhere — the runtime tries Metal then falls back to CPU; CUDA is deferred (no v1 artifact). This replaces the earlier `fastembed`/ONNX (`ort`) path along with its per-thread QoS downclock and arena mitigations. `sqlite-vec` (`vec0`) is still statically linked into the SQLCipher amalgamation (unchanged), and model download is still a desktop-owned `reqwest` (now `rustls`) fetch from Hugging Face (now safetensors layout) — all cross-platform in principle and free of native capture/audio-decode dependencies. Note candle-metal is a **second native ML runtime** in the bundle (links Metal/MPS) alongside `ort`, which now remains only for transcription — flag for the release/notarization checklist. Windows/Linux are **unverified**: claiming support is gated on a **candle-CPU measurement** (CPU%, throughput, RSS) plus a real build/run of the candle-CPU + static `vec0` link on those platforms. See [ADR 0037](docs/adr/0037-semantic-search-embeddings-on-candle-with-pluggable-backend.md). |
| Inactivity detection | [x] | [ ] | [ ] | macOS uses CoreGraphics input idle plus capture-sourced screen/audio activity. |
| Sleep/wake recovery | [x] | [ ] | [ ] | macOS uses AppKit/NSWorkspace + ScreenCaptureKit liveness. |
| Live app privacy exclusion | [x] | [ ] | [ ] | macOS excludes an app's **windows** through ScreenCaptureKit app exclusion filters and its **audio** through the process tap's exclude list — two mechanisms, one privacy list, kept in parity since the tap replaced the SCK content filter that used to silence both ([ADR 0052](docs/adr/0052-system-audio-is-an-independent-capture-family-on-core-audio-process-taps.md)). The audio half is optional: Settings → Privacy → "Filter system audio" (`privacy.filterSystemAudio`, default on) gates whether the privacy list reaches the tap; screen exclusion is unaffected. The tap also excludes Mnema's own process, which is never toggleable. Windows/Linux semantics need design. |
| Active app/window metadata | [x] | [ ] | [ ] | macOS uses NSWorkspace/CoreGraphics. |
| Browser URL metadata | [x] | [ ] | [ ] | macOS reads the active-tab URL through a per-browser **Browser URL Strategy**: AppleScript for supported Chromium and WebKit browsers (no extra permission), and the macOS Accessibility API for Firefox-family (Gecko) browsers — Firefox and Zen — reading `AXURL` off the focused web area. The Accessibility path is opt-in and Gecko-only: it requires the macOS Accessibility permission; if the permission is not granted, Gecko browsers yield no URL. See [ADR 0039](docs/adr/0039-gecko-browser-active-tab-url-via-accessibility-api.md). |
| Recommended app exclusions | [x] | [ ] | [ ] | Current catalog uses macOS bundle IDs. |
| Webview memory-cache purge on blur | [x] | [ ] | [ ] | macOS purges `WKWebsiteDataTypeMemoryCache` when a window blurs (rate-limited) so decoded frame-preview IOSurfaces don't accumulate in the WebContent process (`webview_cache.rs`). WebView2/WebKitGTK cache behavior unassessed. |
| Quick Recall launcher panel | [x] | [~] | [~] | macOS summons a non-activating NSPanel (key without activating Mnema, like Spotlight/Raycast); non-macOS falls back to a plain shown/focused always-on-top window without non-activating semantics. |
| Ask AI (in-process Reasoning Engine) | [x] | [~] | [~] | Quick Recall + Insights Chat run in-process on the shared Reasoning Engine (`crates/ai-runtime` via `rig-core`) — no installed PI/Node runtime, no shim, no `node`/`pi`-on-PATH resolution. Brokered `search`/`timeline`/`show_text`/`recall_context` tools (plus presentation-only `reference_captures`) are injected from the Tauri layer under the All-Retained Ask AI broker scope. Cross-platform Rust like the engine itself; a cloud engine needs the bring-your-own-key in the Encrypted Capture Index key store (macOS Keychain only today, see that row), a local Ollama/Llamafile engine needs no key and is platform-agnostic. Windows/Linux are blocked only on the platform key store for cloud keys. |
| Triggers (Schedule → sealed Ask AI run + delivery) | [x] | [~] | [~] | Walking skeleton (issue #175, ADR 0058): `triggers.json` definitions, the 30s evaluator worker, per-trigger last-fired state, and the sealed-toolbox Ask AI run are cross-platform Rust. Delivery uses `tauri-plugin-notification` (cross-platform in principle, exercised on macOS only). Two macOS-specific edges: the wake catch-up nudge listens to the macOS-only `system_did_wake` notifier (elsewhere the poll tick still catches up within ~30s), and the notification-click → open-that-conversation route rides the macOS `RunEvent::Reopen` activation path (the desktop notification plugin has no click callback). |
| Triggers: Meeting Ends detection (Core Audio mic-holds) | [x] | [ ] | [ ] | macOS-only (issue #177, ADR 0057): the detector reads per-process mic-in-use state from Core Audio process objects (`kAudioProcessPropertyIsRunningInput` via `crates/capture-system-audio`'s cidre binding — the "orange dot" signal). The state machine, Readiness Wait, and firing wiring are cross-platform Rust, but on other platforms the mic snapshot has no backing read, so the detector worker idles and `meeting_ends` triggers never fire. |
| MCP stdio connector login-shell PATH | [x] | [ ] | [~] | Stdio MCP connector children get the user's login-shell PATH (`$SHELL -l -c`, resolved once per process, `/bin/zsh` fallback) set as their `PATH` env, so a packaged GUI app's minimal launchd PATH doesn't break bare commands (`npx`) or their shebang lookups (`#!/usr/bin/env node`). A user-provided PATH env row on the connector overrides it; resolution failure falls back to the inherited PATH unchanged. Unix mechanism (macOS exercised; Linux should behave the same but is unexercised); Windows resolves via its own rules and is unaddressed. |
| MCP connector Node detection (`mcp_check_node`) | [x] | [ ] | [~] | Settings probes for Node (`node --version`) before/after adding a local (stdio) MCP preset, resolving `node` through the same once-per-process login-shell PATH as stdio connector children (row above) — never the packaged app's minimal launchd PATH. Missing Node shows a warn state ("needs Node — install from nodejs.org"); the connector can still be added but starts disabled. Unix mechanism (macOS exercised; Linux unexercised); Windows unaddressed. |
| MCP stdio connector teardown (process-group kill) | [x] | [ ] | [~] | Stdio MCP connectors are spawned as a Unix process-group leader (`process-wrap` `ProcessGroup::leader()` around the `tokio::process::Command` handed to rmcp's `TokioChildProcess`), so dropping the cached client kills the whole group — the launcher (e.g. `npx`) **and** its server grandchildren — instead of leaking the real server. Exercised on macOS (grandchild-kill test in `ask_ai/mcp/transport.rs`); Linux uses the same `killpg` mechanism but is unexercised; Windows is unaddressed (`process-wrap`'s `JobObject` sibling exists when needed). |
| MCP http connector OAuth (browser authorize) | [x] | [ ] | [ ] | OAuth is an **auth mode** on the `Http` transport ([ADR 0051](docs/adr/0051-mcp-oauth-is-an-auth-mode-on-the-http-transport-dcr-only-over-the-deep-link.md)), built on `rmcp`'s `auth` feature (PKCE + RFC 7591 Dynamic Client Registration + `.well-known` discovery + silent refresh; `AuthClient` over reqwest 0.13). Connect runs a foreground browser round-trip whose redirect is captured through Mnema's existing **`mnema://` deep link** (`mnema-dev://` in dev) — the `on_open_url` handler routes `mnema://oauth/callback?code&state` by path, keyed to a CSRF-`state` pending map in `McpManager`; **no loopback listener** (avoids the recurring macOS firewall prompt). The **OAuth Token Set** (access/refresh/expiry/DCR `client_id`) serializes into the **same single keychain slot** the static bearer secret uses (service `com.shaikzeeshan.mnema.mcp-connectors`, account = instance id — macOS Keychain only, see that row), so there is no new keychain service, table, or migration. Disconnect/delete makes a best-effort RFC 7009 revocation then always drops the local token. The `mnema://` scheme registration, keychain, and opener are all macOS on this branch; Windows/Linux need their scheme-registration and keychain siblings (blocked on the platform key store, same as the cloud-key rows). |
| User Context / Reasoning Engine | [x] | [~] | [~] | Reasoning Engine + User Context derivation are cross-platform Rust via `rig-core` with no native capture dependency. A cloud engine (Anthropic/OpenAI bring-your-own-key) works on any platform with a key, but the key is stored in the **Encrypted Capture Index key store** (macOS Keychain only today, see that row). A local engine needs a running Ollama/Llamafile endpoint and no key, so it is platform-agnostic. Windows/Linux are blocked only on the platform key store for cloud keys; local-engine derivation should already work cross-platform. |
| Storage-location folder picker | [x] | [~] | [~] | Save-directory chooser in Settings (Storage) and onboarding uses `@tauri-apps/plugin-dialog` `open({ directory: true })` over a cross-platform `resolve_base_dir` (`get_storage_location` command). Cross-platform Tauri/Rust by construction; only verified on macOS today. |
| Low-disk capture safety | [x] | [ ] | [ ] | Free space on the recordings volume is checked at each new-segment boundary (the **preflight** before the first segment, each **rotation** before the next). Recording refuses to start when free space is below a `1 GiB` reserve floor plus one segment's estimated size (`insufficient_disk_space`); running low at a boundary or filling mid-segment suspends **all** sources (screen, system audio, **and** microphone — they share the volume) as a `LowDisk` transient-liveness **Capture Suspension**, with hysteresis auto-resume once free space rises above a higher resume threshold. A mid-segment disk-full discards the partial file and commits no segment row (no corrupt `.mov`/`.m4a`); dropping below the reserve floor stops the session gracefully ("recording stopped — disk full"). Implemented macOS-first; the Windows branch picks it up after merging `main` (the same design covers the Windows Media-Foundation file-lock case, because partials are discarded rather than left locked). See [ADR 0040](docs/adr/0040-low-disk-safety-is-a-transient-liveness-capture-suspension-kind.md). |
| Status bar / tray | [x] | [~] | [~] | Tauri tray exists cross-platform; current UX includes macOS-only Exclude Current App behavior. |
| Global shortcuts | [x] | [~] | [~] | Uses Tauri global shortcut plugin for background start/stop, pause/resume, and show/hide; platform behavior needs verification. |
| Encrypted Capture Index key store | [x] | [ ] | [ ] | macOS stores the SQLCipher key as a data-protection keychain item in a team-shared access group (`RJYMY4RR97.day.mnema.capture-index`) readable silently by the app and the `mnema-cli` sidecar only; the app migrates the legacy silent `/usr/bin/security` item (migrate-and-delete, gated on a successful database open), the CLI reads new-then-old and never writes, and builds whose entitlement does not validate (ad-hoc/dev) fall back to the old silent item or the `MNEMA_CAPTURE_INDEX_KEY_DIR` file knob ([ADR 0057](docs/adr/0057-capture-index-key-moves-to-a-shared-keychain-access-group.md)). Windows/Linux platform key stores are missing. |
| Licensing & Trial | [x] | [ ] | [ ] | Offline one-time License + server-issued 30-day Trial → Read-Only Mode, verified on-device against a baked-in Ed25519 public key via the licensegate client crate ([ADR 0044](docs/adr/0044-monetize-as-one-time-purchase-with-paid-update-window.md)/[0045](docs/adr/0045-licenses-verified-offline-ed25519-polar-merchant-of-record-only.md)/[0054](docs/adr/0054-licensing-moves-onto-licensegate.md)). The adapter (`apps/desktop/src-tauri/src/licensing/`) and the state/gate are cross-platform Rust, but the license-key + receipt + stamp store (`license_token_store.rs`, service `day.mnema.licensing`) is **macOS Keychain only** — the same platform-key-store gap as the rows above. Update-Window enforcement is auto-updater-only (declines builds dated after `update_through`), never a runtime capture lock. Once-per-machine activation ([ADR 0053](docs/adr/0053-licenses-activate-once-per-machine-via-a-signed-activation-receipt.md)) is **macOS-only**: the machine fingerprint (`machine_id.rs`) reads the hardware UUID via `gethostuuid(2)`, and Windows/Linux stubs return an error. |
| Broker Authorization Channel | [x] | [ ] | [~] | Unix socket implementation works for macOS/Linux shape; Windows needs named pipe/TCP/etc. |
| CLI sidecar build | [x] | [~] | [~] | Script has target-aware `.exe` handling, but packaging/release not verified. |
| Release/updater pipeline | [x] | [ ] | [ ] | Current release workflow ships Apple Silicon macOS only. |

## macOS checklist

### Runtime capture

- [x] Screen capture via ScreenCaptureKit on macOS 15+.
- [x] AVFoundation screen fallback for older macOS backend constraints.
- [x] Microphone capture via AVFoundation.
- [x] System-audio capture through a Core Audio process tap (`crates/capture-system-audio`), independent of screen capture ([ADR 0052](docs/adr/0052-system-audio-is-an-independent-capture-family-on-core-audio-process-taps.md)). The tap excludes Mnema's own process plus the privacy-listed apps' audio processes (parity with the ScreenCaptureKit content filter it replaced), and the writer format follows the tap's device-dependent ASBD (44.1 ↔ 48 kHz) rather than a pinned rate.
- [x] System-audio tap rebuild as the one recovery mechanism: default-output-device change, device death, exclude-list movement, or a zero-watchdog trip (~30 s without sound, backing off 30 s → 600 s, running even while paused for inactivity) tears down tap + aggregate and starts a fresh segment. Events log under the `system-audio-tap:` prefix.
- [x] Segment rotation and finalization for screen, microphone, and system audio.
- [x] User capture pause/resume.
- [x] Inactivity pause/resume for screen, microphone, and system audio families.
- [x] Sleep/wake recovery for screen while preserving microphone and system-audio continuation.
- [x] Screen liveness reconciliation from AppKit wake notifications and ScreenCaptureKit stream delegate failures.
- [x] Dark/deep-idle wake recovery via a Core Graphics display-reconfiguration listener (the panel powering back on re-arms capture even when macOS does not post `NSWorkspaceDidWake`, e.g. Power Nap / "Wake from Deep Idle"); `NSWorkspaceDidWake` is kept as an idempotent fast-path fallback. No polling.
- [x] Display-unavailable recovery: a display sleep/lock/lid-close/disconnect — surfaced as a `DisplayUnavailable` privacy-filter apply error **or** as a ScreenCaptureKit delegate stream-stop (`-3815`) — suspends **screen only** (microphone and system audio keep recording), commits the finalized in-flight tail segment, and auto-resumes when a display returns, instead of failing the session. Segment rotation suspends rather than fails if the screen session is missing without a suspension owner.
- [x] Screen frame export, captured-frame equivalence, OCR batching, and frame-index sidecars.

### Media and processing

- [x] `.mov` screen segment output.
- [x] `.m4a` microphone/system-audio output.
- [x] Exact frame preview extraction from finalized video.
- [x] Scrub preview generation from finalized indexed segments.
- [x] Apple Vision OCR provider.
- [x] Tesseract/PaddleOCR provider integration.
- [x] Apple Speech provider.
- [x] Local Whisper/Parakeet provider integration using AVFoundation decode.
- [x] Deepgram cloud transcription provider: BYO-key HTTPS upload of the finished `.m4a` segment to Deepgram's pre-recorded API (key in keychain account `transcription.deepgram`), gated by a blocking consent dialog before any audio leaves the device; connectivity/auth errors requeue as transient liveness without burning a retry attempt, so transcriptions are never lost. See [ADR 0047](docs/adr/0047-cloud-transcription-is-a-provider-property-with-an-explicit-consent-gate.md) and [ADR 0048](docs/adr/0048-cloud-transcription-errors-are-transient-liveness-not-job-failures.md).
- [x] Speaker analysis via the `speakrs` provider (CoreML + system OpenBLAS) using AVFoundation decode.
- [x] System-audio speech activity using AVFoundation-backed audio decode.
- [x] Semantic Search: on-device **candle** embeddings on the Apple GPU (Metal, F16) behind a pluggable **Semantic Search Backend** (the runtime tries Metal then falls back to candle-CPU; CUDA deferred), `vec0` vectors inside the SQLCipher-encrypted Capture Index, deferred-startup backfill sweep, and desktop-owned model download (now `rustls`, safetensors layout) from Hugging Face. This supersedes the `fastembed`/`ort` path: Metal frees the P-cores by construction, so the macOS-only per-thread QoS downclock and the ONNX arena mitigations are gone. candle-metal is a **second native ML runtime** in the bundle (links Metal/MPS) alongside `ort`, which now stays only for transcription — a release/notarization checklist item. No native capture or audio-decode dependency, so the runtime is platform-neutral in principle; only verified on macOS today. See [ADR 0037](docs/adr/0037-semantic-search-embeddings-on-candle-with-pluggable-backend.md).

### Privacy, metadata, and UX

- [x] Screen and microphone permission checks/prompts.
- [x] System-audio permission without an authorization API: the TCC prompt fires on the first tap read (or from onboarding's Grant button, which runs a throwaway tap), and the state is **inferred** as a tri-state — *not yet requested* / *assumed working* (a tap delivered a sound) / *possibly blocked* (every tap so far delivered only silence) — backed by one persisted `app_settings` evidence row. A "possibly blocked" state raises a dismissible hint deep-linking to Privacy & Security → Screen & System Audio Recording. See [ADR 0052](docs/adr/0052-system-audio-is-an-independent-capture-family-on-core-audio-process-taps.md).
- [x] Open macOS Privacy & Security panes for denied permissions.
- [x] Active app/window metadata from NSWorkspace/CoreGraphics.
- [x] Browser URL metadata for supported Chromium/WebKit browsers via AppleScript. Firefox-family (Gecko) browsers — Firefox and Zen — are supported through the macOS Accessibility API (reading `AXURL` off the focused web area); this path is opt-in and requires the macOS Accessibility permission, and Gecko browsers yield no URL until it is granted. See [ADR 0039](docs/adr/0039-gecko-browser-active-tab-url-via-accessibility-api.md).
- [x] Gecko (Firefox/Zen) Accessibility-permission UX: a Settings → Privacy row (gated on a Gecko browser being installed and browser-URL capture being on) and an optional onboarding item (shown only when a Gecko browser is installed; never gates progression). Both probe trust via `get_browser_url_accessibility_status`, raise the macOS Accessibility prompt via `request_browser_url_accessibility`, deep-link to Privacy & Security → Accessibility via `open_browser_url_accessibility_settings`, and re-poll trust on demand and on window focus. A failed probe leaves the status null so the row simply hides — it never gates capture. See [ADR 0039](docs/adr/0039-gecko-browser-active-tab-url-via-accessibility-api.md).
- [x] Live App Privacy Exclusion through ScreenCaptureKit app filters (windows) and the process tap's exclude list (audio), both fed from the same privacy list.
- [x] Exclude Current App tray action.
- [x] Recommended sensitive app exclusion catalog using macOS bundle IDs.
- [x] Native status-bar tray menu.
- [x] Quick Recall non-activating NSPanel launcher: reclassed `NSPanel` that becomes key without activating Mnema, floating window level, all-Spaces/full-screen-auxiliary collection behavior, `acceptsFirstMouse:` click pass-through, web-layer-owned Escape, and order-out/blur-grace dismissal.
- [x] Dock visibility and macOS terminate handling.
- [x] Global shortcuts through Tauri plugin for background start/stop recording, pause/resume recording, and show/hide Mnema.

### Storage, access, and release

- [x] Storage-location folder picker (Settings Storage + onboarding) via `@tauri-apps/plugin-dialog` `open({ directory: true })` over a cross-platform `resolve_base_dir` (`get_storage_location` command).
- [x] Low-disk capture safety as a `LowDisk` transient-liveness Capture Suspension. Free space (`fs2::available_space`) is probed at each new-segment boundary, not by a healthy-path poll: the **preflight** refuses to start recording when free space is below a `1 GiB` reserve floor plus one segment's estimated size (returns `insufficient_disk_space`), and each **rotation** applies the same check to the next segment. Running low at a boundary (or the disk filling mid-segment) suspends **all** sources — screen, system audio, **and** microphone, since they share the recordings volume — captures nothing while suspended, surfaces "Paused — low disk" on the tray/dashboard with a `warning` notification, and auto-resumes (restarting the microphone session too) once free space climbs above a higher resume threshold (hysteresis). A mid-segment disk-full failure best-effort deletes the partial file and commits no Capture Segment row, so the library never gains a broken `.mov`/`.m4a`. If free space drops below the `1 GiB` reserve floor — which protects Mnema's own SQLite Capture Index, OCR, and previews alongside the OS — the session stops gracefully with a "recording stopped — disk full" (`error`) notification. See [ADR 0040](docs/adr/0040-low-disk-safety-is-a-transient-liveness-capture-suspension-kind.md).
- [x] Encrypted Capture Index key in a shared-access-group data-protection keychain item (app + `mnema-cli` sidecar entitled; silent-item migration on launch; validation drill blocked on Developer ID CI signing — see [ADR 0057](docs/adr/0057-capture-index-key-moves-to-a-shared-keychain-access-group.md) and `docs/agents/capture-index-key-drills.md`).
- [x] Broker Authorization Channel over per-user Unix socket.
- [x] Ask AI (Quick Recall + Insights Chat) brokered tool-calling session: runs in-process on the shared Reasoning Engine (`crates/ai-runtime` via `rig-core`, `run_agent_loop` in `agent_loop.rs`) — NOT the user's installed PI/Node runtime, which is removed (no shim, no `node`/`pi`-on-PATH resolution, no PI auth). It uses the same OS-Keychain provider key as User Context derivation and exposes only the `search`/`timeline`/`show_text`/`recall_context` brokered data tools (plus the presentation-only `reference_captures`), injected from the Tauri layer and enforced through the All-Retained Ask AI broker scope (`BrokeredCaptureAccess::execute_for_ask_ai`). Gating is two-layer: the engine-configured prerequisite plus the independent Ask AI Setting. See [ADR 0033](docs/adr/0033-ask-ai-migrates-onto-shared-reasoning-engine.md).
- [x] Ask AI background completion and live reattach (stateless-per-turn over the persistent conversation store): each turn loads the thread's persisted history, runs one agent loop, and persists the new turn — the resident-session/30-minute-unseen-expiry/resurrect-from-transcript machinery is deleted. A dismissed-but-streaming question finishes its task and writes the turn to the shared conversation store in the Encrypted Capture Index (origin `quick_recall`, governed by Retention Policy, cleared by Wipe User Context), and re-opening reads it back; a thread still generating supports live reattach (the in-flight task persists incremental partial progress, the reopened surface loads that partial then subscribes to ongoing `delta` events). A thread can be continued in the Insights Chat workspace ("Continue in Chat") under the same conversationId, and a Chat thread may pin a per-conversation engine identity. See [ADR 0033](docs/adr/0033-ask-ai-migrates-onto-shared-reasoning-engine.md) and [ADR 0031](docs/adr/0031-quick-recall-and-chat-share-one-persistent-conversation-store.md).
- [x] User Context derivation via the Reasoning Engine (`rig-core`): cloud (Anthropic/OpenAI bring-your-own-key, key in Keychain) or local (Ollama/Llamafile endpoint, no key); only redacted OCR/transcript text crosses the wire for a cloud engine, the dossier stays on-device, and the deterministic Confidence Policy / Sensitive Category Guardrail run with no model.
- [x] Deep-link app reopen fallback.
- [x] Open Captured URL in the default browser: **exclusively** the local desktop `open_captured_url(frame_id)` Tauri command (`apps/desktop/src-tauri/src/app_infra.rs`). It resolves a result's raw captured `browser_url` locally, scheme-gates it to `http`/`https`, and hands it to the `@tauri-apps/plugin-opener` plugin. There is **no broker or CLI path**: the broker's `OpenCapturedUrl` arm is rejected for every caller with `authorization_required` (it was a CSRF/replay sink, deliberately removed), and there is no `mnema open-url` subcommand (a CLI test asserts it fails to parse). The raw URL is local-only — it never enters a broker response, log, audit, or auth channel. Only the guarded host+path **Broker URL Context** crosses the broker boundary (see [ADR 0038](docs/adr/0038-brokered-browser-urls-are-read-time-guarded-host-path-raw-url-opens-app-mediated.md)).
- [x] Mnema CLI sidecar for Apple targets.
- [x] macOS release workflow for Apple Silicon.
- [ ] Developer ID signing and notarization.
- [ ] Universal or Intel macOS release, if needed.

## Windows checklist

Research notes:

- `docs/windows/runtime-capture-research.md` tracks recommended Windows capture APIs and crate options.
- `docs/windows/media-processing-research.md` tracks OCR/transcription/speaker/media-processing provider choices and alternatives.
- `docs/windows/permissions-privacy-metadata-research.md` tracks Windows permission, privacy, app identity, and metadata alternatives.
- `docs/windows/storage-access-release-research.md` tracks Windows 11 storage paths, key-store/CLI access choices, and release/updater options.

### Bring-up / compilation

- [ ] Add Windows CI job for `cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml`, `cargo check --workspace --all-targets`, and `bun run check`.
- [ ] Audit all `cfg(target_os = "macos")` / non-mac stubs and ensure Windows builds cleanly with the desktop feature set.
- [ ] Decide whether capture output remains `.mov`/`.m4a` or becomes Windows-native formats with schema/runtime support for extensions.
- [ ] Remove user-facing “only macOS” errors once Windows adapters exist.
- [ ] Add Windows Tauri bundle config, installer target, signing plan, and updater artifacts.

### Runtime capture

- [ ] Implement a Windows screen capture backend behind `crates/capture-screen`.
  - Candidate APIs: Windows Graphics Capture or DXGI Desktop Duplication.
  - Must support frame timing, segment rotation, stop/error reporting, and output activity samples.
- [ ] Implement Windows microphone capture behind `crates/capture-microphone`.
  - Candidate APIs: WASAPI input, CPAL, or another native audio layer.
  - Must support device listing, default device tracking, selected-device reconnect policy, and VAD PCM feed.
- [ ] Implement Windows system-audio capture.
  - Candidate API: WASAPI loopback.
  - **Decided (macOS, [ADR 0052](docs/adr/0052-system-audio-is-an-independent-capture-family-on-core-audio-process-taps.md)): system audio is an independent capture family, not a rider on screen capture.** A Windows implementation is out of scope here, but it should mirror that shape: own session and inactivity lifecycle, no screen dependency, audio-only sessions allowed, and privacy-listed apps excluded from the audio itself. Only the low-disk suspension stops audio alongside the screen ([ADR 0040](docs/adr/0040-low-disk-safety-is-a-transient-liveness-capture-suspension-kind.md)), because that one is about the volume, not the display.
- [ ] Implement Windows capture session IDs and source-session bookkeeping.
- [ ] Implement segment rotation without dropping OCR/frame-index invariants.
- [ ] Implement user pause/resume and inactivity pause/resume for each requested source family.
- [ ] Implement capture liveness/error propagation equivalent to ScreenCaptureKit delegate stop errors.
- [ ] Implement sleep/wake/session-lock recovery.

### Media writers and previews

- [ ] Implement cross-platform or Windows-specific audio/video writers.
  - Candidate APIs: Media Foundation, FFmpeg, GStreamer, or a Rust media pipeline.
- [ ] Implement finalized video validation equivalent to “openable `.mov` with moov”.
- [ ] Implement screen frame export to JPEG artifacts.
- [ ] Implement frame-index sidecar generation from finalized video timing.
- [ ] Implement exact frame preview extraction from video.
- [ ] Implement scrub preview batch generation.
- [ ] Implement audio trim/convert/finalization for microphone and system-audio outputs.
- [ ] Implement video-only screen output finalization when audio is muxed or recorded together.

### Processing

- [ ] Add Windows audio decode to mono PCM for Local Whisper.
- [ ] Add Windows audio decode to mono PCM for Parakeet.
- [ ] Add Windows audio decode to mono PCM for speaker analysis.
- [ ] Add Windows audio decode to mono PCM for system-audio speech activity.
- [ ] Verify packaged Tesseract/PaddleOCR runtimes and model installation on Windows.
- [ ] Choose Windows OCR default provider.
- [ ] Choose Windows transcription default provider.
- [ ] Hide/disable Apple Vision and Apple Speech providers on Windows.

### Permissions, privacy, and metadata

- [ ] Implement microphone permission/status UX and settings deep link, e.g. `ms-settings:privacy-microphone`.
- [ ] Define Windows screen-capture permission/support semantics.
- [ ] Define Windows system-audio permission/support semantics.
- [ ] Implement system idle detection, likely `GetLastInputInfo`.
- [ ] Implement active app/window metadata.
  - Candidate APIs: foreground window handle, process ID, process executable path, window title.
- [ ] Define Windows app identity model for privacy rules.
  - Candidate identifiers: executable path, process name, AppUserModelID, package family name.
- [ ] Implement app candidate discovery and icon materialization.
- [ ] Design live app privacy exclusion semantics.
  - If per-app exclusion is not possible, expose clear unsupported/degraded UX and rely on pause/delete-recent/app disclosure.
- [ ] Implement “Exclude Current App” using Windows active-window identity, or hide/disable it.
- [ ] Add Windows sensitive app recommendation catalog.
- [ ] Add Windows known-browser catalog and browser capture disclosure.
- [ ] Decide whether browser URL metadata is supported on Windows; do not add browser extension plumbing without an ADR. If the chosen source needs an OS-level permission (the way Gecko URLs need macOS Accessibility), add an equivalent permission-grant/recheck UX in Settings and onboarding mirroring the macOS Gecko Accessibility flow.

### Storage, access, and release

- [ ] Implement Windows Capture Index Key Store using Credential Manager, DPAPI, or another platform-owned secret store.
- [ ] Pick up low-disk capture safety after merging `main`. The `LowDisk` Capture Suspension design is platform-neutral: probe free space at each new-segment boundary (preflight + rotation), refuse to start / suspend all sources / hysteresis auto-resume around a `1 GiB` reserve floor plus one-segment estimate, and stop gracefully below the floor. The same discard-the-partial behavior also covers the Windows Media-Foundation file-lock case, since partials are discarded rather than left locked. See [ADR 0040](docs/adr/0040-low-disk-safety-is-a-transient-liveness-capture-suspension-kind.md).
- [ ] Implement Broker Authorization Channel for Windows.
  - Candidate transports: named pipes, localhost loopback, or another app-mediated IPC.
- [ ] Update `crates/cli` authorization request path for Windows.
- [ ] Verify deep links and app launch fallback on Windows.
- [ ] Verify Open Captured URL on Windows. This is exclusively the local desktop `open_captured_url(frame_id)` command via the `@tauri-apps/plugin-opener` plugin (the broker rejects `OpenCapturedUrl` and there is no CLI `open-url`); verify the captured http(s) URL opens in the default browser and that the raw URL never leaks into a broker response, log, or audit record. Note: the `open_external_url` `cmd /C start "" <url>` branch now receives only internal `mnema://` deep links (via `open_mnema_deep_link`), never captured URLs — verify that deep-link path resolves to the app, not a browser.
- [~] Verify Ask AI (in-process Reasoning Engine) on Windows. There is no PI/Node runtime or shim to spawn — the agent loop runs in-process via `rig-core`. Verify brokered `search`/`timeline`/`show_text`/`recall_context` tool calls round-trip, the All-Retained broker scope/redaction holds, streaming/cancellation/background-completion work, and engine status reports correctly. Cloud-engine use is gated on the Windows Capture Index Key Store (above); a local Ollama/Llamafile engine needs no key.
- [~] Verify Quick Recall launcher behavior on Windows. The non-macOS fallback shows/focuses a plain always-on-top window (no non-activating panel), so verify summon-without-stealing-foreground, focus-into-search-field, and click-away/blur dismissal, or implement a Windows-native equivalent.
- [ ] Verify tray/menu behavior on Windows.
- [ ] Verify global shortcut registration and default shortcut labels on Windows.
- [ ] Add Windows release workflow and update manifest generation.
- [ ] Add Windows install/update signing process.

## Linux checklist

Linux support is not the immediate target, but these are the likely seams if/when Mnema expands beyond macOS/Windows.

### Bring-up / compilation

- [ ] Add Linux CI job for Rust and frontend checks.
- [ ] Audit macOS-only stubs for acceptable Linux unsupported behavior.
- [ ] Decide supported desktop environments/compositors: Wayland-first, X11 fallback, or both.
- [ ] Add Linux Tauri bundle config and release artifacts, e.g. AppImage/deb/rpm.

### Runtime capture

- [ ] Implement screen capture backend.
  - Candidate APIs: PipeWire + xdg-desktop-portal for Wayland, X11 capture fallback if supported.
- [ ] Implement microphone capture.
  - Candidate APIs: PipeWire/PulseAudio/ALSA via CPAL or native bindings.
- [ ] Implement system-audio/loopback capture.
  - Candidate APIs: PipeWire monitor streams or PulseAudio monitor sources.
- [ ] Implement segment rotation, liveness, sleep/wake/session-lock recovery, and source pause/resume.
- [ ] Implement screen activity samples without excessive compositor/GPU cost.

### Media writers and previews

- [ ] Choose Linux media writer/extractor stack.
  - Candidate APIs: FFmpeg or GStreamer.
- [ ] Implement screen frame export and frame-index sidecars.
- [ ] Implement exact video-backed frame previews.
- [ ] Implement scrub preview generation.
- [ ] Implement audio decode/trim/convert for processing providers.

### Processing

- [ ] Verify Tesseract/PaddleOCR packaging on Linux.
- [ ] Add Linux audio decode for Local Whisper/Parakeet.
- [ ] Add Linux audio decode for speaker analysis.
- [ ] Add Linux audio decode for system-audio speech activity.
- [ ] Hide/disable Apple-only providers.

### Permissions, privacy, and metadata

- [ ] Define screen/microphone/system-audio permission UX for portals and desktop environments.
- [ ] Implement system idle detection.
  - Candidate APIs: xdg idle portal, compositor-specific APIs, or X11 screensaver extension.
- [ ] Implement active app/window metadata where available.
- [ ] Define Linux app identity model for privacy rules.
  - Candidate identifiers: desktop file ID, app ID, executable path, process name.
- [ ] Implement app candidate discovery and icons from desktop entries.
- [ ] Design live app privacy exclusion semantics for PipeWire/portal capabilities.
- [ ] Add Linux sensitive app and known-browser catalogs if metadata/disclosure is supported. If a browser-URL source is added and needs an OS-level permission (the way Gecko URLs need macOS Accessibility), add an equivalent permission-grant/recheck UX in Settings and onboarding mirroring the macOS Gecko Accessibility flow.

### Storage, access, and release

- [ ] Implement Linux Capture Index Key Store using Secret Service/libsecret/KWallet or a clear unsupported flow.
- [ ] Carry low-disk capture safety into the Linux capture backend. The `LowDisk` Capture Suspension design is platform-neutral — boundary free-space probe (preflight + rotation), refuse to start / suspend all sources / hysteresis auto-resume around a `1 GiB` reserve floor plus one-segment estimate, discard the partial on mid-segment disk-full, and stop gracefully below the floor — so the Linux writer stack should adopt it alongside the screen/audio backends. See [ADR 0040](docs/adr/0040-low-disk-safety-is-a-transient-liveness-capture-suspension-kind.md).
- [~] Broker Authorization Channel can likely reuse Unix socket shape, but needs Linux config-dir/runtime-dir review.
- [ ] Verify CLI sidecar packaging and app-mediated authorization flow on Linux.
- [ ] Verify Open Captured URL across desktop environments. This is exclusively the local desktop `open_captured_url(frame_id)` command via the `@tauri-apps/plugin-opener` plugin (the broker rejects `OpenCapturedUrl` and there is no CLI `open-url`); verify the captured http(s) URL opens in the default browser under Wayland/X11 portals and that the raw URL never leaks into a broker response, log, or audit record. Note: the `open_external_url` `xdg-open <url>` branch now receives only internal `mnema://` deep links (via `open_mnema_deep_link`), never captured URLs — verify that deep-link path resolves to the app, not a browser.
- [~] Verify Ask AI (in-process Reasoning Engine) on Linux. There is no PI/Node runtime or shim to spawn — the agent loop runs in-process via `rig-core`. Verify brokered `search`/`timeline`/`show_text`/`recall_context` tool calls round-trip under the All-Retained broker scope, streaming/cancellation/background-completion work, and engine status reports correctly across desktop environments. Cloud-engine use is gated on the Linux Capture Index Key Store; a local Ollama/Llamafile engine needs no key.
- [~] Verify Quick Recall launcher behavior across desktop environments/compositors. The non-macOS fallback shows/focuses a plain always-on-top window (no non-activating panel); verify summon focus, always-on-top/all-workspaces behavior, and click-away/blur dismissal under Wayland/X11, or implement a compositor-appropriate equivalent.
- [ ] Verify tray/menu behavior across desktop environments.
- [ ] Verify global shortcuts; Linux support may depend on desktop environment/portal support.
- [ ] Add Linux release workflow and updater artifacts if supported.

## Current macOS-only source map

Use this map when turning checklist items into implementation slices.

| Source area | macOS-only implementation today | Windows/Linux replacement needed |
| --- | --- | --- |
| `crates/capture-screen/src/lib.rs` | ScreenCaptureKit/AVFoundation screen capture (video only), permissions, stream liveness, frame export, frame-index rebuild | Platform capture backend, permission semantics, writers, frame export/index, liveness/recovery |
| `crates/capture-system-audio/src/lib.rs` | Core Audio process tap + aggregate device + IOProc (cidre `core_audio`), exclude-list computation and listeners, zero-watchdog/rebuild engine, tap-following writer format, inferred permission tri-state | WASAPI loopback (Windows) / PipeWire monitor (Linux) adapter as an independent source, plus its own exclusion and permission semantics |
| `crates/capture-microphone/src/lib.rs` | AVFoundation microphone capture, device list/change notifications, permission prompt, VAD PCM feed | WASAPI/CPAL/PipeWire/etc. microphone adapter, device policy, permission UX |
| `crates/capture-writers/src/lib.rs` | AVAssetWriter/AVAudioFile, `afconvert`, AVFoundation duration/decode helpers | Cross-platform writer, duration validation, decode/trim/convert |
| `apps/desktop/src-tauri/src/native_capture/*` | Runtime active sessions and lifecycle operations are mostly macOS-gated | Promote runtime fields/adapters to platform-neutral traits or add Windows/Linux gated implementations |
| `apps/desktop/src-tauri/src/native_capture_metadata.rs` | NSWorkspace, CoreGraphics window list, AppleScript browser URL probe (Chromium/WebKit) | Foreground-window/app metadata and optional browser metadata per OS |
| `apps/desktop/src-tauri/src/native_capture_browser_url_ax.rs` | macOS Accessibility (`AXUIElement`) reader for Gecko (Firefox/Zen) active-tab URL — `AXURL` off the focused→outermost web area, gated by `AXIsProcessTrusted`, with the first-sighting trust prompt | OS-specific Gecko URL source if a non-AppleScript browser must be supported, or explicit unsupported behavior |
| `apps/desktop/src-tauri/src/native_capture/privacy.rs` | ScreenCaptureKit app exclusion filters; the same collected exclusion list is forwarded from the segment loop to the system-audio tap's exclude list | OS-specific live exclusion or explicit unsupported/degraded behavior, for windows **and** audio |
| `apps/desktop/src-tauri/src/native_capture_system_idle.rs` | CoreGraphics idle time | `GetLastInputInfo` on Windows; portal/X11/compositor path on Linux |
| `apps/desktop/src-tauri/src/app_infra/frame_preview.rs` | AVAssetImageGenerator exact/scrub previews | FFmpeg/GStreamer/Media Foundation extractor |
| `crates/audio-transcription/src/macos_audio_decode.rs` | AVFoundation audio decode for Local Whisper/Parakeet | Cross-platform audio decode module |
| `crates/speaker-analysis/src/macos_audio_decode.rs` | AVFoundation audio decode for diarization/recognition; the `speakrs` provider itself is CoreML + OpenBLAS (Apple Silicon only) | Cross-platform audio decode module **and** a non-CoreML diarization provider, since `speakrs` cannot run off Apple Silicon |
| `crates/ocr/src/lib.rs`, `crates/app-infra/src/processing/apple_vision.rs` | Apple Vision OCR | Disable on non-Apple; default to Tesseract/PaddleOCR |
| `crates/audio-transcription/src/providers/apple_speech.rs` | Apple Speech | Disable on non-Apple; default to local providers/cloud if introduced |
| `crates/app-infra/src/capture_index_key_store.rs` | macOS shared-access-group DP keychain (`security-framework`) with legacy `security` CLI item fallback + migration | Windows Credential Manager/DPAPI; Linux Secret Service/KWallet |
| `apps/desktop/src-tauri/src/broker_authorization_channel.rs`, `crates/cli/src/main.rs` | Unix socket app-mediated authorization | Windows named pipe/TCP; Linux runtime-dir Unix socket validation |
| `apps/desktop/src-tauri/src/windows.rs` | macOS rounded content views, Dock visibility, terminate interception, Quick Recall non-activating NSPanel (reclass, style mask, floating level, first-mouse, key/first-responder, order-out) | Windows/Linux window behavior equivalents or no-ops; Quick Recall falls back to a plain shown/focused always-on-top window |
| `.github/workflows/*`, `docs/release-process.md`, `scripts/stage-macos-release-artifacts.sh` | macOS-only release pipeline | Windows/Linux release pipelines and docs |

## Cross-platform implementation principles

- Keep Tauri command handlers thin; native capture orchestration belongs behind the Recording Lifecycle seam in `apps/desktop/src-tauri/src/native_capture/lifecycle.rs` and platform capture crates.
- Keep shared serde/domain types in `crates/capture-types`.
- Keep capture primitives in `crates/capture-screen`, `crates/capture-microphone`, and `crates/capture-writers`; avoid putting OS APIs directly in Svelte or high-level Tauri commands.
- Preserve Capture Session / Capture Segment semantics across platforms, including date-organized output, source-session IDs, retention, OCR job enqueueing, and timeline events.
- Do not silently downgrade privacy. If an OS cannot support live app exclusion, expose an explicit unsupported/degraded capability and rely on App Privacy Exclusion disclosure, Pause Recording, and Delete Recent Capture recovery.
- Prefer capability-driven UI (`get_capture_support`, provider runtime availability, permission state) over direct platform checks in Svelte.
