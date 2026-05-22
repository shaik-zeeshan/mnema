# Frontend Integration Context

This context captures the domain language for the desktop capture app so architecture discussions use stable project terms.

## Language

**Screen Frame Artifact**:
A just-produced native capture output that has not yet been persisted into app-infra.
Screen frame artifacts are currently written as JPEG files.
_Avoid_: captured frame, transient screenshot, raw frame record

**Captured Frame Pipeline**:
A persisted flow for a captured screen frame from intake through batch attachment, OCR planning, and batch-finalization readiness.
_Avoid_: frame processing service, frame ingestion component, OCR pipeline

**Captured Frame**:
A screen image captured during a recording session and persisted as frame metadata plus a file path. A **Captured Frame** is the stored database-backed record, not the just-produced native artifact.
_Avoid_: screenshot, image record

**Frame Batch**:
A time-windowed group of captured frames for one recording session.
_Avoid_: bucket, chunk, frame group

**OCR Job**:
A processing job that recognizes text for one captured frame.
_Avoid_: processor task, recognition work item

**OCR Throughput Budget**:
The user-facing policy for limiting how much **OCR Job** work Mnema admits and schedules over time while preserving useful recognition quality.
_Avoid_: CPU cap, OCR governor, hard CPU limit

**OCR Quality Floor**:
The minimum acceptable recognition quality for automatic **OCR Job** work.
_Avoid_: fast fallback, cheap OCR mode, best-effort recognition

**OCR Settings Selection**:
The user-selected OCR provider, model, language, recognition mode, and provider-specific options used for automatic **OCR Job** admission.
_Avoid_: automatic provider choice, adaptive OCR mode, hidden fallback

**OCR Admission Budget**:
The part of the **OCR Throughput Budget** that decides which automatic **Captured Frame** values should receive an **OCR Job**.
_Avoid_: frame dropper, OCR enqueue throttle, skip rule

**OCR Candidate**:
A **Captured Frame** that is eligible for automatic **OCR Job** admission after budget and value checks.
_Avoid_: sampled frame, selected screenshot, OCR frame

**OCR-Relevant Change**:
A visual or context change in a **Captured Frame** that is likely to change useful recognized text.
_Avoid_: pixel change, screen difference, visual delta

**OCR-Relevance Probe**:
An extra pre-admission analysis pass that tries to detect whether a **Captured Frame** likely contains useful text before admitting an **OCR Job** and costs meaningful CPU beyond already available capture or persistence data.
_Avoid_: lightweight OCR, pre-OCR, text detector

**OCR Admission Reason**:
A runtime/debug explanation for why automatic OCR was or was not admitted for an **OCR Candidate**. It may be exposed through live debug state or logs, but it is not durable user data.
_Avoid_: debug log, skip note, enqueue reason

**OCR Budget Telemetry**:
Live-only cost and usefulness summary data used to inspect **OCR Throughput Budget** behavior during the current app run without duplicating recognized text. It may be kept in bounded memory and logs, but it is not durable user data.
_Avoid_: OCR text copy, performance log, debug-only metric

**OCR Execution Budget**:
The part of the **OCR Throughput Budget** that paces when already queued **OCR Job** values run.
_Avoid_: worker sleep, queue delay, CPU limiter

**OCR Catch-Up**:
The execution mode that processes deferred **OCR Job** backlog after recording stops or the app determines the machine is idle enough for more work.
_Avoid_: batch OCR, drain mode, background sweep

**Captured Frame Reprocessing**:
A request to re-run OCR for an existing **Captured Frame** that is already persisted.
_Avoid_: force processing, rerun pipeline, requeue screenshot

**Scrub Preview**:
A disposable, display-sized preview image for a screen segment time position, used while navigating the dashboard timeline.
_Avoid_: exact frame, OCR source, screenshot, thumbnail

**Scrub Preview Generation**:
Background work that materializes generated **Scrub Preview** cache artifacts for finalized screen segment intervals.
_Avoid_: scrub-time extraction, exact frame preview generation, thumbnail pipeline

**Recording Lifecycle**:
The in-memory control flow for one coordinated recording runtime that starts capture, owns pause/resume decisions, rotates segments, recovers after wake, and stops capture across the requested sources. Screen and system audio share the screen capture backend, while microphone runs as a separate native session.
_Avoid_: capture runtime, recorder service, session manager

**App Privacy Exclusion**:
The user-facing privacy policy that prevents live screen capture of selected apps by app identity. **App Privacy Exclusion** is the only live privacy exclusion guarantee and the only privacy exclusion control exposed in settings. Mnema records visible screen content from apps that are not excluded, including private or incognito browser windows.
_Avoid_: website exclusion, title exclusion, private browser exclusion, per-window privacy, metadata privacy rule

**Live Privacy Filter**:
The native screen-capture filtering mechanism that applies **App Privacy Exclusion** before frames are delivered to Mnema.
_Avoid_: privacy promise, metadata redaction, post-capture filtering

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

**Browser Metadata Collection**:
Native browser URL metadata, governed by metadata settings, for timeline and search context without making live capture privacy decisions.
_Avoid_: metacollection, browser privacy signal, website privacy rule

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

**Downstream Capture Access**:
Access to retained capture content after capture, including search, timeline preview, export, and future local AI features.
_Avoid_: raw SQLite access, direct frame-file access, agent bypass

**Brokered Capture Access**:
A policy-aware app or CLI boundary for downstream access to retained capture content that applies retention, deletion, redaction, and access rules before returning results.
_Avoid_: direct database query, raw media crawl, agent file access

**Encrypted Capture Index**:
A future ADR-backed storage protection for Mnema's SQLite-backed searchable and contextual capture data, excluding original frame, video, and audio media.
_Avoid_: encrypted capture store, media encryption, secure erase

**Capture Index Key Store**:
The platform-owned secret storage boundary that holds **Encrypted Capture Index** keys outside the recording save directory.
_Avoid_: key file, save-directory secret, hard-coded key

**Secret Redaction Pipeline**:
A future ADR-backed downstream mitigation flow that detects likely secrets in searchable derived text and withholds or replaces that text before search, snippets, copy-text, or agent-facing access.
_Avoid_: capture prevention, media redaction, secure erase

**Secure Field Capture Suspension**:
A future ADR-backed product concept that would suspend capture while secure text entry is focused, rather than filtering by app, window, website, or recognized text.
_Avoid_: password-page filter, secure-field redaction, browser login exclusion

**Audio Segment**:
A time-bounded persisted audio recording file produced from one recording source during a recording session.
_Avoid_: audio file, raw microphone file, sound clip

**Audio Transcription**:
Recognized speech text, with optional timing relative to its **Audio Segment**, language, confidence, provider, and model metadata, derived from one **Audio Segment**.
_Avoid_: transcript blob, transcription result, speech text

**Audio Transcription Span**:
A searchable time range of recognized speech inside one **Audio Segment**.
_Avoid_: transcript chunk, audio hit, audio clip

**Audio Transcription Job**:
Background work that recognizes speech for one **Audio Segment**.
_Avoid_: transcription task, transcript worker item, speech job

**Speaker Analysis Job**:
Local diarization and optional recognition work for one microphone **Audio Segment**.
_Avoid_: speaker task, diarization worker item, speaker recognition task

**Speaker Turn Alignment**:
The policy that assigns **Audio Transcription** words or segments to speaker turns. Transcription timing is primary; speaker turns annotate that text and do not stretch, split, or duplicate transcript text.
_Avoid_: transcript rewriting, diarization-owned text timing, speaker text retiming

**Speaker Continuity**:
The session-level policy that keeps a real speaker associated with a stable speaker cluster across **Audio Segment** values with the same `session_id`, provider, and model.
_Avoid_: segment-local speaker identity, provider cluster identity, cross-session speaker identity

**Audio Transcription Provider**:
A local speech recognition option used by an **Audio Transcription Job** to produce an **Audio Transcription**.
_Avoid_: cloud transcription service, transcription engine, ASR backend

**Audio Transcription Model**:
A local model asset selected for an **Audio Transcription Provider** when that provider requires app-managed model files.
_Avoid_: model file, downloaded artifact, checkpoint

**Captured Frame Equivalence**:
The rule for when two **Captured Frame** values should be treated as the same OCR-relevant visual content for downstream decisions such as **OCR Job** admission. **Captured Frame Equivalence** is defined over normalized visual content rather than persisted artifact bytes, and intentionally ignores cursor-sized changes plus limited localized visual noise that does not materially change OCR-relevant content.
_Avoid_: dedupe hash, screenshot sameness, OCR skip heuristic

**Captured Frame Equivalence Scope**:
The rule for where an earlier equivalent **Captured Frame** may be searched when applying **Captured Frame Equivalence**. **Captured Frame Equivalence Scope** is session-wide by default, but narrows to the same hidden segment workspace when the candidate **Captured Frame** originated from a hidden segment workspace artifact path.
_Avoid_: workspace filter, lookup scope, same-segment rule

**Hidden Segment Workspace**:
A hidden per-segment directory (`.<session>-segment-####/`) that stores temporary capture artifacts and exported JPEG frames for one screen segment. A **Hidden Segment Workspace** lives beside its visible sibling segment recording file.
_Avoid_: temp folder, segment scratch dir, hidden segment temp

**Hidden Segment Workspace Repair**:
The scan-and-cleanup flow that classifies a **Hidden Segment Workspace** using **Frame Batch** references, **OCR Job** references, whether the visible sibling segment is present and openable, pending frame artifacts, and whether the workspace is still the active screen session before deciding whether it is safe to remove.
_Avoid_: temp cleanup, workspace GC, segment dir sweep

**Managed Storage Layout**:
The derived on-disk layout rooted at `<saveDirectory>` that owns app-infra state such as SQLite and the recordings tree.
_Avoid_: save dir helper, path utility, storage paths

**Audio Activity Sample**:
A raw audio probe reading such as latest normalized level or last-sample timestamp, exposed for debug visibility but not itself used as the inactivity decision.
_Avoid_: audio activity event, microphone activity, system audio activity

**Audio Activity Decision**:
The threshold-qualified inactivity-policy view of audio activity, including enabled state, threshold, and derived idle used for pause/resume decisions.
_Avoid_: raw audio sample, activity reading, latest level

**Capture Session**:
The logical user recording that groups produced screen, microphone, and system-audio source sessions for retention and deletion policy.
_Avoid_: recording row, source session, runtime session

**Capture Segment**:
The unified retention deletion unit for one produced screen, microphone, or system-audio artifact. A **Capture Segment** points at DB-derived subjects such as **Captured Frame** and **Audio Segment** rows and at DB-owned filesystem paths when present.
_Avoid_: file row, media chunk, retention item

**Capture Segment Duration**:
The configured maximum wall-clock duration for one produced **Capture Segment** before the **Recording Lifecycle** rotates to the next segment.
_Avoid_: recording duration, session length

**Retention Policy**:
The user-selected local retention window for capture data: never, 7 days, 14 days, or 30 days. Calendar policies keep today plus the previous local calendar days, not rolling hours.
_Avoid_: cleanup interval, TTL

**Retention Cleanup**:
The app-infra deletion flow that removes eligible **Capture Segment** values and their derived frames, audio segments, processing jobs/results, speaker rows, segment-derived voice embeddings, and rejections while preserving user-authored **Person Profile** rows.
_Avoid_: purge, vacuum, file cleanup

**Search Result Anchor**:
The domain object and time position a search result opens, either a **Captured Frame** or an **Audio Transcription Span**.
_Avoid_: result row, search document, record match

**Text Search**:
The search tier that matches query terms against recognized screen text, recognized speech text, and capture context.
_Avoid_: keyword-only search, exact search, string filter

**Semantic Search**:
The search tier that matches meaning rather than only literal query terms across searchable captured content.
_Avoid_: embedding support, vector lookup, AI search

**Semantic Search Model**:
A local model asset used to derive meaning vectors for **Semantic Search**.
_Avoid_: cloud embedding service, embedding API, vector model

**Hybrid Search**:
The product search policy that combines **Text Search** and **Semantic Search** so literal and meaning-based matches can rank together.
_Avoid_: search mode toggle, vector-only search, fuzzy search

**Search Index Projection**:
A durable derived view of searchable capture content used to answer search queries.
_Avoid_: search cache, search result table, indexing job output

**Search Context**:
Captured contextual labels that help find or filter a **Search Result Anchor**, such as app name, window title, browser URL, or speaker label.
_Avoid_: metadata blob, result decoration, search tags

**Search Refinement**:
An explicit user control that narrows an active search by retained capture context such as date range, app, source, or result type.
_Avoid_: advanced search, search mode, query syntax

**Search Entry Point**:
A contextual user action that starts search with an initial scope derived from where the user is in Mnema.
_Avoid_: search shortcut, preset search, smart search

**Visible Timeline Search**:
A **Search Entry Point** that scopes search to the captured time range currently visible in the dashboard timeline.
_Avoid_: loaded timeline search, current page search, visible rows search

**Current App Search**:
A **Search Entry Point** that scopes search to the retained app context of the active dashboard **Captured Frame**.
_Avoid_: frontmost app search, Mnema app search, system focus search

**App Search Refinement**:
A **Search Refinement** that narrows frame results by retained app identity, preferring bundle identifier and falling back to app name only when no bundle identifier was captured.
_Avoid_: app-name filter, window filter, bundle filter

**Audio Source Search Refinement**:
A **Search Refinement** that narrows audio results by retained audio recording source, such as microphone or system audio.
_Avoid_: result type filter, media filter, audio mode

**Date Range Search Refinement**:
A **Search Refinement** that narrows results to **Search Result Anchor** values whose captured time overlaps a selected time range.
_Avoid_: loaded range, page window, result date label

**Search Context Alignment**:
A derived relationship that decorates a **Search Result Anchor** with nearby retained context from another capture source without treating that context as native to the anchor.
_Avoid_: inferred source, guessed app, audio app context

**Search Snippet**:
A query-specific preview of why a **Search Result Anchor** matched.
_Avoid_: saved preview text, stored excerpt, result summary

**Search Result Group**:
A collapsed search result that represents multiple equivalent **Search Result Anchor** values.
_Avoid_: duplicate result, grouped row, result cluster

## Relationships

- A **Screen Frame Artifact** becomes a **Captured Frame** only after app-infra persists it.
- A **Captured Frame Pipeline** persists one **Captured Frame**.
- A **Captured Frame Pipeline** attaches each **Captured Frame** to exactly one **Frame Batch**.
- A **Captured Frame Pipeline** may enqueue one **OCR Job** for a **Captured Frame**.
- **Sensitive Capture Protection V1** remains inside **App Privacy Exclusion** and does not promise website-level, private-window, password-page, or secure-field protection.
- **Sensitive Capture Protection V1** is UX and recovery around **App Privacy Exclusion**, not detection of sensitive screen content.
- **App Privacy Exclusion** remains handled through the native **Live Privacy Filter**, not through app-based automatic pause.
- Mnema sanitizes browser URL metadata before persistence; full URL metadata remains an explicit user choice because query strings and fragments may contain secrets.
- **Browser Metadata Collection** uses native browser URL probing only in this branch; another metadata adapter requires a future ADR-backed design.
- **Secret Redaction Pipeline** affects searchable derived text, snippets, copy-text actions backed by OCR or transcripts, and agent-facing derived text access, not original frame, video, or audio media.
- **Secret Redaction Pipeline** V1 targets high-confidence secrets such as API keys, access tokens, private keys, seed-like secrets, structurally obvious passwords, clearly labeled or formatted auth codes, and credential-bearing database connection strings.
- **Secret Redaction Pipeline** V1 does not attempt broad PII, name, email, address, phone, sensitive-business-text, screenshot-region, or image redaction.
- **Secret Redaction Pipeline** V1 uses deterministic high-confidence secret detection rather than broad probabilistic PII model classification.
- **Secret Redaction Pipeline** is always on for searchable and broker-visible derived text once shipped, with no user-facing disable in V1.
- **Secret Redaction Pipeline** runs before persistence of searchable derived text from OCR, microphone transcription, and system-audio transcription, and does not store raw secret-bearing OCR or transcript text by default.
- **Secret Redaction Pipeline** applies to broker-visible or searchable word/token payloads as well as display strings; timing metadata may remain, but original secret token text must not be persisted by default.
- **Secret Redaction Pipeline** may inspect bounded in-memory context around OCR or transcript text to classify high-confidence secrets, but must not persist raw context windows.
- If **Secret Redaction Pipeline** fails before searchable derived text persistence, Mnema must fail closed by not persisting raw text as searchable or broker-visible content.
- **Secret Redaction Pipeline** removes exact secret values from searchability; search may match surrounding non-secret context and redaction categories, but not the original secret value.
- **Secret Redaction Pipeline** may persist redaction spans, categories, detector versions, and aggregate counts against redacted text, but never the original matched secret value.
- User-facing **Secret Redaction Pipeline** metadata should expose coarse categories such as API key, access token, private key, password, auth code, connection string, or seed-like secret, not detector internals, confidence scores, or matched prefixes/suffixes.
- Search ranking may use redaction categories conservatively, but should not strongly boost results merely because a secret was redacted.
- **Secret Redaction Pipeline** reprocessing may add redactions to existing searchable derived text when detectors improve, but it cannot restore original secret values and should not inspect original media by default.
- Future **Semantic Search** embeddings should be derived from redacted text and stored under the **Encrypted Capture Index** boundary.
- Original capture media may still contain redacted content, so **Delete Recent Capture** remains the recovery path for removing media from Mnema's app library.
- UI surfaces that open, preview, copy from, or export original media associated with redaction metadata should warn that original capture may still contain redacted secrets.
- Search and derived-text UI may show non-content redaction metadata such as redaction category, count, and `has redactions` filters, but not matched secret values or detector explanations that include secret text.
- **Brokered Capture Access** is the supported path for AI agents and other downstream tools to inspect retained capture content.
- Agent-facing capture access should be documented only through **Brokered Capture Access**; app-internal APIs and Tauri commands are not agent contracts unless explicitly marked broker-safe.
- Direct SQLite or media-file access by agents is outside Mnema's privacy guarantee.
- **Encrypted Capture Index** protects SQLite-backed searchable and contextual capture data, not original frame, video, or audio media.
- Original media files remain sensitive even when **Encrypted Capture Index** is enabled, and raw media encryption is out of scope for the first storage-security phase.
- **Encrypted Capture Index** should use maintained page-level SQLite encryption rather than hand-rolled field encryption.
- New **Encrypted Capture Index** databases are encrypted by default and should not expose a user-facing plaintext mode.
- **Encrypted Capture Index** keys belong in a **Capture Index Key Store** outside `saveDirectory`; macOS should use Keychain through that abstraction.
- Each **Encrypted Capture Index** should have its own key tied to a stable index identity rather than sharing one global app key.
- An **Encrypted Capture Index** may expose non-secret index identity metadata through a readable header or sidecar so the app and broker can locate the corresponding **Capture Index Key Store** entry.
- If an **Encrypted Capture Index** key is missing or inaccessible, Mnema treats the index as undecryptable unless an explicit future backup/export key flow exists; fallback keys must not live in `saveDirectory`.
- **Brokered Capture Access** should use the app-owned **Capture Index Key Store** path rather than exposing raw encryption keys to agents.
- **Brokered Capture Access** may run when the Mnema app is not running, but it must use the same policy, redaction, retention, tombstone, and **Capture Index Key Store** paths as the app.
- **Brokered Capture Access** should expose a dedicated CLI contract backed by shared Rust policy/query code rather than relying on app-internal Tauri commands as the agent interface.
- **Brokered Capture Access** requires user authorization before an agent or downstream tool can query capture data, and that authorization grants redacted/searchable derived access rather than original media export.
- First **Brokered Capture Access** authorization requires Mnema UI; standalone CLI access may use existing valid grants but should return `authorization_required` when no valid grant exists.
- **Brokered Capture Access** V1 grants are read-only, redacted, time-bounded, revocable, and limited to searchable-content commands such as search, show-text, timeline, and open-in-Mnema.
- **Brokered Capture Access** grants may be time-scoped, and all-retained-history access requires an explicit user choice.
- **Brokered Capture Access** grant state is app access-control state outside the capture index; non-secret grant metadata belongs in app config, while grant secrets or tokens belong in platform secret storage when needed.
- **Brokered Capture Access** audit history stores non-content events such as tool identity, command type, timestamp, and result count, not raw query text, returned snippets, or media paths.
- **Brokered Capture Access** returns redacted derived content and opaque identifiers by default, not raw SQLite rows or media file paths.
- **Brokered Capture Access** should use bounded result limits and opaque pagination, and must not provide an unrestricted dump-all searchable text command.
- **Brokered Capture Access** may return full redacted OCR or transcript text only for a specific opaque result identifier within grant scope, not through bulk all-content commands.
- **Brokered Capture Access** must not expose original-media paths by default because agents could use those paths to recover secrets from frame, video, or audio media outside the redacted searchable-text path.
- **Brokered Capture Access** may provide an open-in-Mnema action for opaque result identifiers so original media inspection stays mediated by app UI warnings and confirmations.
- **Brokered Capture Access** V1 does not include privileged original-media export, media-path return, raw DB dump, or raw OCR/transcript dump commands.
- **Brokered Capture Access** may support app, source, and time refinements, but should minimize returned app/window/browser metadata and avoid returning full browser URLs by default.
- **Recommended App Exclusions** become **App Privacy Exclusion** rules only after user confirmation.
- **Recommended App Exclusions** are shown during onboarding and through a one-time non-blocking prompt for existing users after upgrade when at least one detected recommended app is missing from **App Privacy Exclusion** or has its exclusion disabled.
- **Recommended App Exclusions** prompt dismissal is persisted in **One-Time Prompt State** rather than recording settings or browser local storage.
- The existing-user **Recommended App Exclusions** prompt is one-time for V1 and does not reappear just because a new catalog app is installed later.
- Privacy settings continue to show actionable **Recommended App Exclusions** after the one-time prompt is dismissed.
- Changes to the **Sensitive App Recommendation Catalog** do not retrigger the V1 existing-user **One-Time Prompt** after it has been dismissed or completed.
- A recommended app with an existing disabled **App Privacy Exclusion** is shown as currently off and can be re-enabled instead of added as a duplicate rule.
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
- **App Privacy Exclusion** does not remove or hide historical search, timeline, frame, or audio results that were already captured before the exclusion was added.
- If a live app-exclusion change cannot be applied while recording, Mnema reports that screen/system-audio capture is suspended because privacy exclusions could not be applied, reusing the existing privacy suspension path.
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
- V1 does not change default browser URL metadata settings.
- **Browser Capture Disclosure** may mention browser URL metadata in Privacy settings, but not in fast status-bar recovery flows.
- **OCR Admission Budget** is not a privacy layer and does not skip **Captured Frame** values based on inferred sensitive content in V1.
- **Downstream Capture Access** in app-owned surfaces operates only over retained app-infra data reachable through app-owned APIs.
- Raw SQLite or frame-file access by external agents is outside the **Sensitive Capture Protection V1** privacy guarantee until Mnema introduces an explicit brokered access boundary.
- **Secure Field Capture Suspension** is separate from the **Live Privacy Filter** and requires its own ADR before becoming a product guarantee.
- A **Scrub Preview** represents a screen segment time position, not **Captured Frame** identity.
- Multiple nearby **Captured Frame** values may share one **Scrub Preview** when they fall within the same preview interval.
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
- A generated **Scrub Preview** cache interval is keyed by source segment video offset, while dashboard availability is requested and displayed by timeline time.
- Dashboard timeline mapping for generated **Scrub Preview** intervals uses **Capture Segment** timing plus segment video offset rather than per-frame captured timestamp jitter.
- A timeline interval with a usable frame index but no indexed screen position is unavailable for **Scrub Preview** without treating the whole frame index as missing.
- The generated **Scrub Preview** cache defaults to a 512 MB budget and 7-day last-access window, pruned by segment cache directory rather than individual preview file.
- Generated **Scrub Preview** cache policy is separate from exact frame preview cache policy.
- Existing exact preview cache TTL settings do not control generated **Scrub Preview** disk cache lifetime.
- V1 generated **Scrub Preview** interval, rendition, and cache budget are fixed product policy rather than user-facing recording settings.
- V1 may expose a developer/debug action to clear only generated **Scrub Preview** cache without clearing exact preview cache or adding regular user-facing cache controls.
- V1 developer/debug surfaces may expose generated **Scrub Preview** cache status and queue status for verification.
- **Scrub Preview Generation** runs outside the active scrub interaction path; timeline navigation may request availability, but missing generated **Scrub Preview** values are materialized in background work.
- **Scrub Preview Generation** uses a single coalescing worker where the newest visible timeline window takes priority over stale queued preview intervals.
- **Scrub Preview Generation** queue state is non-durable and rebuilds from finalized-segment events or dashboard availability demand.
- **Scrub Preview Generation** stays outside app-infra processing job lanes and frame/OCR persistence transactions.
- Startup validates/prunes generated **Scrub Preview** cache but does not warm missing previews for existing segments.
- **Scrub Preview Generation** processes interval work in bounded chunks so visible-window demand can preempt full-segment warming.
- A finalized screen **Capture Segment** enqueues full one-second-interval **Scrub Preview Generation**, bounded by the 5-minute **Capture Segment Duration** cap.
- Historical **Capture Segment** values encountered through dashboard demand enqueue only visible-window intervals, not full-segment warming.
- Automatic **Scrub Preview Generation** is triggered after the screen **Capture Segment** is committed, outside capture primitive code.
- App-infra owns **Capture Segment** discovery for **Scrub Preview** availability, while the desktop Tauri layer owns generated **Scrub Preview** cache files, native extraction, asset scope, queueing, and cache-change events.
- Completed **Scrub Preview Generation** chunks notify the dashboard with coalesced cache-change events so visible windows can refresh availability without polling continuously; those events invalidate ranges rather than carrying preview file paths.
- **Scrub Preview** generation failures are non-durable availability states with short-lived retry backoff, not persisted app-infra records.
- Dashboard navigation requests **Scrub Preview** availability by timeline window rather than **Captured Frame** identity.
- Dashboard **Scrub Preview** availability requests may enqueue missing visible-window intervals for background generation but must not synchronously extract preview images.
- Dashboard **Scrub Preview** availability responses include ready and unavailable/queued interval statuses, while display uses only ready intervals.
- **Scrub Preview** availability may report queued status from non-durable in-memory generation queue or in-flight work.
- Dashboard initial load requests **Scrub Preview** availability for the initial visible timeline window once timeline data and viewport dimensions are available.
- Dashboard **Scrub Preview** availability requests cover the visible timeline window plus small overscan, not the entire loaded timeline history.
- Dashboard scroll debounce applies only to backend **Scrub Preview** availability/enqueue requests; already-known ready **Scrub Preview** cache entries should display immediately during timeline movement.
- Dashboard timeline movement does not start exact preview requests as a fallback for missing **Scrub Preview** values; exact preview requests are settle/inspection behavior.
- Dashboard **Scrub Preview** state is interval-based, with active **Captured Frame** display derived from the matching interval instead of a frame-id preview cache.
- **Scrub Preview** availability is derived from screen **Capture Segment** rows and their frame indexes; disposable preview cache entries are not modeled as durable app-infra rows.
- A **Scrub Preview** may be lower resolution or timing-tolerant and is never the source for OCR, copy, download, or **Captured Frame** truth.
- A **Scrub Preview** can stand in only while timeline navigation is in motion; a parked active **Captured Frame** resolves through the exact preview path.
- A **Scrub Preview** may remain visible as a placeholder while the exact preview for the parked active **Captured Frame** is loading.
- A **Scrub Preview** must not populate exact preview cache state or enable exact **Captured Frame** actions.
- When a requested **Scrub Preview** is absent during timeline movement, the dashboard may keep showing the previous available preview rather than blocking movement or showing a loading state.
- Dashboard previous-preview placeholders should be cleared when timeline movement jumps far enough that the displayed preview is no longer near the active interval.
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
- **App Privacy Exclusion** is app-based rather than website-, title-, private-browser-, or private-window-based.
- Metadata-derived website, title, private-browser, and per-window decisions must not feed the **Live Privacy Filter**.
- Shared recording/privacy settings should not expose inactive metadata privacy fields for website, title, private-browser, or per-window exclusion.
- Metadata collection kept after removing metadata privacy rules must serve non-privacy product features such as timeline context, app/window labels, or debug surfaces.
- A **Recording Lifecycle** may pause or resume requested sources based on inactivity policy.
- A **Recording Lifecycle** commits requested audio sources as **Audio Segment** values.
- A **Recording Lifecycle** creates one **Capture Session** for a user recording and **Capture Segment** rows only for produced artifacts.
- A **Capture Session** can filter or group search results, but is not itself a content-bearing **Search Result Anchor**.
- **Capture Segment Duration** applies to each rotated **Capture Segment**, not to total **Capture Session** length.
- **Capture Segment Duration** is capped at 5 minutes in persisted settings, runtime validation, and user-facing settings surfaces.
- A **Capture Segment** supports storage, recovery, and retention boundaries rather than acting as a **Search Result Anchor**.
- An **Audio Segment** comes from exactly one recording source, such as microphone or system audio.
- A **Retention Policy** applies only to the active **Managed Storage Layout** and active app-infra database.
- **Retention Cleanup** skips active capture segments and subjects with running processing/finalize jobs.
- **Retention Cleanup** preserves **Person Profile** values even when derived speaker rows are deleted.
- **Retention Cleanup** best-effort removes generated **Scrub Preview** cache directories for deleted screen **Capture Segment** values, while cache validation and pruning remain responsible for stale orphan safety.
- **Retention Cleanup** reaches generated **Scrub Preview** cache through the desktop Tauri cache service rather than app-infra owning Tauri app-cache paths.
- A dashboard `timeline_data_changed` retention event should prune loaded rows older than the cutoff and preserve the active retained item when possible.
- A microphone **Audio Segment** becomes eligible for an **Audio Transcription Job** when the **Recording Lifecycle** commits it, even if the eventual transcript is empty.
- An **Audio Transcription Job** operates on exactly one **Audio Segment**.
- A **Speaker Analysis Job** operates on exactly one microphone **Audio Segment**.
- A **Speaker Analysis Job** can complete successfully with no speaker turns.
- Too-short, silent, or valid no-speaker audio produces a successful empty speaker-analysis result, not a failed **Speaker Analysis Job**.
- Missing speaker models, audio decode failures, speaker runtime failures, subprocess failures, malformed helper output, and persistence failures are **Speaker Analysis Job** failures.
- Successful **Speaker Analysis Job** diagnostics live in result provenance.
- Failed **Speaker Analysis Job** diagnostics live in `processing_jobs.last_error`.
- **Speaker Analysis Job** execution has a dedicated single-concurrency processing worker so speaker work does not block OCR/frame-batch or audio-transcription lanes.
- The sherpa speaker-analysis helper remains subprocess-per-job in this stage; no persistent helper daemon, in-process model reuse, or generic audio-heavy worker abstraction is part of the current design.
- Each **Speaker Analysis Job** freezes its helper timeout in payload option `helperTimeoutSeconds` when admitted, so later settings changes affect only future jobs.
- The speaker-analysis helper timeout defaults to 600 seconds, clamps to 60-3600 seconds, and timeout failures kill/reap the helper before the job follows the normal failed processing path.
- **Speaker Turn Alignment** treats **Audio Transcription** words or segments as the source timeline and assigns them to the best speaker turn annotation.
- **Speaker Continuity** is limited to **Audio Segment** values in the same recording/session, represented by stable cluster rows rather than provider-local cluster ids.
- Speaker provider cluster ids are provenance from the diarization provider and remain provider-local; they are not rewritten to represent stable identity.
- Speaker merge suggestions are preferred over aggressive automatic merges when continuity matching is ambiguous or only moderately similar.
- VAD-based audio cutting or trimming is outside **Speaker Analysis Job** quality policy; audio segment production remains owned by the recording flow.
- An **Audio Transcription Job** uses exactly one **Audio Transcription Provider**.
- V1 **Audio Transcription Provider** values are local-only: `local_whisper`, `apple_speech_on_device`, and `parakeet`.
- V1 `local_whisper` **Audio Transcription Model** choices are `tiny`, `base`, `small`, and `medium`, with `base` as the default.
- V1 `parakeet` uses `parakeet-tdt-0.6b-v3-onnx` as its **Audio Transcription Model** and runs it through the Rust ONNX Runtime adapter.
- `apple_speech_on_device` uses OS-managed language models rather than an app-managed **Audio Transcription Model**.
- An **Audio Transcription Provider** may require one selected **Audio Transcription Model**.
- A microphone **Audio Segment** gets one **Audio Transcription Job** for the selected **Audio Transcription Provider** when that provider and its required **Audio Transcription Model** are available.
- An **Audio Transcription Job** freezes the selected **Audio Transcription Provider** and **Audio Transcription Model** at admission time.
- Changing the selected **Audio Transcription Provider** or **Audio Transcription Model** affects future **Audio Segment** values, not existing completed **Audio Transcription** values.
- An app-managed **Audio Transcription Model** may be installed, missing, downloading, or failed.
- If the selected **Audio Transcription Provider** or required **Audio Transcription Model** is unavailable, the microphone **Audio Segment** remains eligible but does not get an **Audio Transcription Job** until backfill can enqueue it.
- Missing selected **Audio Transcription Model** status is surfaced when recording starts and in a dedicated Transcription settings surface, not once per committed microphone **Audio Segment**.
- An **Audio Transcription** is derived from exactly one **Audio Segment**.
- An **Audio Transcription** contains zero or more **Audio Transcription Span** values.
- An **Audio Transcription Span** belongs to exactly one **Audio Segment** and is derived from transcript timing when available.
- **Audio Transcription Span** derivation prefers provider transcript segments, falls back to word-derived windows only when segments are absent, and falls back to the whole **Audio Segment** only for untimed transcript text.
- Speaker turns may decorate an **Audio Transcription Span** when available, but they do not define the searchable audio unit.
- **Audio Transcription Span** results may come from microphone or system-audio **Audio Segment** values.
- **Search Context** for an **Audio Transcription Span** should include its recording source.
- Adjacent or overlapping **Audio Transcription Span** hits from the same **Audio Segment** may collapse into one **Search Result Group**.
- **Audio Transcription Span** hits from different **Audio Segment** values or separated moments should remain separate results.
- A search result for screen text should anchor to a **Captured Frame**.
- A **Captured Frame** should be indexed as one searchable screen document even when OCR returns multiple text observations.
- OCR observations may provide highlight context for a **Captured Frame** search result, but they are not separate **Search Result Anchor** values.
- Equivalent **Captured Frame** values that reuse the same OCR text should remain searchable at their own capture times.
- Equivalent **Captured Frame** search hits should collapse into a **Search Result Group** so unchanged screens do not flood the result list.
- A **Search Result Group** for equivalent **Captured Frame** hits should open the newest matching frame by default.
- A search result for audio speech should anchor to an **Audio Transcription Span** when timing is available.
- Selecting a **Search Result Anchor** should navigate the existing dashboard timeline or audio player to that captured point.
- Selecting a search result from the modal should close the modal after navigation starts.
- Selecting an **Audio Transcription Span** should open the audio player at that span and align the dashboard timeline to the **Captured Frame** at the same recording time when such a frame exists.
- Audio-to-frame alignment should prefer the latest **Captured Frame** at or before the **Audio Transcription Span** start time.
- An **Audio Transcription Span** remains a valid **Search Result Anchor** even when no nearby **Captured Frame** exists.
- Search results should not require the dashboard timeline to hide non-matching capture data.
- **Text Search** is the first search tier for searchable **Captured Frame** and **Audio Transcription Span** content.
- **Text Search** should search recognized screen or speech body text plus typed **Search Context** fields.
- Recognized screen or speech body text should rank higher than **Search Context** matches by default.
- **Search Context** should include only context Mnema actually captured and retained.
- **Search Context** should enrich content-bearing search results rather than create standalone search results.
- **Text Search** entries should be updated transactionally with completed OCR or transcription results.
- **Text Search** should be enabled as part of completed OCR and transcription rather than as a separate user-facing setting.
- **Text Search** should include completed OCR and transcription results only.
- Empty completed OCR or transcription results should not create **Search Result Anchor** values.
- Search should not create OCR or transcription work by itself.
- Reprocessing a **Captured Frame** or **Audio Segment** should replace its existing **Search Index Projection** when the new result completes.
- **Semantic Search** augments **Text Search** rather than replacing it.
- **Semantic Search** is local-only.
- **Semantic Search** indexing may run as separate background work because embedding generation is model work, not simple projection maintenance.
- **Semantic Search** uses a **Semantic Search Model** rather than sending searchable capture content to a cloud embedding service.
- **Hybrid Search** is the product direction once **Semantic Search** exists.
- A **Search Index Projection** should produce one mixed result stream over typed **Search Result Anchor** values.
- A **Search Index Projection** may carry type-specific anchor data for **Captured Frame** and **Audio Transcription Span** results.
- **Search Result Group** creation should happen before result pagination is presented to the user.
- **Search Refinement** should apply to **Search Result Anchor** values before choosing the representative anchor for a **Search Result Group**.
- **Search Refinement** values should compose when their result-type constraints are compatible.
- A **Search Result Group** should be included in a **Date Range Search Refinement** when at least one grouped **Search Result Anchor** overlaps the selected time range.
- A **Search Result Group** shown under a **Date Range Search Refinement** should open a representative **Search Result Anchor** inside the selected time range.
- Search results should rank by relevance with a recency bias by default.
- Search input should be plain text by default rather than requiring users to learn advanced query syntax.
- The first user-facing search surface should prioritize one plain search box, with lightweight result-type filtering at most.
- The first user-facing search result surface should be a large dashboard modal.
- The search result modal should not show captured text results for an empty query.
- Search should run live for plain-text queries after a small character threshold, and explicit submit should run immediately.
- Open search results should not automatically reshuffle as new OCR or transcription work completes.
- The search result modal should present **Captured Frame** results and **Audio Transcription Span** results as separate areas.
- The search result modal should initially focus on a small top set of results rather than overwhelming the user with the full match set.
- The default search result modal should show up to five **Captured Frame** results and up to five **Audio Transcription Span** results.
- Search result limits should count visible result cards or **Search Result Group** values, not raw grouped anchors.
- The search result modal should let the user request more **Captured Frame** results independently from more **Audio Transcription Span** results.
- Requesting more search results should preserve separate **Captured Frame** and **Audio Transcription Span** result ranking.
- The search result modal should use result-type tabs or segmented controls rather than side-by-side result columns.
- **Captured Frame** result cards should use image thumbnails.
- **Captured Frame** result-card thumbnails are navigation previews and should not replace exact frame inspection.
- A **Captured Frame** search result should remain visible when its result-card thumbnail is unavailable.
- **Audio Transcription Span** result cards should emphasize recording source, time range, and transcript match rather than speaker labels.
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
- **Search Refinement** values should be applied by search query semantics before pagination rather than by frontend-only result filtering.
- Search responses should expose the normalized **Search Refinement** values that were applied.
- The first **Search Entry Point** values after global search should be visible timeline and current app.
- The first **Search Refinement** controls after result type should be date range, app, and source.
- **Visible Timeline Search** should derive a date-range **Search Refinement** from the timeline viewport time range rather than from the dashboard's loaded rows.
- **Visible Timeline Search** should include both **Captured Frame** and **Audio Transcription Span** results by default.
- **Visible Timeline Search** should freeze the timeline viewport time range when the entry point is invoked rather than tracking later timeline movement.
- **Visible Timeline Search** should be unavailable when the dashboard has no valid timeline viewport time range.
- Initial **Date Range Search Refinement** controls should use contextual or preset ranges rather than a custom date-time picker.
- Preset **Date Range Search Refinement** values should resolve to concrete start and end timestamps when selected rather than rolling while the search modal stays open.
- **Current App Search** from the dashboard should use the active **Captured Frame**'s retained app context rather than the current frontmost macOS app.
- **Current App Search** should be unavailable when the active **Captured Frame** has no retained app identity.
- **App Search Refinement** should use retained bundle identifier as canonical identity when available and app name only as a fallback.
- Initial **App Search Refinement** should be added through **Current App Search** rather than through a full retained-app picker.
- **App Search Refinement** should apply to **Captured Frame** results only until audio results have an explicit **Search Context Alignment** policy.
- **Current App Search** should default to **Captured Frame** results while **App Search Refinement** is frame-only.
- While **App Search Refinement** is frame-only, mixed or audio-only result views should not be selectable until the app refinement is removed.
- Combining **App Search Refinement** with **Date Range Search Refinement** should produce frame results inside the selected time range.
- Result type selection should remain separate from **Audio Source Search Refinement**.
- **Audio Source Search Refinement** should apply only to **Audio Transcription Span** results.
- Selecting an **Audio Source Search Refinement** should switch search to audio results rather than leaving frame results visible without that refinement.
- Initial manual **Search Refinement** controls should be limited to preset date ranges and **Audio Source Search Refinement**.
- Recent searches should be designed separately from the first **Search Refinement** and **Search Entry Point** UX pass.
- Saved searches and watch queries should be designed separately from the first **Search Refinement** and **Search Entry Point** UX pass.
- Search filters such as source, app, date range, or result type should be explicit UI controls when they are added.
- A **Search Index Projection** may be rebuilt from retained OCR and transcription results.
- A **Search Index Projection** is user data because it duplicates searchable recognized text.
- A **Search Snippet** should be generated when the user searches rather than stored ahead of time.
- **Retention Cleanup** must remove **Search Index Projection** rows for deleted **Captured Frame** and **Audio Segment** sources in the same cleanup transaction.
- An empty no-speech **Audio Transcription** is a successful **Audio Transcription**, not a failed job.
- An **Audio Transcription** is produced by an **Audio Transcription Job**.
- A **Managed Storage Layout** is derived from one `saveDirectory` value.
- A **Managed Storage Layout** contains the recordings tree under `<saveDirectory>/recordings`.
- **Captured Frame Equivalence** determines whether a new **Captured Frame** needs a new **OCR Job**.
- **Captured Frame Equivalence Scope** determines which earlier **Captured Frame** values are eligible comparison candidates.
- A **Captured Frame Pipeline** skips a new **OCR Job** when an earlier equivalent **Captured Frame** in the same session is already eligible as the OCR fallback.
- An **OCR Throughput Budget** limits **OCR Job** admission and scheduling rather than guaranteeing a hard process-wide CPU ceiling.
- An **OCR Throughput Budget** must preserve the **OCR Settings Selection** instead of silently switching provider, model, recognition mode, or provider-specific options.
- An **OCR Throughput Budget** has an **OCR Admission Budget** and an **OCR Execution Budget**.
- Current-run **OCR Throughput Budget** state belongs to the desktop runtime, not app-infra durable storage.
- **OCR Admission Budget** memory is scoped to active capture session/workspace state and should be cleared when that recording session stops.
- **OCR Admission Reason** values may remain app-infra pipeline decision types, but live **OCR Budget Telemetry** DTOs belong to the desktop runtime.
- **OCR Admission Budget** behavior should be tested through the desktop runtime memory interface rather than app-infra database queries.
- An **OCR Admission Budget** limits new automatic **OCR Job** creation.
- **OCR Admission Budget** comparisons should remain scoped to the relevant capture session or workspace where appropriate.
- **Captured Frame Equivalence** is the first automatic OCR duplicate filter; an **OCR Admission Budget** may add a conservative second filter for non-equivalent but low OCR-value frames.
- An **OCR Admission Budget** should avoid creating **OCR Job** debt for automatic low OCR-value **OCR Candidate** values.
- An **OCR Admission Budget** should prefer materially changed **OCR Candidate** values over fixed time cadence.
- A fixed time cadence may dampen stable or low-change capture periods but must not hide materially new searchable text.
- **OCR Admission Budget** may use queued plus running **OCR Job** count as a pressure signal during active recording, admitting only stronger automatic **OCR Candidate** values when backlog is high.
- **OCR-Relevant Change** is narrower than any visible pixel change and excludes cursor movement, small animations, spinners, video playback, and tiny layout shifts.
- Foreground context changes such as app bundle, window title, browser URL, or display changes are strong positive **OCR-Relevant Change** signals when the frame is not equivalent to an earlier frame.
- Unchanged foreground context does not prove there is no **OCR-Relevant Change**.
- An **OCR Throughput Budget** may use cheap admission signals that reuse already available capture or persistence data.
- **OCR Admission Budget** memory should not persist budget decisions, but it may read existing **Captured Frame** metadata to detect **OCR-Relevant Change**.
- An **OCR Throughput Budget** should not add an **OCR-Relevance Probe**, because extra pre-admission analysis can become another CPU cost on the capture hot path.
- **OCR Admission Reason** values explain current runtime decisions and should not be persisted as durable per-frame data.
- **OCR Admission Reason** values should not be invented historically for older frames.
- Low OCR-value admission skips should be visible in debug surfaces but not noisy in the normal timeline.
- **OCR Budget Telemetry** should include timing and usefulness summaries such as recognized text length or observation count, but should not duplicate recognized text.
- **OCR Budget Telemetry** usefulness summaries should be computed after **OCR Job** completion, not estimated before admission.
- **OCR Budget Telemetry** should be live-only by default and should not be stored in the main app database.
- **OCR Budget Telemetry** may be exposed through the debug surface as bounded current-run state so developers can see whether **OCR Job** values are executing.
- The debug surface should separate **OCR Admission Budget** events from **OCR Execution Budget** events so skipped candidates are not confused with jobs that ran.
- **OCR Budget Telemetry** debug events should not include full frame or workspace paths.
- Equivalent-frame reuse should produce a live **OCR Admission Budget** debug event with the related **Captured Frame** id even though it does not create a new **OCR Job**.
- The debug surface should paginate recent **OCR Budget Telemetry** events rather than rendering the full bounded ring at once.
- The debug surface may poll current-run **OCR Budget Telemetry** while the OCR debug tab is active and visible.
- OCR debug commands should expose current-run **OCR Throughput Budget** state rather than durable lookup by old frame or job ids.
- An **OCR Execution Budget** paces existing queued **OCR Job** execution.
- An **OCR Execution Budget** should start with deterministic pacing based on recording state and observed OCR job timing rather than live process-wide CPU measurement.
- An **OCR Execution Budget** should use bounded cost-adaptive pacing based on recent **OCR Job** runtime rather than a single fixed cooldown.
- **OCR Execution Budget** pacing should use observed **OCR Job** runtime regardless of whether the job completed or failed.
- **OCR Execution Budget** pacing memory may reset on app startup; debug timing summaries are current-run state, while durable OCR results remain normal app data.
- **OCR Execution Budget** pacing should be global across sessions because OCR work consumes shared machine resources.
- An **OCR Execution Budget** should be more conservative during active recording than during **OCR Catch-Up**.
- Automatic **OCR Job** work still occurs during active recording; active recording is neither OCR-off nor governed by a fixed time cadence for materially changed frames.
- **OCR Catch-Up** may process deferred **OCR Job** backlog more aggressively after recording stops or while the machine is idle.
- **OCR Catch-Up** processes already admitted **OCR Job** backlog and should not automatically create new **OCR Job** values for low OCR-value candidates skipped by admission.
- **OCR Throughput Budget** semantics apply across OCR providers and should use provider/job timing telemetry for future tuning rather than changing the **OCR Settings Selection**.
- Existing queued **OCR Job** values should be preserved and processed under the **OCR Execution Budget** rather than deleted or reclassified by the **OCR Admission Budget**.
- An **OCR Throughput Budget** should start as a default product policy rather than a visible user-configurable resource setting.
- A **Frame Batch** can be finalized only after its **OCR Job** entries are terminal.
- **OCR Job** values that are not admitted by the **OCR Admission Budget** do not block **Frame Batch** finalization.
- An admitted **OCR Job** still blocks its **Frame Batch** from finalizing until the job is terminal.
- **Captured Frame Reprocessing** operates on an existing **Captured Frame**, not on a new **Screen Frame Artifact**.
- **Captured Frame Reprocessing** bypasses the **OCR Admission Budget** but still respects the **OCR Execution Budget**.
- A **Hidden Segment Workspace** may be preserved when an incomplete **Frame Batch** or nonterminal **OCR Job** still references it.
- **Hidden Segment Workspace Repair** removes only **Hidden Segment Workspace** values that are safe to remove.
- An **Audio Activity Sample** can inform an **Audio Activity Decision**, but the two are not interchangeable.
- An **Audio Activity Decision** is what the inactivity policy uses to pause or resume capture.

## Example dialogue

> **Dev:** "When a **Captured Frame** is equivalent to an earlier frame in the same session, does the **Captured Frame Pipeline** still create an **OCR Job**?"
> **Domain expert:** "No — the frame is persisted and attached to its **Frame Batch**, but if an earlier equivalent frame is already eligible, the pipeline reuses that OCR fallback instead of creating another **OCR Job**."

> **Dev:** "Does an **OCR Admission Budget** replace **Captured Frame Equivalence**?"
> **Domain expert:** "No — **Captured Frame Equivalence** remains the duplicate filter, while the budget may conservatively filter non-equivalent low OCR-value frames."

> **Dev:** "Should a low OCR-value automatic **OCR Candidate** still create a deferred **OCR Job**?"
> **Domain expert:** "No — avoid creating OCR debt for that candidate, keep the **Captured Frame**, and allow manual **Captured Frame Reprocessing** later."

> **Dev:** "Should the timeline label every frame skipped by the **OCR Admission Budget**?"
> **Domain expert:** "No — keep normal timeline quiet, but expose admission-skip reasons in debug surfaces."

> **Dev:** "Can an **OCR Admission Reason** live only in logs?"
> **Domain expert:** "No — it should be persisted so the frame remains explainable after restart."

> **Dev:** "Should **OCR Budget Telemetry** store another copy of recognized text?"
> **Domain expert:** "No — store cost and usefulness summaries; recognized text belongs in the normal OCR result."

> **Dev:** "Should equivalent-frame reuse have an **OCR Admission Reason** even though **Captured Frame Equivalence** already works?"
> **Domain expert:** "Yes — equivalence remains the duplicate filter, and the reason explains why no separate **OCR Job** exists."

> **Dev:** "Should Mnema backfill **OCR Admission Reason** for old **Captured Frame** values?"
> **Domain expert:** "No — persist reasons for newly captured frames after the policy exists instead of inventing historical explanations."

> **Dev:** "Should an **OCR Throughput Budget** force Mnema to stay below 30% CPU at every moment?"
> **Domain expert:** "No — it should limit how much **OCR Job** work is admitted and scheduled over time, while short engine-level CPU spikes may still happen."

> **Dev:** "Can the **OCR Throughput Budget** switch automatic Apple Vision OCR from accurate to fast when the machine is busy?"
> **Domain expert:** "No — the budget should run fewer or later **OCR Job** values instead of changing the **OCR Settings Selection**."

> **Dev:** "Should the **OCR Throughput Budget** only slow the worker after every frame already has a queued **OCR Job**?"
> **Domain expert:** "No — the **OCR Admission Budget** should control how much OCR debt is created, and the **OCR Execution Budget** should control how quickly that debt is processed."

> **Dev:** "Should the **OCR Execution Budget** constantly read live CPU usage to decide whether OCR can run?"
> **Domain expert:** "No — start with deterministic pacing from recording state and observed OCR job timing, then add CPU feedback later only if needed."

> **Dev:** "Should **OCR Execution Budget** pacing use one fixed delay after every **OCR Job**?"
> **Domain expert:** "No — use bounded cost-adaptive pacing so expensive recent OCR work slows the next job more than cheap recent OCR work."

> **Dev:** "Should users configure the **OCR Throughput Budget** directly?"
> **Domain expert:** "Not yet — users configure the **OCR Settings Selection**, while the throughput budget starts as a default product policy."

> **Dev:** "Can the **OCR Admission Budget** admit only one **OCR Candidate** every few seconds?"
> **Domain expert:** "No — materially changed **OCR Candidate** values should bypass cadence, while cadence only limits stable or low-change periods."

> **Dev:** "Does every visible pixel difference create an **OCR-Relevant Change**?"
> **Domain expert:** "No — OCR admission should ignore visual noise that is unlikely to change useful recognized text."

> **Dev:** "If the active app, title, browser URL, or display changes, should that count toward **OCR-Relevant Change**?"
> **Domain expert:** "Yes — foreground context changes are strong positive signals, but unchanged context does not prove the text is unchanged."

> **Dev:** "Can the **OCR Throughput Budget** use cheap visual or context signals before admitting automatic OCR?"
> **Domain expert:** "Yes — if the signal reuses already available capture or persistence data; do not add an **OCR-Relevance Probe** that costs meaningful extra CPU."

> **Dev:** "Can the **OCR Admission Budget** reject **Captured Frame Reprocessing**?"
> **Domain expert:** "No — reprocessing is explicit user intent, but the resulting **OCR Job** still runs under the **OCR Execution Budget**."

> **Dev:** "Should deferred **OCR Job** backlog run at the same pace during recording and after recording stops?"
> **Domain expert:** "No — active recording should use a conservative **OCR Execution Budget**, while **OCR Catch-Up** can process backlog more aggressively."

> **Dev:** "Can **OCR Catch-Up** later create **OCR Job** values for low OCR-value frames skipped during admission?"
> **Domain expert:** "No — catch-up drains already admitted OCR backlog; manual **Captured Frame Reprocessing** is the escape hatch for skipped frames."

> **Dev:** "Should the **OCR Throughput Budget** pick different providers when one provider is slower?"
> **Domain expert:** "No — provider choice belongs to **OCR Settings Selection**; the budget can use provider/job timing telemetry for future tuning."

> **Dev:** "Does the conservative active-recording budget mean automatic OCR is off while recording?"
> **Domain expert:** "No — automatic **OCR Job** work still happens for **OCR-Relevant Change**, but stable or low-change periods can be paced or deferred."

> **Dev:** "Is `microphoneActivityLastUnixMs` the same thing as the audio signal the inactivity policy uses?"
> **Domain expert:** "No — that timestamp is an **Audio Activity Sample**; the inactivity pause logic uses an **Audio Activity Decision** derived from threshold-qualified activity."

> **Dev:** "Should users have to choose between exact text matching and meaning-based search?"
> **Domain expert:** "No — start with **Text Search**, then move toward **Hybrid Search** so literal and semantic matches work together."

> **Dev:** "Should speaker turns define the searchable audio result?"
> **Domain expert:** "No — **Audio Transcription Span** comes from transcript timing; speaker turns can label who spoke when that annotation exists."

## Flagged ambiguities

- "pipeline" previously meant both frame intake and OCR execution; resolved: **Captured Frame Pipeline** means frame intake through batch-finalization readiness, while **OCR Job** means the recognition work for one frame.
- "CPU cap" suggested a hard process-wide ceiling; resolved: use **OCR Throughput Budget** for limiting OCR work over time.
- "fast OCR" and provider fallback were considered as optimizations; resolved: the **OCR Throughput Budget** must not change the **OCR Settings Selection**.
- "30% CPU" suggested live CPU feedback; resolved: the first **OCR Execution Budget** should use deterministic pacing rather than live process-wide CPU measurement.
- "**Scrub Preview**" was previously described as a visual representation of a **Captured Frame**; resolved: it is a disposable segment-time preview used during timeline navigation, while exact **Captured Frame** inspection goes through the exact preview path.
- "audio activity" previously referred to both raw probe output and inactivity-policy state; resolved: raw probe output is an **Audio Activity Sample**, while policy-facing threshold-qualified state is an **Audio Activity Decision**.
- "audio file" was used to mean the persisted unit for transcription; resolved: use **Audio Segment** for the time-bounded persisted recording file.
- "provider" was considered for both cloud and local transcription services; resolved: **Audio Transcription Provider** means local-only for v1.
- "search support" was used to mean both literal matching and embedding-based retrieval; resolved: **Text Search** is the first tier, while **Hybrid Search** is the direction once **Semantic Search** exists.
