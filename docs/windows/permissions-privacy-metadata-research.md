# Windows Permissions, Privacy, and Metadata Research

_Last researched: 2026-05-26_

This note records Windows 11 research for the Mnema surfaces that currently depend on macOS permissions, ScreenCaptureKit privacy filters, NSWorkspace/CoreGraphics metadata, AppleScript browser URL probes, and macOS bundle IDs.

## Scope checked in this repo

- `crates/capture-screen/src/lib.rs`: screen permission, system-audio permission, ScreenCaptureKit app exclusion filters, frame activity.
- `crates/capture-microphone/src/lib.rs`: AVFoundation microphone permission and capture.
- `apps/desktop/src-tauri/src/native_capture.rs`: `get_capture_support`, `get_capture_permissions`, `request_capture_permission`, `open_capture_privacy_settings`, app-candidate/icon commands, browser URL support debug.
- `apps/desktop/src-tauri/src/native_capture_metadata.rs`: active app/window metadata, browser URL probe, metadata notifier.
- `apps/desktop/src-tauri/src/native_capture/privacy.rs`: live privacy refresh and ScreenCaptureKit filter application.
- `apps/desktop/src-tauri/src/privacy_redaction_sources.rs`: App Privacy Exclusion settings mutation, currently named `bundleId`.
- `apps/desktop/src-tauri/src/sensitive_capture_recommendations.rs`: macOS-sensitive-app catalog and Known Browser catalog.
- `apps/desktop/src-tauri/Info.plist` and `Entitlements.plist`: macOS microphone/speech usage strings and audio-input entitlement.
- Svelte surfaces using `bundleId`/`appBundleId`: onboarding permissions, privacy settings, timeline app grouping/icons, debug permissions/privacy.

Design constraints from ADR 0006, ADR 0008, and ADR 0013 still apply: live privacy is app-only, browser metadata is metadata-only, and no browser extension/native-host credential-entry suspension should be added without a new ADR.

## Short recommendation

1. **Permissions:** model Windows permissions by capability, not by copying macOS TCC.
   - Screen: WGC picker consent or programmatic-capture access where applicable; no macOS-style persistent screen-recording permission pane.
   - Microphone: WASAPI capture plus Windows microphone privacy status/deep link.
   - System audio: WASAPI loopback has no normal user permission prompt; disclose clearly.
2. **Privacy:** do **not** promise ScreenCaptureKit-style arbitrary app exclusion for full-monitor capture on Windows.
   - Use `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)` only to hide Mnema-owned windows.
   - Hide/degrade “Exclude Current App” on Windows until an equivalent OS-backed exclusion exists, except possibly for selected-window capture semantics.
3. **Metadata:** implement active-window metadata with Win32 foreground-window/process APIs; treat browser URL probing as optional, metadata-only, and off/sanitized by default until reliability is proven.
4. **Identity:** replace macOS-only `bundleId` assumptions with a platform-neutral app identity model before enabling Windows privacy settings.
   - Packaged apps: package family name and/or AppUserModelID.
   - Unpackaged desktop apps: canonical executable path is the strongest practical identifier; process name is only a weak catalog hint.
5. **UX:** make Windows support capability-driven. If live app exclusion is unsupported, say so and rely on Pause Capture, Browser Capture Disclosure, and Delete Recent Capture recovery rather than silently downgrading privacy.

## Current macOS surface to Windows 11 alternative

| Mnema surface today | macOS implementation | Windows 11 alternative / decision |
| --- | --- | --- |
| Screen permission state/request | `CGPreflightScreenCaptureAccess` / `CGRequestScreenCaptureAccess`; opens `x-apple.systempreferences:...Privacy_ScreenCapture` | WGC `GraphicsCaptureSession::IsSupported`; picker-based capture is user consent. Programmatic/borderless capture uses `GraphicsCaptureAccess.RequestAccessAsync` and Settings URIs for packaged capability paths. |
| Microphone permission state/request | AVFoundation `authorization_status_for_media_type(audio)` and `request_access` | WASAPI capture plus WinRT `DeviceAccessInformation::CreateFromDeviceClass(DeviceClass::AudioCapture)` for status where it works; open `ms-settings:privacy-microphone`. Must test with unpackaged Tauri + WASAPI. |
| System-audio permission | Shares macOS screen permission because ScreenCaptureKit supplies system audio | WASAPI loopback from render endpoint. No normal OS permission prompt; protected/DRM audio can be blocked. Consider Application Loopback Audio for process-tree include/exclude later. |
| Live App Privacy Exclusion | ScreenCaptureKit `SCContentFilter` excluding bundle IDs | No equivalent proven for arbitrary app exclusion in full-monitor WGC/DXGI capture. Own-window exclusion only via `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)`. |
| Exclude Current App | Active macOS bundle ID + live app exclusion | Hide/disable on Windows for monitor capture unless implemented as “exclude Mnema windows” or selected-window-specific behavior. |
| Active app/window metadata | NSWorkspace active app + CoreGraphics window list | `GetForegroundWindow`, `GetWindowThreadProcessId`, `OpenProcess`, `QueryFullProcessImageNameW`, `GetWindowTextW`; optional AppUserModelID/package identity. |
| Metadata notifier | NSWorkspace activation/space/app launch notifications | `SetWinEventHook(EVENT_SYSTEM_FOREGROUND, ...)` plus selective polling; optional window name-change/show/hide hooks. |
| Browser URL metadata | AppleScript for supported browsers | Possible UI Automation probe of active browser address bar; not a privacy control; no extension/native host per ADR 0013. Reliability and browser coverage need a prototype. |
| App candidate list/icons | NSWorkspace running apps + `/Applications` bundles + bundle icons | Running apps from foreground/visible windows and process paths; installed apps from Start Menu shortcuts / packaged apps later; icons from executable or shell image APIs. |
| Recommended exclusions | macOS bundle-id catalog | New Windows catalog keyed by selected app identity model; keep finite sensitive categories only. |
| Known browser catalog | macOS browser bundle IDs | Windows browser descriptors keyed by executable path/name plus package/AppUserModelID where available. |
| Timeline app grouping/icons | `appBundleId` + app name | Needs platform-neutral field or compatibility mapping; avoid labeling Windows executable identity as “bundle” in UI. |

## Windows 11 permission details

### Screen capture

Primary runtime capture should still use Windows Graphics Capture (WGC), as covered by `docs/windows/runtime-capture-research.md`.

Important permission/privacy facts from Microsoft docs:

- WGC is for acquiring frames from a display or application window.
- The normal picker flow uses secure system UI for the user to select a display/window, and Windows draws a yellow notification border around actively captured items.
- `GraphicsCaptureSession.IsSupported()` is the support check.
- `GraphicsCapturePicker` exists from Windows 10 1803 and must be initialized with an owner window handle in desktop apps.
- Win32 interop APIs `IGraphicsCaptureItemInterop::CreateForWindow(HWND, ...)` and `CreateForMonitor(HMONITOR, ...)` are documented for Windows 10 1903 / build 18362+.
- Newer `GraphicsCaptureItem.TryCreateFromWindowId/DisplayId` APIs require `GraphicsCaptureAccess.RequestAccessAsync(GraphicsCaptureAccessKind.Programmatic)` and the `graphicsCaptureProgrammatic` package capability. They are Windows 10 build 20348+ APIs.
- `GraphicsCaptureSession.IsBorderRequired = false` requires user consent through `GraphicsCaptureAccessKind.Borderless` and the `graphicsCaptureWithoutBorder` package capability. Mnema should keep the border by default.
- Settings deep links exist for graphics-capture programmatic/borderless capability pages: `ms-settings:privacy-graphicscaptureprogrammatic` and `ms-settings:privacy-graphicscapturewithoutborder`.

Suggested Mnema mapping:

- Picker capture: no separate “screen permission” prompt command; `request_capture_permission("screen")` can open the picker or explain that permission is granted by choosing a capture target.
- Direct Win32 `HWND`/`HMONITOR` capture: treat as a separate “programmatic capture” capability and show explicit Mnema UX before start. Verify whether the chosen packaging path is unpackaged Win32 interop or packaged WinAppSDK capability-gated APIs.
- `CapturePermissionState` should not pretend Windows has macOS TCC. Use support status + consent mode, or add richer Windows-specific permission detail if needed.

### Microphone

Recommended capture backend remains WASAPI from `eCapture` endpoints.

Windows APIs/UX to use:

- Use WinRT `DeviceAccessInformation::CreateFromDeviceClass(DeviceClass::AudioCapture)` to query `CurrentStatus` and subscribe to `AccessChanged` where it works for desktop apps.
- Map `DeviceAccessStatus::Allowed` to Mnema granted; `DeniedByUser` / `DeniedBySystem` to denied; `Unspecified` to unknown/not determined.
- Open `ms-settings:privacy-microphone` when access is denied or unknown.
- If Mnema is ever MSIX-packaged, declare the microphone capability/device capability in the package manifest. NSIS/MSI/unpackaged builds do not use macOS-style usage strings.
- Still test actual WASAPI behavior under Windows 11 privacy toggles; status APIs and WASAPI error behavior can differ by packaged vs unpackaged app and by device policy.

### System audio

Recommended backend is WASAPI loopback from an `eRender` endpoint.

Key facts:

- WASAPI loopback captures the stream being played by a render endpoint and is initialized with `AUDCLNT_STREAMFLAGS_LOOPBACK`.
- It captures the system mix and does not require screen capture.
- Windows 10 1703+ supports event-driven loopback clients.
- Protected/DRM audio may not be captured by loopback.
- There is no normal Windows privacy permission category equivalent to microphone/screen for system loopback audio.

Suggested Mnema mapping:

- `systemAudio` permission should be “supported/available” rather than “granted by OS prompt”.
- UX should disclose that system audio capture is controlled by Mnema settings, not a Windows permission dialog.
- Later privacy improvement: Application Loopback Audio can include or exclude a process tree (`includetree` / `excludetree`) and requires Windows 10 build 20348+. This is audio-only and does not solve screen app exclusion.

## Live privacy and App Privacy Exclusion

### What Windows can do

- `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)` can hide **Mnema-owned top-level windows** from capture on Windows 10 2004+.
- The target window must belong to the current process.
- It is intended for windows such as recording controls that should not appear in capture.
- Microsoft explicitly notes display affinity is not DRM/security; it works only for a defined set of OS capture paths and DWM-composed desktop.

### What Windows does not appear to provide

No researched Windows API provides ScreenCaptureKit-style “capture the monitor but exclude arbitrary app identities” for full-screen WGC/DXGI capture.

Avoid treating these as equivalent privacy guarantees:

- Redacting rectangles after capture by enumerating excluded windows. This can leak during race conditions, occlusion/transparency, shadows, animations, protected/elevated windows, display changes, and z-order mistakes.
- Capturing only non-excluded windows and compositing a desktop substitute. This changes Mnema’s screen semantics and is not a faithful monitor recording.
- Browser URL/title/private-window detection. ADR 0006/0008/0013 reject those as live privacy guarantees.

Product implication:

- On Windows monitor capture, `privacy.excludedApps` should be shown as unsupported/degraded unless we have an OS-backed implementation.
- “Exclude Current App” should be hidden or changed to “Hide Mnema windows from capture” for Windows monitor capture.
- Sensitive Capture Protection V1 on Windows should emphasize Browser Capture Disclosure, Pause Capture, and Delete Recent Capture recovery.

## Windows app identity model

Current shared types use `bundleId` (`ExcludedAppEntry.bundle_id`, `FrameMetadataSnapshot.app_bundle_id`, timeline icon lookup). Windows needs a platform-neutral model before privacy settings are enabled.

Recommended identity precedence:

1. **Packaged app identity** when available: Package Family Name (PFN), package full name, and AppUserModelID.
2. **Explicit AppUserModelID** for windows/apps that set one.
3. **Canonical executable path** for unpackaged desktop apps. Normalize case, resolve symlinks/reparse points where practical, and store display name separately.
4. **Executable file name / process name** only as a weak matching hint for catalogs and search, not as a privacy-rule identity by itself.

Useful APIs:

- `GetForegroundWindow` -> active `HWND`.
- `GetWindowThreadProcessId` -> owning PID.
- `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)` + `QueryFullProcessImageNameW` -> executable path.
- `GetApplicationUserModelId` -> process AppUserModelID where present.
- `GetPackageFamilyName` -> PFN for packaged processes; returns `APPMODEL_ERROR_NO_PACKAGE` for unpackaged processes.
- `SHGetPropertyStoreForWindow` + `System.AppUserModel.ID` -> explicit AppUserModelID on a window where available.
- `GetWindowTextW` -> top-level window caption for cross-process windows, not arbitrary child/control text.

Suggested data shape for a future migration:

```text
AppIdentity {
  platform: "macos" | "windows" | "linux",
  kind: "bundle_id" | "package_family_name" | "app_user_model_id" | "executable_path",
  value: string,
  display_name: string,
  secondary_values?: { process_name?, executable_path?, package_family_name?, app_user_model_id? }
}
```

Compatibility path:

- Keep reading old `bundleId` settings for macOS.
- Do not store raw Windows paths in a field labeled `bundleId` long-term.
- Timeline/UI can continue displaying “App” while backend carries platform/kind.

## Active metadata collection

### Active app/window snapshot

A Windows `NativeActiveWindowSnapshot` equivalent can be built with:

1. `GetForegroundWindow()`.
2. `GetWindowThreadProcessId(hwnd, &pid)`.
3. Process path and identity from `OpenProcess`, `QueryFullProcessImageNameW`, `GetApplicationUserModelId`, `GetPackageFamilyName`.
4. Window title from `GetWindowTextW`.
5. Optional display/window bounds from `DwmGetWindowAttribute(DWMWA_EXTENDED_FRAME_BOUNDS)` / monitor APIs if display IDs are needed later.

Caveats:

- Foreground `HWND` can be `NULL` during activation transitions.
- Elevated/admin apps may block parts of cross-process metadata unless Mnema runs at equivalent integrity; do not require UIAccess for baseline metadata.
- Some packaged apps or hosted windows may require AppUserModelID/PFN to avoid grouping everything under a host process.
- Window titles can contain sensitive document names; continue honoring Metadata Settings and URL sanitization/off modes.

### Notifier replacement

Replace NSWorkspace activation notifications with:

- `SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, ...)` for focus changes.
- Optional `EVENT_OBJECT_NAMECHANGE`, show/hide/minimize events, or low-rate polling for app/window list changes.
- Keep privacy refresh coalesced through `native_capture/privacy.rs` semantics; do not do heavy metadata work in the WinEvent callback.

### Inactivity and session liveness

Adjacent Windows replacements needed by the same lifecycle:

- Input idle: `GetLastInputInfo` (session-specific; tick count not guaranteed strictly incremental).
- Sleep/wake: `WM_POWERBROADCAST`, `PBT_APMSUSPEND`, `PBT_APMRESUMEAUTOMATIC`, `PBT_APMRESUMESUSPEND`.
- Lock/unlock/session changes: `WTSRegisterSessionNotification` and `WM_WTSSESSION_CHANGE`.
- Display changes: window messages / display-change notifications; WGC item close/device-lost handling.

## Browser URL metadata

Current macOS browser URL collection uses AppleScript and only writes metadata according to `metadata.browserUrlMode` (`off`, `sanitized`, `full`). It does **not** drive live privacy decisions.

Windows options:

1. **UI Automation probe** of the active browser’s address bar.
   - Uses Microsoft UI Automation (`IUIAutomation`) to inspect controls in the active browser window.
   - Likely browser-specific heuristics are still needed.
   - No browser extension/native-host setup.
   - Fragile across browser UI changes, elevated/browser-protected contexts, profiles, split views, PWAs, and localization.
2. **Browser debugging protocols** are not suitable as default because they require users/apps to launch browsers with remote debugging enabled.
3. **Browser extension/native messaging** is rejected in this branch by ADR 0013.
4. **History/session database reading** is not appropriate for live active-tab metadata and is privacy-invasive.

Recommended Windows stance:

- Ship active app/window metadata first.
- Keep browser URL metadata off or sanitized by default until a UI Automation prototype proves coverage.
- Treat URL metadata as best-effort context only. Never use it for live privacy, credential-entry suspension, or automatic pause.
- Add Windows Known Browser descriptors only after the app identity model is settled.

## App candidates, icons, and catalogs

### Candidate discovery

Start with running/visible apps:

- `EnumWindows` / foreground-window metadata.
- Filter to visible, non-tool, non-empty top-level windows.
- Resolve identity through PID -> process path/PFN/AppUserModelID.

Add installed candidates later:

- Start Menu shortcuts from known folders (`FOLDERID_Programs`, `FOLDERID_CommonPrograms`) and `.lnk` targets/AppUserModelID.
- Packaged app enumeration if needed for Store/UWP/WinAppSDK apps.
- Avoid broad Program Files scanning as the primary catalog; it creates noisy/non-launchable candidates.

### Icons

Options:

- Shell/executable icon extraction (`SHGetFileInfo`, `IShellItemImageFactory`, or `ExtractIconEx`) into the existing app-owned icon cache.
- For packaged apps, resolve package logo assets later.

### Recommended exclusions and browsers

Windows sensitive recommendations must be a separate finite curated catalog. Candidate categories can include password managers/authenticators/banking apps after identity verification, but do not broadly recommend developer tools, terminals, messaging, email, or System Settings.

Potential Windows browser descriptors to verify later: Edge, Chrome, Chrome Canary, Firefox, Brave, Arc. Key them by identity/path/package data, not by macOS bundle IDs.

## Tauri and packaging notes

- Existing Tauri capabilities (`apps/desktop/src-tauri/capabilities/default.json`) are frontend command permissions, not Windows OS privacy permissions.
- Existing macOS `Info.plist` usage strings and `Entitlements.plist` do not translate to NSIS/MSI Windows builds.
- If Mnema chooses MSIX/packaged Windows distribution, package manifest capabilities matter:
  - `microphone` for microphone access.
  - `graphicsCapture` for picker-based graphics capture in packaged app contexts.
  - `graphicsCaptureProgrammatic` for programmatic `TryCreateFromWindowId/DisplayId` access.
  - `graphicsCaptureWithoutBorder` only if we ever request borderless capture; not recommended for Mnema default.
- Settings links should be opened through Tauri opener, e.g. `ms-settings:privacy-microphone`.

## Command/UI implications

| Command / UI | Windows behavior to design |
| --- | --- |
| `get_capture_support` | Return source support separately from permission state; system audio can be supported without permission prompt. |
| `get_capture_permissions` | May need richer per-source detail than current `granted/denied/not_determined/unsupported/unknown`. |
| `request_capture_permission("screen")` | Picker consent or programmatic access request depending on selected screen-capture mode. |
| `open_capture_privacy_settings("screen")` | No generic screen pane; open programmatic/borderless graphics privacy page only if relevant. |
| `open_capture_privacy_settings("microphone")` | Open `ms-settings:privacy-microphone`. |
| `open_capture_privacy_settings("systemAudio")` | Likely explain “no Windows permission pane” instead of opening settings. |
| `list_privacy_app_candidates` | Return platform-neutral app candidates; do not expose only `bundleId` forever. |
| `resolve_app_icons` | Resolve from Windows app identity/exe/package icon cache. |
| `add_privacy_excluded_app` / settings | Disable or mark unsupported if live app exclusion cannot be enforced for monitor capture. |
| Status bar “Exclude Current App” | Hide/degrade on Windows unless semantics are changed. |
| Timeline app grouping/icons | Group by platform-neutral app identity; update labels away from “bundle”. |
| `check_browser_url_support` | Windows browser URL support should be false/unknown until UI Automation prototype exists. |
| Sensitive recommendations | Windows-specific catalog and browser disclosure only after identity model exists. |

## Suggested tracer-bullet plan

1. Add platform-neutral `AppIdentity` / metadata DTO plan while keeping macOS compatibility.
2. Prototype `get_active_window_metadata_windows()` with foreground `HWND`, PID, exe path, title, AppUserModelID/PFN.
3. Prototype Windows app icon materialization from executable path.
4. Prototype microphone privacy status with `DeviceAccessInformation(AudioCapture)` and actual WASAPI failures under Windows 11 privacy toggles.
5. Prototype WGC picker capture and decide whether first Windows release uses picker consent or direct monitor capture.
6. Apply `WDA_EXCLUDEFROMCAPTURE` to Mnema Tauri windows and verify with WGC monitor capture.
7. Gate/hide App Privacy Exclusion and Exclude Current App on Windows until full semantics are explicit.
8. Only after active-window metadata works, prototype browser URL UI Automation as sanitized metadata-only.

## Open questions / test matrix

- Does `DeviceAccessInformation(AudioCapture)` reliably reflect microphone privacy for unpackaged Tauri + WASAPI on Windows 11?
- Which Windows packaging target is first: NSIS/MSI unpackaged or MSIX packaged? This changes manifest capability requirements.
- Should first Windows screen capture use WGC picker consent, direct monitor capture, or both behind a setting?
- What exact stable identity should Windows privacy settings persist for unpackaged apps: executable path, signed publisher + product, AppUserModelID when present, or a composite?
- Can UI Automation collect active URL reliably for Edge/Chrome/Brave/Firefox/Arc without over-collecting sensitive UI text?
- How should old `bundleId` frontend copy/types be renamed without breaking macOS persisted settings?

## Sources checked

- Microsoft Learn: Screen capture / `Windows.Graphics.Capture` — https://learn.microsoft.com/en-us/windows/uwp/audio-video-camera/screen-capture
- Microsoft Learn: `GraphicsCapturePicker` — https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscapturepicker
- Microsoft Learn: `IGraphicsCaptureItemInterop::CreateForWindow` — https://learn.microsoft.com/en-us/windows/win32/api/windows.graphics.capture.interop/nf-windows-graphics-capture-interop-igraphicscaptureiteminterop-createforwindow
- Microsoft Learn: `IGraphicsCaptureItemInterop::CreateForMonitor` — https://learn.microsoft.com/en-us/windows/win32/api/windows.graphics.capture.interop/nf-windows-graphics-capture-interop-igraphicscaptureiteminterop-createformonitor
- Microsoft Learn: `GraphicsCaptureAccess.RequestAccessAsync` — https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscaptureaccess.requestaccessasync
- Microsoft Learn: `GraphicsCaptureAccessKind` — https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscaptureaccesskind
- Microsoft Learn: `GraphicsCaptureSession.IsBorderRequired` — https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscapturesession.isborderrequired
- Microsoft Learn: Launch Windows Settings / `ms-settings:` URIs — https://learn.microsoft.com/en-us/windows/apps/develop/launch/launch-settings
- Microsoft Learn: `DeviceAccessInformation` — https://learn.microsoft.com/en-us/uwp/api/windows.devices.enumeration.deviceaccessinformation
- Microsoft Learn: `DeviceAccessStatus` — https://learn.microsoft.com/en-us/uwp/api/windows.devices.enumeration.deviceaccessstatus
- Microsoft Learn: `DeviceClass.AudioCapture` — https://learn.microsoft.com/en-us/uwp/api/windows.devices.enumeration.deviceclass
- Microsoft Learn: WASAPI Loopback Recording — https://learn.microsoft.com/en-us/windows/win32/coreaudio/loopback-recording
- Microsoft Learn sample: Application Loopback Audio Capture — https://learn.microsoft.com/en-us/samples/microsoft/windows-classic-samples/applicationloopbackaudio-sample/
- Microsoft Learn: `SetWindowDisplayAffinity` — https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowdisplayaffinity
- Microsoft Learn: `GetForegroundWindow` — https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getforegroundwindow
- Microsoft Learn: `GetWindowThreadProcessId` — https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getwindowthreadprocessid
- Microsoft Learn: `GetWindowTextW` — https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getwindowtextw
- Microsoft Learn: `QueryFullProcessImageNameW` — https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-queryfullprocessimagenamew
- Microsoft Learn: `GetApplicationUserModelId` — https://learn.microsoft.com/en-us/windows/win32/api/appmodel/nf-appmodel-getapplicationusermodelid
- Microsoft Learn: `GetPackageFamilyName` — https://learn.microsoft.com/en-us/windows/win32/api/appmodel/nf-appmodel-getpackagefamilyname
- Microsoft Learn: Application User Model IDs — https://learn.microsoft.com/en-us/windows/win32/shell/appids
- Microsoft Learn: `System.AppUserModel.ID` — https://learn.microsoft.com/en-us/windows/win32/properties/props-system-appusermodel-id
- Microsoft Learn: `SHGetPropertyStoreForWindow` — https://learn.microsoft.com/en-us/windows/win32/api/shellapi/nf-shellapi-shgetpropertystoreforwindow
- Microsoft Learn: UI Automation overview — https://learn.microsoft.com/en-us/windows/win32/winauto/entry-uiauto-win32
- Microsoft Learn: Security Considerations for Assistive Technologies — https://learn.microsoft.com/en-us/windows/win32/winauto/uiauto-securityoverview
- Microsoft Learn: `SetWinEventHook` — https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwineventhook
- Microsoft Learn: WinEvent event constants — https://learn.microsoft.com/en-us/windows/win32/winauto/event-constants
- Microsoft Learn: `GetLastInputInfo` — https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getlastinputinfo
- Microsoft Learn: `WM_POWERBROADCAST` — https://learn.microsoft.com/en-us/windows/win32/power/wm-powerbroadcast
- Microsoft Learn: `WTSRegisterSessionNotification` — https://learn.microsoft.com/en-us/windows/win32/api/wtsapi32/nf-wtsapi32-wtsregistersessionnotification
- Microsoft Learn: App capability declarations — https://learn.microsoft.com/en-us/windows/uwp/packaging/app-capability-declarations
- crates.io/docs.rs survey: `windows`, `uiautomation`.
