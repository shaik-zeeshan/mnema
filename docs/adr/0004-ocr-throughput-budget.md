# ADR 0004: OCR Throughput Budget

## Status

Accepted.

## Context

Continuous desktop capture can create more OCR work than the app should run immediately. OCR should remain searchable and respect the user's selected provider and mode, but it should not create sustained CPU pressure that makes recording feel heavy.

Apple Vision `fast` is not an acceptable automatic fallback because it recognizes materially less text than `accurate`.

## Decision

Mnema uses an OCR Throughput Budget:

- An execution budget paces admitted OCR jobs with deterministic cooldowns based on observed OCR runtime.
- An admission budget records current-run decisions in memory and skips only low-value candidates during active recording under high OCR queue pressure.
- During active recording the admission budget also admits a frame on **Visual Novelty** — when its captured-frame equivalence fingerprint (`equivalence_hint`) is new in the current admission scope — so a one-off readable screen inside an otherwise unchanging window is still read. This path reuses only the existing fingerprint (no new image analysis or OCR-relevance probe) and is bounded by three guards: it never fires under high OCR queue pressure, it is rate-capped to at most one novelty read per scope every couple of seconds, and a sustained run of continuously-novel frames (video/animation) suppresses it back to plain time-sampling until a repeated frame resets the run. See the amendment below.
- Existing captured-frame equivalence remains the first duplicate filter.
- Manual frame reprocessing bypasses admission filtering, but still respects execution pacing.

The budget records admission and execution telemetry as bounded current-run state and structured debug logs. The main app database stores user/domain data such as captured frames, OCR jobs, and OCR results; it does not store OCR budget bookkeeping.

## Alternatives Rejected

- Hard CPU caps: process-wide CPU enforcement is brittle and not needed for the first budget.
- Live CPU feedback: live measurement adds platform noise and tuning complexity before current-run OCR timing is useful enough.
- Provider or mode switching: Mnema must not silently change the user's selected OCR quality.
- OCR off during recording: this would remove searchable changes when users most need them.
- Pre-OCR text detection: an OCR-relevance probe adds extra image work and another model/policy surface.
- Unbounded visual-fingerprint novelty admission: admitting *every* frame whose captured-frame equivalence fingerprint looks new was rejected — the fingerprint is too coarse to detect new text within an unchanging window, and admitting on novelty alone re-admits video/animation frame-by-frame. The dwell gap for *repeated* screens is instead closed by OCR Fallback Eligibility — equivalence reuse must defer only to an earlier frame that has an OCR Job, so an admission-skipped textless frame no longer cancels a later representative admission. A *bounded* novelty path for *one-off* screens was later adopted; see the amendment below.
- Durable SQLite budget telemetry: budget decisions and timing summaries are operational/debug data, and keeping them in the main app database stores low-value data longer than needed.

## Consequences

OCR remains accurate and searchable, but automatic low-value OCR candidates can be skipped during active high-pressure recording. Existing queued OCR jobs are preserved and paced. Debug surfaces can inspect bounded current-run admission reasons, queue pressure, frozen OCR payload choices, run duration, queue wait, result text length, and observation count. This diagnostic state resets on app restart and does not participate in retention cleanup.

## Amendment: bounded visual-novelty admission (2026-05-26)

The original rejection of visual-fingerprint novelty admission did not weigh the CLI find-by-content path. `mnema search`/`timeline` and tools built on the brokered access layer read only the pre-built `search_documents` index; there is no on-demand OCR anywhere in the broker. So a one-off readable screen that is skipped at capture time — e.g. a scrolled GitHub PR view dwelled on inside an unchanging window — has no equivalent neighbor to borrow text from and is permanently unsearchable. A read-side or dashboard fallback fundamentally cannot satisfy a programmatic query, because the text has to already be in the index. Inspection of a real recording also confirmed the fingerprint *does* flip on scroll (a distinct `equivalence_hint` from the prior frame), so it is a usable novelty signal for large visible changes even though it stays blind to tiny edits (a single typed word, one new log line).

Mnema therefore adopts a **bounded** visual-novelty admission, distinct from the unbounded form rejected above. The two coarseness concerns are addressed with guards rather than by abandoning the signal: a per-scope rate cap is the firm cost bound (at most one novelty read per scope every couple of seconds), and a continuous-novelty suppressor falls back to plain time-sampling once frames have been novel for a sustained run, so video/animation does not trigger per-frame OCR. The novelty path still reuses only the existing fingerprint (no OCR-relevance probe), the low-queue-pressure gate still applies, captured-frame equivalence reuse still runs first (so genuinely repeated frames keep sharing one read and OCR Fallback Eligibility is unchanged), and novelty memory stays live-only in-memory like the rest of the budget. This closes the large-visible-change searchability gap for one-off screens; small incremental edits remain covered by the existing time-sampled representative. The change is forward-looking only — existing textless frames are not backfilled.
