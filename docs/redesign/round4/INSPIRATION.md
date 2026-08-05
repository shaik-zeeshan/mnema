# Round 4 — Design inspiration

Research pass for the full-app redesign. Five areas, real products, concrete anatomy.
Screenshots live in `inspiration/` — **deliberately untracked** (third-party product
screenshots stay out of history; regenerate via the firecrawl sources named per item).
Items marked ⚑ are from the captured screenshot; unmarked ones are established product
behavior I did not re-capture.

---

## 1. Bento / widget dashboards

**Apple widgets — the size rhythm is 1×1, 2×1, 2×2, 4×2, never anything else.**
- Concrete grid (iPhone 393×852, from HIG spec table): small `158×158`, medium `338×158`,
  large `338×354`, XL `450×338`. So medium = 2·small + **22pt gutter**; large = 2·medium
  stacked + gutter (354 ≈ 2·158 + 38, the extra is the taller row). One cell unit, one
  gutter, four legal footprints. No 3-wide, no 1.5.
- Standard inner margin **16pt**; tighten to **11pt** only for nested groupings (graphics,
  buttons, background shapes). Never text below **11pt**.
- Corner radius of inner content must be *derived* from the tile radius (`ContainerRelativeShape`),
  not a fixed number — nested rounding tracks the parent.

**Apple Control Center — a control is symbol + title + value, and it drops parts as it shrinks.** ⚑
- One anatomy, three renderings: at the smallest size only the SF Symbol shows; at medium it
  adds title and value ([`apple-control-anatomy.png`](inspiration/apple-control-anatomy.png)).
- State is carried by **symbol swap + tint**, not by a switch: `door.garage.open`/`.closed`,
  untinted off → tinted on ([`apple-control-center.png`](inspiration/apple-control-center.png)).
  Symbol animates on the transition; indefinite animation while a long action runs.
- Every control declares a **placeholder title/value** for when real data is unavailable
  (gallery, locked device). Redacted state is designed, not an accident.

**Raycast extension tiles — identical chrome, free-form payload.** ⚑ ([`raycast-bento-cards.png`](inspiration/raycast-bento-cards.png))
- This is the answer to "how does a number, a list, and a strip of images share one grid":
  every tile is the same ~`360×545` card, same 16px radius, same header row (32px app icon +
  title left, circular chevron button right), same 2-line description at the same baseline.
  Below that line the payload is completely different per tile — Linear: abstract arc + a row
  of 4 glyph buttons; Translate: a stacked text list with opacity falloff; Spotify: album art
  square + 3 transport buttons.
- The payload **bleeds off the card edge and is clipped by the radius** (the translate list
  runs off the right, the arc off the bottom). That single move is what stops a bento from
  reading as a form.
- Per-tile background tint pulled from the source app's brand color, with a radial glow from
  the top edge. Chrome constant, color variable.
- Section title left, **category segmented pill top-right on the same baseline**; carousel with
  the 4th card deliberately half-cut to signal horizontal scroll.

**Raycast Store — list rows beat cards once content is homogeneous.** ⚑ ([`raycast-store-grid.png`](inspiration/raycast-store-grid.png))
- Three tiers on one page: 3-up "Featured" cards (icon centered, name, one-line desc, author
  chip, Install button) → then a **2-column list** of rows for the long tail.
- Row anatomy: 24px icon + bold name left, `Install` button pinned right, description on line 2,
  meta line 3 as icon+value chips (author avatar · `6 Commands` · `499,677` installs · `macOS only`).
- Filter bar is one row: segmented `All / Recently Added / Most Popular` left, list-vs-grid
  toggle + scoped search right. Sort, view mode, and search never get separate rows.

**Linear Insights — analytics as a *panel*, not a page.** ([`linear-insights.png`](inspiration/linear-insights.png))
- Insights opens as a right-hand sidebar over whatever view you're in (`⌘⇧I`), inheriting that
  view's filters as the dataset. No separate dashboard to keep in sync.
- Two dropdowns (**Measure** × **Slice**) generate both a graph and its backing table. The chart
  is a query builder output, not a hand-placed widget.

### Steal this
- Fix a cell unit and allow exactly 4 footprints (1×1, 2×1, 2×2, 4×2). One gutter value.
- Identical tile chrome (header row + one description line at a fixed baseline); everything
  below is a free payload zone that may bleed and clip.
- Tint the tile from its subject, keep the frame constant.
- Derive inner radii from the tile radius; 16px margins, 11px only inside nested groups.
- Design the redacted/empty rendering of every tile as a first-class state.
- Dashboard filters inherit from the current view rather than being re-picked per tile.

---

## 2. Settings with personality

**Raycast — horizontal toolbar tabs, no left rail.** The Settings window is a native
`NSToolbar` of icon+label tabs (General, AI, Extensions, Cloud Sync, Advanced, Account, About);
the window *animates its height* to fit the selected pane, so no pane scrolls if it doesn't
have to. The one place a sidebar appears is inside the Extensions tab — a list of extensions
left, that extension's generated form right. Sidebar where it earns its keep, nowhere else.

**LM Studio — one segmented control gates the entire option surface.** ⚑
([`lmstudio-statusbar.png`](inspiration/lmstudio-statusbar.png)) A persistent bottom status bar:
app name + version left, then `User | Power User | Developer` segmented — this is the *whole*
progressive-disclosure mechanism for the app's settings. Right side is a live monospace resource
readout: `RAM: 17.47 GB | CPU: 0.00 %`, then account + gear icons. The cost of what you enabled
is displayed permanently, in the same bar as the control that enabled it.
Model selection is a **pill in the title bar** — `openai/gpt-oss-20b ▾` with a separate `Eject`
button ([`lmstudio-model-pill.png`](inspiration/lmstudio-model-pill.png)) — so the active model
is global chrome, not a setting you go and find.

**Model pickers with size/RAM badges** (LM Studio's download browser, Ollama GUIs): each row is
`name · quant badge (Q4_K_M) · size in GB` plus a **fit verdict** rendered as a colored chip —
green "full GPU offload possible" / amber "may be slow" / red "likely too large for this machine",
computed against the actual machine. The verdict, not the number, is the control's real output.

**Autosave without a clipped bottom bar** — in order of preference: (1) **row-level echo**, a
"Saved" check right-aligned in the changed row, fading after ~1.5s — locality tells you *what*
saved (Vercel, Linear); (2) **bottom-center toast with Undo** — the undo affordance is what makes
silence acceptable; (3) **optimistic apply, revert on failure**, error inline under the offending
control. A persistent Save bar is the anti-pattern: it implies unsaved state exists, and it clips
on short windows.

**Custom per-type inputs worth copying (control anatomy):**
- **Cost slider**: slider track + a live derived readout *below* it in the unit the user cares
  about, not the unit you store — "2 fps → **~1.4 GB/day**, ~42 GB/month". Docker Desktop's
  resource sliders do this ("of 16 GB available"); the denominator is what makes it legible.
- **Retention picker**: not a duration dropdown — a horizontal segmented ladder
  (`7d · 30d · 90d · 1y · Forever`) with the projected footprint under it, and the current
  actual footprint marked on the same axis. Time Machine/Backblaze-style bar.
- **Duty-cycle dial**: two-ended, showing *both* halves (work vs cooldown) as a single split bar
  with the resulting temperature/throughput implication written beneath as prose.
- **Shortcut recorder**: field shows current combo as individual **keycap badges**; click →
  "Type shortcut", Esc cancels, and a conflicting binding surfaces inline in red *under* the
  field naming the conflicting owner. Raycast/Rectangle/CleanShot all converge on this.
- **Toggle row with consequence preview**: the description line under the toggle changes when
  toggled, stating the consequence in the present tense ("System audio is being recorded for
  all apps except 3 excluded ones"), rather than describing the option abstractly.

### Steal this
- Kill the left rail: horizontal icon+label tabs, window height fits the pane.
- One global `Simple / Advanced` segmented control instead of ten "Show advanced" disclosures.
- Put the live cost readout (disk/day, CPU%) permanently in the same bar as the controls.
- Active model as a title-bar pill with an eject affordance, not a buried setting.
- Every model row carries `size · quant · machine-fit verdict chip`.
- Every numeric control renders its consequence in user units, with a denominator.
- Row-level "Saved" echo + Undo toast. No save bar, ever.

---

## 3. Command palettes / quick access

**Raycast — the palette is a two-pane app, and the bottom bar is the verb.** ⚑
([`raycast-launcher-clipboard.png`](inspiration/raycast-launcher-clipboard.png))
- Top row: back chevron ← + search field, and a **scope dropdown pinned to the right of the same
  field** ("All Types"). Mode/scope lives *in* the search row, not above it.
- Body splits ~40/60: left result list with muted section headers ("Today"); right a **preview
  pane** (the actual content, rendered) over an **Information table** of right-aligned key/value
  rows (`Application → VS Code`, `Content Type → Color`, `Copied → Today, 13:35:07`).
- Bottom bar: current context breadcrumb left (extension icon + "Clipboard History"); right the
  primary action with its `↵` keycap, a divider, then `Actions ⌘K`. The palette always tells you
  what Enter will do.
- Mode switching is a small floating icon dock below the window with the mode name in a tooltip
  above it — modes are spatial, not a dropdown.

**macOS Tahoe/Golden Gate Spotlight — AI mode is a result row, not a mode toggle.** ⚑
Apple's own copy: *"when you search in Spotlight, you can choose **'Ask Siri' as the top hit**."*
That is the cleanest resolution of search-vs-ask in one field: you type once, ranked results
appear, and the ask-the-model option is the first row — free to ignore, one ↓↵ to take.
Spotlight also splits results into labeled category sections with a "show all" per section, and
exposes app *actions* (not just apps) as rows with their own keyboard shortcut hints.

**Linear ⌘K** — context-scoped: the palette's available commands change with the focused view,
and the header shows the scope as a chip. Results are a flat list with the command's own
keyboard shortcut right-aligned on the row — the palette doubles as shortcut discovery.

**Handoff to a bigger window**: the pattern that works is the palette *becoming* the window —
the query and any composed context carry over, and the palette closes only after the window has
the same state. Raycast does this per-extension ("Open in…"); ChatGPT's launcher does it by
promoting the mini composer into the full chat with the draft intact.

### Steal this
- Scope selector inside the search row, right-aligned.
- "Ask AI about this" as the **top result row**, not a segmented mode. Ranked, dismissible.
- Right-hand preview + right-aligned key/value info table for the selected result.
- Persistent bottom action bar: context breadcrumb left, `primary action ↵` + `Actions ⌘K` right.
- Section headers as small muted labels inside the one list; never separate tabs.
- Handoff carries the query and the draft; the palette closes last.

---

## 4. "Ask AI about my screen" overlays

**Cluely — the reference anatomy.** ⚑ ([`cluely-assist-panel.png`](inspiration/cluely-assist-panel.png),
[`cluely-bar-hero.png`](inspiration/cluely-bar-hero.png))
Two *detached* floating pieces, top-center of the display, with a ~12px gap between them:
1. **Control pill** (always present): app glyph, `Hide ▾` disclosure, a stop/record square.
   Fully rounded, small, draggable. This survives when the panel is hidden.
2. **Answer panel** below it: ~14px radius, dark translucent, containing —
   - a mode chip (`Assist`) **overhanging the panel's top-right edge** like a tab;
   - a context header in small muted text: **"Viewed screen"** — the whole context-attachment
     indication is a *label*, no thumbnail, no filmstrip;
   - the answer prose;
   - a row of quick-action chips separated by `·` dots: `Assist · What should I say? ·
     Follow-up questions · Recap`;
   - an input whose placeholder contains **inline keycap badges**:
     "Ask about your screen or conversation, or `⌘` `↵` for Assist";
   - bottom-left a model chip (`Smart`) + `…` overflow; bottom-right a circular blue send button.
- Product-level invariants worth noting: the window is excluded from screen shares/recordings,
  and it's positionable so it never covers what you're asking about.

**ChatGPT desktop — context as inline entity chips in the sentence.** ⚑
([`chatgpt-desktop.png`](inspiration/chatgpt-desktop.png)) The prompt reads
"Look across my `📅 Google Calendar`, `Slack` and `Google Drive` and create a leadership brief" —
attached sources are colored favicon chips **inside the composed sentence**, not in a separate
attachment tray. You edit the context by editing the sentence.

**Apple (Siri screen awareness)** — the attachment gesture is the screenshot: *"Just take a
screenshot of what's on your display… to get answers, search, or take action."* No separate
"attach screen" button; the existing, already-understood system gesture becomes the affordance.

**Windows Copilot Vision** — opt-in per session with an explicit start/stop, and the shared
window is outlined so you can always see *what* the model can see. The "which window" question
is answered on the screen itself rather than in the panel's text.

### Steal this
- Split the overlay into a persistent control pill and a dismissible answer panel.
- Context attachment = one muted label line ("Viewed screen · 14:32") — resist the thumbnail.
- Mode chip overhanging the panel's top-right corner as a tab.
- Quick-action chips (`Recap`, `What was I doing?`, `Follow-ups`) on their own dot-separated row.
- Keycap badges inline in the placeholder to teach the shortcut in place.
- If more than one source is attached, render them as inline chips inside the prompt sentence.
- Outline/indicate the captured region on screen, not just in the panel copy.

### Avoid
- A big attached-frame thumbnail in the panel — it costs the panel's whole width and the user
  already knows what's on their screen.
- Modal "attach screen?" dialogs. Make the capture implicit and the *indicator* explicit.
- A single fused bar+panel: hiding the answer then also hides the controls.

---

## 5. Timeline / history scrubbers

**Rewind — the date jump is a pill anchored to the playhead.** ⚑
([`rewind-timeline-strip.png`](inspiration/rewind-timeline-strip.png))
- The ribbon is a thin capsule-segmented strip at the bottom edge of the screen. **One segment
  per app session**, colored by app, length proportional to duration.
- The **app icon sits centered on its segment and is taller than the ribbon**, breaking the bar's
  silhouette. That's what makes a 4px strip scannable — you read icons, not colors.
- A full-height white **playhead line** extends past the ribbon into the frame area above.
- Directly above the playhead: a **`Now ▾` pill** — this is the entire date-navigation control.
  Label = current position ("Now", else the date/time), chevron opens the jump menu. It moves
  with the playhead, so the readout is never in a different place from the thing it reads out.

**Photos — zoom levels are the navigation.** Years / Months / Days / All Photos is a segmented
control that changes the *aggregation* of the same scroll surface; a date label pins to the top
of the viewport as you scroll and updates continuously. There is no separate date picker for
coarse movement — you zoom out, tap the year, zoom back in.

**Video-editor scrubbers (Screen Studio et al.)** ([`screen-studio.png`](inspiration/screen-studio.png))
- Tick density is tied to zoom: labels thin out to round units (`:15`, `:30`, `1:00`) rather than
  shrinking. Never render a label you'd have to rotate.
- Hover shows a ghost playhead + a floating time bubble *before* you commit the click; click
  commits. Drag on the ruler scrubs, drag on the content pans — different verbs per band.
- Shift/⌥ modifiers change scrub granularity (frame vs second) without any UI.

### Steal this (date-range selector + secondary controls only)
- Replace a date-range dropdown with a **`{current date} ▾` pill anchored above the playhead**
  that both reads out position and opens the jump menu. One control, two jobs.
- Jump menu contents: `Today`, `Yesterday`, a day-of-week list for the last 7 days, then a
  month calendar — with **days that have no recording rendered as disabled**. Never let the user
  pick an empty day.
- Coarse navigation by **zoom level** (Hour / Day / Week / Month) as a segmented control, not by
  a second date input; the range selector then only has to handle "jump", not "span".
- Density-adaptive tick labels; round units only.
- Hover ghost-playhead + time bubble before commit.
- App icons riding on top of their segments; color alone won't survive a thin strip.

### Avoid
- Two date inputs (from/to). A history scrubber has a position, not a range — the visible span
  is a consequence of zoom.
- A separate mini-map above the ruler; the segmented ribbon already is one.

---

Also saved, not linked above: [`cluely-bar-hero.png`](inspiration/cluely-bar-hero.png) and
[`cluely-overlay.png`](inspiration/cluely-overlay.png) (overlay in situ + the "invisible to
screen share" framing), [`rewind-ask-panel.png`](inspiration/rewind-ask-panel.png) (Ask-Rewind
panel floating over the timeline).
