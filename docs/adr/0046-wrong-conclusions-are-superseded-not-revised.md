---
status: accepted
date: 2026-07-02
---

# Wrong Conclusions are superseded by a replacement belief, never revised in place

A **Conclusion** whose statement turns out to be wrong is **superseded**: the corrected belief forms
as a **new** row at formation confidence, and the wrong row is retired (status `'superseded'`,
history kept, linked to its successor). The distillation **Reasoning Engine** signals this in the
same single call that reinforces and forms beliefs — each emitted belief may cite a `supersedes_id`
alongside its optional `reinforces_id`. Statements remain frozen forever; there is no in-place
rewrite. This extends [ADR 0043](0043-conclusion-identity-is-per-belief-within-a-subject.md)
(per-belief identity) with the third verb its two-verb model was missing.

**Context — reinforce-or-fork leaves wrong beliefs immortal.** Under 0043 an emitted belief either
cites the `reinforces_id` of an existing statement it restates, or forms a new row. When an early
Conclusion is *wrong* — formed the moment the count-based formation bar cleared, typically from 2–3
correlated Activities out of one bad capture window — later, better-informed passes have no honest
move against it. The model correctly refuses to cite `reinforces_id` on a statement it no longer
agrees with, so the store's only option is a new sibling row. Confirmed in-app (2026-07-02): subject
"mnema" carried "Is working on the mnema learning platform, navigating its learning history and
content modules" at 66% (`initial_confidence(3)` exactly), warming, formed from garbled OCR — and
the only paths that could ever remove it were the 30-day silence half-life, `contradict_refs` the
model in practice never emits, or a manual Dismiss. Wrong early beliefs both persist at authoritative
confidence and breed better-worded siblings next to themselves.

**Decision.**

- **Supersede, not revise.** The corrected claim is a *different sentence*, so it earns its own
  confidence from formation like any belief. Rewriting the cited row's statement in place was
  rejected: the old row's confidence and warming trajectory were earned by a different sentence, so
  an in-place rewrite reattaches an earned trajectory to words that never earned it — the same
  statement/history dishonesty 0043 was written to kill, moved from evidence-space to time — and it
  reintroduces displayed-text churn. A bad supersede hides a good row recoverably; a bad revise
  destroys its text.
- **Same-call cited id.** `DistilledConclusion` gains `supersedes_id: Option<i64>`, prompt rule:
  cite it only when an existing conclusion shown in the KNOWN SUBJECTS block is *wrong* in light of
  the evidence and this belief replaces it. The store validates exactly as for `reinforces_id`
  (shown id, same subject, not dismissed); retirement and formation share one transaction. A
  separate reconciliation pass and user-only correction were rejected for the same reasons 0043
  chose single-call citing: candidates are already in the prompt, no extra calls, the model errs
  visibly and reversibly.
- **Supersede composes with reinforce.** One belief may carry `reinforces_id = A` and
  `supersedes_id = B` (A ≠ B, same subject): fresh evidence reinforces the correct existing sibling
  *and* retires the wrong one — the already-forked case existing databases hold today. Formation-
  with-supersede is the `reinforces_id = None` case of the same rule; `superseded_by` points at the
  reinforced (or newly formed) successor.
- **Three deterministic guardrails at the persist site.** (1) **Pinned is untouchable** — a
  `supersedes_id` naming a pinned row is ignored (the citing belief still persists normally).
  (2) **The superseder must survive its own gates** — retirement happens only after the citing
  belief passes the sensitive-category guardrail, formation bar, and resurface gate; a dropped
  draft retires nothing. (3) **Retire only downward** — instant retirement only when the old row is
  weaker than the citing belief's formation value (`old.confidence ≤ initial_confidence(support)`);
  against a stronger row the supersede degrades to one `apply_contradiction` drop (−0.35), so
  killing a well-earned belief takes repeated independent agreement, while a one-off model hiccup
  costs 0.35 that fresh reinforcement wins back.
- **Superseded statements enter the dismissal machinery.** A supersede writes a
  `user_context_dismissals` row (statement, evidence fingerprint, support count) tagged
  `source = 'supersede'` (user dismissals become `source = 'user'`). The do-not-reconstitute prompt
  block lists it as "already replaced", and the deterministic resurface gate blocks re-formation:
  the evidence set at retirement can never rebuild it, and clearing the 2× resurface bar — the old
  belief was right after all — flips the *retained* row back to visible with its history, rather
  than inserting a duplicate. This deliberately converts a latent landmine into the resurface path:
  the formation upsert's `ON CONFLICT (subject, statement) DO UPDATE ... status='visible'` would
  otherwise silently resurrect a retired row the next time the model re-derived the same wrong
  statement from the still-in-window evidence. A hard tombstone was rejected — the supersede itself
  can be the mistake.
- **Retired rows leave every read surface except the audit trail.** Status `'superseded'` is
  excluded from the dossier/strip and counts, `recall_context`, the KNOWN SUBJECTS block, and the
  decay beat. The successor's unified timeline renders one event from the `superseded_by` link —
  "replaced an earlier take — *old statement*" at retirement time — so a model-initiated removal of
  something the user may have read is always auditable in place. Silent removal (trust cost) and
  keeping retired rows visibly de-emphasized in the strip (the wrongness lingers, the original
  complaint) were rejected.
- **Observable from day one, shipped with the owed 0043 validation.** The `derivation_run` ledger's
  gate counters grow `superseded` / `supersede_degraded` / `supersede_blocked`; the worker log line
  reports retirements. This design widens the matcher's job to a three-way judgment (reinforce /
  supersede / new) on a model whose *two-way* reliability is still an open 0043 validation — one
  dogfooding pass validates both. If the counters show the model cannot hold the distinction, the
  escalation is the focused second-pass reconciliation 0043 already reserved (or a stronger pinned
  distillation model), not a redesign.

**Schema.** One **fresh migration file** (the in-place-edit habit ends here; no users yet, last
cheap moment to switch): nullable `superseded_by` on `user_context_conclusions`, `source` on
`user_context_dismissals`, new ledger counters. `status` has no CHECK constraint, so
`'superseded'` is a new string, not a table rewrite.

**Consequences.**

- Wrong early beliefs stop being immortal: the model can retire them the moment better evidence
  arrives, in the same pass, and the lingering already-forked siblings in existing data are
  cleanable via reinforce+supersede.
- Formation is **deliberately out of scope**: the count-based formation bar over correlated
  same-window evidence still lets a wrong belief be born at ~54–66%. This ADR fixes persistence and
  proliferation, not birth; raising evidence-independence at formation is its own future decision.
- The worst case under the guardrails is "a strong belief dips −0.35 and recovers", not "a strong
  belief vanishes"; the worst *composed* case (reinforced the right row, retired the wrong sibling)
  is undone by the resurface path.
- Dismissal provenance is now two-valued; anything consuming `user_context_dismissals` must treat
  `source = 'supersede'` rows as machine corrections (phrasing, resurface target = retained row),
  never as user actions.
