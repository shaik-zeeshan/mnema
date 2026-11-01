# On Linux, privacy exclusion pauses the screen while an excluded app is in front, and a desktop Stop is a normal event

## Status

Accepted (2026-11-01) for the Linux port; not yet implemented.
Amends [ADR 0006](0006-use-app-level-live-privacy-exclusion-only.md) for Linux only: its promise
that excluded apps never reach the recording holds on macOS and on Linux desktops rated **Full**
below, and nowhere else. macOS is unchanged.
Related: [ADR 0021](0021-recover-from-display-unavailable-as-transient-liveness.md) (transient
liveness), [ADR 0052](0052-system-audio-is-an-independent-capture-family-on-core-audio-process-taps.md)
(audio is independent of the screen). Source: `docs/linux-port-report.md` §4.1, §4.2, §7, §10
decisions 3, 4, 8 and 12.

## Context

On macOS, ScreenCaptureKit's content filter removes excluded apps before a frame exists (ADR 0006).
On Linux, every Wayland screen recording goes through the xdg-desktop-portal ScreenCast API. Its
`SelectSources` call offers source types, multiple sources, cursor mode and `persist_mode`. It has
no way to leave apps or windows out of a monitor stream.

What each desktop offers instead:

- **KDE Plasma 6.6+:** "Hide from ScreenCast" (`ExcludeFromCapture`). Unverified: whether an app can
  set it from a KWin script, and whether it applies to a monitor cast.
- **Hyprland 0.50+:** the `noscreenshare` window rule, settable live. Unverified with xdph.
- **Sway and other wlroots compositors:** nothing at capture. The front app is knowable.
- **X11:** no compositor in the way. Mnema can black out excluded windows' visible rectangles.
- **GNOME:** nothing at capture, and an app can't even learn which app is in front without a Shell
  extension.
- **COSMIC:** nothing at capture; the front app needs COSMIC's own `cosmic-toplevel-info` protocol.

A second Linux fact: GNOME and KDE show a screen-sharing indicator with a Stop control for as long
as a portal cast runs. The user can end Mnema's screen capture from outside Mnema at any time. It
can't be hidden.

Today the whole privacy refresh pipeline is macOS-only (`apps/desktop/src-tauri/src/native_capture/privacy.rs:1`),
and the tray's "Exclude Current App" tells Linux users it is "available only on macOS"
(`apps/desktop/src-tauri/src/status_bar.rs:684-691`).

## Decision

**1. Linux privacy exclusion is a capability ladder, detected per session and stated plainly.**

| Capability | Where | What it does | Settings wording |
|---|---|---|---|
| **Full** | KDE 6.6+ and Hyprland, only if the hands-on spike proves app-settable hiding works on monitor casts | Excluded windows are hidden at capture, as on macOS | Same as macOS |
| **FocusGated** | GNOME with the Mnema extension, KDE otherwise, wlroots, COSMIC | Screen frames pause while an excluded app is in front and resume when it leaves | "Excluded apps pause screen recording while they're in front. If one is visible behind another window, it is recorded." |
| **Masked** | X11 | Excluded windows' visible areas are blacked out before encode | "Excluded windows are blacked out." |
| **Unavailable** | No front-app source (GNOME without the extension) | Nothing | "This desktop doesn't let apps hide other apps from recordings. Use Pause or Delete Recent Capture." |

The tray's "Exclude Current App" is hidden whenever the front app is unknown. If a front-app source
stops reporting mid-session (the GNOME extension is disabled, the KWin script is unloaded), the
capability drops to Unavailable and Settings says so. It never keeps claiming FocusGated.

**2. The FocusGated pause is a new screen-only, transient suspension kind.** It does not reuse
`CaptureSuspensionKind::PrivacyFilter`, which means "a content-filter apply failed", caps recovery
and escalates to a manual restart (`apps/desktop/src-tauri/src/native_capture/runtime.rs:36-51`).
The new kind behaves like `DisplayUnavailable` (ADR 0021): it never gives up, it resumes on its own
when the excluded app leaves the front, and it touches only the screen. The microphone and system
audio follow their own rules (ADR 0052; per-app audio exclusion is a separate Linux design, report
§4.5).

**3. Front-app sources, one trait, picked at startup.** A KWin script on KDE; a Mnema GNOME Shell
extension on GNOME; `zwlr_foreign_toplevel_management_v1` plus compositor IPC for the PID on
wlroots; `cosmic-toplevel-info` on COSMIC (built in v1, because Pop!_OS 24.04 ships COSMIC by
default); `x11rb` on X11. The app's `.desktop` id is stored in the existing `bundle_id` fields.

**4. The GNOME extension is bundled and side-loaded in v1.** Settings shows "App tracking needs the
Mnema GNOME extension: Install (requires log out)". One click copies it to
`~/.local/share/gnome-shell/extensions` and enables it. Publishing to extensions.gnome.org comes
later. The per-GNOME-release `shell-version` bump goes on the release checklist.

**5. A Stop from the desktop's indicator is a normal event, not an error.** When the portal session
closes because the user pressed Stop, Mnema stops screen capture cleanly, commits the finished
segment, and shows "Screen recording stopped by your desktop. Audio is still recording." with
**Resume screen** and **Pause everything** buttons. The Stop ends the screen only: the microphone and
system audio keep recording (ADR 0052), and the message says so, so nobody believes they are fully
off when they are not. Resume screen re-runs the portal start with the saved restore token. Mnema never retries on its
own, because a silent retry would just raise the consent dialog again. Onboarding says up front:
"Your desktop shows a screen-sharing icon the whole time Mnema records. That's expected."

## Considered options

- **Call it Unavailable everywhere except Full desktops.** Honest, but it leaves GNOME, Sway and
  COSMIC users with no exclusion at all when a weaker, clearly worded one is cheap.
- **Claim parity with macOS.** Rejected: an excluded window visible in the background is recorded
  under FocusGated. Over-promising here is a privacy bug.
- **Hide or suppress the sharing indicator.** Not possible on GNOME or KDE, so the product has to
  treat the indicator and its Stop button as part of normal use.

## Consequences

- ADR 0006's guarantee now depends on the platform. Every privacy surface (Settings, onboarding,
  tray, docs, SUPPORTS.md) must show the capability for the current desktop, not one shared claim.
- GNOME users must install an extension and log out once to get any exclusion. Every GNOME release
  needs an extension update; a missed bump silently drops users to Unavailable (made visible by
  rule 1).
- Five front-app backends to maintain. The wlroots and COSMIC ones share most of their Wayland code.
- Self-capture filtering on Linux tags frames whenever a Mnema window has focus, because GNOME
  frames carry no app id without the extension (the macOS filter matches by app id in
  `crates/app-infra/src/user_context/capture_source.rs:335-346`).
- Hands-on unknowns that can move a desktop between rungs: KDE `ExcludeFromCapture` from a script on
  monitor casts, Hyprland `noscreenshare` with xdph, and restore-token behaviour across reboot, lock
  and monitor changes on each desktop (including COSMIC).
- A desktop Stop leaves audio recording (owner decision, 2026-11-01). Someone who expected Stop to
  end everything has to read the message to learn audio is still on; Pause everything is one click.
