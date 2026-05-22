---
name: mnema-data
description: Access Mnema user activity data through the brokered mnema CLI. Use when the user asks an AI agent to search, summarize, reconstruct, audit, or open local Mnema activity from recordings, OCR text, captured frames, audio transcripts, speaker turns, timeline windows, or saved app data, especially for requests about user activity, Mnema recordings, brokered access, or agent access to Mnema.
---

# Mnema Data

Use this skill to answer user questions from Mnema's local personal record through the brokered `mnema` CLI. Treat the data as private user data: read only what is needed, summarize narrowly, and avoid dumping long derived text unless the user explicitly asks.

## Safety Rules

- Use the brokered `mnema` CLI as the agent contract. Do not query Mnema SQLite directly, inspect media paths, read raw frame/audio files, edit broker grant JSON, or call app-internal Tauri commands for agent data access.
- Keep access read-only. The supported commands are `auth status`, `search`, `show-text`, `timeline`, and `open-in-mnema`.
- Require user authorization. If the CLI returns `authorization_required`, tell the user to grant access in Mnema Settings, Privacy, Agent Access, then retry. Do not create or modify grants outside the Mnema app.
- Prefer search snippets and concise synthesis. Use `show-text` only for a specific `opaqueId` when the snippet is insufficient, and avoid pasting long OCR/transcript text unless requested.
- Use `open-in-mnema` when the user wants to inspect the original record. Do not open media files or export frame images yourself unless the user explicitly asks.
- Use project terms from `CONTEXT.md`: **Captured Frame**, **Audio Segment**, **Audio Transcription**, **Speaker Turn**, **Capture Session**, **Capture Segment**, and **Managed Storage Layout**.
- Remember that **Scrub Preview** is not source-of-truth. For exact inspection, open the broker result in Mnema rather than relying on preview cache artifacts.

## Quick Start

First check whether the installed CLI is available and authorized:

```bash
command -v mnema
mnema auth status
```

Typical queries:

```bash
mnema search --query "invoice" --limit 10
mnema search --query "standup" --from 2026-05-21T09:00:00+05:30 --to 2026-05-21T18:00:00+05:30 --limit 20
mnema timeline --from 2026-05-21T09:00:00+05:30 --to 2026-05-21T10:00:00+05:30 --limit 50
mnema show-text f2a
mnema open-in-mnema f2a
```

The CLI prints JSON. Preserve useful anchors such as `opaqueId`, `kind`, `startedAt`, and `endedAt` in notes, but cite them sparingly in final answers.

If `mnema` is not on `PATH` and you are working from the Mnema repo, use the development fallback:

```bash
cargo run -p app-infra --bin mnema-cli -- auth status
cargo run -p app-infra --bin mnema-cli -- search --query "invoice" --limit 10
```

Use the fallback only as a way to run the same brokered CLI during development. Do not replace it with direct database access.

## Workflow

1. Convert the user's time wording into concrete RFC3339 timestamps with timezone. If they say "today", use the current local date from the conversation.
2. Run `mnema auth status` before data queries when authorization is unknown.
3. Use `mnema search --query ...` for keyword and semantic reconstruction from broker-visible OCR/transcript search results. Add `--from`, `--to`, and `--limit` when the request implies a window.
4. Use `mnema timeline --from ... --to ...` for coarse activity intervals in a known window. In the current CLI this is mostly useful for audio activity intervals; combine it with search when reconstructing work.
5. Use `mnema show-text <opaqueId>` only after a search result needs more context.
6. Use `mnema open-in-mnema <opaqueId>` when the user asks to inspect the source in the app.
7. Answer with concise synthesized findings. Mention uncertainty when the broker returns only snippets, no hits, or a time-scoped grant limits the search.

## Helper Commands

- `mnema auth status`: report whether at least one active broker grant exists.
- `mnema search --query <text> [--from RFC3339] [--to RFC3339] [--limit n]`: search broker-visible redacted derived text and return snippets plus opaque result IDs.
- `mnema show-text <opaqueId>`: return broker-visible derived text for one result.
- `mnema timeline --from RFC3339 --to RFC3339 [--limit n]`: return broker-visible activity intervals for a bounded window.
- `mnema open-in-mnema <opaqueId>`: deep-link Mnema to one result.

## Output Guidance

- Normalize `<mark>` tags from snippets into plain emphasis or remove them in final prose.
- Do not expose config paths, grant file paths, raw database paths, or media paths in final answers unless directly relevant and requested.
- Cite timestamps and opaque IDs when they help the user verify a claim, for example `2026-05-21T09:42:10+05:30`, `frame f2a`, or `audio a17`.
- If a query is blocked by authorization, missing CLI installation, or an expired grant, stop and explain the exact next action.
