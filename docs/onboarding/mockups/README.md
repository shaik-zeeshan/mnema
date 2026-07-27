# Onboarding rework (#195) — chosen design

Chosen direction: **Cinematic / Rewind**. Source of truth for markup, CSS tokens, copy and
layout is `chosen-cinematic-rewind.html` (the design brief document — each screen is rendered
inside a `.frame` element; port those frames, not the surrounding annotation chrome).

Open the file in a browser to see every screen. Screenshots are deliberately not committed — they
are derived from this HTML, and each screen's exact line range is tabulated in
[`../IMPLEMENTATION-BRIEF.md`](../IMPLEMENTATION-BRIEF.md).

| # | Screen |
|---|---|
| — | Flow map — the two hard gates, all eight steps |
| 01 | **Welcome** — "Your memory, on rewind", filmstrip rewind motion |
| 02a | **Permissions** — Microphone denied, deep-link recovery |
| 02b | **Permissions** — System audio: Request again, live meter, never a green check |
| 03 | **Capture & Storage** — four rows; both hard gates rendered |
| 04 | **Your settings** — eight rows of name + one short value |
| 05 | **Change settings** — the one dense screen, four sections |
| 06 | **Setup** — non-blocking downloads, per-item states |
| 07a | **Voice** — model ready, take rejected |
| 07b | **Voice** — speaker model still downloading |
| 08a | **Finale** — capture running, first frame, first words |
| 08b | **Finale** — nothing granted, so nothing is recording |
| — | Bookend motion notes and alternates |

## Density rule (enforced by the design)

One line per row. One sentence per screen. No screen carries more than ~7 content lines
(excluding heading and action row). Provider names, model identifiers and per-row byte sizes
live **only** on *Change settings* — that screen is deliberately dense, and that is what lets
every other screen be light.

Plan of record: repo-root `PLAN.md`.
