# ADR 0003: OCR Throughput Budget

## Status

Accepted.

## Context

Continuous desktop capture can create more OCR work than the app should run immediately. OCR should remain searchable and respect the user's selected provider and mode, but it should not create sustained CPU pressure that makes recording feel heavy.

Apple Vision `fast` is not an acceptable automatic fallback because it recognizes materially less text than `accurate`.

## Decision

Mnema uses an OCR Throughput Budget:

- An execution budget paces admitted OCR jobs with deterministic cooldowns based on observed OCR runtime.
- An admission budget records current-run decisions in memory and skips only low-value candidates during active recording under high OCR queue pressure.
- Existing captured-frame equivalence remains the first duplicate filter.
- Manual frame reprocessing bypasses admission filtering, but still respects execution pacing.

The budget records admission and execution telemetry as bounded current-run state and structured debug logs. The main app database stores user/domain data such as captured frames, OCR jobs, and OCR results; it does not store OCR budget bookkeeping.

## Alternatives Rejected

- Hard CPU caps: process-wide CPU enforcement is brittle and not needed for the first budget.
- Live CPU feedback: live measurement adds platform noise and tuning complexity before current-run OCR timing is useful enough.
- Provider or mode switching: Mnema must not silently change the user's selected OCR quality.
- OCR off during recording: this would remove searchable changes when users most need them.
- Pre-OCR text detection: an OCR-relevance probe adds extra image work and another model/policy surface.
- Durable SQLite budget telemetry: budget decisions and timing summaries are operational/debug data, and keeping them in the main app database stores low-value data longer than needed.

## Consequences

OCR remains accurate and searchable, but automatic low-value OCR candidates can be skipped during active high-pressure recording. Existing queued OCR jobs are preserved and paced. Debug surfaces can inspect bounded current-run admission reasons, queue pressure, frozen OCR payload choices, run duration, queue wait, result text length, and observation count. This diagnostic state resets on app restart and does not participate in retention cleanup.
