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

**Keyboard Shortcuts Settings**:
The top-level settings surface for viewing shortcuts and changing user-editable **Keyboard Bindings**.
_Avoid_: capture settings keyboard section, hotkey debug panel, shortcut help overlay

**Keyboard Bindings**:
Persisted user preferences that assign editable key combinations to supported Mnema actions, including app-wide/native shortcuts and in-app shortcuts.
_Avoid_: shortcut reference, keyboard help rows, recording setting

**Pause/Resume Recording Shortcut**:
A **Keyboard Binding** action that pauses an active **Capture Session** when recording is not user-paused and resumes it when the session is in **User Capture Pause**.
_Avoid_: inactivity pause shortcut, stop/start recording shortcut, privacy mode shortcut

**Shortcut Scope**:
The active UI context in which a **Keyboard Binding** can fire, used to decide whether two bindings conflict.
_Avoid_: settings tab, settings ownership domain, native registration scope

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

**Quick Recall**:
A global, summon-anywhere overlay window (Spotlight/Raycast-style) for fast lookups and questions over retained capture without opening the main Mnema window. Hosts two connected capabilities: **Quick Search** and **Ask AI**.
_Avoid_: second dashboard, mini main window, floating settings surface

**Quick Search**:
Full-fidelity in-app search inside **Quick Recall** that reuses the existing `search_capture` engine (real app/window titles, thumbnails, exact snippets) and is not redacted, because results stay on the machine.
_Avoid_: brokered search, redacted-snippet search, second search engine

**Ask AI**:
A **Quick Recall** action that pivots from a **Quick Search** query into a PI Agent SDK conversation seeded with the **Brokered Capture Access** (redacted) results for that same query; the bridge from searching to asking.
_Avoid_: in-app raw-data assistant, separate chat window, agent with privileged app-infra access

**Answer Source**:
A brokered capture — a **Captured Frame** or an **Audio Transcription Span** — that the **Ask AI** model explicitly nominated as evidence behind its answer, by declaring it through the `reference_captures` presentation tool. **Answer Sources** are rendered below the finished answer as horizontal Screen/Audio card rows that hand off to the dashboard like a **Quick Search** result.
_Avoid_: citation, footnote, search result, consulted capture, every frame the agent read

**Quick Recall Filter Chip**:
The launcher counterpart of a dashboard **Search Refinement** chip: a removable, plain-language pill (such as `Safari`, `Microphone audio`, or `May 1 – May 30`) that a desugared **Field Operator** produces in **Quick Search**. It carries the visible, reversible scope in **Quick Recall** so typed `app:`/`source:`/date operators never persist as hidden inline text.
_Avoid_: raw operator token, dashboard chip rail, hidden inline scope, operator-syntax label

**Quick Recall Filter Value List**:
The single surface in **Quick Recall** that lists the selectable values for one **Field Operator** — distinct captured apps for `app:`, the three fixed source rows for `source:`, or preset/relative date rows for `date:`/`after:`/`before:` — rendered in the results region and fully replacing live results while active. It is reached two ways: typing the operator (caret in an un-committed operator value lands directly on that operator's list) or through the **Quick Recall Filter Picker** category step. Selecting a value commits a **Quick Recall Filter Chip** and clears the operator text.
_Avoid_: dashboard suggestion dropdown, always-on listbox, split results+list view, calendar picker, From/To date fields

**Quick Recall Filter Picker**:
The category front-door onto the **Quick Recall Filter Value List**, summoned by `/` on empty input or `Ctrl+F` anywhere. It offers three plain-language categories — App, Source, Date range — and selecting one writes that operator's stub (`app:` / `source:` / `date:`) into the input and immediately hands off to the same **Quick Recall Filter Value List** a typed operator would reach, then closes (it has served its purpose). It is a thin door only: it never selects values itself and has no second navigable list of its own. It is the zero-knowledge discovery path for users who do not know the operator syntax.
_Avoid_: dashboard suggestion dropdown, always-on listbox, body-operator menu, second value surface

## Relationships

- **Quick Recall** is a standalone overlay surface, not a **Search Entry Point** into the dashboard search modal; it has its own **Quick Search** results backed by the same `search_capture` engine.
- **Quick Search** reuses the existing `search_capture` engine rather than the redacted broker path, because **Quick Recall** results stay local to the machine and the user is reading their own data.
- **Ask AI** seeds the PI agent with **Brokered Capture Access** results for the same query the user just ran in **Quick Search**, so the user sees full-fidelity results while the cloud agent receives only redacted context.
- Selecting a **Quick Search** result opens the main Mnema window at that **Search Result Anchor** (timeline jump for **Captured Frame** results, audio player for **Audio Transcription Span** results) rather than inspecting inside **Quick Recall**; the dashboard remains the single capture *inspection* surface.
- **Quick Recall** does not duplicate exact-frame preview, OCR copy/download, or audio playback; those remain dashboard-owned and are reached by handing off to the main window.
- **Ask AI** is off by default and gated by a standing opt-in **Ask AI Setting** with a disclosure that questions send redacted capture context to PI's cloud model; enabling the setting is the consent gate rather than a time-bounded **CLI Access Grant**, and revoking is turning the setting off. See [ADR 0022](../../docs/adr/0022-ask-ai-sends-redacted-capture-context-to-cloud-agent.md).
- Once enabled, **Ask AI** does not re-prompt per query; the explicit Ask AI action is the per-use intent, preserving the fast launcher UX.
- **Ask AI** still reads capture context through **Brokered Capture Access** redaction/retention policy and appears in access audit history even though its consent gate is a setting rather than a broker grant.
- The **Ask AI** PI agent is tool-enabled: beyond the seeded query it may issue follow-up **Brokered Capture Access** queries (`search`, `timeline`, `show-text`) as tools during the conversation, bounded to that existing broker command set with no new tools, media paths, or raw app-infra access. `open` remains the app-mediated handoff to the dashboard rather than a data tool.
- The **Ask AI Setting** lives in **Access Settings**, is surfaced during onboarding, and is Rust-owned app configuration rather than recording settings or browser local storage.
- **Ask AI** holds no provider credentials in Mnema: it delegates entirely to PI's own provider auth (the user's configured PI providers via `pi login` / PI's `auth.json` / env vars), so Mnema ships no API-key field, stores no provider secrets, and runs no OAuth refresh. Mnema operates no backend and no token proxy, so neither captured context nor conversations transit Mnema servers.
- The **Brokered Capture Access** redaction/retention boundary for **Ask AI** is enforced by the broker tools executing Rust-side (thin Tauri commands wrapping existing broker policy/query code), independent of where PI's agent loop runs.
- Because PI owns provider auth, **Ask AI** requires a usable PI runtime with at least one configured provider; Mnema detects this the way **Access Settings** detects the installed `mnema` CLI, and otherwise presents **Ask AI** as unavailable with a set-up-PI pointer rather than collecting credentials. Delegating to PI's own auth implies using PI's real Node runtime (driven over RPC) rather than embedding `pi-agent-core` in the panel webview, since PI's auth only exists inside that runtime.
- **Ask AI** uses the user's already-installed, already-signed-in PI and its stored auth (`~/.pi/agent/auth.json`) as-is; because relying on stored PI auth presumes PI is installed and configured, Mnema uses the installed PI rather than bundling a PI/Node runtime, and stays all-native.
- PI exposes headless auth only for API keys (`setRuntimeApiKey`) and no-auth local models, not for consumer OAuth subscription sign-in (which is interactive-TUI-only). Rather than build provider auth, V1 **Ask AI** deliberately serves only users who have already set up PI; an in-app non-technical "sign in" waits until PI offers headless OAuth. See [ADR 0023](../../docs/adr/0023-ask-ai-delegates-auth-to-installed-pi.md).
- **Ask AI** tools operate at **All Retained Broker Scope** rather than the broker's external-client last-day default, because **Quick Search** already spans the user's full retained history and capping the agent at recent history would make it unable to answer about anything the user can search. The **Ask AI Setting** disclosure states plainly that Ask AI can draw on the full retained history (redacted); the redaction/retention guarantees do the protecting, not a scope cap.
- Summoning **Quick Recall** defaults to the **Keyboard Binding** `CommandOrControl+Alt+Space` (`⌥⌘Space`), a modifier chord like the other native-background bindings, and is user-editable.
- **Quick Recall** is ephemeral: each summon opens fresh on the **Quick Search** field, an in-progress **Ask AI** conversation lives only while the panel stays open, and V1 persists no conversation history to disk.
- **Quick Search** supports the same opt-in **Search Query Syntax** as the dashboard (**Field Operator** and **Body Match Operator** tokens), but surfaces it through a **launcher-native autocomplete** rather than the dashboard's two-tier **Search Operator Suggestion** dropdown. See [ADR 0025](../../docs/adr/0025-quick-recall-search-syntax-uses-launcher-native-autocomplete.md).
- Bringing syntax to **Quick Search** is mostly frontend surfacing: `search_capture` already parses operators from any query and returns `applied_refinements`, `residual_query`, and `parse_errors`, so the work is rendering chips, the error line, and the autocomplete rather than new engine behavior.
- **Quick Search** autocomplete has two surfaces with one job each: inline **ghost-text** completes the **Field Operator** *name* (`ap`→`p:`) and, once a value is being typed, the *top matching value* (`app:saf`→`ari`); the **Quick Recall Filter Value List** owns all value *selection*. Ghost-text accepts on `Tab` (any caret position) or `→` (only when the caret is at the end of the input, `fish`-style); it never consumes Arrow/Enter, so live results keep their keys.
- The **Quick Recall Filter Value List** appears whenever the caret sits inside an un-committed operator value (works mid-query, not just at the input start), and fully replaces live results while active so exactly one list consumes Arrow/Enter at a time. With no value typed it shows the full list and no ghost-text; with a value typed it shows the filtered list plus ghost-text on the top row, and the ghost retreats the moment the user arrows into the list. The partial operator is never sent to the backend — `search_capture` re-runs only when the committed scope changes — so a half-typed `app:saf` never reads as empty results.
- While the **Quick Recall Filter Value List** is up it owns Arrow/Enter: `↑/↓` move, `Enter` commits the highlighted value. `Esc` abandons the in-progress operator (clears the operator token at the caret, keeps the rest of the query); a second `Esc` closes **Quick Recall**. The **Ask AI** pivot (`Ctrl+Enter`) is suppressed while the list is up.
- The **Ask AI** pivot is `Ctrl+Enter` only; `Tab` is reserved for ghost-text accept rather than pivoting to **Ask AI**.
- Accepting an operator *name* (`ap`→`app:`) only completes the text and opens the **Quick Recall Filter Value List**; accepting a *value* — by `Tab`, `→`, or `Enter` on the highlighted row — commits the **Quick Recall Filter Chip**, clears the operator text, and re-runs the search on the new scope.
- The **Quick Recall Filter Value List** only commits real, result-bearing values: the `app:` list is the distinct-captured-apps set, so with no match `Enter` is a no-op and an empty/zero state shows a quiet in-list message (`No matching app` / `No apps captured yet`) rather than a blank region. A phantom filter is reachable only by deliberately typing past the list (`app:zzz` + space desugars at the backend), which then rides the normal empty-results/error treatment.
- The **Quick Recall Filter Value List** reflects active **Quick Recall Filter Chips**: because an `app:` chip and an audio `source:` chip cannot coexist, the conflicting rows are shown disabled with a one-line reason rather than auto-replacing the existing chip; the inline error line stays only for the typed-desugar conflict path. `source:screen` is compatible with an `app:` chip.
- Operator *name* ghost-text is fenced to avoid firing on ordinary words: it appears only at a token start (input start or after whitespace), needs at least two characters that uniquely point at one operator name, and self-dismisses the instant the typed prefix diverges from that name.
- Selecting a **Quick Recall Filter Picker** category writes that operator's stub (`app:` / `source:` / `date:`) into the input in one click/enter and immediately closes the picker, handing off to the matching **Quick Recall Filter Value List** (the same surface, and the same arrow/Enter navigation, a typed operator reaches — the door has no second navigable list). `Esc` from the open picker returns straight to the input (closing the picker); once the value list is up, `Esc` abandons the in-progress operator and a second `Esc` at the plain input closes **Quick Recall**. A `/` that summoned the picker stays as literal text on close so slash-leading searches remain typeable.
- A desugared **Field Operator** becomes a **Quick Recall Filter Chip**, removable by its `×` or by `Backspace` at caret position 0; removing a chip reruns **Quick Search** with the remaining scope.
- Because **Quick Recall** has no result-type tabs, a `source:`/`app:` **Quick Recall Filter Chip** narrows which section shows: an audio `source:` collapses to the Audio section, while `source:screen` or any `app:` collapses to the Screen section; the audio-`source:` + `app:` conflict surfaces on the inline error line instead.
- A **Search Query Syntax** parse error in **Quick Search** surfaces as a single inline error line under the input with a paused-results state (first error only), not the dashboard's full span-highlighted treatment; the **Ask AI** pivot stays available even while the query has a parse error.
- Pivoting to **Ask AI** inherits the full visible **Quick Search** scope: the seed **Brokered Capture Access** search stays structurally scoped by the active **Quick Recall Filter Chips**, while the question the agent receives gets a natural-language scope suffix derived from those chips (`in Safari`, `from … to …`, `in microphone audio`) that also renders as the read-only question header; **Body Match Operators** stay verbatim in the residual question text.
- Recent searches and saved/watch searches stay out of the **Quick Search** syntax work: recent searches are a follow-up of their own because they would reverse **Quick Recall** ephemerality with a persisted query-history store and a new privacy surface, and saved searches remain a separate deferred feature. **Body Match Operators** stay text-only with a small static syntax help affordance near the input rather than **Quick Recall Filter Picker** entries.
- **Ask AI** may be invoked without a prior **Quick Search** query; seeding the agent with broker results for the current query is an optimization when a query exists, not a precondition for asking.
- **Ask AI** inherits the **Brokered Capture Access** precondition that Mnema onboarding is complete.
- An **Answer Source** is model-nominated evidence, distinct from a **Quick Search** result, from seeded broker context, and from every capture the agent merely consulted: only the captures the model chose to vouch for through `reference_captures` become **Answer Sources**.
- The agent declares **Answer Sources** by passing back the opaque ids it already received from `search`/`show-text`; `reference_captures` returns no capture data to the model, so it is an app-mediated presentation signal like `open`, not a new **Brokered Capture Access** data tool, and it does not widen the redaction/retention boundary. See [ADR 0024](../../docs/adr/0024-ask-ai-uses-pi-tool-shim-over-installed-runtime.md).
- The host validates each nominated opaque id (HMAC), drops any that fail, and decodes the rest to their **Captured Frame** / **Audio Transcription Span** identity; **Answer Source** cards are then hydrated from local full-fidelity data (thumbnail by frame id, app/window/time), the same non-redacted path **Quick Search** uses, because the cards render in the user's own app and no raw frame data crosses the model boundary.
- **Answer Source** relevance is the model's nomination order, not a score: the strip renders most-relevant-first left-to-right, and no numeric relevance is shown.
- **Answer Sources** split into a horizontal Screen row (**Captured Frame** thumbnail cards) and a separate horizontal Audio row (**Audio Transcription Span** cards), never interleaved in one row, mirroring the **Quick Search** Screen/Audio sections.
- **Answer Source** cards use a dedicated horizontal card component showing thumbnail, metadata (app/window), and time, distinct from the vertical `SearchResultCard` used by **Quick Search**; they are not the search card reflowed.
- The decoded **Answer Sources** reach the panel through a single `ask_ai_source` event, held until `ask_ai_done` and emitted once; its payload carries everything the frontend needs to render and hand off — per source the kind, the **Captured Frame** / **Audio Transcription Span** id, app/window metadata, and time span — in the model's nomination order, with the `conversationId` so stale events are ignored. Thumbnails are fetched locally by frame id, not carried in the event.
- Selecting an **Answer Source** card hands off to the dashboard at its **Search Result Anchor** (frame→timeline, audio→player) and closes **Quick Recall**, exactly like selecting a **Quick Search** result; **Quick Recall** stays a launcher, not an inspection surface.
- The **Answer Sources** strip is held until the **Ask AI** answer completes and revealed once with the finished answer, below the answer prose; if the model never calls `reference_captures` there is no strip, and a repeat call replaces rather than appends (one authoritative evidence set per answer).
- Web URLs the **Ask AI** answer references are rendered as labeled Markdown links in the answer prose (opening in the browser), distinct from **Answer Source** cards; URL labeling relies on URLs already present in redacted broker text, not on new browser-metadata exposure through the broker.
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
- **Keyboard Shortcuts Settings** is a top-level settings surface with settings tab id `shortcuts`, sidebar label `Shortcuts`, page title `Keyboard Shortcuts`, and description equivalent to `View and customize Mnema keyboard shortcuts`.
- **Keyboard Shortcuts Settings** replaces the old Capture-tab global-shortcuts section rather than duplicating it.
- **Keyboard Bindings** remain the persisted **Settings Ownership Domain** for editable shortcuts even when shown inside **Keyboard Shortcuts Settings**.
- **Keyboard Bindings** include both app-wide/native actions and in-app actions; read-only shortcut help should render from the same effective bindings where possible.
- V1 editable **Keyboard Bindings** cover command shortcuts; behavior/accessibility shortcuts such as close-on-Escape and focus trapping remain fixed unless a later design explicitly expands editability.
- **Keyboard Bindings** use scoped uniqueness: two actions may share a shortcut only when their **Shortcut Scope** values cannot be active together.
- App-wide/native **Keyboard Bindings** are reserved against conflicting in-app bindings when both could fire in the same foreground context.
- Native background registration is limited to Start/Stop Recording, **Pause/Resume Recording Shortcut**, Show/Hide Mnema, and summoning **Quick Recall**; other **Keyboard Bindings** are foreground-only even when they are app-wide UI actions. Summoning **Quick Recall** is added to the native-background set because a Spotlight-style overlay must be reachable from any app, and like the other native bindings it requires a modifier chord.
- Foreground **Keyboard Bindings** may use single-key shortcuts or modifier chords, while native/background **Keyboard Bindings** require a modifier chord except for future explicitly-supported safe keys.
- Foreground **Keyboard Bindings** should remain suppressed while typing in text inputs, textareas, editable content, and equivalent interactive controls.
- **Keyboard Shortcuts Settings** should support clearing/unsetting an editable action, per-action reset to default, and a confirmed restore-all-defaults action.
- **Keyboard Shortcuts Settings** should show scoped shortcut conflicts inline and disable saving until the user resolves them; V1 should not automatically replace another action's binding.
- The **Pause/Resume Recording Shortcut** defaults to `CommandOrControl+Alt+P` (`⌥⌘P` on macOS).
- The **Pause/Resume Recording Shortcut** controls only **User Capture Pause** and must not resume an inactivity-paused session unless the user had explicitly paused it.
- A **Settings Ownership Domain** is defined by persistence and validation ownership rather than by the visual settings tab where its controls appear.
- Autosave should save one **Settings Ownership Domain** at a time so an older settings draft cannot overwrite unrelated preferences from another domain.
- Autosave may save different **Settings Ownership Domain** values concurrently, but saves within one domain should be serialized.
- Settings change notifications should preserve full canonical settings for compatibility while also identifying the changed **Settings Ownership Domain** for draft-safe frontend synchronization.
- Settings surfaces should resync draft state for the changed **Settings Ownership Domain** without resetting unrelated in-progress drafts from the full canonical settings payload.
- New domain-specific settings mutation APIs and events should return the changed **Settings Ownership Domain** together with the full canonical settings.
- Initial **Settings Ownership Domain** values are Capture Source Settings, Capture Timing Settings, Video Settings, Storage Settings, Display Settings, Metadata Settings, App Privacy Exclusion, Inactivity Settings, Processing Settings, and Developer Settings.
- Stable **Settings Ownership Domain** ids are `capture_sources`, `capture_timing`, `video`, `storage`, `display`, `metadata`, `app_privacy_exclusion`, `inactivity`, `processing`, `developer`, `keyboard_bindings`, `microphone_controller`, `app_update`, `access`, and `one_time_prompt_state`.
- Keyboard Bindings, Microphone Controller Preferences, App Update Settings, Access Settings, and **One-Time Prompt State** remain separate existing **Settings Ownership Domain** values rather than being folded into recording settings.
- `keyboard-bindings.json` is the single persisted source of truth for editable **Keyboard Bindings**, including app-wide/native actions and in-app command shortcuts.
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
- Typed **Field Operator** tokens should surface as visible, removable **Search Refinement** chips identical to the controls produced by UI refinement actions, rather than staying as hidden inline scope.
- The search input should stay plain text by default; **Search Query Syntax** is opt-in and an operator-free query behaves exactly as today's plain-text search.
- A **Search Query Syntax** parse error should surface inline in the search modal and highlight the offending operator rather than running a misleading or empty search.
- A frontend pre-parse of **Search Query Syntax** is a display optimization that may optimistically show **Field Operator** chips while typing, but the backend parse is authoritative and the frontend reconciles chips and errors to the search response.
- The frontend pre-parse should be limited to recognizing known **Field Operator** prefixes for optimistic chips and must not reimplement value validation, date parsing, or **Body Match Operator** to FTS5 translation.
- A **Search Operator Suggestion** dropdown is two-tier: it first offers operator names (`app:`, `source:`, `date:`, `after:`, `before:`) for discovery, then the values for the chosen operator.
- Selecting a **Search Operator Suggestion** value commits the corresponding **Search Refinement** chip and removes the operator text from the input, converging the dropdown with typed **Field Operator** desugaring.
- `date:`, `after:`, and `before:` **Search Operator Suggestion** values offer preset and relative tokens with a typed-date hint rather than a custom date-time picker.
- A **Search Operator Suggestion** dropdown captures arrow, Enter, and Escape keys while open, where Enter selects the highlighted value and Escape closes only the dropdown, and falls back to submit and close-modal behavior when the dropdown is closed.
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
- "the search modal remains the single result surface" was written when the dashboard modal was the only search surface; resolved: that rule governs in-dashboard **Search Entry Point** behavior (contextual searches prefill the modal rather than spawning their own result surfaces) and does not forbid the standalone **Quick Recall** overlay. The dashboard remains the single capture *inspection* surface; **Quick Recall** is a separate *launcher* surface that hands off to it.
