# User Context — Insights mockups: build guide

These are the **final, approved** UI mockups for Mnema's **User Context** feature — the
new **Insights** surface and its sub-surfaces. They are static HTML/CSS/JS (no framework,
no build step) that mirror Mnema's monospace, terminal-style desktop look in both dark and
light themes. This README is the master guide for turning them into the real app: each
mockup also carries an inline `<!-- BUILD NOTES -->` comment that points back here.

The real app is a **SvelteKit SPA** in `apps/desktop/src` with a **Rust/Tauri** backend.
Build the real UI in Svelte against the app's own design tokens (not by copying this CSS),
wiring it to new Rust Tauri commands and storage that **do not exist yet** (design stage).

**Source of truth for the domain model and decisions:** [`../CONTEXT.md`](../CONTEXT.md).
Where this guide and `CONTEXT.md` ever disagree, `CONTEXT.md` wins.

## How to view

Open [`index.html`](index.html) in a browser — it links every surface. Use the **☀ / ☾**
toggle (top-right of every page) to check both themes; the whole surface re-skins from a
single `data-theme` flip. There is no server or build step; just open the files.

## 1. Chosen design direction

A synthesis settled during design review:

- **Overview = a narrative-first Briefing.** One full-width centered column led by **The read**
  — an engine-tier AI-narrative hero that synthesizes the range *and* owns the headline numbers —
  with the metric charts demoted below it into a quieter **Exhibits** supporting-evidence strip,
  then the actionable **What changed / Needs attention** tail, and a docked **Ask** bar. The free
  tier swaps the AI hero for a deterministic factual read plus an enable-the-engine invite.
- **Chat is its own dedicated surface**, a Claude/ChatGPT-style conversation workspace — not
  a popover. It shares one engine and one persistent conversation store with Quick Recall.
- **Top segmented surface-switcher nav, no left rail.** Inside **Main**, a titlebar segmented
  control switches **Timeline ⇄ Insights**; inside Insights, a slim horizontal sub-surface
  tab bar (`.subnav`) switches Overview / Subjects / Context / Chat. Content runs full-width.
- **One unified segmented-control pattern** for every toggle on the surface (Timeline/Insights,
  the surface/sub-surface switcher, Day/Week/Month, Subjects sort). Keep that visual consistency.

## 2. Files in this folder

| File | What it is |
| --- | --- |
| `index.html` | Mockup index — links every surface, hosts the theme toggle. Not an app screen. |
| `main-shell.html` | **#103** — Main window shell: Timeline/Insights surface toggle + a compact Overview preview. |
| `overview.html` | **#104 + #105** — Overview sub-surface: AI read-hero (owns headline numbers) + demoted exhibits charts + dossier tail. |
| `subjects-index.html` | **#106** — browsable grid of Subjects (each card = multiple Conclusions). |
| `subject.html` | **#106** — single Subject detail: per-Conclusion confidence trajectories + evidence inspector. |
| `context.html` | **#107** — user-authored Context composer + authored-statement list. |
| `chat.html` | Insights **Chat** sub-surface: persistent threads, graphical inline answers. |
| `_shell.html` | **Shared shell template** — canonical titlebar + subnav chrome to copy into each surface. |
| `tokens.css` | **Shared** — design tokens, **mirroring** the app's real tokens (see §6); do not fork these into the build. |
| `app.css` | **Shared** — all mockup component styles (cards, charts, segmented controls, chat, etc.). |
| `app.js` | **Shared** — cosmetic-only behavior (theme toggle persistence). |

`tokens.css`, `app.css`, `app.js`, and `_shell.html` are the shared shell every surface page consumes.

## 3. Surface → issue slice → domain → tier

| Surface | Issue slice | Core domain terms | Tier |
| --- | --- | --- | --- |
| `main-shell.html` | #103 Main shell + Timeline/Insights toggle | Main, Surface, Timeline, Insights | chrome |
| `overview.html` | #104 Usage Charts; #105 categorized/focus charts + dossier | Usage Charts; Activity Category, Focus Classification, Conclusion, Activity | FREE + ENGINE |
| `subjects-index.html` | #106 browsable Subjects view | Subject, Conclusion, Confidence | ENGINE |
| `subject.html` | #106 Subject detail | Subject, Conclusion, Confidence History, Activity, Pin/Dismiss | ENGINE |
| `context.html` | #107 user-authored Context | Context (authored), Sensitive Category Guardrail | ENGINE (input) |
| `chat.html` | Insights Chat sub-surface | Chat, Quick Recall, `recall_context`, Conclusion/Activity | ENGINE |
| Pin/Dismiss + category/focus correction | #108 Correction UI (lives inside `subject.html` + the Overview actionable tail; not its own page) | Pin, Dismiss, Dismissal State, Activity Category, Focus Classification | ENGINE |
| User Context **settings** | #109 — **NOT MOCKED** (see §8) | master toggle, engine/model picker, BYO key, Derivation Budget, Wipe User Context | — |

## 4. Per-surface build notes

### Main shell — `main-shell.html` (#103)

- **What it is.** The primary Mnema window (**Main**). "dashboard" is retired: Main is just a
  shell hosting two switchable **Surfaces** — **Timeline** (the existing capture-inspection
  view) and **Insights** (the new AI workspace). Insights is itself a workspace of sub-surfaces.
- **Data / tier.** Chrome only. The Insights panel here is a *compact Overview preview*, not the
  full surface — the real surfaces live in the other files.
- **Reuse.** Mount the **existing dashboard Timeline view verbatim** for the Timeline panel
  (do not rebuild it). Theme toggle → `ThemeModeControl.svelte`; any on/off toggles → `Switch.svelte`.
- **Interactions / handoffs.** Titlebar segmented control swaps Timeline ⇄ Insights panels; the
  Insights subnav routes to Overview / Subjects / Context / Chat.
- **Domain / ADRs.** Main, Surface, Timeline, Insights. Engine is Rust-side rig-core
  ([`../adr/0028-ai-features-call-models-rust-side-via-rig-core.md`](../adr/0028-ai-features-call-models-rust-side-via-rig-core.md)).
  This rename spans existing desktop docs/code and must be reconciled with `apps/desktop/CONTEXT.md` when built.

### Overview — `overview.html` (#104 + #105)

- **Data / tier — two tiers in one surface:**
  - **FREE (#104) — Usage Charts.** Grayscale, **counting-only**, always-on, **no engine**.
    Aggregated from already-captured **Search Context** (app/window/URL/time): time per app,
    time per site (domain-level, only where URL metadata exists), the app-interaction graph
    (frontmost-app switch sequence), and an activity-intensity heatmap (busy ≠ focused). **No
    categories, no focus judgment** here.
  - **ENGINE (#105) — the "color".** Categorized charts driven by **Activity Category** (fixed
    v1 taxonomy: Coding, Research, Communication, Design, Testing, Personal, Distractions…) and
    **Focus Classification** (focused-vs-distracted), **plus the dossier** = **Conclusion** values
    + the **Activity** narrative. Gated on **Reasoning Engine** opt-in.
- **Layout — the Briefing (narrative-first, one centered ~60% column):**
  - **The read** (top, full-width hero, ENGINE). The engine's synthesis of the range — headline +
    prose — and the **single source of truth for the range's headline numbers** (Tracked, Daily avg,
    Deep focus %, Top category, a per-day sparkbar). Owning the numbers here kills the old duplication
    between a lede and a "This week" tile.
  - **Exhibits** (demoted below the hero, quieter supporting-evidence strip). The metric charts —
    Time + Categories on the first row, Focus full-width on the second — with an "open category
    detail →" affordance. They justify the narrative rather than being a co-equal dashboard. **Tier
    semantics unchanged:** Time = FREE counting; Categories + Focus = ENGINE color.
  - **What changed / Needs attention** (actionable tail, ENGINE). Conclusion deltas with Pin/Dismiss
    + evidence, and uncategorized Activities with inline category correction.
  - **Ask** (docked, sticky to the bottom). The "Ask about your history → Chat" bar.
- **FREE / no-engine state:** the hero becomes a **deterministic factual read** (no AI) plus the free
  headline numbers (Tracked, Daily avg, sparkbar only) and a single enable-the-engine CTA; Exhibits
  show live Time with Categories/Focus locked. Overview is never empty — the old standalone no-engine
  card is gone.
- **Components / reuse.** Charts are **hand-built inline SVG/CSS — no chart library** (on-brand;
  keep it lightweight SVG in the real build). Use `SearchResultCard.svelte` /
  `AnswerSourceCard.svelte` wherever capture references appear.
- **Interactions / handoffs.** Pin/Dismiss on Conclusions = Correction UI (#108); category/focus
  labels are correctable the same way. Subject chips → Subjects surface. "View evidence" on a
  Conclusion → its **Activity** → hands off to **Timeline** at the Activity's span (see Subject notes).
  Faded Conclusions: a **display floor** removes them from the visible dossier but keeps history —
  faded is not deleted.
- **ADRs.** [`0028`](../adr/0028-ai-features-call-models-rust-side-via-rig-core.md),
  [`0030`](../adr/0030-user-context-sensitive-category-guardrail.md) (sensitive categories never surfaced).

### Subjects index — `subjects-index.html` (#106)

- **Data / tier.** ENGINE. A **Subject** is a **browsable entity**, not a tag. Each card surfaces
  **multiple individual Conclusions** about the subject — **never a single rolled-up sentiment score**.
- **Components / reuse.** Card styling akin to `SearchResultCard.svelte`. Sort segmented control
  (Most active / Recently moved / A–Z) follows the unified segmented-control pattern.
- **Interactions / handoffs.** Each card drills into `subject.html`. Faded subjects (below the
  display floor) are kept for history. Pin protects from decay; Dismiss is a high-bar resurface.
- **ADRs.** [`0028`](../adr/0028-ai-features-call-models-rust-side-via-rig-core.md),
  [`0030`](../adr/0030-user-context-sensitive-category-guardrail.md).

### Subject detail — `subject.html` (#106)

- **Data / tier.** ENGINE. Shows the subject's **individual Conclusions**, each with its **own
  Confidence-over-time trajectory** (**Confidence History**, a stored time-series) — the literal
  "warmed up to a thing, then cooled" picture. **Not** a single rolled-up score.
- **Layout.** An overlay trajectory chart (one line per Conclusion) above a master-detail
  Conclusions list, with a sticky **Evidence inspector** listing the **Activity** values each
  Conclusion is grounded in. Charts are hand-built SVG.
- **Components / reuse.** `SearchResultCard.svelte` for evidence rows.
- **Interactions / handoffs.** Pin/Dismiss per Conclusion = Correction UI (#108). Evidence "view in
  Timeline" = **Activity-span handoff**: lands Timeline at the Activity's start + highlighted range
  (a small extension of the existing **Search Result Anchor** navigation). The **Timeline stays
  raw** — v1 does **not** paint a semantic Activity ribbon onto it. Below the **display floor** a
  Conclusion leaves the dossier but its history persists, so the arc still renders here.
- **ADRs.** [`0028`](../adr/0028-ai-features-call-models-rust-side-via-rig-core.md),
  [`0029`](../adr/0029-user-context-outlives-raw-retention-privacy-delete-cascades.md) (derived data outlives raw retention).

### Context — `context.html` (#107)

- **Data / tier.** **User-authored** Context — what the user tells Mnema directly about themselves
  ("I'm a designer," "I care about X"). It **complements** the inferred **Conclusion** layer,
  steering the dossier up front rather than only correcting after the fact, and is available to the
  engine like the rest of User Context.
- **Key rules.** Authored context is **NOT subject to Confidence/decay** (the user asserted it — it
  never fades). It **is** still subject to the **Sensitive Category Guardrail** for what the engine
  will *surface* (health/beliefs/etc. are never inferred or surfaced even if mentioned).
- **Layout.** Composer + authored-statement list (main column) beside a steering/explainer side rail
  that links authored statements to the inferred Conclusions they support.
- **Interactions / handoffs.** Add / edit / delete authored statements; steering chips → Subjects.
- **ADRs.** [`0030`](../adr/0030-user-context-sensitive-category-guardrail.md),
  [`0028`](../adr/0028-ai-features-call-models-rust-side-via-rig-core.md).

### Chat — `chat.html` (Insights Chat sub-surface)

- **Data / tier.** ENGINE. A persistent, searchable chat workspace (new chat, history list, search
  over chats) answering questions over the user's history.
- **Shared store.** Shares **one engine and one conversation store with Quick Recall**
  (`apps/desktop/src/routes/quick-recall/+page.svelte`) — two doors into the **same** persistent
  threads; a thread started in one resumes in the other. This **reverses ADR 0027's disk-ephemerality**:
  conversations now **persist** in the **Encrypted Capture Index**, under **Retention Policy** and
  cleared by **Wipe User Context**.
- **Graphical answers.** Answers render **inline graphically** (reuse the same chart/dossier
  components as Overview/Subject), not just prose.
- **On-request personalization only.** The broker tool **`recall_context`** (working name; alongside
  `search`/`timeline`/`show_text`) returns **only the question-relevant Conclusion/Activity pieces**
  (redacted, **All-Retained Broker Scope**) — **never the whole dossier**. The guardrail already keeps
  sensitive Conclusions out of the dossier, so `recall_context` physically cannot return them.
- **Components / reuse.** `AnswerSourceCard.svelte` for the Answer Sources strip; chart components
  from Overview/Subject. The Quick Recall chat UI is the closest existing reference.
- **ADRs.** [`0031`](../adr/0031-quick-recall-and-chat-share-one-persistent-conversation-store.md),
  [`0028`](../adr/0028-ai-features-call-models-rust-side-via-rig-core.md).

### Correction UI (#108) — not a separate page

Pin/Dismiss per **Conclusion** and **Activity Category** / **Focus Classification** correction live
**inside** `subject.html` and the Overview actionable tail. **Dismiss** removes a Conclusion and feeds
**Dismissal State** into the next derivation batch (a high-bar resurface, not a permanent veto).
**Pin** protects a Conclusion from Confidence decay. There are no user-facing "fade rate" sliders —
Pin/Dismiss and the Derivation Budget tier are the only user controls over confidence behavior.

## 5. Design system & theming

- **Tokens are the source of truth in the app, not here.** The real theme tokens are defined on
  `:root` / `[data-theme="light"]` in `apps/desktop/src/routes/+layout.svelte`. This folder's
  `tokens.css` **mirrors** them so the mockups render standalone — the real build **consumes the app
  tokens directly; do NOT fork tokens.css into the app.**
- **Theme.** Set via `data-theme` on `<html>`; a single flip fully re-skins (including the hand-built
  chart palette, which is token-driven). Both light and dark must look right — verify every surface in both.
- **Typography.** Monospace font stack throughout (terminal aesthetic).
- **Unified segmented control.** Every toggle uses the same segmented-control pattern: the
  Timeline/Insights surface toggle, the Insights subnav, Day/Week/Month, Subjects sort. Keep them
  visually identical; do not introduce a second toggle style.
- **Reusable components to build against:** `SearchResultCard.svelte` (evidence/result cards),
  `AnswerSourceCard.svelte` (Chat Answer Sources), `Switch.svelte` (on/off), `ThemeModeControl.svelte`
  (theme), all under `apps/desktop/src/lib/components/`. The Chat surface should share the Quick Recall
  chat UI at `apps/desktop/src/routes/quick-recall/+page.svelte`.
- **Charts.** Hand-built inline SVG, no chart library — keep that in the real build to stay on-brand
  and token-themeable.

## 6. Backend / data notes

- **Engine = Rust-side via `rig-core`** ([ADR 0028](../adr/0028-ai-features-call-models-rust-side-via-rig-core.md)).
  **No Node, no PI/flue shim** for User Context — the agent loop, redaction, and broker stay in one
  Rust process. **Cloud** = bring-your-own-key stored in the **OS keychain** (same Keychain boundary as
  the Capture Index Key Store; never in `saveDirectory` or a config file), talking straight to the
  provider (Mnema runs no proxy). **Local** = an Ollama/Llamafile endpoint, no key. The local/cloud
  choice is a per-user Reasoning Engine *choice* applied to **both** Activity and Conclusion derivation,
  not a layer boundary. Cloud egress for continuous background derivation is its **own** opt-in,
  separate from the Ask AI Setting.
- **Storage = the Encrypted Capture Index** (page-level SQLCipher, key in the OS keychain). All derived
  data — **Activity**, **Conclusion**, **Dismissal State**, **Confidence History**, and the now-persisted
  **conversations** — lives there, not in a plaintext sidecar or JSON under `saveDirectory`.
- **Deletion cascade rules** ([ADR 0029](../adr/0029-user-context-outlives-raw-retention-privacy-delete-cascades.md)):
  **Retention Policy** (time-based housekeeping) does **NOT** cascade — derived Activity summaries
  survive when raw media ages out (the durable evidence floor). **Delete Recent Capture** (the panic
  button) **DOES** cascade hard — it purges Activities from the deleted window and re-judges/drops
  Conclusions that leaned on them. A Conclusion that loses all evidence is dropped.
- **Sensitive Category Guardrail** ([ADR 0030](../adr/0030-user-context-sensitive-category-guardrail.md)):
  soft instruction to the engine plus a hard deterministic post-filter that drops Conclusions whose
  Subject lands in an off-limits category (health, sexual orientation, religion, politics…). Errs toward
  over-suppression. **Non-user-facing** — no toggle, enforced at derivation (so `recall_context` cannot
  leak it).
- **Broker `recall_context`.** New brokered tool (working name) at All-Retained scope, returning only
  question-relevant Conclusion/Activity pieces; appears in access audit history like other broker tools.
  Whole-dossier seeding into a conversation is rejected.
- **New backend work needed (design stage — none of this exists yet):** new **Tauri commands** and new
  **sqlx migrations** for derivation, storage, querying, Pin/Dismiss, Wipe User Context, and the broker
  tool. **No owning crate yet**; derivation/storage will most likely land in `crates/app-infra` (it
  already owns SQLite, background jobs, and the processing pipeline) and surfacing in `apps/desktop`.
  **Do not invent command or migration names** — treat them as "to be created."
- **Derivation cadence (for context, not UI):** Activity derivation is frequent/batched over recent
  history (OCR Catch-Up pattern, off the hot path); Conclusion re-distillation is slower over
  accumulated Activities. Both paced by the **Derivation Budget**. **History Backfill** on first enable
  is a budget-paced background trickle, newest-first, bounded window with a go-deeper action — never a
  synchronous one-time token bill. A "building your understanding…" progress state sets expectations.

## 7. Known gaps / not yet mocked

- **#109 — User Context settings surface is NOT mocked.** Build it following the existing card patterns
  in `apps/desktop/src/routes/settings/+page.svelte`. It is its **own dedicated surface**, *not* folded
  into Access Settings, and owns: the **master toggle**; the **local/cloud engine + model picker** (with
  the always-on cloud-egress consent + plain disclosure sitting next to the engine picker where the
  choice is made); the **bring-your-own-key** field; the **Derivation Budget** tier + **tokens-used
  readout**; and **Wipe User Context** (a confirmed Tauri-dialog destructive action that clears all
  derived data — Activity, Conclusion, Dismissal State, conversations — without touching raw captures or
  settings, and also turns the engine off). Disabling the engine alone is **not** a wipe (it stops new
  derivation but leaves the dossier readable). Structure the rig-core config as shared "AI runtime
  settings" that User Context is the first consumer of (Ask AI migrates onto it later).
- **Onboarding card** (a light, optional, off-by-default local/cloud choice that defers to the settings
  surface) is referenced in `CONTEXT.md` but not mocked here.
- **Deferred sub-surfaces** (Plugins / Automations / Project) are explicitly **not v1** — the Insights
  shell can grow into them, but do not build them now.

## 8. "Mock vs real" caveats

- These are **static HTML/CSS** with **no framework, no backend, no real data** — every number, chart,
  Conclusion, Subject, and chat message is **placeholder content** chosen to illustrate layout and tier.
- **Interactions are cosmetic only.** The theme toggle persists; everything else (segmented controls,
  Pin/Dismiss buttons, chat input, composer) is non-functional styling. Wire real behavior in Svelte.
- **Charts are hand-built SVG/CSS**, not data-bound — the real build keeps the SVG approach but renders
  from live aggregates.
- **`tokens.css` mirrors the app's tokens; it is not the source of truth** — build against
  `apps/desktop/src/routes/+layout.svelte` tokens (see §5).
- **Backend names are illustrative.** No Tauri commands, crates, or migrations for this feature exist
  yet; the mockups imply data shapes but do **not** define an API. Treat all of §6 as design intent.
