# Linux browser URLs come from the accessibility bus, behind one opt-in setting

## Status

Accepted (2026-11-01) for the Linux port; not yet implemented.
[ADR 0013](0013-browser-integrated-sensitive-capture-v2.md) stands: no browser extension in v1.
Extends [ADR 0039](0039-gecko-browser-active-tab-url-via-accessibility-api.md) (Gecko URLs via the
accessibility API, no guessing) to every browser on Linux. URLs stay metadata only, guarded at read
time by [ADR 0038](0038-brokered-browser-urls-are-read-time-guarded-host-path-raw-url-opens-app-mediated.md).
Source: `docs/linux-port-report.md` §4.3, §10 decision 7.

## Context

On macOS, Mnema reads the active tab's URL with AppleScript for Chromium/WebKit browsers and with
the Accessibility API for Gecko browsers (`crates/capture-metadata/src/lib.rs:282`,
`BrowserUrlStrategy`). The URL is metadata only; it never drives privacy decisions
(`collect_browser_url_for_privacy: false`, `crates/capture-metadata/src/lib.rs:260-269`).

Linux has no AppleScript. The only browser-neutral source is the desktop accessibility bus
(AT-SPI2), read with the pure-Rust `atspi` crate on the zbus already in the lockfile. It needs
desktop accessibility to be on, and turning it on is a desktop-wide change:

- **Firefox and Zen** watch `org.a11y.Status.IsEnabled` live and start exporting their tree when it
  flips, with no restart. They publish the page URL as `DocURL`.
- **Chromium-family browsers** check only once, at startup (environment variables, then
  `IsEnabled`, then the GNOME `toolkit-accessibility` key). They publish the URL as `URI`.
  Unverified: whether the page's tree, and with it the URL, appears without
  `--force-renderer-accessibility`. That needs a hardware test.
- With accessibility on, every GTK, Qt and Electron app exports its UI tree onto a bus any
  same-user process can read. It costs some performance and widens what other local programs can
  see.

There is no permission dialog for any of this, so Mnema must ask before it changes anything.

## Decision

**1. One setting, "Read browser URLs", off by default.** Before it writes anything, Mnema explains
what it does in plain words: it turns on desktop accessibility, which other apps can also see, and
Chrome-type browsers need one restart.

**2. Turning it on** sets `IsEnabled` and the `toolkit-accessibility` key, and drops
`ACCESSIBILITY_ENABLED=1` into `~/.config/environment.d/`. Mnema records exactly which of these it
changed.

**3. Turning it off undoes exactly what Mnema changed** and nothing else. If accessibility was
already on before (a screen-reader user, say), Mnema leaves it on.

**4. One AT-SPI reader serves every browser** behind the existing `BrowserUrlStrategy::Accessibility`
path. It mirrors the macOS Gecko reader: find the focused element, walk up to the outermost web
document, read `DocURL` or `URI`. Same time budgets; a failed read leaves the URL blank, never
guessed (ADR 0039).

**5. Browser support at launch:**

- **Firefox and Zen:** full parity with macOS.
- **Chromium family:** supported only if the hardware test shows the URL appears after a normal
  restart, without `--force-renderer-accessibility`. If it doesn't, Settings says "Reading URLs
  from Chrome-type browsers isn't available on Linux", and a browser extension is reconsidered in a
  new ADR.

**6. The browser catalog gains Linux keys:** `.desktop` ids and Flatpak app ids instead of macOS
bundle ids.

## Considered options

- **A browser extension posting to a local port** (the ActivityWatch model). Works in Snap and
  Flatpak browsers and on every compositor with no accessibility cost, but it overturns ADR 0013,
  needs store accounts and Firefox signing, and opens a pairing-token attack surface next to the
  broker. Held back as the answer if the Chromium test fails.
- **Extension plus a native messaging host.** Breaks with Snap and Flatpak browsers, including
  Ubuntu's default Firefox Snap.
- **Turn accessibility on silently.** Rejected: it changes the whole desktop and what other apps can
  read.
- **Parse window titles.** Rejected by ADR 0039: titles hold the page title, not the URL.

## Consequences

- Linux users get no browser URLs unless they opt in. Timeline and search work without them.
- Chromium-family support depends on one hardware result; the copy for both outcomes is decided now.
- If GNOME locks down tree reading (the Wayland-native "Newton" accessibility work, or allowlisting
  readers the way GNOME 48 allowlisted key-event listeners), the reader could stop working. The
  setting would then report itself unavailable instead of failing silently.
- On GNOME, AT-SPI may also be a usable front-app source; that is unverified and tracked with the
  front-app work, not here.
