# Desktop App Context

Svelte UI surfaces, user-facing settings language, prompts, status-bar-facing actions, dashboard behavior, and desktop UX policy.

Root entry point: [CONTEXT-MAP.md](../../CONTEXT-MAP.md).

## Language

**App Privacy Exclusion**:
The user-facing privacy policy that prevents live screen capture of selected apps by app identity. **App Privacy Exclusion** is the only live privacy exclusion guarantee and the only privacy exclusion control exposed in settings. Mnema records visible screen content from apps that are not excluded, including private or incognito browser windows.
_Avoid_: website exclusion, title exclusion, private browser exclusion, per-window privacy, metadata privacy rule

**Sensitive Capture Protection V1**:
The product scope for helping users avoid accidental sensitive capture without expanding Mnema's live screen-capture privacy guarantee beyond **App Privacy Exclusion**.
_Avoid_: password-page blocking, website privacy filter, private-window protection

**Recommended App Exclusions**:
A user-confirmed recommendation surface for adding high-confidence sensitive apps to **App Privacy Exclusion**.
_Avoid_: silent privacy defaults, sensitive-content detection, automatic browser blocking

**One-Time Prompt State**:
App-owned UX state that records whether dismissible one-time prompts have already been shown or dismissed.
_Avoid_: recording setting, browser local storage, per-component flag

**One-Time Prompt**:
A dismissible app prompt identified by a stable prompt id and tracked with shown, dismissed, and completed timestamps.
_Avoid_: recurring alert, local component state, boolean-only banner flag

**Sensitive App Recommendation Catalog**:
An auditable exact-bundle-id list used to propose **Recommended App Exclusions**.
_Avoid_: fuzzy sensitive-app classifier, name keyword matcher, category inference

**Browser Capture Disclosure**:
An explicit notice that browser screen content is recorded unless the browser app is added to **App Privacy Exclusion**.
_Avoid_: browser privacy mode, incognito protection, website blocking

**Known Browser App**:
A browser app identity used for browser-related product disclosure and metadata support.
_Avoid_: sensitive app recommendation, browser privacy rule, website filter

**Automatic Browser Suspension Rule**:
Mnema does not ship automatic credential-entry or browser add-on capture suspension in this branch; privacy controls stay explicit.
_Avoid_: silent pause, password-page detector, browser add-on recorder

**Exclude This App**:
A just-in-time user action that adds one app identity to **App Privacy Exclusion** for future capture.
_Avoid_: retroactive exclusion, delete this app's history, sensitive content removal

**Exclude Current App**:
A status-bar shortcut that confirms and then adds the current frontmost app to **App Privacy Exclusion** for future capture.
_Avoid_: app picker replacement, retroactive cleanup, timeline privacy action

**Delete Recent Capture**:
A user-triggered recovery action that deletes capture data in a recent time window across screen, microphone, and system-audio sources.
_Avoid_: OCR-only cleanup, search-only cleanup, hide from results

**Pause Capture**:
A user control that temporarily stops recording requested sources without changing **App Privacy Exclusion**.
_Avoid_: private mode, sensitive mode, privacy filter

**User Capture Pause**:
A user-initiated paused recording state that persists until the user resumes capture.
_Avoid_: inactivity pause, stopped recording, private mode

**Scrub Preview**:
A disposable, display-sized preview image for a screen segment time position, used while navigating the dashboard timeline.
_Avoid_: exact frame, OCR source, screenshot, thumbnail

**Access Settings**:
The top-level settings surface for local tool access controls such as **CLI Access**.
_Avoid_: Privacy settings, Developer settings, Agent Access tab

**About Settings**:
The settings surface for app identity, version details, release channel information, and manual **App Update** checks.
_Avoid_: status-bar updater, automatic update prompt, release dashboard

**Settings Ownership Domain**:
A coherent settings area whose values can be loaded, validated, saved, and broadcast without overwriting unrelated user preferences.
_Avoid_: UI tab, settings category, full settings payload

**CLI Access Request**:
A request-bound app surface that lets the user approve or deny a pending **CLI Access Grant** request.
_Avoid_: settings page, generic prompt, login screen

**Secure Field Capture Suspension**:
A future ADR-backed product concept that would suspend capture while secure text entry is focused, rather than filtering by app, window, website, or recognized text.
_Avoid_: password-page filter, secure-field redaction, browser login exclusion

**App Update**:
A user-visible Mnema version replacement delivered through a selected release channel.
_Avoid_: draft release update, silent update, internal-only artifact

**Prerelease Build**:
An internal/test Mnema build reviewed and installed manually before it is eligible for the stable update channel.
_Avoid_: stable update, production update

**Stable Update**:
An **App Update** delivered from published non-prerelease GitHub Releases.
_Avoid_: preview update, draft release update, notarized update

**Preview Update**:
An opt-in **App Update** delivered from preview release artifacts that may be less stable and may be ad hoc signed or not notarized.
_Avoid_: stable update, forced beta, hidden prerelease

**Startup Update Check**:
A background **App Update** availability check that runs when Mnema starts without downloading, installing, or restarting by itself.
_Avoid_: automatic install, forced update, startup restart

## Relationships

- **Sensitive Capture Protection V1** remains inside **App Privacy Exclusion** and does not promise website-level, private-window, password-page, or secure-field protection.
- **Sensitive Capture Protection V1** is UX and recovery around **App Privacy Exclusion**, not detection of sensitive screen content.
- **CLI Access Grant** creation requires completed Mnema onboarding.
- Mnema onboarding takes precedence over **CLI Access Request** handling; an incomplete app should finish onboarding before presenting CLI authorization.
- Opening More Options keeps the same **Broker Authorization Channel** request alive but does not reset the CLI wait indefinitely.
- The default native **Broker Authorization Channel** prompt should offer Allow, Cancel, and More Options when platform dialog support permits; More Options keeps the request alive and opens the **CLI Access Request** window.
- The default native **Broker Authorization Channel** prompt should use user-facing copy equivalent to `Allow CLI Access?` and explain that the named client wants access to searchable Mnema text from the last day for 24 hours.
- The default native **Broker Authorization Channel** prompt actions should be Allow, More Options, and Cancel when platform dialog support permits.
- If the native prompt cannot support a clear More Options action, Mnema should open the **CLI Access Request** window directly rather than shipping a dead-end two-button prompt.
- A **CLI Access Request** is separate from **Access Settings** and exists to resolve one pending authorization request rather than manage standing access.
- The **CLI Access Request** window is a compact Rust-owned Svelte/Tauri desktop surface rather than an ad hoc frontend-created window.
- The **CLI Access Request** dedicated window uses label `cli-access-request` and route `/access/request`.
- Closing the **CLI Access Request** window cancels the pending authorization request and creates no grant.
- A **CLI Access Request** may let the user choose among V1 scope and duration options, but should prevent choices that cannot satisfy the pending broker command.
- **CLI Access Request** approval uses deliberate visible controls and choice-specific button text rather than type-to-confirm in V1.
- A **CLI Access Request** window should show client identity, identity provenance, the local identity-not-verified warning, command type, scope choices, duration choices, and Cancel plus choice-specific Allow controls.
- **Access Settings** does not rename **Broker Client Identity** values in V1 because identity is part of grant matching.
- **Access Settings** supports revoking individual **Brokered Capture Access** grants and revoking all grants for a **Broker Client Identity**; revocation affects future commands immediately but does not cancel already-running V1 commands.
- **Access Settings** groups **CLI Access Grant** values and recent non-content access history by **Broker Client Identity**.
- **Access Settings** treats expired and revoked **CLI Access Grant** values as history rather than active access.
- **Access Settings** is a top-level settings surface distinct from Privacy and Developer settings.
- **About Settings** is a top-level settings surface with settings tab id `about`.
- A **Settings Ownership Domain** is defined by persistence and validation ownership rather than by the visual settings tab where its controls appear.
- Autosave should save one **Settings Ownership Domain** at a time so an older settings draft cannot overwrite unrelated preferences from another domain.
- Autosave may save different **Settings Ownership Domain** values concurrently, but saves within one domain should be serialized.
- Settings change notifications should preserve full canonical settings for compatibility while also identifying the changed **Settings Ownership Domain** for draft-safe frontend synchronization.
- Settings surfaces should resync draft state for the changed **Settings Ownership Domain** without resetting unrelated in-progress drafts from the full canonical settings payload.
- New domain-specific settings mutation APIs and events should return the changed **Settings Ownership Domain** together with the full canonical settings.
- Initial **Settings Ownership Domain** values are Capture Source Settings, Capture Timing Settings, Video Settings, Storage Settings, Display Settings, Metadata Settings, App Privacy Exclusion, Inactivity Settings, Processing Settings, and Developer Settings.
- Stable **Settings Ownership Domain** ids are `capture_sources`, `capture_timing`, `video`, `storage`, `display`, `metadata`, `app_privacy_exclusion`, `inactivity`, `processing`, `developer`, `keyboard_bindings`, `microphone_controller`, `app_update`, `access`, and `one_time_prompt_state`.
- Keyboard Bindings, Microphone Controller Preferences, App Update Settings, Access Settings, and **One-Time Prompt State** remain separate existing **Settings Ownership Domain** values rather than being folded into recording settings.
- **App Privacy Exclusion** stays a dedicated **Settings Ownership Domain** with purpose-built mutation commands instead of full-payload autosave.
- Normal settings autosave and visible controls should use **Settings Ownership Domain** mutation APIs rather than a full recording-settings payload.
- A full recording-settings update path may remain only as a compatibility, migration, import, or debug backstop.
- Domain-specific recording settings mutations should initially merge into the canonical `recording-settings.json` store rather than splitting that file by domain.
- Domain-specific recording settings mutations should validate the proposed domain change against the current canonical settings so cross-domain invariants are preserved.
- A domain mutation that violates another **Settings Ownership Domain** should fail clearly unless there is one obvious normalization that preserves user intent.
- **Access Settings** layout should include CLI install/status, active CLI access grouped by client, recent non-content access history, and inactive or revoked history.
- **Access Settings** should not expose a manual create-grant button in V1 because grant creation is request-bound through **Broker Authorization Channel** approval.
- New frontend/Tauri APIs for **Access Settings** should use access-language names even when app-infra keeps broker-language internals.
- New frontend/Tauri APIs for **Access Settings** should use names such as `get_cli_access_status`, `install_cli`, `reinstall_cli`, `list_cli_access_grants`, `revoke_cli_access_grant`, `revoke_cli_access_for_client`, and `list_cli_access_history`.
- The old Privacy **Agent Access** section should be removed when **Access Settings** ships so CLI grant management is not duplicated or mislabeled.
- User-facing CLI access copy should avoid implementation terms such as broker and should describe access as searchable Mnema text, including screen text and audio transcripts, plus timeline results, with original media and raw database rows not shared by V1 commands.
- **Recommended App Exclusions** become **App Privacy Exclusion** rules only after user confirmation.
- **Recommended App Exclusions** are shown during onboarding and through a one-time non-blocking prompt for existing users after upgrade when at least one detected recommended app is missing from **App Privacy Exclusion** or has its exclusion disabled.
- **Recommended App Exclusions** prompt dismissal is persisted in **One-Time Prompt State** rather than recording settings or browser local storage.
- The existing-user **Recommended App Exclusions** prompt is one-time for V1 and does not reappear just because a new catalog app is installed later.
- Privacy settings continue to show actionable **Recommended App Exclusions** after the one-time prompt is dismissed.
- Changes to the **Sensitive App Recommendation Catalog** do not retrigger the V1 existing-user **One-Time Prompt** after it has been dismissed or completed.
- **Recommended App Exclusions** may include password managers, authenticator apps, Keychain/Passwords, and high-confidence app-based banking matches, but browser apps are called out separately rather than silently preselected.
- **Recommended App Exclusions** may include installed or running apps when they exactly match the **Sensitive App Recommendation Catalog**.
- User-facing copy for **Recommended App Exclusions** should name concrete categories such as password managers and authenticator apps rather than relying on vague "sensitive app" language.
- **Recommended App Exclusions** should not include broad workflow apps such as System Settings, Terminal, developer tools, messaging, or email by default.
- App-based banking entries belong in **Recommended App Exclusions** only when they are exact high-confidence native app matches that Mnema is willing to maintain.
- Future dismissible one-time dialogs should reuse **One-Time Prompt State** instead of adding prompt-specific persistence files.
- **One-Time Prompt State** stores stable **One-Time Prompt** ids with shown, dismissed, and completed timestamps.
- **One-Time Prompt** ids are stable and versioned, such as a V1 suffix for a V1 prompt.
- The **Sensitive App Recommendation Catalog** uses exact bundle identifiers rather than fuzzy app-name, category, website, title, or content matching.
- The **Sensitive App Recommendation Catalog** and recommendation matching are Rust-owned; frontend surfaces render app-owned recommendation results.
- Entries in the **Sensitive App Recommendation Catalog** include a finite curated category or reason such as password manager, authenticator, Apple Passwords, or banking.
- **Known Browser App** values are kept separate from the **Sensitive App Recommendation Catalog**.
- **Browser Capture Disclosure** may offer one-click browser app exclusion, but it does not imply browser-domain, private-window, or password-page protection.
- **Browser Capture Disclosure** is persistent onboarding/settings copy; a **One-Time Prompt** may point existing users to it when screen capture is enabled and a known browser is not excluded.
- **Browser Capture Disclosure** is based on known browser app identity, not URL, domain, title, private-window state, or login-page signals.
- **Browser Capture Disclosure** explicitly says private or incognito browser windows are recorded unless the browser app is excluded.
- **Browser Capture Disclosure** explicitly says Mnema does not detect browser password pages or password fields.
- **Exclude This App** applies from the time the app exclusion is added and does not remove already persisted **Captured Frame** or **Audio Transcription** data.
- **Exclude Current App** is a native status-bar shortcut for the frontmost app, while Privacy settings remains the full app-picker surface.
- **Exclude Current App** is available while recording and while stopped; while recording it affects future frames in the current recording, and while stopped it affects future recordings.
- **Exclude Current App** is disabled when the current app is Mnema itself or another target that cannot be meaningfully excluded.
- **Exclude Current App** targets the frontmost app captured when the action is invoked and keeps that target through confirmation rather than recomputing after the confirmation dialog opens.
- **Exclude Current App** reports an app as already excluded when it has an enabled **App Privacy Exclusion** instead of mutating settings again.
- **Exclude Current App** re-enables an existing disabled **App Privacy Exclusion** for the target app instead of adding a duplicate rule.
- **Exclude Current App** may offer **Delete Recent Capture** as an explicit second confirmed step, but it must not automatically delete prior capture.
- **Exclude Current App** does not offer historical per-app cleanup in V1.
- **Delete Recent Capture** removes the selected recent capture window's **Capture Segment** data, **Captured Frame** data, OCR/search data, **Audio Segment** data, transcription data, speaker-derived data, and derived preview cache where applicable.
- When invoked during recording, **Delete Recent Capture** first creates a recording boundary so active writer-owned data becomes finalized **Capture Segment** data before deletion.
- **Delete Recent Capture** deletes finalized **Capture Segment** values whose time ranges overlap the selected recent window; bounded over-delete is acceptable because **Capture Segment Duration** is capped.
- **Delete Recent Capture** deletes whole overlapping screen **Capture Segment** media rather than trimming video files or rewriting frame indexes.
- **Delete Recent Capture** deletes whole overlapping **Audio Segment** values rather than trimming media or retiming transcripts.
- **Delete Recent Capture** exposes fixed fast-recovery windows, with the last one minute as the primary/default action and longer windows such as five or fifteen minutes as secondary choices.
- **Delete Recent Capture** computes the selected recent window from app wall-clock time rather than stretching backward to the latest retained capture.
- **Delete Recent Capture** always requires explicit confirmation and describes that overlapping **Capture Segment** values may be removed.
- **Delete Recent Capture** does not need additional dynamic warnings based on **Capture Segment Duration** beyond explaining overlap deletion.
- **Delete Recent Capture** does not run a preview/count step before confirmation in the V1 fast recovery flow.
- If **Delete Recent Capture** cannot create the needed recording boundary while recording, it fails clearly instead of silently performing a partial older-segment deletion.
- **Delete Recent Capture** does not stop recording by itself; after the deletion boundary, recording continues with the same requested sources unless the user separately stops or pauses capture.
- **Delete Recent Capture** may run during **User Capture Pause** and leaves the **Capture Session** paused afterward.
- **Delete Recent Capture** feedback reports deletion counts and tombstone status without displaying content-bearing filenames, app/window titles, OCR text, or transcripts.
- **Delete Recent Capture** is separate from **Retention Cleanup** even if it reuses app-infra deletion helpers.
- **Delete Recent Capture** should cancel, retire, or otherwise make affected running processing work non-runnable rather than silently skipping matching retained data.
- If **Delete Recent Capture** removes app-infra rows but file deletion fails, the content is treated as removed from Mnema's app library and the remaining file work is tracked as tombstone status.
- **Delete Recent Capture** should best-effort clear generated and exact preview caches for affected retained data.
- **Delete Recent Capture** removes data from Mnema's app library and does not promise secure erase from storage media, snapshots, or backups.
- **Delete Recent Capture** does not create a content-bearing deletion history in V1.
- **Delete Recent Capture** is available from the status-bar recovery flow first and may also appear near dashboard recording controls.
- **Pause Capture** creates a **User Capture Pause** for all requested sources; V1 avoids "private mode" naming because no sensitive-content detection is promised.
- **User Capture Pause** is distinct from automatic inactivity pause and must not resume because activity is detected.
- **User Capture Pause** keeps the **Capture Session** alive, finalizes the active **Capture Segment**, records nothing during the pause, and starts new **Capture Segment** values when the user resumes.
- **Pause Capture** may offer **Delete Recent Capture** as an explicit separately confirmed recovery action after pausing.
- User-facing labels for **Pause Capture** should use "Pause Recording" and "Resume Recording" to match existing recording controls.
- User-facing controls should expose **Pause Capture** in addition to stop recording, because pause preserves the **Capture Session** while stop ends it.
- **Pause Capture** does not stop processing work for already retained data; deletion of queued or completed processing work belongs to **Delete Recent Capture**.
- **Pause Capture** and **Delete Recent Capture** are available even for audio-only recording, while app-exclusion actions clearly apply to screen capture privacy.
- **Pause Capture** may be exposed through global shortcut preferences without a default shortcut, while **Delete Recent Capture** has no default destructive shortcut in V1.
- **User Capture Pause** is exposed through capture session state/events so native status-bar and frontend surfaces stay synchronized.
- User-facing **App Privacy Exclusion** copy should refer to screen content rather than all capture sources.
- **Browser Capture Disclosure** may mention browser URL metadata in Privacy settings, but not in fast status-bar recovery flows.
- Raw SQLite or frame-file access by external agents is outside the **Sensitive Capture Protection V1** privacy guarantee until Mnema introduces an explicit brokered access boundary.
- **Secure Field Capture Suspension** is separate from the **Live Privacy Filter** and requires its own ADR before becoming a product guarantee.
- **Stable Update** checks do not target draft releases or **Prerelease Build** values.
- A **Prerelease Build** can become eligible for **Stable Update** only after it is published as a stable release.
- V1 **Stable Update** uses the stable GitHub Release `latest.json` asset as its update feed.
- V1 **Preview Update** is opt-in and must disclose that preview builds may be less stable and may show macOS security warnings until Developer ID signing and notarization are available.
- Normal **Preview Update** UI explains practical risk and possible macOS security warnings rather than exposing signing implementation details.
- V1 **Preview Update** uses a separate static preview update feed rather than GitHub Releases `latest`.
- V1 **Preview Update** feed is published through GitHub Pages.
- V1 **Preview Update** feed URL is `https://shaik-zeeshan.github.io/mnema/updates/preview/latest.json`.
- **Stable Update** continues to use GitHub Releases `latest`; only **Preview Update** uses a custom static feed.
- **App Update** feeds and artifacts are public HTTPS resources; Mnema does not store GitHub credentials for update checks.
- V1 **App Update** adds no app-owned analytics or telemetry beyond the necessary HTTPS feed/artifact requests.
- V1 **App Update** channels follow [ADR 0018](../../docs/adr/0018-support-opt-in-preview-update-channel.md).
- V1 **App Update** release artifacts and feed generation follow [ADR 0017](../../docs/adr/0017-use-tauri-action-for-app-update-release-artifacts.md).
- V1 **App Update** supports macOS Apple Silicon builds only.
- V1 **Stable Update** and **Preview Update** artifacts are signed with the same Tauri updater keypair.
- **Preview Update** publishing uses the same protected release environment and updater signing secrets as **Stable Update** publishing.
- V1 **App Update** does not support automated downgrades when switching from preview back to stable.
- **Preview Update** builds use SemVer prerelease versions, such as `0.3.0-preview.1`; **Stable Update** builds use plain SemVer versions.
- Release tags derive **App Update** channel: `vX.Y.Z` targets **Stable Update**, while `vX.Y.Z-preview.N` targets **Preview Update**.
- Published GitHub prereleases may be referenced by **Preview Update**, while draft releases are not feed-visible for any **App Update** channel.
- **App Update** release builds are smoke-tested as draft releases before a separate publish step makes them feed-visible.
- **Preview Update** rollback repoints the preview feed only for users below the bad version; users already on a bad preview receive a newer preview hotfix rather than an automated downgrade.
- Mnema may check for an **App Update** while a **Capture Session** is active, but it must not install an **App Update** until the **Capture Session** has ended.
- **User Capture Pause** still counts as an active **Capture Session** for **App Update** installation gating.
- **App Update** installation does not automatically stop recording or convert an active **Capture Session** into **User Capture Pause**.
- **About Settings** disables **App Update** installation while a **Capture Session** is active, and Rust still rejects installation if capture starts before the install command runs.
- V1 **App Update** controls live in **About Settings**, not in the status bar.
- V1 **About Settings** exposes manual **App Update** checks and app version details.
- V1 **About Settings** shows product name, current version, selected update channel, platform/architecture, and optionally the bundle identifier.
- V1 **App Update** installation does not restart Mnema automatically; the user chooses when to restart after installation.
- V1 **About Settings** shows compact **App Update** status for checking, availability, download/install progress, restart-required, recording-blocked, and failed states.
- V1 **About Settings** exposes stable/default and preview/opt-in update channel selection.
- Switching into **Preview Update** requires explicit confirmation, but installing later preview updates does not require a separate preview-specific confirmation beyond the normal install action.
- **About Settings** confirms **Preview Update** opt-in inline rather than with a blocking OS dialog.
- Switching from **Preview Update** back to **Stable Update** does not require confirmation and may explain that Mnema will wait for a newer stable version rather than downgrade.
- Changing **App Update** channel triggers an immediate check against the newly selected channel after the setting is saved.
- V1 includes a **Startup Update Check** for the selected update channel.
- A **Startup Update Check** runs only after onboarding is complete.
- When a **Startup Update Check** finds an update, Mnema surfaces it non-blockingly in the visible app and does not force-open a window while hidden.
- **Startup Update Check** availability uses the existing app notification rail with an action to open **About Settings**.
- **Startup Update Check** failures do not create user notifications in V1; manual checks surface failures in **About Settings**.
- **About Settings** owns **App Update** download, install, progress, and restart-required UI in V1.
- V1 **App Update** download/install operations are not user-cancellable; failures return to a retryable **About Settings** state.
- V1 **App Update** availability is runtime-derived from update checks and is not persisted across app restarts.
- V1 **App Update** restart-required state is kept for the current app process and is not persisted separately across full app restarts.
- **About Settings** shows **App Update** target version, selected channel, and short release notes when available, but not signature internals or artifact filenames.
- **About Settings** presents **App Update** failures as user-friendly categories while detailed updater errors stay in logs or debug surfaces.
- If an update feed has a newer version but no compatible artifact for the current Mac, **About Settings** reports that no compatible update is available rather than claiming Mnema is up to date.
- **About Settings** displays **App Update** notes from the final publish-time release notes when the update feed provides them.
- V1 generated **Scrub Preview** interval, rendition, and cache budget are fixed product policy rather than user-facing recording settings.
- V1 may expose a developer/debug action to clear only generated **Scrub Preview** cache without clearing exact preview cache or adding regular user-facing cache controls.
- Shared recording/privacy settings should not expose inactive metadata privacy fields for website, title, private-browser, or per-window exclusion.
- **Capture Segment Duration** is capped at 5 minutes in persisted settings, runtime validation, and user-facing settings surfaces.
- Missing selected **Audio Transcription Model** status is surfaced when recording starts and in a dedicated Transcription settings surface, not once per committed microphone **Audio Segment**.
- **Text Search** should be enabled as part of completed OCR and transcription rather than as a separate user-facing setting.
- The first user-facing search surface should prioritize one plain search box, with lightweight result-type filtering at most.
- The first user-facing search result surface should be a large dashboard modal.
- A **Scrub Preview** represents a screen segment time position, not **Captured Frame** identity.
- Multiple nearby **Captured Frame** values may share one **Scrub Preview** when they fall within the same preview interval.
- A generated **Scrub Preview** cache interval is keyed by source segment video offset, while dashboard availability is requested and displayed by timeline time.
- Dashboard timeline mapping for generated **Scrub Preview** intervals uses **Capture Segment** timing plus segment video offset rather than per-frame captured timestamp jitter.
- V1 developer/debug surfaces may expose generated **Scrub Preview** cache status and queue status for verification.
- Dashboard navigation requests **Scrub Preview** availability by timeline window rather than **Captured Frame** identity.
- Dashboard **Scrub Preview** availability requests may enqueue missing visible-window intervals for background generation but must not synchronously extract preview images.
- Dashboard **Scrub Preview** availability responses include ready and unavailable/queued interval statuses, while display uses only ready intervals.
- Dashboard initial load requests **Scrub Preview** availability for the initial visible timeline window once timeline data and viewport dimensions are available.
- Dashboard **Scrub Preview** availability requests cover the visible timeline window plus small overscan, not the entire loaded timeline history.
- Dashboard scroll debounce applies only to backend **Scrub Preview** availability/enqueue requests; already-known ready **Scrub Preview** cache entries should display immediately during timeline movement.
- Dashboard timeline movement does not start exact preview requests as a fallback for missing **Scrub Preview** values; exact preview requests are settle/inspection behavior.
- Dashboard **Scrub Preview** state is interval-based, with active **Captured Frame** display derived from the matching interval instead of a frame-id preview cache.
- A **Scrub Preview** may be lower resolution or timing-tolerant and is never the source for OCR, copy, download, or **Captured Frame** truth.
- A **Scrub Preview** can stand in only while timeline navigation is in motion; a parked active **Captured Frame** resolves through the exact preview path.
- A **Scrub Preview** may remain visible as a placeholder while the exact preview for the parked active **Captured Frame** is loading.
- A **Scrub Preview** must not populate exact preview cache state or enable exact **Captured Frame** actions.
- When a requested **Scrub Preview** is absent during timeline movement, the dashboard may keep showing the previous available preview rather than blocking movement or showing a loading state.
- Selecting a **Search Result Anchor** should navigate the existing dashboard timeline or audio player to that captured point.
- Selecting an **Audio Transcription Span** should open the audio player at that span and align the dashboard timeline to the **Captured Frame** at the same recording time when such a frame exists.
- Search results should not require the dashboard timeline to hide non-matching capture data.
- A recommended app with an existing disabled **App Privacy Exclusion** is shown as currently off and can be re-enabled instead of added as a duplicate rule.
- **App Privacy Exclusion** does not remove or hide historical search, timeline, frame, or audio results that were already captured before the exclusion was added.
- **Downstream Capture Access** in app-owned surfaces operates only over retained app-infra data reachable through app-owned APIs.
- Dashboard previous-preview placeholders should be cleared when timeline movement jumps far enough that the displayed preview is no longer near the active interval.
- A dashboard `timeline_data_changed` retention event should prune loaded rows older than the cutoff and preserve the active retained item when possible.
- Selecting a search result from the modal should close the modal after navigation starts.
- Search input should be plain text by default rather than requiring users to learn advanced query syntax.
- The search result modal should not show captured text results for an empty query.
- Search should run live for plain-text queries after a small character threshold, and explicit submit should run immediately.
- The search result modal should initially focus on a small top set of results rather than overwhelming the user with the full match set.
- Search result limits should count visible result cards or **Search Result Group** values, not raw grouped anchors.
- The search result modal should use result-type tabs or segmented controls rather than side-by-side result columns.
- **Captured Frame** result cards should use image thumbnails.
- **Captured Frame** result-card thumbnails are navigation previews and should not replace exact frame inspection.
- A **Captured Frame** search result should remain visible when its result-card thumbnail is unavailable.
- **Search Snippet** matches should be visibly highlighted in search result cards when highlight data is available.
- **Search Snippet** highlight markup should be parsed into escaped text segments rather than rendered as trusted captured-content HTML.
- Processing provenance may support search debugging or invalidation, but normal result cards should not display provider/model details by default.
- **Search Refinement** controls should narrow the active search without requiring users to learn query syntax.
- **Search Entry Point** values should prefill or scope the normal search surface rather than creating separate search result surfaces.
- A **Search Entry Point** should appear as a visible removable **Search Refinement** when it scopes results.
- **Search Entry Point** actions should live near the dashboard context that defines their scope, while the search modal remains the single result surface.
- A **Search Entry Point** should be unavailable when its contextual scope cannot be derived.
- Active **Search Refinement** values should be visible as removable controls near the search input.
- Adding manual **Search Refinement** values should be secondary to the plain search input.
- Removing a **Search Refinement** should rerun the active search with the remaining query and refinements rather than closing or resetting the search modal.
- Opening global search should not retain **Search Refinement** values from a previous contextual **Search Entry Point**.
- **Search Refinement** values should persist across query changes within the same open search modal until the user removes them or opens global search anew.
- The first **Search Entry Point** values after global search should be visible timeline and current app.
- The first **Search Refinement** controls after result type should be date range, app, and source.
- **Visible Timeline Search** should derive a date-range **Search Refinement** from the timeline viewport time range rather than from the dashboard's loaded rows.
- **Visible Timeline Search** should freeze the timeline viewport time range when the entry point is invoked rather than tracking later timeline movement.
- **Visible Timeline Search** should be unavailable when the dashboard has no valid timeline viewport time range.
- Initial **Date Range Search Refinement** controls should use contextual or preset ranges rather than a custom date-time picker.
- Preset **Date Range Search Refinement** values should resolve to concrete start and end timestamps when selected rather than rolling while the search modal stays open.
- **Current App Search** from the dashboard should use the active **Captured Frame**'s retained app context rather than the current frontmost macOS app.
- **Current App Search** should be unavailable when the active **Captured Frame** has no retained app identity.
- Initial **App Search Refinement** should be added through **Current App Search** rather than through a full retained-app picker.
- **Current App Search** should default to **Captured Frame** results while **App Search Refinement** is frame-only.
- While **App Search Refinement** is frame-only, mixed or audio-only result views should not be selectable until the app refinement is removed.
- Result type selection should remain separate from **Audio Source Search Refinement**.
- Selecting an **Audio Source Search Refinement** should switch search to audio results rather than leaving frame results visible without that refinement.
- Initial manual **Search Refinement** controls should be limited to preset date ranges and **Audio Source Search Refinement**.
- Recent searches should be designed separately from the first **Search Refinement** and **Search Entry Point** UX pass.
- Saved searches and watch queries should be designed separately from the first **Search Refinement** and **Search Entry Point** UX pass.
- Search filters such as source, app, date range, or result type should be explicit UI controls when they are added.
- Low OCR-value admission skips should be visible in debug surfaces but not noisy in the normal timeline.
- **OCR Budget Telemetry** may be exposed through the debug surface as bounded current-run state so developers can see whether **OCR Job** values are executing.
- The debug surface should separate **OCR Admission Budget** events from **OCR Execution Budget** events so skipped candidates are not confused with jobs that ran.
- The debug surface should paginate recent **OCR Budget Telemetry** events rather than rendering the full bounded ring at once.
- The debug surface may poll current-run **OCR Budget Telemetry** while the OCR debug tab is active and visible.
- **Broker Authorization Channel** uses a native app prompt only for the default recent-history grant and a dedicated Mnema authorization surface for expanded scope or duration choices.
- The default native **Broker Authorization Channel** prompt is limited to last-day scope and 24-hour duration; any non-default scope or duration uses the dedicated authorization surface.
- **Broker Authorization Channel** prompts should not make Allow the default focused action when the platform permits a safer default.
- **Broker Authorization Channel** should surface the minimum authorization UI needed for the decision and should not open the main dashboard or settings as part of the normal approval path.
- **Broker Authorization Channel** approval should update grant management and audit state without requiring a second blocking main-app notification.
- CLI-triggered **All Retained Broker Scope** expansion may open the dedicated authorization surface directly from the **Broker Authorization Channel** flow rather than requiring manual navigation through settings.
- **Brokered Capture Access** prompts and management surfaces should show whether **Broker Client Identity** was explicit, environment-provided, inferred from non-sensitive markers, or defaulted.
- **Brokered Capture Access** prompts should disclose that locally declared or inferred **Broker Client Identity** is not cryptographically verified.
- **Broker Authorization Channel** prompts may show **Broker Client Identity**, command type, requested scope, and duration, but should not show raw query text, returned snippets, OCR text, transcripts, app/window titles, browser URLs, or media paths.
- Open search results should not automatically reshuffle as new OCR or transcription work completes.
- The search result modal should present **Captured Frame** results and **Audio Transcription Span** results as separate areas.
- The default search result modal should show up to five **Captured Frame** results and up to five **Audio Transcription Span** results.
- The search result modal should let the user request more **Captured Frame** results independently from more **Audio Transcription Span** results.
- Requesting more search results should preserve separate **Captured Frame** and **Audio Transcription Span** result ranking.
- **Visible Timeline Search** should include both **Captured Frame** and **Audio Transcription Span** results by default.
- **Audio Source Search Refinement** should apply only to **Audio Transcription Span** results.
- **Audio Transcription Span** result cards should emphasize recording source, time range, and transcript match rather than speaker labels.

## Flagged Ambiguities

- "**Scrub Preview**" was previously described as a visual representation of a **Captured Frame**; resolved: it is a disposable segment-time preview used during timeline navigation, while exact **Captured Frame** inspection goes through the exact preview path.
- "settings category" was used ambiguously for both visual tabs and save boundaries; resolved: save boundaries are **Settings Ownership Domain** values, while tabs are only presentation.
