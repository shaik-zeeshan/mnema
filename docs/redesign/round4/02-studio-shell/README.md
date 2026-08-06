# 02 · Studio Shell

Pro-app bones — Final Cut's viewer, Logic's inspector, Activity Monitor's honesty about what the
machine is doing right now.

1. **Four fixed pieces, one scrolling region.** A 38px title bar, a 30px contextual tool strip, the
   content, and a 24px status strip welded to the bottom edge. Chrome is dense (22px controls) so
   content gets the room.
2. **A right inspector, 256px, on every surface.** It carries the detail of whatever is selected:
   frame metadata on Timeline, the tile's selected row on Overview, the result's record in Quick
   Access, the changed setting in Settings. One panel, four subjects — so no tile, cell or row ever
   has to grow a second column.
3. **The status strip carries live state.** Capture rate, disk written today, the projected monthly
   pace, the queue — and the save state. Because it is structural, it cannot clip.
4. **Settings has no navigation rail at all.** One scroll, a filter field in the tool strip, sticky
   section headers that keep your address.
5. **Densest of the five directions.** 28px content rows, 22px chrome rows, hairline separations
   instead of card edges — tight, never cramped.
6. **Journal, Subjects and Context are destinations, not surfaces.** The main window still has exactly
   two surfaces (Timeline · Overview). Each destination opens from its own Overview tile header and
   comes back through the tool strip's first control; the old Insights rail is gone. Inside each one
   the shape is unchanged — tool strip navigates, one region scrolls, the inspector carries the
   selection's record, the status strip carries live state.

## What it does with each founder ask

| Ask | This direction |
|---|---|
| **Keep the bento** | Kept whole — same heterogeneous tiles (moments strip, digest, capture, storage, conversations, subjects, context, week, ask). Re-skinned flatter and denser: a hairline under each tile header instead of a card look, 12px inset = 12px gutter. Selecting a row inside a tile fills the inspector, which is what lets the tiles stay headlines. |
| **Timeline nav** | Layout untouched (bar → stage → rail, newest right → mic/sys lane → readout → drawer, drawn open). The date control becomes a **`Mon, Aug 3 · 14:32 ▾` pill anchored to the playhead** — it reads out position *and* opens the jump menu (Today / Yesterday / last 7 days with per-day capture bars / a month calendar where empty days are disabled). Coarse movement is a **Hour·Day·Week·Month segmented control**, so the jump control only ever answers "where", never "how wide". The loose OCR controls become one labelled group; hover shows a ghost playhead and a time bubble before the click commits. |
| **Search vs Ask** | Different shape, not a different tint. **Search** = neutral grey mode chip, scope selector inside the query row, a results grid, an inspector holding the result's record, and a bottom bar that always states what ⏎ does. **Ask** = accent mode chip, a single 70ch prose column, cited moments as a horizontal media rail, the active model as a pill in the chrome, and the composer at the bottom. The bridge is a ranked first result — "Ask AI about this" — not a mode you must choose. |
| **Ask about the current frame** | Four frames on page 04. ⌘⇧S collapses the 1120×720 window to a **560px bar pinned top-centre**, with the control pill as a *separate* object (hiding the answer must never hide Stop). The frame is captured on collapse, not on send. Two indicators: the display is **outlined on screen** where the thing being seen actually is, and the frame is a **chip inside the sentence** (34×20 thumbnail + app + window + timestamp + ⌫). The answer panel grows beneath the bar with the mode tab overhanging its corner, a past-tense context line, quick-action chips, and a live follow-up composer. |
| **Settings navigation** | The rail is deleted. One scrolling page, a filter field in the tool strip (typing "aud" cuts 48 settings to 4 across two sections), sticky section headers carrying the section name and its position in the 48. The inspector shows the focused setting's detail — value, previous value, what it costs, when it takes effect, recent changes — instead of being navigation. |
| **Autosave** | Three placements, none of them clippable. The **row** echoes "Saved ✓" (tells you *what*), the **status strip** carries "All changes saved · 14:41" permanently at a fixed window edge with an **Undo** (tells you *whether*), and the **inspector** shows what the change did. There is no save bar and no unsaved state. |
| **Custom inputs** | Six, on page 06, each where the raw number is meaningless on its own: a rate slider with a consequence line (`2 fps → ~1.4 GB/day, ~42 GB/month`), a retention ladder with your real footprint marked on its own axis, a two-halves duty-cycle bar (`work 20% / cool 80%` → `~430 frames/hour, +3 °C, fans off`), model rows carrying `size · quant · fit verdict computed against this Mac`, a shortcut recorder that names the conflicting owner, and a toggle whose description is the present-tense consequence. Nineteen of the twenty-two Intelligence settings use no custom input at all. |

## Files

| file | contents |
|---|---|
| `01-overview.html` | bento Overview at 1100×720 and 800×600 |
| `02-timeline.html` | timeline with the playhead-anchored jump control, zoom segmented, drawer open, frame inspector |
| `03-quick-access-search.html` | Search mode, plus the empty and no-match states |
| `04-quick-access-ask.html` | Ask mode, plus the collapsed / frame-attached / answering states of "ask about this screen" |
| `05-settings-general.html` | the no-sidebar settings shell — General + Capture, and the filter applied |
| `06-settings-intelligence.html` | Intelligence, and the six custom inputs at working size |
| `07-components.html` | component sheet, type + spacing specimen, per-page pattern audit |
| `08-journal.html` | the day as a river — digest lede, four honest stats, the four card states, and the receipt (frames, scrub with evidence ticks, filmstrip, 1×/2×/8×/16×, speaker transcript) |
| `09-subjects.html` | tiers by conviction with the sparkline as the row's hero, plus one subject opened — conclusion strip, hero, and the story-over-time spine |
| `10-context.html` | authored context: composer, the standing ledger with edit/delete, the dismissed archive with restore, and Authored-vs-Inferred in the inspector |
| `11-settings-complete.html` | every registered settings row — 5 groups, 19 sections, **96 rows** — at true size and then unrolled at full length, with the G7/G8/G9/G10 amendments applied |
| `shots/` | rendered verification screenshots, both themes, 1280×800 full-page |

Each page is self-contained (inline CSS, inline SVG, no external assets), designs both themes, and
carries a light/dark toggle top-right that overrides the system preference.
