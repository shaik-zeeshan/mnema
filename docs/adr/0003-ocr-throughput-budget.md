# ADR 0003: OCR Throughput Budget

## Status

Accepted.

## Context

Continuous desktop capture can create more OCR work than the app should run immediately. OCR should remain searchable and respect the user's selected provider and mode, but it should not create sustained CPU pressure that makes recording feel heavy.

Apple Vision `fast` is not an acceptable automatic fallback because it recognizes materially less text than `accurate`.

## Decision

Mnema uses an OCR Throughput Budget:

- An execution budget paces admitted OCR jobs with deterministic cooldowns based on observed OCR runtime.
- An admission budget records a durable decision for each newly captured automatic frame and skips only low-value candidates during active recording under high OCR queue pressure.
- Existing captured-frame equivalence remains the first duplicate filter.
- Manual frame reprocessing bypasses admission filtering, but still respects execution pacing.

The budget records admission and execution telemetry in SQLite for tuning and debugging.

## Alternatives Rejected

- Hard CPU caps: process-wide CPU enforcement is brittle and not needed for the first budget.
- Live CPU feedback: live measurement adds platform noise and tuning complexity before durable OCR timing exists.
- Provider or mode switching: Mnema must not silently change the user's selected OCR quality.
- OCR off during recording: this would remove searchable changes when users most need them.
- Pre-OCR text detection: an OCR-relevance probe adds extra image work and another model/policy surface.

## Consequences

OCR remains accurate and searchable, but automatic low-value OCR candidates can be skipped during active high-pressure recording. Existing queued OCR jobs are preserved and paced. Debug surfaces can inspect admission reasons, queue pressure, frozen OCR payload choices, run duration, queue wait, result text length, and observation count.
