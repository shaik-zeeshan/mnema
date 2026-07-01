# User Context

The standing, continuously-updated understanding of the user that Mnema builds from
its raw capture record — "what you did and how you did it," and what that adds up to.

Code has landed (issue #88). Ownership is split across two crates: `crates/app-infra/src/user_context/`
owns the **Encrypted Capture Index** storage (`store.rs` over the `user_context_*` tables) and the
deterministic policy — the **Confidence Policy** (`confidence.rs`) and the **Sensitive Category
Guardrail** (`guardrail.rs`) — plus the capture-window reader (`capture_source.rs`), with **no
`ai-runtime`/`rig-core` dependency**. `apps/desktop/src-tauri/src/user_context/` owns the
**Reasoning Engine** orchestration (`derivation.rs`), the derivation `worker.rs`, and the Tauri
`commands.rs`. The wire DTOs and the `UserContextSettings` settings domain live in
`crates/capture-types`. Because ownership is split, this file stays here as the cross-cutting
User Context context rather than moving beside one crate; keep the
[CONTEXT-MAP.md](../../CONTEXT-MAP.md) row pointing at it.

Root entry point: [CONTEXT-MAP.md](../../CONTEXT-MAP.md).

## Language

**User Context**:
The standing, continuously-updated understanding of the user, derived from the raw capture
record and kept/refined over time. It is the umbrella for the **Activity** (evidence) and
**Conclusion** (distilled belief) layers. Distinct from **Search Context**, which is per-result
captured labels (app/window/URL/speaker) — **User Context** is synthesized understanding, not a
label on a search hit.
_Avoid_: search context, profile settings, person profile, static user profile

**Activity**:
A derived episode of what the user did and how, summarized from the underlying captures
(**Captured Frame** + OCR text, **Audio Transcription Span**, **Search Context**). An **Activity**
is a **semantic task** — a coherent unit of work/intent — and its boundaries are *intent shifts*
("stopped wrestling the deploy, started the design doc"), so one **Activity** can span multiple
apps. The app/window/URL **Search Context** is *input the engine reads to find boundaries*, not the
boundary itself; an **Activity** is NOT a mechanical per-app or fixed-time-window slice, and NOT the
**Capture Session** recording grouping it was derived from. The evidence layer: Activities are what
**Conclusion** values point back to.
_Avoid_: capture session, recording, timeline row, raw frame, per-app slice, app-usage timer

**Conclusion**:
An open-ended, plain-language statement about the user (for example "Has been increasingly
interested in Apple", "Prefers async communication", "Is in a Rust learning phase"). Each
**Conclusion** carries a **Subject** it is about, a **Confidence** that rises and falls over time,
and links to the **Activity** values that are its evidence. Open-ended rather than a fixed
`subject+attribute+value` schema, because the set of things worth noticing about a person is
unbounded.
_Avoid_: tag, sentiment score, structured fact, static profile attribute, personality trait

**Reasoning Engine**:
The user-selected model that derives both **Activity** and **Conclusion** values. It may be a
local model or a cloud model — the local/cloud split is a user *choice*, not a fixed layer
boundary, because not every machine can run a capable local model and local models are not on par
with cloud models. Both layers honor the same selection. **AI is called from the Rust side** via
`rig-core` (a native Rust LLM framework: multi-provider, tool calling, structured/typed extraction),
selecting a provider/model that is cloud (HTTPS to the provider with a **bring-your-own-key**) or
local (an Ollama/Llamafile endpoint). No Node, no shim, no bundled JS runtime — the agent loop,
redaction, broker, and capture data stay in one Rust process. This abandons the PI/flue/Node-shim
direction (the shim made the user supply a runtime, which is not shippable). See
[ADR 0028](../adr/0028-ai-features-call-models-rust-side-via-rig-core.md).
_Avoid_: flue, Node shim, bundled JS runtime, installed PI delegation, fixed local/cloud seam, Mnema-operated backend proxy

**Subject**:
The thing a **Conclusion** is about (e.g. "Apple"). More than a tag: a **browsable entity** with its
own view on the **Insights** surface that shows every **Conclusion** about it, each with its own
**confidence-over-time line** — the literal picture of warming up to a thing and then cooling off
(the founding "likes Apple, then cools" example). The Subject page shows the *individual* Conclusions
and their trajectories, NOT a single rolled-up "sentiment score" (that would resurrect the structured
sentiment model rejected in the **Conclusion** definition). A **Subject** is also the **identity key
for reinforcement**: recurring evidence about the same subject reinforces its existing **Conclusion**
rather than minting a reworded duplicate (see the subject-centric reinforcement Relationships below).
_Avoid_: knowledge-graph node, single net sentiment score, rolled-up stance scalar, search-filter-only

**Confidence History**:
A stored time-series of a **Conclusion**'s **Confidence** — periodic snapshots, not just the current
value — that powers the **Subject** trajectory line. Tiny (a few floats per snapshot interval) and
aggressively prunable, since recency-weighting means old snapshots stop mattering. It exists because
the trajectory is the headline view; without it Mnema would know a stance moved but could never show it.
Both directions are recorded: the slow decay beat snapshots the **down** steps, and reinforcement
snapshots the **up** step when **Confidence** ratchets higher (plus one seed point on formation), so a
positive slope — what the **Subject** view's "warming" tier detects — is now reachable. (Originally
only the decay beat wrote points, so trajectories were monotonically non-increasing and "warming"
could never appear.)
_Avoid_: current-value-only, decay-only snapshots, full audit log, per-frame confidence trace

**Confidence**:
A strength value on a **Conclusion** that is **recency-weighted evidence**: recent supporting
**Activity** values push it up; as those Activities age without fresh support it sinks on its own
(silence fades a **Conclusion**, even with nothing contradicting it); contradicting **Activity**
values push it down faster. One rule yields both the quiet fade and the active reversal, and it
falls out of the grounding — the evidence links' recency *is* the confidence.
_Avoid_: static score, relevance rank, probability, contradiction-only revision

**History Backfill**:
What the **Reasoning Engine** does to existing retained captures when first enabled. It is **paced
by the Derivation Budget** — background trickle over hours/days, never a synchronous one-time token
bill — and runs **newest-first** (which falls out of recency-weighted **Confidence**: recent history
drives current conclusions; ancient history barely moves present confidence). It defaults to a
**bounded window** (recent weeks/months) with an explicit **go-deeper** action, capping the cost
surprise even at the Thorough tier. A "building your understanding…" progress state sets expectations.
**Usage Charts** (free counting) cover all of history immediately regardless, so the surface is rich
on day one while the engine tier fills in.
_Avoid_: synchronous backfill, instant whole-history bill, oldest-first, blocking enable on backfill

**Derivation Budget**:
The policy that paces background **Reasoning Engine** work over time — the **OCR Throughput Budget**
analog for **User Context**. For a local engine it is fixed product policy (resource pacing only,
like OCR, no user knob). For a cloud engine it is partly user-facing because tokens cost real money:
the user picks a **named intensity tier** (e.g. Light / Balanced / Thorough), with a **tokens-used
readout** so actual spend is visible. Prefer named tiers over a hard token ceiling that would
silently pause understanding mid-period.
_Avoid_: OCR throughput budget (that is CPU-only and user-invisible), hard token cap, per-question budget

**Sensitive Category Guardrail**:
A hard policy that prevents **Conclusion** values in off-limits inference categories (health and
mental health, sexual orientation, religion, politics, and similar protected/intimate domains) from
ever being stored or shown. Enforced two ways: a **soft** instruction to the **Reasoning Engine**
not to form such conclusions, plus a **hard** deterministic post-filter that drops any **Conclusion**
whose **Subject** lands in a sensitive category as a backstop for when the model ignores the
instruction. Suppressed by default and **not user-enableable in v1** (no "infer my mental health"
toggle). Deliberately errs toward **over-suppression** — false-suppress a benign conclusion that
brushes a sensitive category rather than false-surface a real sensitive inference. See
[ADR 0030](../adr/0030-user-context-sensitive-category-guardrail.md).
_Avoid_: opt-in sensitive inferences, flagged-but-shown sensitive conclusions, soft-only filter, content sensitivity scoring

**Confidence Policy**:
The fixed product policy (not user-facing sliders) governing how **Confidence** forms, fades, hides,
and resurfaces, biased toward **stability** so the dossier reads as a considered judgment, not a mood
ring. Four knobs: a **formation bar** (repeated evidence before a **Conclusion** appears, no flimsy
one-afternoon conclusions), a slow **fade speed** (a quiet stretch does not erase a trait — that is
what **Pin** protects), a **display floor** (below it a **Conclusion** leaves the visible dossier but
its **Confidence History** persists, so the **Subject** page can still show the faded arc — faded is
not deleted), and a high **resurface bar** (overturning a **Dismiss** takes substantially more fresh
evidence than forming the conclusion took, so correction never feels ignored). Exact values are
tuning; the stability bias and the fixed-as-policy stance are the decision. The user's controls are
**Pin**/**Dismiss** per conclusion and the **Derivation Budget** tier — never a "fade rate" knob.
_Avoid_: user-facing decay sliders, responsive/twitchy tuning, faded-means-deleted, equal resurface bar

**Dismiss**:
A user correction on a **Conclusion** meaning "you're wrong." The **Conclusion** is removed AND the
next derivation batch is told not to simply reconstitute it. A **Dismiss** is a **reset with a high
bar to resurface**, not a permanent veto: the **Conclusion** may return only if substantial *fresh*
**Activity** evidence rebuilds it later — never from the same evidence just rejected. The bar must be
high enough that a dismissal never feels ignored.
_Avoid_: permanent veto, hard block, hide-only flag, silent delete

**Pin**:
A user correction on a **Conclusion** meaning "this is true, keep it." A pinned **Conclusion** is
protected from **Confidence** decay so it does not quietly fade during a quiet stretch.
_Avoid_: favorite, bookmark, manual conclusion

**Dismissal State**:
Engine-carried state recording that the user rejected a particular **Conclusion**, with which
evidence and when, fed as input to every derivation pass so the engine can tell *fresh* evidence
from the evidence already vetoed and honor the high-bar-resurface rule. Real state, not a hidden flag.
_Avoid_: tombstone flag, blocklist row, deleted boolean

**Main**:
The primary Mnema window/shell — what older docs loosely called "the dashboard." **Main** hosts
switchable **Surface** values and is not itself a content view. "dashboard" is retired in favor of
**Main** plus the named surfaces.
_Avoid_: dashboard, main dashboard, home screen

**Surface**:
A top-level content view inside **Main**, switched by a toggle. The two surfaces are **Timeline**
and **Insights**.
_Avoid_: tab, page, dashboard view

**Timeline** (surface):
The existing capture-timeline view (the older "dashboard timeline"), one of the two **Main**
surfaces. Owns capture inspection: **Scrub Preview** navigation, exact **Captured Frame** preview,
OCR copy/download, audio playback.
_Avoid_: dashboard, dashboard timeline, main view

**Insights** (surface):
The other **Main** surface: the AI **workspace** for understanding yourself. A container hosting
**sub-surfaces**, not a single view. v1 sub-surfaces are **Overview**, **Chat**, and **Context**;
**Plugins**, **Automations**, and **Project** are deferred (the shell can grow into them, but they
are not v1). Capture *inspection* still belongs to **Timeline**; **Insights** is read/understand/ask.
_Avoid_: analytics dashboard, stats page, single informative view, profile page

**Overview** (sub-surface):
The charts-and-dossier sub-surface of **Insights** (renamed from the earlier "informative view" to
avoid an Insights-containing-Insights name trap). Two tiers. The **free tier** is **Usage Charts**
(grayscale counting, always-on, no engine). The **engine tier** — gated behind the **Reasoning
Engine** opt-in — is the *color*: the categorized/focus charts (driven by **Activity Category** and
**Focus Classification**) plus the **User Context** dossier (**Conclusion** values + the **Activity**
story). The engine "lights up" **Overview**; without it, it still works, just grayscale.
The layout is **narrative-first** (a "Briefing"): one full-width column led by **"The read"** —
the engine's synthesis of the range, and the single home for the headline numbers (tracked /
daily avg / deep-focus % / top category / sparkbar) — with the metric charts **demoted** to a
quieter **"Exhibits"** strip (Time / Categories / Focus) that supports the narrative, then the
actionable tail (What changed / Needs attention) and a docked Ask bar. FREE swaps the AI hero for
a deterministic factual read plus an enable-engine invite. This replaced the originally-approved
"bento glance band + narrative story feed" (a 4-tile at-a-glance grid above a separate centered
feed): the two-paradigm split read as disorganized and the lede's stat row duplicated the
"This week" tile, so leading with the engine's read — interpretation first, charts as supporting
evidence — matches what **Overview** is *for* and gives the headline numbers one source of truth.
Only the information architecture changed; tier semantics are unchanged (FREE = counting from
**Search Context**, ENGINE = categorized/focus + dossier).
_Avoid_: insights tab, stats page, profile page, bento glance band, parallel widget grid + story feed

**Chat** (sub-surface):
The conversational sub-surface of **Insights**: a persistent, searchable chat workspace (new chat,
chat history list, search over chats) that answers questions over the user's history and renders
**graphical answers** (the same chart/dossier components inline, not just prose). It shares one
engine *and one conversation store* with **Quick Recall**: **Quick Recall** is the fast summon-anywhere
door, **Chat** is the sit-down deep door, and the *same* conversation opens in either. Engine-tier.
_Avoid_: ephemeral launcher chat, text-only answers, a second conversation store, a separate engine

**Context** (sub-surface):
The "providing context to AI" sub-surface of **Insights**: **user-authored context** the user tells
Mnema directly about themselves ("I'm a designer," "I care about X"). It *complements* the *inferred*
**Conclusion** layer — steering the dossier up front rather than only correcting it after the fact —
and is available to the engine like the rest of **User Context**.
_Avoid_: settings page, inferred conclusions, custom model instructions only, a prompt textbox with no role

**Usage Charts**:
The **free tier** of **Overview**: purely-quantitative, grayscale, **counting-only** visuals computed
by aggregating already-captured **Search Context** (app/window/URL/time) — **no Reasoning Engine, no
LLM, no opt-in**, always-on for every user. The v1 set: **time per app**, **time per site**
(domain-level, only where URL metadata was captured), the **app-interaction graph** (which app you
switched to from which, from the frontmost-app sequence), and an **activity-over-time heatmap by
intensity** (when you were busy — *not* colored by focus). Contains NO categories and NO
focus/distraction judgment; those are the engine tier.
_Avoid_: categories, focus/distraction coloring, productivity score, dossier, LLM summary

**Activity Category**:
A label the **Reasoning Engine** assigns to each **Activity** from a **fixed v1 taxonomy** of
profession-neutral *work modes* (what kind of thing the user was doing), not developer-specific
domains: **Creating** (producing anything — code, documents, designs, slides, music),
**Communication** (asynchronous text — email, chat, messages), **Meetings** (real-time
conversation — calls, video meetings), **Research** (reading/searching in service of a current
task), **Learning** (deliberate skill-building for its own sake — courses, tutorials),
**Organizing** (structuring work/time — calendar, task managers, planning, admin paperwork),
**Personal** (life errands regardless of subject — shopping, banking, health, travel), and
**Entertainment** (videos, games, social feeds, browsing for fun). Boundary rules: synchronous
conversation → Meetings, async text → Communication; in service of the task at hand → Research,
skill-building for its own sake → Learning; life errands → Personal even when work-adjacent
(buying a work laptop), structuring work → Organizing. Categories describe content type only; the
focused/derailed judgment belongs exclusively to **Focus Classification** — there is deliberately
no "Distractions" category ("2h Entertainment" is a fact, "2h Distractions" is a scolding).
Engine-tier (judgment, not a static app→category catalog, because the same app serves different
purposes in context). **Correctable** like a **Conclusion**. Powers the categorized charts —
per-category treemap, category→app Sankey, categorized time-distribution bars.
_Avoid_: static bundle-id catalog, user-defined custom taxonomy (v1), fixed app mapping,
distraction-as-category (focus-axis leakage), profession-specific categories (Coding, Testing),
"Browsing" catch-all (aimless browsing is Entertainment or Research by intent)

**Focus Classification**:
A focused-vs-distracted judgment the **Reasoning Engine** assigns over **Activity** / time, powering
the focus-and-distraction heat map. Engine-tier; it *will* be wrong sometimes, so it is
**correctable** and framed as a gentle observation, not scolding — the spiciest label on the surface
and the one that brushes the trust line. Subject to the conservative posture, not preachy.
_Avoid_: productivity score, discipline grade, hard distraction blocklist, judgmental nudges

## Relationships

- **User Context** is built FROM the raw capture record but is not part of it; it is derived,
  synthesized understanding layered on top.
- **Main** hosts two switchable surfaces, **Timeline** and **Insights**; "dashboard" is retired and
  its old usages remap to **Main** (the window) or **Timeline** (the capture view). This rename
  spans existing desktop docs/code and must be reconciled with `apps/desktop/CONTEXT.md` when built.
- **Insights** is a **workspace** hosting sub-surfaces; v1 is **Overview** + **Chat** + **Context**,
  with **Plugins** / **Automations** / **Project** deferred (shell-only, not built in v1).
- The **Overview** sub-surface is two-tier: the **free tier** (**Usage Charts** — grayscale counting)
  renders for everyone with no **Reasoning Engine**, while the **engine tier** (the categorized/focus
  charts driven by **Activity Category** + **Focus Classification**, plus the **User Context** dossier)
  appears only once a **Reasoning Engine** is selected. Overview is never empty for a no-engine user —
  grayscale charts with the engine tier inviting opt-in. **Chat** and **Context** are engine-tier.
- **Quick Recall** and **Chat** share **one engine and one conversation store**: they are two doors
  into the same conversations, not two features. **Quick Recall** is the fast summon-anywhere door;
  **Chat** is the sit-down deep door; a thread started in one resumes in the other. This **reverses
  the disk-ephemerality of ADR 0027** — conversations now persist, stored in the **Encrypted Capture
  Index**, under **Retention Policy** and cleared by **Wipe User Context**, because a saved
  conversation about the user's life is as sensitive as the dossier it discusses. See
  [ADR 0031](../adr/0031-quick-recall-and-chat-share-one-persistent-conversation-store.md).
- **Context** (user-authored) complements the inferred **Conclusion** layer: the user steers the
  dossier directly, and authored context is available to the engine alongside derived **User Context**.
  Authored context is not subject to the **Confidence**/decay machinery (the user asserted it); it is,
  however, still subject to the **Sensitive Category Guardrail** for what the engine will *surface*.
- Categorization and focus are **judgment, so they are engine-tier**, never a cheap static catalog: a
  coarse always-on app→category catalog would show confidently-wrong labels with no engine to correct
  them, and a wrong "Distraction" label is exactly the preachy, untrustworthy moment to avoid.
- **Activity Category** and **Focus Classification** are correctable the same way a **Conclusion** is
  (**Dismiss**/edit feeds back), and the category taxonomy is fixed in v1 rather than user-defined.
- The **app-interaction graph** and **time per app/site** are pure counting and stay in the free tier;
  only the *category* and *focus* axes require the engine.
- Capture *inspection* (exact frame, OCR copy/download, audio playback) stays on the **Timeline**
  surface; **Insights** is a read/understand surface, and an **Activity** or **Answer Source** hands
  off to **Timeline** for inspection rather than inspecting in place.
- An **Activity** is a handoff anchor, not a Timeline overlay: selecting one in **Insights** lands the
  **Timeline** at the Activity's *span* (start + highlighted range, a small extension to the existing
  **Search Result Anchor** navigation), and the **Timeline stays raw**. v1 does NOT paint a semantic
  Activity ribbon onto the Timeline — derived, engine-gated, lagging labels must not sit on the
  ground-truth surface; that overlay is a later enhancement once the labels have earned trust.
- **Ask AI** consumes the dossier **on demand, relevant pieces only** — never the whole dossier.
  A new brokered tool (working name `recall_context`), alongside `search`/`timeline`/`show_text`,
  returns the few **Conclusion**/**Activity** values relevant to a question (redacted, at
  **All Retained Broker Scope**). The cloud model sees scattered, question-relevant pieces, never
  the assembled whole — the same guarantee shape that protects raw captures. This makes **User
  Context** a way to *analyze the user's whole history* (point lookups and broad "how did X change
  over the year" questions alike), not just to personalize single answers.
- Whole-dossier seeding into an **Ask AI** conversation is rejected: it would send the assembled
  profile to the cloud, breaking the dossier-stays-local guarantee. Personalization is therefore
  "on request" (the model must call the tool), which is the safer trade.
- The **Sensitive Category Guardrail** already keeps sensitive **Conclusion** values out of the
  dossier, so `recall_context` physically cannot return them; guardrailing happens at derivation,
  not at the broker boundary.
- `recall_context` access is **Brokered Capture Access** and appears in access audit history like
  the other broker tools.
- **User Context** has its **own dedicated settings surface**, not folded into **Access Settings**
  (Access is about granting tools access; this is about running an always-on derivation engine). It
  owns the master toggle, the local/cloud engine + model picker, the **bring-your-own-key** field,
  the **Derivation Budget** tier + tokens-used readout, and **Wipe User Context**. The always-on
  cloud-egress consent lives next to the engine picker where the choice is made, with its plain
  disclosure. The **Sensitive Category Guardrail** is not user-facing (no toggle).
- The rig-core engine config (provider/model + BYO key) is **shared**: **Ask AI** (Quick Recall +
  Chat) reuses the same keychain key and provider/key configuration rather than each feature owning
  its own ([ADR 0033](../adr/0033-ask-ai-migrates-onto-shared-reasoning-engine.md)). Model *selection*
  stays split per workload — background derivation's model in `AiRuntimeSettings`, interactive
  Ask AI's in `access.askAiModel` — and `AiRuntimeSettings` holds a small *set* of configured engines
  so a Chat thread can pin `{provider, model}` among them. Availability is two-layer: a shared
  **engine-configured** prerequisite, then two independent feature opt-ins (the **Ask AI Setting** and
  the continuous-derivation opt-in below).
- Onboarding gets a **light optional card** (off by default, choose local or cloud, defers to the
  settings surface), not a heavy onboarding step — onboarding must fit a fixed-height stage. Default
  off, like the **Ask AI Setting**.
- A **Conclusion** is grounded: it links to the **Activity** values that are its evidence, so it
  is always explainable and correctable rather than a free-floating assertion.
- An **Activity** is derived from **Captured Frame**, **Audio Transcription Span**, and
  **Search Context** values, but is a semantic episode in its own right, not those raw records.
- **Activity** derivation runs in periodic **batches over recent history** (the engine needs to see
  a stretch of activity to locate intent-shift boundaries), not frame-by-frame in real time; a lag
  between doing something and its appearing as an **Activity** is accepted.
- A **Conclusion** is open-ended natural language, not a structured `subject+attribute+value`
  row; its **Subject** is a grouping handle, not a rigid schema slot.
- **Conclusion identity is subject-centric, so recurring evidence reinforces rather than duplicates.**
  The upsert matches on **Subject alone** (case-insensitive, non-dismissed) and reinforces that
  subject's **canonical row — highest **Confidence**, ties broken by lowest id** — rather than matching
  the `(subject, statement)` pair; a new row is inserted only for a genuinely new subject. On reinforce
  it bumps **Confidence**, replaces evidence, and snapshots the up-step into **Confidence History**, but
  **freezes the statement text** (the displayed phrasing stays as first formed — an accepted cosmetic
  cost) to keep one clean per-row trajectory and dodge the `UNIQUE(subject, statement)` index, which
  stays as a safety net. This is **forward-only**: the legacy near-duplicate rows (the pre-fix sprawl,
  e.g. one subject with 133 reworded rows) are left to **self-fade** under decay rather than collapsed
  by a migration. See [ADR 0042](../adr/0042-subject-centric-conclusion-identity-and-pre-retrieval-candidate-selection.md).
- **The distillation Reasoning Engine is the matcher; it is shown the beliefs it already holds.**
  The distillation prompt now carries a **"KNOWN SUBJECTS — reuse these handles"** block (alongside the
  user-authored and dismissed blocks) and a preamble instruction to reuse a handle verbatim when a
  belief is about an existing subject and coin a new one only for a genuinely new subject. Lexical
  *matching* (as the identity decider) was rejected at the source (measured rephrasing overlap ~31%, too
  low to decide identity) — but lexical overlap is a fine *recall* signal, so the candidate handles are
  the **union of three recall legs**, deduped case-insensitively with the **recency floor first** so the
  freshest (most duplication-prone) handles always survive the `KNOWN_SUBJECTS_CHAR_CAP=4000`-char cap:
  - **Recency floor:** the newest `KNOWN_SUBJECTS_RECENCY_FLOOR=30` distinct non-dismissed handles
    (`list_subject_handles_by_recency`, newest-supported-first). Always present, model or not.
  - **Lexical leg (model-free):** `list_subject_handles_by_lexical_overlap` ranks ALL non-dismissed
    subjects by whole-word IDF overlap (name-boosted) of name/statements against the recent Activity
    text, keeping the best `KNOWN_SUBJECTS_LEXICAL_LIMIT=20`. Reuses the `recall_*` tokenizer/stemmer
    (the `recall_context` broker tool's) lifted into a shared `crate::lexical` module — the Rust twin of
    the frontend `subjectSearch.ts`. **No embedding, no backfill lag**, so it catches the common case (a
    reworded duplicate shares words) even in the no-model config.
  - **Semantic leg — Mode 1 (embedding model installed):** per-activity embed the distillation window
    (`EmbedKind::Query`), KNN top-`K_PER_ACTIVITY=5` against the subject vectors, union+dedup
    case-insensitively (keep max similarity), drop below cosine floor `SUBJECT_CANDIDATE_COSINE_FLOOR=0.3`,
    cap at `SUBJECT_CANDIDATE_CAP=40` handles. Catches *non-lexical* relatedness ("Apple" ↔ "iPhone");
    empty no-op when no model is installed.

  This union replaced an earlier `semantic OR recency` **either/or** that was a live duplication bug: the
  embedding backfill embeds a new subject's vector only *after* the distillation that creates it, so the
  freshest subjects were invisible to semantic KNN, and a non-empty semantic set suppressed the recency
  fallback — letting "Marvel Rivals / gaming" get reworded into "Marvel Rivals gaming videos". The recency
  floor and lag-free lexical leg both close that gap. **Graceful degradation is load-bearing** (prod
  ships zero embedding model today): with no model the recency + lexical legs still run. Gated on
  `default_semantic_search_enabled()`, default model `nomic-embed-text-v1.5`, **not bundled** (opt-in
  download). The semantic floor/`K`/cap are **starting points pending calibration against real subject
  clusters** once a model runs on real data. See [ADR 0042](../adr/0042-subject-centric-conclusion-identity-and-pre-retrieval-candidate-selection.md).
- **Subject vectors live in their own plain table, embedded by a desktop backfill worker.** Migration
  `0043` adds `user_context_subject_vectors(subject TEXT PRIMARY KEY COLLATE NOCASE, embedding BLOB,
  embedded_at_ms INTEGER)` and migration `0044` adds `embedded_model TEXT` — **not `vec0`**: at ~2k
  subjects, loading BLOBs and brute-force f32 cosine in
  Rust is microseconds. `SubjectVectorStore` (in `crates/app-infra/src/user_context/subject_vectors.rs`)
  does upsert/get/mark-stale/needs-embedding/cosine-KNN, and **app-infra stays embedding-free** — it
  stores BLOBs and computes cosine, never holds an embedder (the same boundary that keeps `ai-runtime`
  out of it). The desktop backfill worker (`subject_vector_worker.rs`, spawned on the deferred-startup
  seam) embeds distinct subjects via the shared **Semantic Search** embedder, embedding text
  `"{subject}: {canonical_statement}"` (statement enrichment so terse handles carry context). It is an
  **idle no-op when no model is installed** (self-runs the day one is), and **Dismiss** marks the
  affected subject's vector stale for lazy re-embed. **Vectors are model-identity aware:** each row
  records the `provider/model_id` it was embedded under; the worker's "needs embedding" query treats a
  vector embedded under a *different* model as stale (so the worker continuously re-embeds the whole
  dossier after a model switch — no separate reconciliation pass), and KNN ranks only vectors under the
  active model (so a stale cross-model vector never produces a garbage cosine while it waits to be
  re-embedded). This is stricter than the **Semantic Search** index, whose dimension-only `vec0` rebuild
  is forced by the fixed-dimension table rather than chosen — the plain subject table has no such
  constraint, so it keys off model identity directly. The subject vectors are derived **User Context**:
  cleared by **Wipe User Context**, never cascaded by **Retention Policy**
  ([ADR 0029](../adr/0029-user-context-outlives-raw-retention-privacy-delete-cascades.md)). The embedder
  reuses the **Semantic Search** machinery ([ADR 0036](../adr/0036-semantic-search-v1-hybrid-fastembed-vectors-with-fts5.md) /
  [ADR 0037](../adr/0037-semantic-search-embeddings-on-candle-with-pluggable-backend.md)).
- Derivation runs on **two cadences**: **Activity** derivation is opportunistic and frequent
  (batched, preferring idle time or just-after-recording, the **OCR Catch-Up** pattern, off the hot
  path), while **Conclusion** re-distillation runs on a slower beat over accumulated Activities, plus
  whenever decay or fresh evidence has made a **Subject**'s conclusions stale. Cheap-and-frequent for
  the diary, expensive-and-occasional for the dossier.
- The **Derivation Budget** governs both cadences; for a cloud engine its intensity is the
  user-chosen named tier, for a local engine it is fixed pacing policy.
- A **Conclusion**'s **Confidence** is recency-weighted evidence: it fades on silence and drops
  faster on contradiction, rather than holding firm until actively contradicted.
- The local/cloud choice is a **Reasoning Engine** selection, not a layer boundary: both
  **Activity** and **Conclusion** derivation run through whichever model the user selected.
- When the selected **Reasoning Engine** is a cloud model, only redacted text (recognized screen
  text and transcripts, through the existing broker redaction) crosses the wire — never raw frame
  images or audio; when it is a local model, nothing leaves the device.
- The assembled dossier (the set of **Conclusion** values) is stored only on-device regardless of
  engine; a cloud **Reasoning Engine** is stateless reasoning that sees redacted summaries passing
  through and never holds the assembled profile.
- A cloud **Reasoning Engine** uses a **bring-your-own-key** credential: the user supplies their own
  provider API key, stored in OS platform secret storage (the same Keychain boundary as the
  **Capture Index Key Store**, never in `saveDirectory` or a config file) and handed to flue's
  provider configuration at runtime. A local **Reasoning Engine** needs no key. Mnema operates no
  backend and no token proxy — the user's key talks straight to their provider. This reverses the
  *mechanism* of ADR 0023 (Mnema goes from holding zero credentials to storing the user's key) while
  keeping its spirit (no Mnema servers, no Mnema-owned credentials). A local engine instead points
  rig-core at an Ollama/Llamafile endpoint and needs no key. See
  [ADR 0028](../adr/0028-ai-features-call-models-rust-side-via-rig-core.md).
- Using a cloud **Reasoning Engine** for continuous background derivation is its own explicit opt-in,
  separate from the **Ask AI Setting**: enabling Ask AI must never silently start always-on
  background egress. It carries its own disclosure (continuous redacted egress + ongoing token cost)
  and its own model selection. With it off, **User Context** runs on a local **Reasoning Engine** if
  one is selected, otherwise it is unavailable (as **Ask AI** is unavailable without a usable runtime).
- **User Context** was the first feature built on the **Rust-side `rig-core`** path; **Ask AI**
  (Quick Recall + Chat) then migrated off the PI/Node shim onto the same engine
  ([ADR 0033](../adr/0033-ask-ai-migrates-onto-shared-reasoning-engine.md)), so there is now one way
  to interface with AI. flue and the PI/Node shim are dropped; rig-core's agent/tool modules cover
  Ask AI's interactive, streaming, tool-enabled loop Rust-side (a tool-agnostic capability of the
  engine crate, with broker tools injected from the Tauri layer) without reintroducing Node.

- Capture deletion interacts with derived data by *type of deletion*. **Retention Policy**
  (time-based housekeeping) does NOT cascade: when raw **Captured Frame** / **Audio Segment** media
  ages out, derived **Activity** summaries survive and become the durable evidence floor, so a
  short-retention user can still accumulate a deep dossier; drilling back to the original frame is
  then best-effort. **Delete Recent Capture** (the privacy panic button) DOES cascade hard: it
  purges any **Activity** derived from the deleted window and re-judges any **Conclusion** that
  leaned on it by re-applying the formation bar: a **Conclusion** whose surviving support falls
  below the bar is dropped, not only one that loses all its evidence. A Pin exempts a Conclusion
  from that re-check down to one surviving support, but never past zero — no ungrounded
  conclusions.
- Because the derived dossier deliberately outlives the raw-capture **Retention Policy** window,
  this longer memory must be clearly disclosed and backed by a wipe-my-**User Context** control;
  it is a conscious expansion of how long Mnema remembers the user, not a silent default. See
  [ADR 0029](../adr/0029-user-context-outlives-raw-retention-privacy-delete-cascades.md).

- **Activity**, **Conclusion**, and **Dismissal State** are stored inside the **Encrypted Capture
  Index** (page-level SQLCipher, key in the OS keychain via the **Capture Index Key Store**) — not a
  plaintext sidecar or a JSON file under `saveDirectory`. The dossier is more sensitive than the OCR
  text already protected there, so it gets at least the same protection. The schema lands in
  app-infra migrations `0022`–`0025` (INTEGER unix-millis timestamps); the derived tables carry
  **no foreign key to frames/audio_segments** so the dossier survives **Retention Policy** aging
  (the [ADR 0029](../adr/0029-user-context-outlives-raw-retention-privacy-delete-cascades.md) evidence floor).
- Derivation runs in the background `worker.rs` (`spawn_user_context_worker`, started in deferred
  startup beside the retention worker, off the capture hot path in the **OCR Catch-Up** pattern) on
  three cadences: a frequent **Activity** beat (forward catch-up plus newest-first **History
  Backfill** bounded by the **Derivation Budget**), a slower **Conclusion**-distillation beat, and a
  slowest **Confidence**-decay-and-snapshot beat. The worker resolves the **Reasoning Engine** from
  `AiRuntimeSettings` (plus the **bring-your-own-key** from the keychain) and runs only redacted
  OCR/transcript text past a cloud engine; the assembled dossier stays on-device. Token usage is an
  ESTIMATE (≈4 chars/token), not a billed figure.
- **Wipe User Context** is a deliberate, confirmed action (Tauri dialog, like other destructive
  flows) that clears all derived data — **Activity**, **Conclusion**, **Dismissal State** — without
  touching raw captures or settings. It is the inverse of derivation: the raw record stays, the
  *understanding* is forgotten. The engine can rebuild from raw captures later only on re-opt-in.
- Disabling the **Reasoning Engine** is NOT a wipe: it stops new derivation but leaves the existing
  dossier readable. **Wipe User Context**, however, also turns the engine off — wiping implies "I'm
  done," and rebuilding is a deliberate re-opt-in.

## Flagged Ambiguities

- "context" is overloaded: **Search Context** is per-result captured labels, while **User Context**
  is synthesized standing understanding. Keep them distinct.
- "what you did" (the **Activity** story) versus "what that says about you" (the **Conclusion**
  layer) are two layers, not one; **Conclusion** values are grounded in **Activity** values.
- "hybrid" (local/cloud) was first read as a *layer* boundary (Activities local, Conclusions cloud);
  resolved: it is a per-user **Reasoning Engine** *choice* applied to both layers, since not every
  machine can run a capable local model.
- "the cloud builds my profile" vs "the cloud never holds my profile": resolved — a cloud
  **Reasoning Engine** sees redacted summaries passing through and **Ask AI** fetches relevant pieces
  on demand, but the assembled dossier is stored only on-device and is never sent whole.
- "delete my captures" is two different actions for derived data: **Retention Policy** keeps the
  dossier, **Delete Recent Capture** cascades into it. They are not the same and must not be
  collapsed.
- "turn it off" vs "wipe it": disabling the **Reasoning Engine** stops derivation but keeps the
  dossier; **Wipe User Context** clears the dossier and also turns the engine off.
