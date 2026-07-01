# Mnema — Whole-App UI/UX Audit

## Executive summary

Mnema is a mature, design-system-driven desktop app: a single `--app-*` token system with full light/dark parity, a 6-step `--text-*` scale, unusually thorough loading/empty/error/success state coverage, and accessibility wired to assistive-tech depth (focus rings, aria-live, roving tabindex, reduced-motion guards). Across all 12 review units the audit surfaced **85 findings — 0 high, 26 medium, 44 low, 15 nit** — none of which break trust or core usability. The dominant pattern is *leaky consumption of an excellent foundation*: dozens of hardcoded px font-sizes and black scrims/shadows bypass tokens that already exist (so several surfaces won't flip cleanly to light mode), and the destructive-red family is overloaded for benign playback/active states. The two surfaces re-reviewed in chunks after an earlier degenerate pass — **Settings (25 findings)** and **Onboarding (10)** — are well-engineered but carry the most concentrated hierarchy, gating, and feedback-locality gaps. Verdict by lens: **Theme adherence B+** (world-class system, undisciplined consumption), **User feedback A−** (exemplary coverage, a handful of locality/parity gaps), **At-a-glance clarity B+** (strong, but hero/primary-action hierarchy is soft in several panels).

## Verdict by lens

### Theme adherence — B+
Foundation is excellent: one mono typeface, a documented token system with WCAG reasoning, zero hardcoded hex/named colors in component CSS, and tinted (not inverted) light-mode shadows. The leaks are systemic-but-shallow:
- **Detail modals hardcode a black scrim** that flips wrong in light mode (`--app-overlay-bg` exists and is ignored).
- **Shared form-control inset recess is hardcoded `rgba(0,0,0,.25)`** so it never softens in light mode.
- **Destructive/error red reused for playback, active, and selected** affordances, diluting the error signal.
- **Type scale bypassed** by ~22 distinct hardcoded px sizes (plus fractional half-steps and sub-10px floors) across the app, and again inside Settings (63 px literals) and Onboarding (down to 9px).
- **Two icon systems** at seven stroke weights; **category palette collides** with reserved danger/accent hues.
What's good: complete dark+light token blocks, fully tokenized text color, soft canonical popover shadow, uniform tokenized focus rings.

### User feedback — A−
Among the best state coverage I'd expect in production: shared controls all carry hover/focus/active/disabled+error+reduced-motion; empty/loading/error states are designed almost everywhere; destructive flows use plugin-dialog confirms; micro-interactions (copy success/failure flash, 500ms latched reload spin) are handled. Gaps are localized:
- **Duplicate "Thinking…" indicator** stacks during the reasoning phase (reads as a glitch).
- **ActionSelect async** dims to "disabled" not "busy" with no visible spinner.
- **Settings save acknowledgment** is a 6px dot in the rail, spatially detached from the edit.
- **Status pill stays amber** on healthy OCR/Speakers/UserContext cards (chip contradicts its green card).
- **Conflict banner detached** from the offending shortcut row; **rail/.btn families miss `:active`**.
What's good: live regions, focus traps + return-focus, aria-busy, two-tier exit gating with self-explaining disabled buttons.

### At-a-glance clarity — B+
Strong active-nav treatments and skeleton-matched loading mean "you are here" and "still working" read well. The recurring weakness is hierarchy of the *most important* thing on a surface:
- **Settings About has no visual hero** — the product name is a plain row label.
- **No primary-action hierarchy in the Data panel** — Browse, Restart, Run cleanup, and destructive Revoke are all the same ghost button.
- **Same model-download CTA is primary in one panel, ghost in five others.**
- **Hero KPI numbers carry no period-over-period delta/direction.**
- **Shortcut categories read as siblings of their parent section.**
What's good: textbook multi-signal active nav, scannable semantic status dots, deliberate consent-screen trust hierarchy.

## Top priorities (do these first)

1. 🟠 **Status pill stays amber on an available card** — the green "available" card carries an amber "available" chip, an internal contradiction that undermines the status surface. Add `class:model-status__pill--ok={…available}` (matching Transcription/AskAi/Providers). `Ocr.svelte:238`, `Speakers.svelte:127`, `UserContext.svelte:97`
2. 🟠 **Destructive-red reused for playback/active/selected** — the play button, scrub fill, active tick and selected bar use the same `--app-danger` as real errors in the same drawer. Repoint to the existing `--app-status-running-*` / `--app-record-glyph-start` recording-red tokens. `routes/+page.svelte:8104`, `:8169`, `:10204`, `:10273`
3. 🟠 **Detail modals hardcode a black scrim (breaks light mode)** — backdrops stay dark while the rest of the app's overlays flip light. Replace `rgba(0,0,0,0.42)` with `var(--app-overlay-bg)`. `CategoryDetailModal.svelte:304`, `AppDetailModal.svelte:161`, `FocusDetailModal.svelte:265`
4. 🟠 **Data panel has no primary-action hierarchy** — Restart (resolves a pending-restart warning) and the destructive Revoke look identical to a benign Browse. Promote contextual `btn--primary` for Restart when `pendingRestart`, reuse the existing `btn--danger` for Revoke. `Storage.svelte:146`, `Access.svelte:102`
5. 🟠 **Same "Download (size)" CTA is primary in one panel, ghost in five** — emphasis no longer signals importance. Pick one convention (primary = install step) and apply it uniformly; fix the stale `btn--primary` comment. `SemanticSearch.svelte:171/188` vs `Ocr.svelte:274`, `Transcription.svelte:287`, `Speakers.svelte:160`
6. 🟠 **Tool-call-limit controls stay live when Ask AI is off** — the model picker greys out but the limit switch/stepper don't, an inconsistent inert signal. Add `disabled={!rec.draftAskAiEnabled}`. `AskAi.svelte:66`, `:72`
7. 🟠 **"On Disconnect" policy is shown and inert in System Default mode** — the resolver only reads it for `SpecificDevice`. Gate the row on `draftPreferenceMode === 'specific_device'` like the device-pick row. `Audio.svelte:114`
8. 🟠 **Shortcut conflict banner is detached from the offending row** — save is globally blocked but the red row may be scrolled off in a sibling group with no jump. Make the banner anchor/scroll to the first conflict. `Shortcuts.svelte:92`
9. 🟠 **No obvious exit from Settings** — the only return is a dimmed surface-toggle segment; re-clicking the gear is a no-op. Add an explicit Done/Back, or make the gear toggle back to the last surface. `routes/+layout.svelte:1373`, `:1162`
10. 🟠 **Duplicate "Thinking…" indicator** — the live reasoning header and the standalone working line both render during the thinking phase. Add `&& !reasoningIsLive(turn)` to the working-line guard. `quick-recall/+page.svelte:4440`
11. 🟠 **Shared form-control inset recess hardcoded (breaks light mode)** — every field/select/combobox/stepper outside Settings keeps a 25%-black inner shadow on white. Hoist `--app-input-recess` to a global token (0.25→0.08 light) and consume it. `Input.svelte:61`, `Select.svelte:207`, `Combobox.svelte:242`, `Stepper.svelte:205`
12. 🟠 **CLI not-installed / not-in-PATH reads as muted boilerplate** — a real failure shows in the same dim grey as a neutral privacy note. Lift it into a warn-tinted note (reuse `.cloud-egress-disclosure`'s `--app-warn`/`--app-warn-border` or `.group-hint--warn`). `Access.svelte:59`

## Findings by surface

### App shell / layout
*The titlebar is a mature, theme-disciplined surface with full light/dark parity, focus rings/aria-live/reduced-motion on essentially every control, and responsive degradation that never hides record/nav/settings; the weaknesses are left-cluster hierarchy/signifiers and a couple of state gaps.*
**Strengths:** exemplary token discipline (one stray hardcoded color in the whole chrome), multi-signal recording state, aria-live status/notifications, focus-trapping modals, priority-ordered responsive shedding.

| Severity | Lens | Finding | User impact | Fix | Evidence |
|---|---|---|---|---|---|
| 🟠 | feedback | Notifications bell conditionally mounted | Bell pops in/out so the gear/help/theme icons shift under the cursor when alerts arrive/clear | Keep a persistent bell slot (quiet rest state) or animate insertion | `+layout.svelte:1224` |
| 🟠 | at-a-glance | No obvious way out of Settings | Exit is only a dimmed surface-toggle segment; gear re-click is a no-op | Add explicit Done/Back or toggle gear back to last surface | `+layout.svelte:1373`, `:1162` |
| 🟡 | at-a-glance | Idle "Record" primary action understated | Record pill shares dark-pill chrome with neighbors (it is the lone red element, so overstated) | Filled accent/danger treatment, bolder glyph | `+layout.svelte:1577`, CSS `2686` |
| 🟡 | at-a-glance | Source toggle pills read as passive status chips | New users may not read idle toggles as clickable (they do carry check/slash + bg diff) | Strengthen at-rest distinction vs live pills | `+layout.svelte:1086`, CSS `2516` |
| 🟡 | feedback | Bell has no open/pressed state | No tie between open popover and the bell, unlike the help button | Add `class:active={notificationsOpen}` | `+layout.svelte:1226` |
| 🟡 | at-a-glance | Full-width titlebar control-dense; nav competes with Search | Surface toggle shares chrome with the adjacent Search pill | Give toggle primacy (size/weight/separation) | `+layout.svelte:2181`, `:2238` |
| ⚪ | theme | Active-accent token differs across peer controls | Toggle uses `--app-accent`, icons use `--app-accent-strong` (intentional, contrast-driven) | Optional alignment; defensible as-is | `+layout.svelte:2219`, `:2749` |
| ⚪ | theme | Hardcoded white inset highlight on `kbd` | Faint keycap sheen vanishes in light mode (bevel survives via token) | Tokenize the highlight | `+layout.svelte:3181` |

### Dashboard (main)
*A media-review/scrubber surface (not a KPI dashboard) executed with rigor — exemplary state coverage, token-driven theming with a full hand-built light-mode block; the main weakness is semantic color reuse.*
**Strengths:** load-error-with-retry, distinct empty states, OCR running/empty/missing/error variants, audio-drawer media/transcript/speaker states with role=alert/status; `SearchResultCard` as a model component; real `role=slider` scrubber.

| Severity | Lens | Finding | User impact | Fix | Evidence |
|---|---|---|---|---|---|
| 🟠 | theme | Danger red reused for playback/active/selected | Error red no longer pops next to the same-red transport | Repoint to `--app-status-running-*`/`--app-record-glyph-start` | `+page.svelte:8104`, `:8169`, `:10204`, `:10273` |
| 🟡 | theme | OCR "running" bg is a hardcoded warn literal | Running tint won't flip in light mode | `color-mix(in srgb, var(--app-warn) 6%, transparent)` | `+page.svelte:9626` |
| 🟡 | feedback | Audio-bar hover ring hardcoded white | Near-invisible hover feedback in light mode | Token-drive ring (`--app-border-hover`) or light override | `+page.svelte:10186` |
| ⚪ | theme | Audio-drawer close uses text "×" not SVG X | Slightly heavier/off-center vs sibling close glyphs | Swap to the 1.4-stroke SVG X | `+page.svelte:7346` |
| ⚪ | feedback | Reject hover adopts affirmative accent | Amber "Reject" turns accent-green on hover like Confirm | Warn-tinted hover for the reject variant | `+page.svelte:8845` |

### Insights / Chat
*One of the most carefully built surfaces — near-exemplary state handling, SIDEBAR-correct rail, strong token discipline; the biggest weakness is modal theme parity plus a couple of at-a-glance gaps.*
**Strengths:** loading-skeleton/empty/error-with-retry on every tile (skeletons match layout), multi-signal rail active state, Send→Stop morph that switches accent→danger, ARIA-separator drag resizer, per-element chart aria-labels.

| Severity | Lens | Finding | User impact | Fix | Evidence |
|---|---|---|---|---|---|
| 🟠 | theme | Detail modals hardcode scrim + shadow | Black veil in light mode while other overlays flip light | `var(--app-overlay-bg)` for scrim | `AppDetailModal.svelte:161`, `CategoryDetailModal.svelte:304`, `FocusDetailModal.svelte:265` |
| 🟠 | at-a-glance | Hero KPI numbers show no delta/direction | Can't tell "up or down vs last period" at a glance | Add a small delta line, color only the delta | `Overview.svelte:1200` |
| 🟡 | at-a-glance | Collapsing rail hides ALL navigation | Loses "you are here" + sub-surface switching when collapsed | Collapse to a slim icon rail | `InsightsRail.svelte:86`, `insights/+page.svelte:326` |
| 🟡 | at-a-glance | Category chart uses 8 hues | Beyond ~5-6 hues, segment→legend mapping breaks (legend mitigates) | Cap at top ~5 + "Other" | `activity-helpers.ts:10` |
| 🟡 | theme | Category palette collides with semantic colors | "Entertainment"=danger red, "Creating"=accent green in both themes | Rotate those two hues off the exact tokens | `+layout.svelte:1732`, `:1725` |
| ⚪ | theme | `--app-shadow-pop` typo on jump pill | Always falls back to a heavier off-token shadow | Fix to `--app-shadow-popover` (keep small geometry) | `Chat.svelte:1545` |
| ⚪ | feedback | Non-clickable exhibit cards highlight on hover | Locked/empty tiles imply interactivity | Move hover border to `.exhibit--clickable:hover` only | `Overview.svelte:2037` |

### Quick Recall (Ask AI)
*High-quality, product-grade door: designs for every phase, 100% tokenized color, strong a11y; weaknesses are small feedback/at-a-glance polish.*
**Strengths:** seeding/thinking/writing/done/error/empty/stopped all distinct; copy button designs both success and failure flashes; reduced-motion gated for the surface; polite phase announcer + aria-busy transcript.

| Severity | Lens | Finding | User impact | Fix | Evidence |
|---|---|---|---|---|---|
| 🟠 | feedback | Duplicate "Thinking…" indicator | Two stacked thinking lines read as a render glitch | Add `&& !reasoningIsLive(turn)` to the working-line guard | `+page.svelte:4440` |
| 🟡 | feedback | Streaming caret ignores prefers-reduced-motion | Lone surviving animation under Reduce Motion | Add reduced-motion block (`animation:none`) | `AnswerProse.svelte:379` |
| 🟡 | feedback | Initial ask input lacks the composer's focus ring | Two identical-looking textareas give different focus feedback | Pick one idiom (add or drop the ring) | `+page.svelte:5731`, `:5896` |
| ⚪ | at-a-glance | "Open in browser" chip invisible until hover | Mouse users never learn the captured page can be opened | Faint resting state (opacity ~0.35) | `AnswerSourceCard.svelte:339` |

### Access request (consent)
*One of the strongest surfaces — a security-literate consent dialog with a full state machine and deliberate trust signaling; weaknesses are minor.*
**Strengths:** loading/error-with-retry/empty/receipt/inline-error states + 6s watchdog; neutral padlock (not a green shield); focus parked on Deny, Esc denies, Enter unbound; warn-tinted "cannot be verified" caveat; semantic color per COLORS.md.

| Severity | Lens | Finding | User impact | Fix | Evidence |
|---|---|---|---|---|---|
| 🟡 | at-a-glance | Identity-source chip identical for all provenances | Explicit/inferred/none differ only by text (broad warn-note already present) | Tint inferred/none with `--app-warn`; don't green "explicit" | `+page.svelte:686`, `:270` |
| 🟡 | theme | Requester name hardcoded 14px (off scale) | Load-bearing "who is asking" line won't track the scale | `var(--text-md)` or add a token | `+page.svelte:679` |

### Shared control library
*A genuinely strong, professional kit with careful a11y and full state coverage; weaknesses are token discipline plus one async-feedback gap.*
**Strengths:** stable aria wiring, roving tabindex + focus-follows-selection, WKWebView focus workaround, single `--app-disabled-opacity`, reduced-motion guards, Select/Combobox empty+loading popover states, `FieldWarning` persistent inline error.

| Severity | Lens | Finding | User impact | Fix | Evidence |
|---|---|---|---|---|---|
| 🟠 | feedback | ActionSelect async dims to "disabled", no spinner | In-flight pick reads as unavailable; loading row is in a closed popover | Inline busy spinner + `--app-busy-opacity` on the closed trigger | `ActionSelect.svelte:73`, `Select.svelte:131` |
| 🟡 | theme | Recessed inset shadow hardcoded black | 25%-black inner shadow on near-white fields in light mode | Add/consume `--app-shadow-inset` (softer light value) | `Input.svelte:61`, `Select.svelte:207`, `Combobox.svelte:242`, `Stepper.svelte:205` |
| 🟡 | theme | Font sizes hardcoded incl. off-scale 11.5px | Scale change won't propagate; 11.5px maps to no token | Use tokens; 11.5px→`--text-sm` | `Select.svelte:336`, `Combobox.svelte:404`, `Switch.svelte:94` |
| 🟡 | theme | Focus rings reinvented inline | Future ring tweak silently misses these controls | Consume `--app-ring`/`--app-ring-danger` | `Switch.svelte:122`, `Checkbox.svelte:129`, `Stepper.svelte:176` |
| 🟡 | at-a-glance | Eyebrow letter-spacing 0.12em vs 0.08em | Same uppercase label idiom tracked two widths, co-located | Promote to one shared class (0.12em) | `Select.svelte:191`, `ScreenResolutionControl.svelte:98` |
| ⚪ | feedback | Toggles lack pressed/`:active` state | No momentary press cue before the state flip (flip mitigates) | Subtle `:active`; gate Checkbox hover for parity | `Switch.svelte:99`, `Checkbox.svelte:123` |

### Settings
*Well-engineered shell + Codex rail (token-driven colors, textbook active-nav, thorough loading/empty/error states) across General/Capture/Intelligence/Data panels; the weak spots cluster in at-a-glance hierarchy (no About hero, flat shortcut nesting, no primary-action emphasis), a px type scale that bypasses `--text-*` (63 literals, one below the 10px floor), feedback locality (save dot, conflict banner), and conditional-control gating (inert disconnect policy, ungated tool-call controls, amber-on-green status pills).*
**Strengths:** multi-signal active nav (8% tint + glowing left bar + tinted icon + strong label); genuinely complete state coverage (checking pill, loading notices, no-match empty, "Copied" min-width swap, per-source retry banners); fully tokenized colors with a `[data-theme=light]` override; visually-hidden `<h1>`, roving tabindex, aria-current, aria-live save status; professionally stateful shortcut-capture; uniform SettingGroup/SettingRow scaffolding; consent-led AI disclosures.

| Severity | Lens | Finding (area) | User impact | Fix | Evidence |
|---|---|---|---|---|---|
| 🟠 | at-a-glance | About has no visual hero (About) | "Mnema" renders at the same 13px/550 as action rows | Dedicated identity block: name at `--text-lg/--text-xl` bold | `About.svelte:73`, `SettingRow.svelte:135` |
| 🟠 | feedback | Save acknowledgment is a far-off 6px dot (rail footer) | Edits near bottom-right can't tell the change registered | Raise footer salience on change or transient inline confirm | `SettingsRail.svelte:244` |
| 🟠 | at-a-glance | Shortcut categories read as siblings of parent (General) | Five equal-weight sections instead of one parent + four children | Differentiate nested titles (smaller/lighter/inset) | `Shortcuts.svelte:104`, `SettingGroup.svelte:70` |
| 🟠 | theme | Shortcut rows off-scale 12px + inverted weight (General) | Nested 12px/700 title heavier than the 13px/550 parent label | Route through `--text-*`; keep title ≤550 weight | `settings-groups.css:77`, `SettingRow.svelte:136` |
| 🟠 | feedback | Conflict banner detached from row (General) | Save blocked but red row scrolled off below with no jump | Anchor/scroll banner to first conflicting row | `Shortcuts.svelte:92`, `:112` |
| 🟠 | at-a-glance | "On Disconnect" shown + inert in System Default (Capture) | A setting that silently does nothing | Gate row on `specific_device` (mirror device-pick row) | `Audio.svelte:114` |
| 🟠 | feedback | Status pill amber on available card (Intelligence) | Green card carries an amber "available" chip — internal contradiction | Add `class:model-status__pill--ok={…available}` | `Ocr.svelte:238`, `Speakers.svelte:127`, `UserContext.svelte:97` |
| 🟠 | at-a-glance | Same download CTA primary here, ghost elsewhere (Intelligence) | Identical task looks like two priorities | One convention app-wide; fix stale comment | `SemanticSearch.svelte:171/188` vs `Ocr.svelte:274` |
| 🟠 | feedback | Tool-call-limit controls live when Ask AI off (Intelligence) | Switch/stepper interactive though inert, while model picker greys | `disabled={!rec.draftAskAiEnabled}` on both | `AskAi.svelte:66`, `:72`, `:102` |
| 🟠 | at-a-glance | No primary-action hierarchy (Data) | Browse, Restart, cleanup, destructive Revoke all ghost | `btn--primary` Restart (pendingRestart); `btn--danger` Revoke | `Storage.svelte:146`, `Access.svelte:102` |
| 🟠 | feedback | CLI not-installed/not-in-PATH reads as boilerplate (Data) | Real failure shown as dim muted body text | Warn-tinted note (`--app-warn`/`--app-warn-border`) | `Access.svelte:59`, `settings-theme.css:196` |
| 🟡 | theme | Type scale hardcoded px, one below 10px floor (rail/groups) | Scale change won't propagate; 9px dt labels too small | Map to tokens; raise dt to ≥`--text-xs` | `settings-layout.css:224`, `settings-groups.css:309` |
| 🟡 | at-a-glance | Source/Release-notes links barely read interactive (About) | Look like muted body text until hover | Persistent signifier (underline or accent label) | `About.svelte:107`, `settings-groups.css:331` |
| 🟡 | at-a-glance | Per-row reset/clear icon-only + overloaded glyph (General) | Meaning only on hover tooltip; rotate-ccw = both reset & restore-all | Distinct icons or hover text labels | `Shortcuts.svelte:143`, `:76/:150` |
| 🟡 | feedback | Row edit controls live during in-flight save (General) | Capture/reset/clear not disabled mid-save while header is | Gate row controls on `savingKeyboardBindings` | `Shortcuts.svelte:121`, `:143`, `:152` |
| 🟡 | theme | Unicode glyphs vs lucide in capture notices (Capture) | Mixed icon language; ⏳ ignores the palette | Swap literals for lucide (info/loader/triangle-alert/check) | `Video.svelte:46`, `Audio.svelte:8` |
| 🟡 | at-a-glance | Bitrate stacks redundant notices (Capture) | Always-on compat notice duplicates the group hint verbatim | Remove the compat notice; keep active preset/custom hint | `Video.svelte:113`, `:167` |
| 🟡 | at-a-glance | Title and pill restate the same word (Intelligence) | Chip wastes its distinctness; Providers title/pill disagree ("ready"/"available") | Title = sentence, pill = one canonical word; align Providers | `AskAi.svelte:130/136`, `Providers.svelte:310/326` |
| 🟡 | at-a-glance | Semantic Search hint is a paragraph wall (Intelligence) | ~63-word hint buries the cost caveat | Trim to one line; move cost to a warn line by the switch | `SemanticSearch.svelte:56` |
| 🟡 | theme | RetentionPicker chip hardcodes 12px (Data) | Chips won't track the type scale | `font-size: var(--text-base)` | `RetentionPicker.svelte:133` |
| 🟡 | at-a-glance | Read-only Save Directory looks editable (Data) | Recessed input invites typing, but it's readonly | Render as a path/code chip or de-chrome the field | `Storage.svelte:118` |
| 🟡 | feedback | Destructive-cleanup confirmation is dim grey (Data) | Permanent deletion confirmed only by muted hint text | Accent-tinted success note (`--app-accent`; no success token) | `Storage.svelte:182` |
| ⚪ | feedback | Rail buttons/nav have no `:active` (rail) | Clicks feel unacknowledged on slow switches; `.btn` family has `:active` | Add subtle `:active` to `.rail-back`/`.nav-item`/`.about-link` | `settings-layout.css:103`, `:232` |
| ⚪ | feedback | Microphone loading is plain text (Capture) | Body copy static (header ReloadButton already spins) | Optional inline spinner beside the text | `Audio.svelte:23`, `:29` |
| ⚪ | theme | Stack gaps off the 4-pt grid, disagree 10 vs 12px (Intelligence) | Minor vertical-rhythm drift between panels | Standardize to one grid value via shared class | `AskAi.svelte:160`, `Providers.svelte:382` |

### Onboarding flow
*A polished, well-instrumented flow — welcome → configure → finish — with outstanding gating clarity (disabled CTAs name what blocks them and jump to it), thorough beyond-happy-path state coverage, and disciplined semantic pill system across permission/model bodies; the gaps are an inability to exit before the finale, hardcoded sub-token font sizes (down to 9px on the most complex screen), a missing pressed state on the shared `.btn` family, and a few motion/scannability polish items.*
**Strengths:** blockReason/finaleBlockReason name the blocker and offer a one-click jump; loading/error-with-retry/download/success states everywhere with reduced-motion fallbacks; two-tier finale exit gating; semantic pill vocabulary learned once; hoisted lock-callouts kept clickable outside the inert subtree; live-region phase announcements + programmatic heading focus.

| Severity | Lens | Finding (area) | User impact | Fix | Evidence |
|---|---|---|---|---|---|
| 🟠 | theme | AskAiBody scoped CSS hardcodes fonts to 9px (AskAiBody) | Provider labels/tags won't scale; 9-10px hard to read on the busiest screen | Use `--text-base/sm/xs`; move 9px tag to `--text-xs` | `AskAiBody.svelte:270`, `:304`, `:314` |
| 🟠 | theme | Hardcoded 11/12px bypass the scale (Welcome/Finale/Stack) | Error/hint/jump text drifts on a scale change | `var(--text-sm)`/`var(--text-base)` | `WelcomeScreen.svelte:133`, `FinaleScreen.svelte:107`, `FeatureStack.svelte:193` |
| 🟠 | feedback | Shared `.btn` family has no `:active` (onboarding-body.css) | Every secondary action gives no press feedback (unlike `.cta`) | Add `.btn:active:not(:disabled)` press state | `onboarding-body.css:458-505`, `.cta:active` `:390` |
| 🟡 | at-a-glance | No way to abandon before the finale (flow shell) | finish(false) escape lives only on the last screen | Optional "Skip for now" on Welcome/Configure | `WelcomeScreen.svelte:40`, `FinaleScreen.svelte:76` |
| 🟡 | at-a-glance | Faster "recommended" path demoted under heavy note (Welcome) | Lower-friction route reads as the heavier one (ghost + 3-sentence note) | Co-primary the recommended path or tighten the note | `WelcomeScreen.svelte:56`, `:73` |
| 🟡 | feedback | Primary CTA glows on infinite loop + 2 ring pulses (Finale) | Restless ambient motion reads nervous (reduced-motion guarded) | Glow on entrance/hover only; quiet one ring | `onboarding-screens.css:417`, `:327` |
| 🟡 | at-a-glance | Configure collapses to one dot — no intra-step progress (stepper) | The 11-row accordion is one undifferentiated dot | Show "X of Y ready" or a thin active-segment fill | `+page.svelte:34`, `:158` |
| 🟡 | at-a-glance | Status label duplicated in meta + pill (Ocr/Tx/Speakers bodies) | Same word twice weakens the pill's job | Follow SemanticSearchBody (status only in pill) | `OcrBody.svelte:126/129`, `TranscriptionBody.svelte:132/135` |
| 🟡 | feedback | Download disables silently with no provider (SemanticSearchBody) | Greyed button, no "why" for a no-provider model | Show "No download source — pick another" hint | `SemanticSearchBody.svelte:132` |
| ⚪ | feedback | Attention-jump hover color is a no-op (FeatureStack) | Hover re-sets the identical `--app-warn`; only underline changes | Hover to `--app-warn-strong` + add `:active` | `FeatureStack.svelte:221`, `:231` |

### Theme & color consistency (whole app)
*Theme discipline is excellent — one source of truth with full light+dark blocks, WCAG reasoning in comments, zero hardcoded hex/named colors in components; remaining issues are narrow token-consumption leaks, mostly light-mode parity.*
**Strengths:** complete parity token system, documented contrast, remarkably token-clean components, properly tinted light-mode shadows.

| Severity | Lens | Finding | User impact | Fix | Evidence |
|---|---|---|---|---|---|
| 🟠 | theme | Three Insights modals hardcode a black scrim | Dark veil in light mode vs the app's light overlays | `var(--app-overlay-bg)` | `CategoryDetailModal.svelte:304`, `FocusDetailModal.svelte:265`, `AppDetailModal.svelte:161` |
| 🟡 | theme | Chat jump pill references non-existent shadow token | Off-token fallback shadow always wins | Fix typo to `--app-shadow-popover` (keep small geometry) | `Chat.svelte:1545` |
| 🟡 | theme | Timeline OCR warn-hue alpha fill is a magic number | Won't flip in light mode; drifts if warn retuned | `color-mix(in srgb, var(--app-warn) 6%, transparent)` | `+page.svelte:9626` |
| 🟡 | theme | Shadows largely un-tokenized | Light-mode depth reads inconsistently | Promote popover/modal shadows to light+dark tokens | `+layout.svelte:1655`, `CategoryDetailModal.svelte:316` |

### Typography system (whole app)
*Strong intentional foundation — one mono typeface, a centralized 6-step scale, 100% tokenized text color, genuinely tightened large headings; the weakness is scale discipline.*
**Strengths:** single brand mono face, fully tokenized text greys clearing AA, tightened display titles, measure-limited reading-comfort prose.

| Severity | Lens | Finding | User impact | Fix | Evidence |
|---|---|---|---|---|---|
| 🟠 | theme | 6-step scale bypassed by ~22 hardcoded px sizes | Hierarchy reads fuzzy (12 vs 12.5 vs 13px) | Treat `--text-*` as source of truth; add named steps | `+layout.svelte:1657`, repo-wide literals |
| 🟠 | at-a-glance | Sub-floor 7-9px sizes (9px ×40+) | Tiny labels genuinely hard to read on a mono face | Hard 10px floor; de-emphasize via color, not shrinking | `SubjectDetail.svelte:1152`, `MiniBars.svelte:128` |
| 🟡 | theme | Fractional half-steps (10.5/11.5/12.5px) | Blur on non-Retina; read as ad-hoc nudges | Snap to nearest token | `Select.svelte:334`, `Combobox.svelte:402` |
| 🟡 | at-a-glance | 18px headings off-scale, inherit 1.6 leading | Title floats from subtitle; ladder jumps 16→20 | Add ~18px step + `line-height:1.2` + tighter tracking | `Overview.svelte:1906`, `CategoryDetailModal.svelte:346` |

### States & feedback coverage (whole app)
*Unusually strong, mature coverage — among the best expected in production; the few weaknesses are theme bypasses and a couple of localized feedback gaps.*
**Strengths:** shared controls are a real design system; empty/loading/error states with CTAs nearly everywhere; plugin-dialog destructive confirms; latched 500ms reload spin; live regions, focus traps, aria-busy/aria-invalid wiring.

| Severity | Lens | Finding | User impact | Fix | Evidence |
|---|---|---|---|---|---|
| 🟡 | theme | OCR "running" bg hardcoded amber | Won't flip in light mode (6% wash, transient) | `var(--app-warn-bg)` or `color-mix` | `+page.svelte:9626` |
| 🟡 | feedback | Dedicated-window Close lacks focus ring + `:active` | Keyboard users can't tell it's focused; no press cue | Add `:focus-visible` (`--app-ring`) + `:active` | `+layout.svelte:2109` |
| 🟡 | feedback | Record/Stop/Pause text-only loading | Highest-stakes async action can look frozen | Add shared spinner + aria-busy on stop/pause | `+layout.svelte:1001`, `:988`, `:977` |
| ⚪ | theme | Insights modals hardcode dark panel shadow | Future central elevation change won't reach them | Route through a token (`--app-shadow-modal`) | `CategoryDetailModal.svelte:316` |
| ⚪ | theme | Insights modal scrim raw black, no token | No central scrim token (appearance fine) | Add a dark-in-both-modes `--app-scrim` (not `--app-overlay-bg`) | `CategoryDetailModal.svelte:304` |

### Icons, shadows & depth (whole app)
*Depth/elevation handled with real token-level discipline (lift by lightness, soft canonical popover shadow, uniform focus rings); weaknesses are consistency leaks where surfaces bypass tokens, plus two parallel icon systems.*
**Strengths:** by-the-book dark-mode depth, textbook-soft popover shadow, consistently tokenized focus feedback, single managed Lucide family on settings/insights rails.

| Severity | Lens | Finding | User impact | Fix | Evidence |
|---|---|---|---|---|---|
| 🟠 | theme | Form controls hardcode dark inset recess | 25%-black inner shadow muddies white fields in light mode | Hoist `--app-input-recess` global (0.25→0.08 light) | `Input.svelte:61`, `Select.svelte:207`, `Combobox.svelte:242`, `Stepper.svelte:205` |
| 🟠 | theme | Two icon systems, inconsistent stroke weights | Chrome doesn't read as one icon family (2 vs 1.7 vs 1.6) | Standardize bespoke glyphs onto Lucide / one ~1.7 stroke | `+layout.svelte:955`+, `section-icons.ts` |
| 🟡 | theme | Popovers/modals hardcode one-off shadows | TimelineJumper's 0.55-black pops harder than all others | Pull outliers to house 0.22; consume `--app-shadow-popover` | `TimelineJumper.svelte:685`, `+page.svelte:8536` |
| 🟡 | theme | `--app-shadow-pop` typo → harsher fallback | Jump pill's 35%-black shadow can't be retuned centrally | Define a small `--app-shadow-pop` or lower fallback to 0.22 | `Chat.svelte:1545` |
| ⚪ | at-a-glance | Source-card thumbnail glyph uses 1.1 stroke | Raw-value outlier (rendered weight/faintness intentional) | Nudge raw stroke to ~1.4-1.5; keep faint color | `AnswerSourceCard.svelte:90`, `SearchResultCard.svelte:87` |

## What's working well

- **A real, single token system.** `--app-*` colors with full light+dark parity and WCAG reasoning in comments; zero hardcoded hex or named colors in component CSS.
- **State coverage is the app's signature strength.** Loading-skeleton / empty / error-with-retry / populated is designed almost everywhere, skeletons match final layout, and destructive flows use plugin-dialog confirms.
- **Accessibility reaches assistive tech, not just sighted users.** Focus rings, focus traps + return-focus, aria-live regions, aria-busy, roving tabindex, and reduced-motion guards are pervasive.
- **Multi-signal active states.** Both the Settings rail and Insights rail stack ≥2 signals (tint + bar + tinted icon + strong label) so "you are here" survives the squint test.
- **Deliberate trust design** on the consent surface (neutral padlock, Deny-focused, Esc-denies, warn-tinted unverifiable caveat).
- **Thoughtful micro-interactions:** copy success/failure flashes, latched 500ms reload spin, Send→Stop accent→danger morph, two-tier onboarding exit gating with self-explaining disabled buttons.

## Methodology & confidence

- **Scope:** 12 review units — **8 per-page** (App shell, Dashboard, Insights/Chat, Quick Recall, Access request, Shared controls, Settings, Onboarding) and **4 cross-cutting** (Theme & color, Typography, States & feedback, Icons/shadows/depth) — evaluated against the `ui-ux-patterns` skill heuristics and the app's own `--app-*` / `--text-*` token system.
- **Verification:** every finding was adversarially verified against the actual source; the merged set carries `confirmed`/`adjusted` verdicts, and several were downgraded (e.g. idle Record, identity chip, modal shadows/scrims) or had counts/line numbers corrected where the original overstated impact.
- **Re-review note:** Settings and Onboarding were re-reviewed in small chunks after an earlier degenerate pass produced unusable output; their 35 findings here come from that fresh, code-grounded re-review (5 Settings chunks + 2 Onboarding chunks, folded into one section each).
- **Totals:** 85 findings — 0 high, 26 medium, 44 low, 15 nit.
- **Blind spots:** this is static code review only — no live screenshots or running app, so runtime states (hover/active/light-mode rendering, animation feel, real OCR/download timing) are inferred from CSS and markup rather than observed. Light-mode parity claims are derived from token definitions and the presence/absence of `[data-theme=light]` overrides, not pixel inspection.