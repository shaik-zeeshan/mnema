# Desktop Native Runtime Context

Tauri startup, native recording runtime, live privacy application, local desktop cache/runtime services, and native authorization-channel server behavior.

Root entry point: [CONTEXT-MAP.md](../../../CONTEXT-MAP.md).

## Language

**Scrub Preview Generation**:
Background work that materializes generated **Scrub Preview** cache artifacts for finalized screen segment intervals.
_Avoid_: scrub-time extraction, exact frame preview generation, thumbnail pipeline

**Recording Lifecycle**:
The in-memory control flow for one coordinated recording runtime that starts capture, owns pause/resume decisions, rotates segments, recovers after wake, and stops capture across the requested sources. On macOS, screen and system audio share the screen capture backend while microphone runs as a separate native session; on Windows, microphone and system audio are each independent native audio sessions decoupled from screen capture.
_Avoid_: capture runtime, recorder service, session manager

**Live Privacy Filter**:
The native screen-capture filtering mechanism that applies **App Privacy Exclusion** before frames are delivered to Mnema.
_Avoid_: privacy promise, metadata redaction, post-capture filtering

**Browser Metadata Collection**:
Native browser URL metadata, governed by metadata settings, for timeline and search context without making live capture privacy decisions.
_Avoid_: metacollection, browser privacy signal, website privacy rule

**Audio Activity Sample**:
A raw audio probe reading such as latest normalized level or last-sample timestamp, exposed for debug visibility but not itself used as the inactivity decision.
_Avoid_: audio activity event, microphone activity, system audio activity

**Audio Activity Decision**:
The threshold-qualified inactivity-policy view of audio activity, including enabled state, threshold, and derived idle used for pause/resume decisions.
_Avoid_: raw audio sample, activity reading, latest level

**App Update Service**:
The Rust-owned Tauri boundary that checks, downloads, installs, and restarts for **App Update** while enforcing Mnema runtime policy.
_Avoid_: Svelte updater flow, generic updater plugin surface, frontend install policy

**App Update Settings**:
Rust-owned app configuration that stores the user's selected **App Update** channel.
_Avoid_: recording settings, browser local storage, one-time prompt state

## Relationships

- **App Privacy Exclusion** remains handled through the native **Live Privacy Filter**, not through app-based automatic pause.
- **Browser Metadata Collection** uses native browser URL probing only in this branch; another metadata adapter requires a future ADR-backed design.
- A generated **Scrub Preview** interval is one second and is represented by the first indexed screen position inside that one-second video-offset bucket.
- The v1 generated **Scrub Preview** rendition is JPEG quality 72 with a 360 px maximum dimension at one preview per second.
- A generated **Scrub Preview** is an app-owned cache artifact under the app cache directory, not a durable artifact under the **Managed Storage Layout** recordings tree.
- A generated **Scrub Preview** cache identity is tied to the source screen segment, preview interval, rendition settings, and source video/frame-index freshness.
- Generated **Scrub Preview** source freshness uses canonical source path identity plus source video/frame-index size and modified time, not full media content hashing.
- Generated **Scrub Preview** files live under a dedicated app-cache scrub preview root, grouped by rendition and source segment cache directory.
- The dedicated generated **Scrub Preview** cache root is app-owned and may be allowed recursively through Tauri asset scope.
- A generated **Scrub Preview** segment cache directory requires valid metadata matching source freshness before its preview files can be returned.
- Generated **Scrub Preview** cache access is tracked at segment-directory granularity with throttled last-access updates for pruning.
- **Scrub Preview** availability returns only source-fresh cache files; missing or stale indexed intervals may be enqueued for background regeneration.
- A timeline interval with a usable frame index but no indexed screen position is unavailable for **Scrub Preview** without treating the whole frame index as missing.
- The generated **Scrub Preview** cache defaults to a 512 MB budget and 7-day last-access window, pruned by segment cache directory rather than individual preview file.
- Generated **Scrub Preview** cache policy is separate from exact frame preview cache policy.
- Existing exact preview cache TTL settings do not control generated **Scrub Preview** disk cache lifetime.
- **Scrub Preview Generation** runs outside the active scrub interaction path; timeline navigation may request availability, but missing generated **Scrub Preview** values are materialized in background work.
- **Scrub Preview Generation** uses a single coalescing worker where the newest visible timeline window takes priority over stale queued preview intervals.
- **Scrub Preview Generation** queue state is non-durable and rebuilds from finalized-segment events or dashboard availability demand.
- **Scrub Preview Generation** stays outside app-infra processing job lanes and frame/OCR persistence transactions.
- Startup validates/prunes generated **Scrub Preview** cache but does not warm missing previews for existing segments.
- **Scrub Preview Generation** processes interval work in bounded chunks so visible-window demand can preempt full-segment warming.
- A finalized screen **Capture Segment** enqueues full one-second-interval **Scrub Preview Generation**, bounded by the 5-minute **Capture Segment Duration** cap.
- Automatic **Scrub Preview Generation** is triggered after the screen **Capture Segment** is committed, outside capture primitive code.
- App-infra owns **Capture Segment** discovery for **Scrub Preview** availability, while the desktop Tauri layer owns generated **Scrub Preview** cache files, native extraction, asset scope, queueing, and cache-change events.
- Completed **Scrub Preview Generation** chunks notify the dashboard with coalesced cache-change events so visible windows can refresh availability without polling continuously; those events invalidate ranges rather than carrying preview file paths.
- **Scrub Preview** generation failures are non-durable availability states with short-lived retry backoff, not persisted app-infra records.
- **Scrub Preview** availability may report queued status from non-durable in-memory generation queue or in-flight work.
- **Scrub Preview** availability is derived from screen **Capture Segment** rows and their frame indexes; disposable preview cache entries are not modeled as durable app-infra rows.
- An existing **Screen Frame Artifact** may satisfy a **Scrub Preview** for its segment time position without generating a separate preview.
- A generated **Scrub Preview** depends on a screen segment frame index; frames without indexed segment timing fall back to the exact preview path instead of guessed scrub output.
- Generated **Scrub Preview** values apply only to finalized screen **Capture Segment** values; live or incomplete segments rely on existing **Screen Frame Artifact** paths or return no **Scrub Preview**.
- **Scrub Preview Generation** eligibility requires a finalized screen **Capture Segment** with an openable screen recording and usable frame index.
- Historical finalized screen **Capture Segment** values are eligible for demand-driven **Scrub Preview Generation** when they have a usable binary or legacy frame index.
- Automatic full-segment **Scrub Preview Generation** runs only after a screen **Capture Segment** finalizes, not while that segment is actively being captured.
- Automatic **Scrub Preview Generation** is opportunistic: it may defer under shutdown, source invalidity, cache pressure, or higher-priority visible-window demand, and it must not block segment finalization.
- **Scrub Preview Generation** prefers an existing matching **Screen Frame Artifact** when available, then falls back to the finalized screen segment recording plus frame index.
- **Hidden Segment Workspace** cleanup does not wait on **Scrub Preview Generation**; existing frame artifacts are used opportunistically but the finalized segment recording remains the regeneration source.
- A **Recording Lifecycle** coordinates screen, microphone, and system-audio capture within one recording runtime.
- A **Recording Lifecycle** applies **App Privacy Exclusion** through the **Live Privacy Filter** when screen capture is requested.
- Whether system audio requires screen capture is a platform capability, not a fixed rule: macOS couples system audio to the screen backend, while Windows treats system audio as an independent source. See [ADR 0022](../../../docs/adr/0022-system-audio-is-an-independent-source-on-windows.md).
- On Windows, screen loss from system suspend, session lock, or monitor/display change is a transient liveness condition the **Recording Lifecycle** recovers from by reusing the inactivity pause/resume mechanism with a pause-reason discriminator, not by ending the session. See [ADR 0023](../../../docs/adr/0023-windows-transient-capture-recovery-reuses-inactivity-pause.md).
- Metadata-derived website, title, private-browser, and per-window decisions must not feed the **Live Privacy Filter**.
- A **Recording Lifecycle** may pause or resume requested sources based on inactivity policy.
- A **Recording Lifecycle** commits requested audio sources as **Audio Segment** values.
- A **Recording Lifecycle** creates one **Capture Session** for a user recording and **Capture Segment** rows only for produced artifacts.
- **App Update** installation is gated outside the **Recording Lifecycle** and waits for the active **Capture Session** to end rather than stopping or pausing capture itself.
- The **App Update Service** owns update policy and exposes app-specific commands/events to Svelte rather than exposing generic updater plugin behavior as product logic.
- The **App Update Service** selects the update feed endpoint at runtime from the user's selected update channel.
- **App Update Settings** persist under Tauri `app_config_dir()` and default to the stable channel.
- The **App Update Service** may run a startup availability check only after onboarding is complete.
- **App Update** restart uses Tauri relaunch only after Mnema graceful shutdown work has completed.
- **Retention Cleanup** best-effort removes generated **Scrub Preview** cache directories for deleted screen **Capture Segment** values, while cache validation and pruning remain responsible for stale orphan safety.
- **Retention Cleanup** reaches generated **Scrub Preview** cache through the desktop Tauri cache service rather than app-infra owning Tauri app-cache paths.
- A microphone **Audio Segment** becomes eligible for an **Audio Transcription Job** when the **Recording Lifecycle** commits it, even if the eventual transcript is empty.
- An **Audio Activity Sample** can inform an **Audio Activity Decision**, but the two are not interchangeable.
- An **Audio Activity Decision** is what the inactivity policy uses to pause or resume capture.
- The desktop app starts and maintains the **Broker Authorization Channel** server during app runtime, with request-level rejection for states such as incomplete onboarding.
- Desktop Tauri **Access Settings** commands and app-facing access types belong in a dedicated CLI access module separate from the socket protocol module.
- Desktop window ownership for **CLI Access Request** stays in the shared window helper module, while authorization protocol decisions stay outside the window helper.
- Legacy broker authorization request files should be translated into the new **CLI Access Request** flow rather than opening the old Privacy Agent Access section.
- If a live app-exclusion change cannot be applied while recording, Mnema reports that screen/system-audio capture is suspended because privacy exclusions could not be applied, reusing the existing privacy suspension path.
- Historical **Capture Segment** values encountered through dashboard demand enqueue only visible-window intervals, not full-segment warming.
- Metadata collection kept after removing metadata privacy rules must serve non-privacy product features such as timeline context, app/window labels, or debug surfaces.
- Current-run **OCR Throughput Budget** state belongs to the desktop runtime, not app-infra durable storage.
- **OCR Admission Budget** behavior should be tested through the desktop runtime memory interface rather than app-infra database queries.
- **OCR Execution Budget** pacing memory may reset on app startup; debug timing summaries are current-run state, while durable OCR results remain normal app data.
- OCR debug commands should expose current-run **OCR Throughput Budget** state rather than durable lookup by old frame or job ids.
- **Broker Authorization Channel** startup may unlink a stale socket path after confirming it is not serving a live endpoint, creates the socket parent with user-only permissions where practical, and removes the socket on clean shutdown.
- The desktop app owns the **Broker Authorization Channel** server because authorization decisions require app UI, while CLI code owns the client side and app-infra owns broker policy and grant persistence.
- The **Broker Authorization Channel** remains available while the desktop app is running hidden or menu-bar-only.
- Desktop Tauri **Broker Authorization Channel** server lifecycle and pending-request state belong in a dedicated broker authorization channel module rather than `lib.rs`.
- Tauri `lib.rs` should stay limited to startup wiring, reopen fallback hooks, and command registration for CLI access work.
- macOS app reopen handling should honor pending legacy authorization fallback requests instead of only opening the main dashboard.
- Existing settings-focus based authorization handoff code is provisional and should not define the finalized **Broker Authorization Channel** design.

## Example Dialogue

> **Dev:** "Is `microphoneActivityLastUnixMs` the same thing as the audio signal the inactivity policy uses?"
> **Domain expert:** "No — that timestamp is an **Audio Activity Sample**; the inactivity pause logic uses an **Audio Activity Decision** derived from threshold-qualified activity."

## Flagged Ambiguities

- "audio activity" previously referred to both raw probe output and inactivity-policy state; resolved: raw probe output is an **Audio Activity Sample**, while policy-facing threshold-qualified state is an **Audio Activity Decision**.
