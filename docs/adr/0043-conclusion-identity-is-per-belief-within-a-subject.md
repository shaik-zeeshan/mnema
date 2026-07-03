---
status: accepted
date: 2026-07-02
---

# Conclusion identity is per-belief within a Subject, and the distiller matches by citing an existing Conclusion's id

A **Conclusion**'s identity becomes the **individual atomic belief**, not the **Subject** it is filed
under. A **Subject** holds as many **Conclusion** values as the user has distinct beliefs about it;
recurring evidence reinforces the *specific belief it restates* rather than a single per-subject
canonical row. The distillation **Reasoning Engine** is the matcher one level deeper: the prompt shows
it each candidate subject's existing **Conclusion** values (`id: statement`), and each atomic belief it
emits either cites the `id` it reinforces or is new. This **supersedes the subject-centric /
one-canonical-row write rule of [ADR 0042](0042-subject-centric-conclusion-identity-and-pre-retrieval-candidate-selection.md)**
while keeping 0042's candidate-recall legs and Subject-vector store intact.

**Context — the collapse produced statements that lie.** 0042 (accepted 2026-06-30) made a
**Conclusion**'s identity its **Subject** alone: the upsert matched on subject and reinforced the
subject's canonical row, freezing that row's statement and replacing its evidence with each pass's
window. That fixed duplicate *subjects* and made *warming* reachable, but it collapsed a subject's
genuinely distinct beliefs into one row — and because the statement was frozen while its evidence was
overwritten every pass, the displayed belief and its linked evidence drifted apart *by construction*.
Confirmed in-app: a "Gaming" conclusion read "Plays Genshin Impact via a Windows VM and watches Marvel
Rivals / OW2 streams" (first seen ~19h earlier) while its only linked activities were ≤3h old and about
*007 First Light* and game-name brainstorming — none about Genshin. The corollary: "confidence" had
quietly become *subject salience* (ratcheted by any activity about the subject), not confidence in the
sentence. A **Conclusion** must "earn the name": one atomic belief, its own evidence, its own trajectory.

**Decision.**

- **Atomic beliefs.** The distiller splits compound statements into one belief per *distinct claim that
  can independently gain evidence, be confirmed, dismissed, or fade* — not one per proper noun (splitting
  "watches Marvel Rivals and OW2 streams" per-game would recreate the sprawl one level down). Prompt-only
  change.
- **Beliefs are shown to the matcher.** 0042's candidate-subject recall (recency floor + lexical +
  semantic legs) already surfaces the subjects this window touches; the **KNOWN SUBJECTS** block now
  lists, under each, that subject's existing **Conclusion** values (`id: statement`), confidence-ordered
  under a char budget — so "relevant conclusions" ride along with the relevant subjects, needing no
  separate retrieval. Steady-state per-subject count is small (the point of reinforce-not-duplicate); the
  char budget degrades the *least-relevant* subjects to handle-only (today's behavior), never a cliff.
- **The store trusts the cited id.** `upsert_conclusion_with_evidence` stops matching subject-only. Each
  emitted belief carries an optional `reinforces_id`: present and valid (row exists, same subject,
  non-dismissed) → reinforce that row (bump **Confidence**, replace *that belief's* evidence, snapshot the
  up-step, freeze the statement); absent or stale/foreign/dismissed → form a new row at formation
  confidence. A new belief about an *existing* subject inserts a new row — the case the subject-only match
  got wrong. `UNIQUE(subject, statement)` stays as the exact-repeat net; a new statement under an existing
  subject inserts cleanly.
- **Freeze is retained and now honest.** Freezing the statement on reinforce keeps one clean per-row
  trajectory and dodges the UNIQUE index, exactly as in 0042 — but under per-belief identity the frozen
  wording is *that belief's own* and its evidence is *that belief's own*, so statement and evidence cannot
  drift.
- **Warming is unchanged.** The up-step **Confidence History** snapshot is per-conclusion and stays; the
  **Subject** view already aggregates trend across a subject's conclusions (`deriveTrend` averages
  per-conclusion slopes). No display-side change.

**Considered Options.**

- **Keep subject-only, fix only the evidence (unfreeze or append):** rejected — the beliefs are genuinely
  distinct, so no single statement is honest for "plays Genshin" *and* "plays 007", and unfreezing
  reintroduces the displayed-text churn 0042 removed.
- **A tool the distiller calls to fetch conclusions on demand (agentic loop):** rejected for the
  *background* pass. It moves retrieval from push to pull — the model must *remember to look up* before
  writing, adding a miss mode rather than removing one — and multiplies token cost (against the
  **Derivation Budget**), raises the capability floor above small local models (cf. DeepSeek rejecting
  rig's `tool_choice`), and fits the batch-of-many-beliefs shape poorly. Reserved for interactive
  **Ask AI**, where it already lives and the cost model fits.
- **Deterministic similarity auto-merge (lexical/semantic threshold) as the matcher or backstop:**
  rejected — a wrong merge silently fuses two distinct beliefs ("plays Genshin" into "stopped playing
  Genshin") with no un-merge, whereas the LLM errs visibly and reversibly (**Dismiss**). Same reason 0042
  rejected code-side merge for subjects.
- **A focused second-pass reconciliation call:** deferred — the single-call cite-`id` is primary; a
  targeted yes/no second check ("same claim as any of these?") is the escalation *if* runtime shows the
  model misses too often, not built up front.

**Consequences.**

- **Forward-only, no migration.** The cited `id` is transient LLM output — nothing new is stored, so there
  is no schema change and none of the in-place-migration hazard. Legacy collapsed/compound rows self-fade
  under decay; **Wipe User Context** is the opt-in clean rebuild. Transitional duplicates (a compound row
  beside its atomic replacements) show until the stale one fades below the display floor.
- **Residual duplication is bounded, not zero.** A belief the model was shown but failed to cite mints one
  near-duplicate that then self-fades — a far lower rate than the pre-0042 ~100% miss (the model was shown
  nothing then), and the accepted floor unless runtime warrants the second-pass escalation.
- **Warming becomes honest but dilutable.** A subject with one warming belief among many steady ones
  averages toward "steady" in the rolled-up glyph; the per-conclusion sparklines in the expanded view
  always show it. Tuning knob if it proves too muted: confidence-weight the trend average (calibrate-later,
  not build-now).
- **Runtime validation still outstanding** (as with 0042): that the model reliably reuses a supplied `id`,
  and that per-belief warming appears on real data.
