# Frontend Integration Context

This context captures the domain language for the desktop capture app so architecture discussions use stable project terms.

## Language

**Screen Frame Artifact**:
A just-produced native capture output that has not yet been persisted into app-infra.
Screen frame artifacts are currently written as JPEG files.
_Avoid_: captured frame, transient screenshot, raw frame record

**Captured Frame Pipeline**:
A persisted flow for a captured screen frame from intake through batch attachment, OCR planning, and batch-finalization readiness.
_Avoid_: frame processing service, frame ingestion component, OCR pipeline

**Captured Frame**:
A screen image captured during a recording session and persisted as frame metadata plus a file path. A **Captured Frame** is the stored database-backed record, not the just-produced native artifact.
_Avoid_: screenshot, image record

**Frame Batch**:
A time-windowed group of captured frames for one recording session.
_Avoid_: bucket, chunk, frame group

**OCR Job**:
A processing job that recognizes text for one captured frame.
_Avoid_: processor task, recognition work item

**Captured Frame Reprocessing**:
A request to re-run OCR for an existing **Captured Frame** that is already persisted.
_Avoid_: force processing, rerun pipeline, requeue screenshot

**Recording Lifecycle**:
The in-memory control flow for one coordinated recording runtime that starts capture, owns pause/resume decisions, rotates segments, recovers after wake, and stops capture across the requested sources. Screen and system audio share the screen capture backend, while microphone runs as a separate native session.
_Avoid_: capture runtime, recorder service, session manager

**Captured Frame Equivalence**:
The rule for when two **Captured Frame** values should be treated as the same OCR-relevant visual content for downstream decisions such as **OCR Job** admission. **Captured Frame Equivalence** is defined over normalized visual content rather than persisted artifact bytes, and intentionally ignores cursor-sized changes plus limited localized visual noise that does not materially change OCR-relevant content.
_Avoid_: dedupe hash, screenshot sameness, OCR skip heuristic

**Captured Frame Equivalence Scope**:
The rule for where an earlier equivalent **Captured Frame** may be searched when applying **Captured Frame Equivalence**. **Captured Frame Equivalence Scope** is session-wide by default, but narrows to the same hidden segment workspace when the candidate **Captured Frame** originated from a hidden segment workspace artifact path.
_Avoid_: workspace filter, lookup scope, same-segment rule

**Hidden Segment Workspace**:
A hidden per-segment directory (`.<session>-segment-####/`) that stores temporary capture artifacts and exported JPEG frames for one screen segment. A **Hidden Segment Workspace** lives beside its visible sibling segment recording file.
_Avoid_: temp folder, segment scratch dir, hidden segment temp

**Hidden Segment Workspace Repair**:
The scan-and-cleanup flow that classifies a **Hidden Segment Workspace** using **Frame Batch** references, **OCR Job** references, visible sibling presence, and pending frame artifacts before deciding whether it is safe to remove.
_Avoid_: temp cleanup, workspace GC, segment dir sweep

**Managed Storage Layout**:
The derived on-disk layout rooted at `<saveDirectory>` that owns app-infra state such as SQLite and the recordings tree.
_Avoid_: save dir helper, path utility, storage paths

**Audio Activity Sample**:
A raw audio probe reading such as latest normalized level or last-sample timestamp, exposed for debug visibility but not itself used as the inactivity decision.
_Avoid_: audio activity event, microphone activity, system audio activity

**Audio Activity Decision**:
The threshold-qualified inactivity-policy view of audio activity, including enabled state, threshold, and derived idle used for pause/resume decisions.
_Avoid_: raw audio sample, activity reading, latest level

## Relationships

- A **Screen Frame Artifact** becomes a **Captured Frame** only after app-infra persists it.
- A **Captured Frame Pipeline** persists one **Captured Frame**.
- A **Captured Frame Pipeline** attaches each **Captured Frame** to exactly one **Frame Batch**.
- A **Captured Frame Pipeline** may enqueue one **OCR Job** for a **Captured Frame**.
- A **Recording Lifecycle** coordinates screen, microphone, and system-audio capture within one recording runtime.
- A **Recording Lifecycle** may pause or resume requested sources based on inactivity policy.
- A **Managed Storage Layout** is derived from one `saveDirectory` value.
- A **Managed Storage Layout** contains the recordings tree under `<saveDirectory>/recordings`.
- **Captured Frame Equivalence** determines whether a new **Captured Frame** needs a new **OCR Job**.
- **Captured Frame Equivalence Scope** determines which earlier **Captured Frame** values are eligible comparison candidates.
- A **Captured Frame Pipeline** skips a new **OCR Job** when an earlier **Captured Frame** in the same session already has the same content fingerprint.
- A **Frame Batch** can be finalized only after its **OCR Job** entries are terminal.
- **Captured Frame Reprocessing** operates on an existing **Captured Frame**, not on a new **Screen Frame Artifact**.
- A **Hidden Segment Workspace** may be preserved when an incomplete **Frame Batch** or nonterminal **OCR Job** still references it.
- **Hidden Segment Workspace Repair** removes only **Hidden Segment Workspace** values that are safe to remove.
- An **Audio Activity Sample** can inform an **Audio Activity Decision**, but the two are not interchangeable.
- An **Audio Activity Decision** is what the inactivity policy uses to pause or resume capture.

## Example dialogue

> **Dev:** "When a **Captured Frame** has the same fingerprint as an earlier frame, does the **Captured Frame Pipeline** still create an **OCR Job**?"
> **Domain expert:** "No — the frame is persisted and attached to its **Frame Batch**, but duplicate content does not need another **OCR Job**."

> **Dev:** "Is `microphoneActivityLastUnixMs` the same thing as the audio signal the inactivity policy uses?"
> **Domain expert:** "No — that timestamp is an **Audio Activity Sample**; the inactivity pause logic uses an **Audio Activity Decision** derived from threshold-qualified activity."

## Flagged ambiguities

- "pipeline" previously meant both frame intake and OCR execution; resolved: **Captured Frame Pipeline** means frame intake through batch-finalization readiness, while **OCR Job** means the recognition work for one frame.
- "audio activity" previously referred to both raw probe output and inactivity-policy state; resolved: raw probe output is an **Audio Activity Sample**, while policy-facing threshold-qualified state is an **Audio Activity Decision**.
