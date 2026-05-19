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
