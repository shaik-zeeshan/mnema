---
status: proposed
---

# Derived User Context outlives raw-capture retention; the privacy panic button cascades into it

**User Context** is derived from raw captures: an **Activity** summarizes **Captured Frame** /
**Audio Transcription Span** content, and a **Conclusion** is grounded in **Activity** values. Mnema
deletes captures two ways, and they must behave oppositely toward this derived data. **Retention
Policy** (time-based housekeeping, e.g. "keep 7 days") does **not** cascade: when raw frame/audio
media ages out for disk reasons, the derived **Activity** summaries survive and become the durable
evidence floor, so a short-retention user can still accumulate a months-deep dossier. **Delete Recent
Capture** (the privacy panic button) **does** cascade hard: it purges any **Activity** derived from
the deleted window and re-judges or drops any **Conclusion** that leaned on it; a **Conclusion** that
loses all its evidence is dropped, because there are no ungrounded conclusions.

The derived **Activity**/**Conclusion** data lives inside the **Encrypted Capture Index** (page-level
SQLCipher, key in the **Capture Index Key Store**), as the dossier is more sensitive than the OCR
text already protected there. Because the dossier deliberately outlives the raw-capture **Retention
Policy** window, this longer memory is disclosed and backed by a **Wipe User Context** control that
clears all derived data (**Activity**, **Conclusion**, **Dismissal State**) without touching raw
captures or settings; wiping also turns the **Reasoning Engine** off, since wiping implies "I'm done."
Merely disabling the engine is not a wipe — it stops new derivation but leaves the dossier readable.

**Considered Options**

We rejected **cascading retention into derived data** (delete the frame → delete what was learned
from it): it keeps grounding maximally pure, but it would make the entire feature near-worthless for
the privacy-minded short-retention users who most want local-first software — their evidence would
evaporate weekly and the dossier could never accumulate. We rejected **never cascading any deletion**:
that turns **Delete Recent Capture** into a lie, leaving "spent 5 minutes on [the sensitive thing]"
in the dossier after the user explicitly wiped that window — the exact liability the panic button
exists to prevent.

**Consequences**

Mnema's effective memory of the user now has two horizons: raw media bounded by **Retention Policy**,
and *derived understanding* that can persist far longer. This is a real expansion of how long Mnema
remembers someone and must be a surfaced, conscious choice, not a silent default — hence the
disclosure and the explicit **Wipe User Context** action. Drilling a **Conclusion** all the way back
to an original **Captured Frame** becomes best-effort once that frame has aged out under retention;
the surviving **Activity** summary remains the evidence. **Delete Recent Capture** must make affected
in-flight derivation non-runnable, the same way it already handles affected capture processing work.
