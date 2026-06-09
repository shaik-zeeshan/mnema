---
status: proposed
---

# User Context suppresses sensitive-category conclusions by a hard guardrail, with no opt-in

The same **Reasoning Engine** that produces "likes Apple" will, pointed at a person's whole digital
life, just as readily produce "appears depressed," "is likely job-hunting," "is probably pregnant,"
"leans politically X." A grounded, confident, persisted inference about health, sexuality, religion,
or politics is a different category of liability than a note about app preference — it is the thing
that turns **User Context** from helpful into the most incriminating file on the user's disk, and it
cuts hard against Mnema's existing conservative, app-only, no-content-classification privacy posture.
We therefore enforce a **Sensitive Category Guardrail**: **Conclusion** values (and **Activity
Category** / **Focus Classification** labels) in off-limits inference categories — health and mental
health, sexual orientation, religion, politics, and similar protected/intimate domains — are never
stored or shown.

The guardrail is enforced **two ways**, because neither alone is enough: a **soft** instruction tells
the **Reasoning Engine** not to form such conclusions, and a **hard** deterministic post-filter drops
any conclusion whose **Subject** lands in a sensitive category before it is ever persisted or
surfaced, as the backstop for when the model ignores the instruction. The guardrail is **suppressed
by default and not user-enableable in v1** — there is no "infer my mental health" toggle — and it
deliberately errs toward **over-suppression**: false-suppress a benign conclusion that brushes a
sensitive category rather than false-surface a real sensitive inference.

**Considered Options**

We rejected **no guardrail** (conclude anything): maximally useful, maximally dangerous, and
irreconcilable with a local-first tool whose trust proposition is that it is safe to run. We rejected
a **soft-only** instruction: an LLM told to avoid a category will sometimes do it anyway, and "the
prompt said not to" is no defense once the sensitive conclusion is on screen and stored. We rejected
**surface-but-flag** (show sensitive inferences with a warning label): the liability is in the
inference *existing and persisting*, not in whether it carries a badge. We rejected an **opt-in**
("yes, infer my health/mood"): even consented, it is a setting that ages badly and invites exactly
the headline the guardrail exists to prevent.

**Consequences**

The hard filter will sometimes suppress a legitimately benign conclusion that merely brushes a
sensitive category (e.g. a nutrition interest tripping a health filter), so the dossier has a
deliberate, invisible blind spot by design — the right error to make here. The guardrail runs at
*derivation* time, so sensitive conclusions never enter the dossier and the `recall_context` broker
tool that **Ask AI** uses physically cannot return them; guardrailing is not re-implemented at the
broker boundary. The off-limits category policy is a maintained list that will need to evolve.
