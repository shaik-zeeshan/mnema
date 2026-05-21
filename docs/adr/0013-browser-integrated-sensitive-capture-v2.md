# Add browser-integrated Sensitive Capture V2

Mnema will add browser-integrated **Credential Entry Capture Suspension** as a V2 **Capture Safety Suspension** that pauses all requested capture sources when trusted OS, accessibility, native framework, or first-party browser-extension secure-entry signals indicate credential entry. This supersedes ADR 0010's browser stance: native macOS Accessibility-only detection is not reliable enough for browser credential entry, so Safari/Chromium support may use a **Browser Secure-Entry Signal** that inspects focused DOM control structure and emits only coarse secure-entry state. Browser integration may also provide a separate **Browser Metadata Signal** for **Browser Metadata Collection**, but credential-entry suspension runs independently of metadata settings and must not consume browser metadata as a safety input.

## Considered Options

Native AX-only keeps implementation smaller and avoids browser extension permission prompts, but it is not reliable enough for browser credential entry. A first-party Safari/Chromium extension can observe focused DOM control structure more directly, but it requires explicit website access and native app pairing. Mnema chooses the browser-extension path for browser credential entry while keeping native Accessibility and native framework secure-entry signals as primary coverage for non-browser apps and native secure text fields.

## Consequences

The feature is enabled by default after first-run disclosure and remains configurable in Settings. Missing Accessibility, native, browser-extension, native-messaging, or website permission is shown as unavailable rather than silently degrading. Suspension starts immediately when a trusted signal appears, resumes only after a short clear delay, and creates a visible **Capture Safety Gap** only when an active recording is suspended by the trigger. Suspension creates a recording boundary: active segments are finalized on suspend, no requested sources record while suspended, and new segments start on resume; if the boundary cannot be finalized safely, the **Recording Lifecycle** fails closed by keeping capture suspended and reporting the failure clearly. V2 has no one-click record-anyway override while the trusted secure text-entry signal remains active. The gap stores only coarse reason and time bounds, not app identity, window title, URL, domain, field label, recognized text, transcript, screenshot preview, or other content-bearing data.

The first-party browser integration has two separate contracts. The **Browser Secure-Entry Signal** is live-only and may inspect focused DOM control structure, but it emits only coarse secure-entry state and never URL, title, domain, page text, field value, field label, selector, form action, screenshot, OCR, or media-derived data. The **Browser Metadata Signal** is governed by metadata settings and may report active-tab URL metadata only under the existing browser URL metadata modes; it must not report DOM text, selected text, page title, field labels, placeholders, selectors, form actions, favicon, screenshots, page summaries, credential-control structure, or credential-field state. Browser metadata may feed timeline/search context, but it must not drive the **Live Privacy Filter** or **Credential Entry Capture Suspension**.

Reliable browser-integrated credential-entry suspension requires **Browser Integration Coverage**: the extension is installed, paired to Mnema, native messaging is available, the browser is supported, and the extension has all-sites website access. Per-site extension access may provide partial coverage, but it must be labeled partial rather than reliable browser credential-entry suspension. Missing extension installation, pairing, native messaging, website permission, browser support, or page support makes browser-integrated credential-entry suspension unavailable for that browser or page; Mnema must not use browser `activeTab`-style temporary access, URL/title metadata, OCR, screenshots, or classifiers as fallback safety signals.

Onboarding includes a dedicated browser extension setup step for browser-integrated Sensitive Capture V2, but the step is non-blocking and skipping it must not block first recording. Skipping browser extension setup makes browser-integrated **Credential Entry Capture Suspension** and **Browser Metadata Collection** partial or unavailable according to **Browser Integration Coverage**. Settings and onboarding show coverage per source family, including native apps, Safari, and Chromium browsers, with reliable, partial, and unavailable states.

Product copy should promise that Mnema pauses during supported browser credential entry when browser integration coverage is available, not that passwords are never recorded. Copy should disclose that Mnema does not inspect field values and does not use URL/domain/title/page guessing for credential-entry suspension, and that unsupported browsers/pages, denied website access, extension disconnects before detection, and capture before a trusted signal arrives may still be recorded. Browser-integrated Sensitive Capture V2 should not be described as redaction, private browsing protection, website privacy, or password manager mode.

For browser pages, active credential entry is defined by focused DOM control structure: a focused password control, a focused editable text control in the same form or nearest credential group as a password control, or a focused text-like control with standards-based credential autocomplete tokens such as `username`, `current-password`, `new-password`, or `one-time-code` when paired with credential structure. The extension may inspect control type, focus, form/group relationship, and standards-based autocomplete tokens, but not visible labels, placeholders, button text, URL, title, copied field values, OCR, or classifier guesses. One-time code entry is included even though many codes are not reusable after successful submission, because retained capture may contain the code before it is used or before it expires.

Suspension starts immediately when any trusted safety source reports active credential entry. Resume requires every currently tracked active safety source to report clear and remain clear for a 1500ms debounce window. If a safety source that was active disconnects, becomes unavailable, or misses heartbeat before reporting clear, Mnema stays suspended and reports the coverage failure; a browser or page that is merely uncovered before any active signal is shown as uncovered rather than causing automatic suspension. **Capture Safety Gap** records are coalesced across secure-entry flicker until final resume, and **Capture Safety Suspension** must not auto-resume over **User Capture Pause**.

Browser-integrated **Credential Entry Capture Suspension** pauses screen, system audio, and microphone together when those sources are requested. It creates one **Capture Safety Gap** for the suspension, not per-source gaps, and resumes only the sources that were requested before the safety suspension; unrequested, stopped, or user-paused sources must not be started by safety resume. If any requested source cannot finalize safely at the suspension boundary, the **Recording Lifecycle** fails closed for the whole suspension rather than continuing other sources as if protected.

Durable audit stores only non-content gap and coverage facts. A **Capture Safety Gap** may persist start/end time, coarse trigger category, coarse source family, and coarse terminal status; **Browser Integration Coverage** audit may persist non-content coverage changes such as installed, paired, native messaging available, website permission available, supported, unsupported, and unavailable. Mnema must not persist the per-event secure-entry stream, safety-tied URL/title/domain, field type history, selector, form identity, frame identity, extension tab id, app/window title, page labels, placeholders, values, screenshots, OCR, transcript, or media-derived data. Live debug may show current coverage and the latest non-content safety reason, but should not retain a detailed per-focus browser event timeline.

Current browser extension setup, pairing, and coverage state belongs in app config as app/runtime configuration. Durable **Browser Integration Coverage** audit events belong in the **Encrypted Capture Index** when they are shown alongside the capture timeline or used to explain **Capture Safety Gap** records. Low-level browser integration diagnostics and per-event extension errors remain logs or live debug only, without content-bearing identifiers.

Browser integration events terminate in a Rust-owned **Browser Integration Runtime**, not in Svelte state. That runtime validates event schema, pairing/authenticity, sequence, heartbeat, browser support, and permission/coverage state before updating Mnema state. The **Recording Lifecycle** consumes only summarized browser safety state from the runtime, while **Browser Metadata Collection** consumes only sanitized metadata state. Svelte may configure browser integration and display coverage/debug state, but must not own safety debounce, fail-closed decisions, or recording pause/resume decisions.

**Browser Integration Pairing** requires an explicit per-install pairing secret in addition to native messaging host registration. Pairing secrets belong outside `saveDirectory`, preferably in platform secret storage, and must not be persisted in the capture index or logs. The browser extension stores only the token needed to authenticate to Mnema's native host, Settings supports rotating and revoking pairing, and browser integration events without valid pairing are treated as `extension_not_paired`. If pairing is lost while a secure-entry signal was active, **Credential Entry Capture Suspension** fails closed until user-visible recovery.

Native Accessibility and native framework secure-entry signals remain primary for non-browser apps and native secure text fields. **Browser Secure-Entry Signal** coverage is primary for Safari/Chromium browser credential entry. Native Accessibility may supplement browser suspension only when it reports a trusted secure-entry signal, but browser credential-entry coverage should not require native Accessibility and native Accessibility should not override an active browser signal. Missing **Browser Integration Coverage** means browser credential-entry coverage is unavailable or partial even if native Accessibility permission is present; missing native Accessibility permission means native-app secure-entry coverage is unavailable, but supported-browser extension coverage may still work.

The app-facing browser safety event contract is:

```ts
type BrowserSecureEntrySignalV1 = {
  version: 1;
  kind: "browser_secure_entry_signal";
  browserFamily: "safari" | "chromium";
  state: "active" | "clear" | "unavailable";
  reason:
    | "focused_password_control"
    | "focused_related_credential_control"
    | "focused_autocomplete_credential_control"
    | "no_focused_credential_control"
    | "extension_not_installed"
    | "extension_not_paired"
    | "native_messaging_unavailable"
    | "website_permission_unavailable"
    | "browser_unsupported"
    | "page_unsupported";
  observedAtUnixMs: number;
  sequence: number;
};
```

The app-facing contract must not include tab id, frame id, URL, domain, title, selector, field type, autocomplete token, or browser-specific permission objects. Those details may exist only transiently inside the extension or **Browser Integration Runtime** when needed to compute the fixed non-content state.

The app-facing browser metadata event contract is:

```ts
type BrowserMetadataSignalV1 = {
  version: 1;
  kind: "browser_metadata_signal";
  browserFamily: "safari" | "chromium";
  state: "available" | "unavailable";
  reason:
    | "active_tab"
    | "metadata_disabled"
    | "url_mode_off"
    | "extension_not_installed"
    | "extension_not_paired"
    | "native_messaging_unavailable"
    | "website_permission_unavailable"
    | "browser_unsupported"
    | "page_unsupported";
  observedAtUnixMs: number;
  sequence: number;
  url?: string;
};
```

The URL field is present only when metadata is enabled and browser URL metadata mode is not off. Mnema still sanitizes browser URL metadata before persistence according to `BrowserUrlMode`; the extension should avoid sending URL when metadata is disabled or URL mode is off. The app-facing metadata contract must not include page title, selected text, DOM text, favicon, selector, field state, tab id, or frame id.

For **Browser Metadata Collection**, Mnema should prefer **Browser Metadata Signal** URL metadata when browser integration is paired and covered. Existing native browser URL probing may remain as a metadata-only fallback under metadata settings and browser URL metadata mode, but it must not affect **Browser Integration Coverage**, enable **Credential Entry Capture Suspension**, or imply browser credential-entry coverage. Debug should label browser URL metadata source as `browser_extension`, `native_browser_url_probe`, or `unavailable`.
