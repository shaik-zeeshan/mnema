# Plan: Quick Recall search redesign — list + detail pane

Mockup (signed off, single source of design truth): `docs/quick-recall/mockups/search-redesign.html`.
Deep-link states for reference while building: `#sel=N`, `#chip=chrome`, `#picker`, `#syn`, `#more`, `#hover=1`, `#light`.

## Problem

Quick Recall's search results are 96px-thumbnail rows capped at 5 per source. Thumbnails are too small to recognize, so users can't tell what a result *is* without opening it — and opening it closes the window, making inspection destructive. Users need to recognize and preview results in place, see more than 5 hits, and understand when matches happened.

## Solution

Rebuild the SEARCH mode of the Quick Recall window to the mockup: a 1120×720 panel with a relevance-ordered result list (left, two modality sections SCREEN/AUDIO, 150×94 thumbnails, Raycast row anatomy) beside a detail pane (right, ~420px) that previews the selected result — full frame or transcript, OCR/transcript context, URL, times. A thin 8-day timeline strip places every hit at its true time with hover previews. Selection previews; Enter/⌘O remain the exits. Existing filter chips, picker, syntax help, and states are kept and restyled; section caps rise with a "show more" row.

**Semantic change (deliberate, decided):** selecting a result previews it in the detail pane instead of opening it in the main window and closing Quick Recall. Enter = open in timeline (main window) + close; ⌘O = open captured page. ⌘1–9 changes from "open Nth" to "jump selection to Nth".

## User Stories

1. As a user, I want result thumbnails large enough to recognize the app and content, so that I can pick the right result without opening each one.
2. As a user, I want selecting a result to preview it (full frame, context, URL, time) inside Quick Recall, so that inspection doesn't destroy my search.
3. As a user, I want more than 5 results per source with a "show more" expansion, so that the answer isn't cut off arbitrarily.
4. As a user, I want a timeline strip showing when matches happened, so that I can navigate results by time.
5. As a user, I want audio results shown as attributed quotes with source and duration, so that I can tell what was said and where, without a misleading screenshot.
6. As a user, I want one filter system (typed operators and the ⌘F picker both becoming removable chips), so that filtering is predictable.

## Implementation Decisions

- **Backend is untouched except window size.** Search limits are already request parameters (`clamp_limit`, max 50, `crates/app-infra/src/search/retrieval.rs:14`); detail-pane data comes from existing commands. No schema changes, no migrations.
- **Window**: `apps/desktop/src-tauri/src/windows.rs` `QuickRecall` arm → `inner_size (1120, 720)`, `min_inner_size (960, 600)`. List column is `flex: 0 0 ~700px` at design size; detail pane flexes.
- **Fetch once, expand client-side**: request `frameLimit: 24, audioLimit: 12` in one `search_capture` call; render 8 screen / 3 audio; "↓ show N more" reveals already-fetched rows per section (no pagination round-trip). `hasMoreFrames/Audio` beyond 24/12 is ignored (out of scope).
- **Row anatomy** (rework `SearchResultCard.svelte`): screen = 150×94 thumb, app-strong + window-muted title line, one marked snippet line, right accessories (relative time, match count, found-by-meaning/redacted pills). Audio = source-colored waveform tile (no screenshot; `alignedFrame` deliberately NOT the hero), quoted snippet as title, `source · duration` attribution, "N adjacent". Accessories duplicated in the detail pane are stripped from the selected row (Raycast rule). The current hover "open in browser" host chip on rows is removed; the URL lives in the detail pane and ⌘O keeps the shortcut.
- **Detail pane data** (fetched lazily on selection, cached per result):
  - Hero frame: `get_frame_scrub_previews` with `maxPixelSize: 1280` for the selected result's `thumbnailFrameId` (thumbnails keep the existing 200px batch).
  - OCR context: `list_processing_results`/`get_processing_result` for the representative frame; render flat `result_text`, client-side highlight of residual-query terms. (Search results carry no OCR text; the TS `FrameDto.ocrText` field is never emitted by Rust — do not use it.)
  - Transcript context: `list_speaker_turns { audioSegmentId }`, rendered around `spanStartMs` with a match-position marker on the waveform.
  - Two round-trips are acceptable; no new convenience command.
- **Timeline strip**: client-side dots from result timestamps (`groupStartAt` / `absoluteStartAt`) over an 8-day axis, min-gap pass for hoverability (port the mockup's algorithm), hover preview reuses the 200px thumbnail cache, click selects (expanding a collapsed section if needed). When chips filter the set, non-matching dots dim and a legend explains ("4 of 17 shown · app: Chrome"). Dots reflect fetched results only.
- **Filters**: keep the existing backend operator parsing (`appliedRefinements`/`residualQuery`/`parseErrors`), chip band, ⌘F picker, value list, ghost completion, and syntax popover — restyle to the mockup. The mockup's per-app match counts in the picker are dropped (no backend support; `list_searchable_apps` has no counts).
- **Selection**: keep the flattened roving index + `aria-activedescendant` + wraparound. Rewire: select → render detail pane; Enter → `open_capture_result_in_main_window` + close; ⌘O unchanged; ⌘1–9 → select. Selecting a timeline dot for a collapsed row auto-expands its section.
- **States**: keep all existing branches (orientation, loading skeleton + dimmed-refetch, error + Retry, results-paused parse error, no-matches + recovery, semantic hint) and restyle per mockup, including the skeleton's dimmed detail pane. The results-paused state has no mockup panel; style it consistently with the error/no-match pattern. Error text uses `--app-danger-text` (matches shipped `quick-recall__state--error`).
- **File split (800-line rule)**: `+page.svelte` is 6450 lines and may not grow. Extract the search mode into `apps/desktop/src/lib/quick-recall/`: `searchStore.svelte.ts` (class + `$state`/`$derived` singleton, per `conversationStore` idiom) owning query/results/selection/expansion/detail state; pure helpers in plain `.ts` with colocated bun tests (timeline positioning, show-more slicing, context highlighting); components `ResultsList.svelte`, `DetailPane.svelte`, `TimelineStrip.svelte`, `FilterPicker.svelte`, `SyntaxHelp.svelte`. `+page.svelte` keeps the window shell, mode cross-fade, and Ask AI mode. Every touched/new file ends under 800 lines.
- **Ask AI mode is not redesigned**; only its door (accent "Ask AI ⌃↵" button, ⌃↵ pivot, empty-state CTA) is restyled.
- **Design tokens**: use the app tokens from `+layout.svelte` exactly as the mockup does (it was token-synced during polish: danger for errors, `--app-shadow-popover` for popovers, green-tinted `--app-surface-active` selection in light). Both themes must render correctly; `prefers-reduced-motion` disables shimmer/transitions.
- Assumption: 1120×720 fits the smallest supported display; onboarding already ships 1120×800.

## Testing Decisions

- Bun tests (colocated `*.test.ts`) for the pure logic: timeline dot mapping + min-gap pass, section slicing/show-more/selection-visibility (collapse hides selection → clamp), residual-term context highlighting, detail-cache keying.
- `bun run check` for UI; `cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml` for the windows.rs change (needs the `mnema-cli` sidecar prepared).
- Manual verification against the mockup's deep-link states: selection follows arrows with wraparound, Enter opens in main window + closes, ⌘O opens page, ⌘F picker → chips → list+timeline dim, show-more expands, timeline hover/click, all six states, both themes, reduced motion.
- Do not test Svelte component internals; test through the store's observable state and pure helpers.

## Slices

1. **Search-mode extraction (no behavior change)**
   - Goal: move current search state/flow out of `+page.svelte` into `lib/quick-recall/searchStore.svelte.ts` + component shells; page renders identically.
   - Areas: `quick-recall/+page.svelte`, new `lib/quick-recall/`.
   - Acceptance: `bun run check` green; search behaves exactly as shipped; every file <800 lines.
   - Depends on: none. Parallel: no (everything else builds on it).
2. **Window + shell layout**
   - Goal: 1120×720 window; list+detail split; section headers with counts; footer hints per mockup.
   - Areas: `windows.rs`, `lib/quick-recall/` shell, footer.
   - Acceptance: window opens at new size; two-pane layout in both themes.
   - Depends on: 1. Parallel: with 3.
3. **Row redesign**
   - Goal: 150×94 screen rows + waveform audio rows per mockup anatomy; hover host chip removed; selected-row accessory stripping.
   - Areas: `SearchResultCard.svelte` (or replacement in `lib/quick-recall/`).
   - Acceptance: rows visually match mockup; pills/accessories correct; snippet marks render.
   - Depends on: 1. Parallel: with 2.
4. **Selection semantics + show-more**
   - Goal: select=preview wiring, Enter/⌘O exits, ⌘1–9 jump, raised limits (24/12), show-more rows, collapse clamping.
   - Areas: searchStore, keyboard hub, results list.
   - Acceptance: bun tests for slicing/clamping; manual keyboard walk.
   - Depends on: 2, 3.
5. **Detail pane**
   - Goal: hero frame (1280px preview), id row, badges, URL, times, scrollable OCR/transcript context with highlights, audio waveform + match marker.
   - Areas: `DetailPane.svelte`, searchStore detail cache, `get_frame_scrub_previews` / `list_processing_results` / `list_speaker_turns` wiring.
   - Acceptance: pane follows selection for both kinds; context loads lazily and caches; missing data degrades gracefully (no context → pane still renders).
   - Depends on: 4.
6. **Timeline strip**
   - Goal: 8-day axis, true-time dots + min-gap, hover preview, click-select (+auto-expand), filtered dimming + legend.
   - Areas: `TimelineStrip.svelte`, searchStore.
   - Acceptance: bun tests for dot mapping; hover/click verified manually.
   - Depends on: 4. Parallel: with 5.
7. **Filters + states restyle**
   - Goal: chip band, ⌘F picker, syntax popover, and all six states restyled to mockup (danger-red error; skeleton with dimmed pane; results-paused styled consistently).
   - Areas: `FilterPicker.svelte`, `SyntaxHelp.svelte`, state branches.
   - Acceptance: `#picker`/`#syn`/states match mockup in both themes.
   - Depends on: 2. Parallel: with 5, 6.

Parallel groups: [1] → [2, 3] → [4] → [5, 6, 7].

## Out of Scope

- Match-distribution histogram command (timeline dots cover fetched results only).
- Per-app match counts in the filter picker.
- Pagination beyond the 24/12 fetch (`frameOffset`/`audioOffset` stay 0).
- Audio playback in the detail pane.
- Ask AI mode redesign; cross-segment transcript context; any `crates/app-infra` search changes.
- Repairing the pre-2026-07-03 sidecar `video_offset_ms` data (known, deliberately unrepaired).

## Further Notes

- The mockup's page header documents the select=preview semantic change; mirror that note in the PR description so reviewers don't flag it as a regression.
- Detail-pane preview files come from the existing preview cache infra; `missingReason` from `get_frame_scrub_previews` must render a fallback (reuse the card's SVG glyph fallback pattern).
- After the redesign lands, update `docs/agents/capture-pipeline.md` only if new gotchas surfaced; the mockup file stays in `docs/quick-recall/mockups/` as the design record.
