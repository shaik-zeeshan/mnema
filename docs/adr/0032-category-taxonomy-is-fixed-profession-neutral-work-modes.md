---
status: accepted
---

# Activity Category taxonomy is a fixed set of profession-neutral work modes

The v1 **Activity Category** taxonomy (Coding, Research, Communication, Design, Testing, Personal,
Distractions) was pitched at the wrong altitude: most of its members are *developer domains*, so a
lawyer's, writer's, or student's whole day collapses into "Research" or uncategorized. Since Mnema
is built for many kinds of users, we replace the members — but keep the taxonomy **fixed** — with
eight profession-neutral *work modes*: **Creating, Communication, Meetings, Research, Learning,
Organizing, Personal, Entertainment**. The specificity lost by generalizing ("Coding" → "Creating")
is not actually lost: the Activity title and summary already carry it; the category's only job is
to bucket time for trends, and buckets work best when they are few, stable, and predictable. The
canonical member definitions and boundary rules (Meetings vs Communication, Research vs Learning,
Personal vs Organizing) live in the **Activity Category** glossary entry in
[`docs/user-context/CONTEXT.md`](../user-context/CONTEXT.md).

**Distractions is deliberately not a category.** Mnema has two separate judgment axes — Activity
Category (what kind of thing) and **Focus Classification** (how focused) — and a "Distractions"
category leaks the focus axis into the category axis: the same Twitter scroll is a derailment
mid-task and intentional leisure on a Saturday evening. The neutral content label is
**Entertainment**; the derailed/focused judgment belongs exclusively to Focus Classification, which
is already correctable and gently framed. "2h Entertainment" is a fact; "2h Distractions" is a
scolding — exactly the preachy moment the User Context posture avoids.

**Considered Options**

We rejected **AI-generated per-user vocabulary** (the engine coins categories, anchored to a
registry it must reuse): the most personally resonant option, but the most machinery (registry,
normalization, label-drift control, extending the Sensitive Category Guardrail to generated label
text) and the least predictable across thousands of users — an unvetted emergent label is a
product-voice risk a fixed set structurally cannot have. We rejected **user-defined custom
categories**: a power-user feature most users never configure, which makes it a bad *default* for
a broad audience (CONTEXT.md already lists it under *Avoid* for v1). We rejected **free-form
model labeling** outright: without a stable label space, "coding"/"programming"/"development"
fragment and every trend view dies. Fixed-but-generalized keeps the closed-enum guarantees (the
guardrail cannot leak a sensitive label, the #108 correction loop steers within a stable space)
while fixing the actual complaint. Nothing here closes the door on AI/custom categories later —
the DB column is plain `TEXT`, so that evolution needs no data migration.

**Consequences**

Existing rows are relabeled by a one-time app-infra migration over both `category` and
`corrected_category` (Coding/Testing/Design → Creating, Distractions → Entertainment; the rest
carry over), so the finer-grained original labels are gone after migration — user corrections are
preserved but coarsened into a superset. `parse_category` keeps the old engine strings as aliases
mapped to the new values, so a model that habitually emits "coding" lands in Creating instead of
uncategorized. Relabeled Activities change the digest input fingerprint, so cached digests
regenerate on next view (no fingerprint version bump needed — the digest's output shape is
unchanged). The derivation prompt taxonomy, correction picker, and category display labels all
move to the new set together.
