# Use app-level live privacy exclusion only

Mnema uses **App Privacy Exclusion** as its only live screen-capture privacy exclusion guarantee. Users can exclude entire apps from recording by app identity. Mnema records visible screen content from apps that are not excluded, including private or incognito browser windows.

## Context

Earlier privacy behavior allowed metadata-derived exclusions for websites, browser titles, and private browser windows. Those policies require per-window live filtering when they resolve to specific windows.

Thermal measurements during screen-only recording showed that app-level ScreenCaptureKit filtering stayed low-cost, while window-level filtering drove high WindowServer and GPU/combined power:

- no privacy filtering: about 649 mW combined CPU/GPU/ANE power
- app-only filtering: about 867 mW combined power
- window-only filtering: about 2245 mW combined power
- app exclusion with excepted windows: about 2889 mW combined power

The hot path was not live video writing, frame artifact persistence, JPEG export, or OCR.

## Decision

Mnema will expose and apply only app-level live privacy exclusion.

Metadata-derived website, title, private-browser, and per-window privacy decisions must not feed the **Live Privacy Filter**. Shared recording/privacy settings should not expose inactive metadata privacy fields for those policies.

Any metadata collection kept after this removal must serve non-privacy product features such as timeline context, app/window labels, or debug surfaces.

## Consequences

The privacy model is simpler and predictable: Mnema records what is visible from non-excluded apps. Users who do not want browser content recorded should exclude the browser app.

Mnema no longer promises website-level, title-level, private-browser, or per-window live exclusion.

The runtime avoids ScreenCaptureKit window-filter paths for privacy, reducing WindowServer/GPU power during recording.

Because old capture data and settings do not need migration, inactive metadata privacy settings can be removed rather than preserved for compatibility.

## Amendment (2026-10-24): exclude an app only while one of its windows shows

The "app-only filtering stayed low-cost" figure above counted CPU, GPU and ANE only. Whole-Mac power with the IOReport memory rails tells a different story (M4 Air, bare ScreenCaptureKit stream, Mnema quit, 2 min per state): a plain display filter costs ~+0.2 W, but the same stream excluding **one running app** costs ~+2.4 W — GPU busy 64%, DRAM power ×5, the rest off-chip — and WindowServer keeps that cost for minutes after the stream stops. It applies even when the excluded app shows nothing, and excluded apps usually don't: menu-bar only, hidden, minimized, on another Space, or parked by AeroSpace as a 1-pt corner sliver. On the founder's Mac this was most of screen recording's +3.7 W.

Changes:

- The live filter excludes an app only while one of its on-screen windows covers at least 2×2 pt of the captured display (`crates/capture-screen/src/privacy_visibility.rs`, `plan_privacy_content_filter`). A menu-bar icon (status-item window layer) doesn't count; its menus and panels do. The other listed apps stay "unresolved", so the 1 s privacy poll keeps re-checking them; a filter that excludes anything also keeps the poll running, so an exclusion is dropped once its app hides.
- A filter for "display 0" is always built for the display the stream records, never whichever display is listed first; if that display is gone, the update fails as display-unavailable.
- A frame guard covers the gap until the next refresh: after each frame arrives, the stream handler checks the on-screen window list and drops the frame if a running listed app the filter leaves out has a window showing. If the window list can't be read, it drops the frame. While a filter update is in flight, and after one fails, it guards the union of the old and new left-out apps.

This narrows the gap; it does not close it. A listed app's window can still reach a recorded frame when it:

- appears and disappears between the frame being composed and the check (a sub-second popup);
- is in a frame already in flight when a filter update that stops excluding it lands;
- belongs to a process macOS hasn't yet listed as a running app with that bundle id (just launched);
- shows less than 2 pt on either axis.

The decision itself stands: privacy exclusion is still app-level only.
