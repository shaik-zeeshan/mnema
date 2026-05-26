---
name: mnema-data
description: Access Mnema user activity data through the brokered mnema CLI. Use when the user asks an AI agent to search, summarize, reconstruct, audit, or open local Mnema activity from recordings, OCR text, captured frames, audio transcripts, speaker turns, timeline windows, or saved app data, especially for requests about user activity, Mnema recordings, brokered access, or agent access to Mnema.
---

# Mnema Data

Use this skill to answer user questions from Mnema's local personal record through the brokered `mnema` CLI. Treat the data as private user data: read only what is needed, summarize narrowly, and avoid dumping long derived text unless the user explicitly asks.

## Safety Rules

- Use the brokered `mnema` CLI as the agent contract. Do not query Mnema SQLite directly, inspect media paths, read raw frame/audio files, edit broker grant JSON, or call app-internal Tauri commands for agent data access.
- Keep data access read-only. The data commands are `search`, `timeline`, `show-text`, and `open`. Access-management commands live under `access`; use `access request`, `access status`, `access revoke`, or `access revoke-client` only when the user asks to manage CLI Access.
- Require user authorization through Mnema. Before any data command, check `mnema access status`; if there is no active grant for this client, run `mnema access request --scope last-day --duration 24h` and wait for approval before continuing. If the CLI returns `authorization_required`, `authorization_timeout`, `authorization_denied`, or `app_unavailable`, stop and tell the user the exact Mnema approval action needed. Do not create or modify grants outside the Mnema app.
- Prefer search snippets and concise synthesis. Use `show-text` only for a specific signed opaque result ID returned by `search` when the snippet is insufficient, and avoid pasting long OCR/transcript text unless requested.
- Use `open` when the user wants to inspect the original record in Mnema. Do not open media files or export frame images yourself unless the user explicitly asks.
- Use project terms from `CONTEXT.md`: **Captured Frame**, **Audio Segment**, **Audio Transcription**, **Speaker Turn**, **Capture Session**, **Capture Segment**, and **Managed Storage Layout**.
- Remember that **Scrub Preview** is not source-of-truth. For exact inspection, open the broker result in Mnema rather than relying on preview cache artifacts.

## Quick Start

First check whether the installed CLI is available and has an active grant for this client:

```bash
command -v mnema
mnema access known-clients
mnema access status
```

`mnema access known-clients` is the CLI-owned source of truth for agent harnesses it can auto-detect. If the current agent is listed, do not pass `--client`; let the CLI infer it. If the current agent is not listed, use `--client <name>` or `MNEMA_CLI_CLIENT=<name>` for that session.

`mnema access status` prints the resolved client label and identity source. Use it as a runtime check: `inferred` means auto-detect worked, while `mnema CLI (defaulted)` means the current process was not recognized.

If status reports an active grant for an inferred client, data commands must use that same inferred identity. For example, if status prints `Client: OpenCode (inferred)` and `1 active grant(s)`, run `mnema search ...`; do not override it with `mnema --client "Codex" ...` or `MNEMA_CLI_CLIENT="Codex"`.

If status reports `0 active grant(s)`, request a grant before searching, reading text, opening a result, or querying the timeline:

```bash
mnema access request --scope last-day --duration 24h
mnema access status
```

Typical queries:

```bash
mnema --format toon search --query "invoice" --limit 10
mnema search --query "invoice" --limit 10
mnema search --query "standup" --from 2026-05-21T09:00:00+05:30 --to 2026-05-21T18:00:00+05:30 --limit 20
mnema search --query "roadmap" --app Linear --window-title "Grooming" --limit 10
mnema timeline --from 2026-05-21T09:00:00+05:30 --to 2026-05-21T10:00:00+05:30 --limit 50
mnema timeline --from 2026-05-21T09:00:00+05:30 --to 2026-05-21T10:00:00+05:30 --app Linear --window-title "Grooming" --limit 50
mnema show-text '<id-from-search>'
mnema open '<id-from-search>'
```

Data commands print JSON by default, but agents should prefer `--format toon` over JSON for compact structured output. Preserve useful anchors such as result `id`, `kind`, `startedAt`, `endedAt`, and allowlisted `context` in notes, but cite them sparingly in final answers.

For non-default grants, request approval through the app:

```bash
mnema access request --scope last-day --duration 24h
mnema access request --scope all-retained --duration 7d
```

Supported request scopes are `last-day` and `all-retained`; supported durations are `1h`, `24h`, and `7d`. The default interactive data-command prompt requests last-day access for 24 hours.

If `mnema` is not on `PATH` and you are working from the Mnema repo, use the development fallback:

```bash
cargo run -p cli -- access status
cargo run -p cli -- search --query "invoice" --limit 10
```

Use the fallback only as a way to run the same brokered CLI during development. Do not replace it with direct database access.

The bundled sidecar binary is named `mnema-cli`, but the user-facing installed command is `mnema`. The Mnema app installs it from Settings, Access.

## Workflow

1. Convert the user's time wording into concrete RFC3339 timestamps **in the user's local timezone**. "Today" means local midnight to now in that timezone (for example `2026-05-26T00:00:00+05:30` to now), which maps to a UTC range that begins on the *previous* UTC calendar date for east-of-UTC offsets — do not assume the local date equals the UTC date, or you will silently drop early-morning local activity. Apply the same care to "this morning", "last night", "yesterday", and other day-relative wording.
2. Run `mnema access known-clients` when deciding whether the current agent should rely on auto-detect or pass `--client`.
3. Run `mnema access status` before any data query.
4. If status reports an inferred active grant, run data commands without `--client` so they use the same inferred identity.
5. If there are `0 active grant(s)` for this client, run `mnema access request --scope last-day --duration 24h`, wait for approval, then rerun `mnema access status`. Do not run `search`, `timeline`, `show-text`, or `open` until an active grant exists.
6. Use `mnema search --query ...` for keyword and semantic reconstruction from broker-visible OCR/transcript search results. Add `--from`, `--to`, `--limit`, `--app`, or `--window-title` when the request implies a time window or screen context. `--app` matches app bundle ID or app name; `--window-title` is a case-insensitive substring filter. App/window-filtered search is frame-only.
7. Use `mnema timeline --from ... --to ...` for coarse activity intervals in a known window. Without app/window filters, timeline returns broker-visible audio activity intervals. With `--app` or `--window-title`, timeline returns matching screen intervals from broker-visible searchable frame projections.
8. Use `mnema show-text <resultId>` only after a search result needs more context.
9. Use `mnema open <resultId>` when the user asks to inspect the source in the app.
10. Answer with concise synthesized findings. Mention uncertainty when the broker returns only snippets, no hits, or a time-scoped grant limits the search.

## Helper Commands

- `mnema access status [--all-clients]`: report active CLI Access grants. This command is human-readable.
- `mnema access known-clients`: report the CLI-owned list of agent client labels that can be auto-detected from known harness markers.
- `mnema access request [--scope last-day|all-retained] [--duration 1h|24h|7d]`: ask Mnema for a grant through the app-owned authorization channel.
- `mnema access revoke <grantId>`: revoke one grant when the user asks.
- `mnema access revoke-client <clientName> --yes`: revoke active grants for one client when the user asks.
- `mnema search --query <text> [--from RFC3339] [--to RFC3339] [--limit n] [--app appOrBundleId] [--window-title text]`: search broker-visible redacted derived text and return snippets plus signed opaque result IDs. App/window filters apply to screen results; `--app` matches bundle ID or app name, and `--window-title` is a case-insensitive substring.
- `mnema show-text <resultId>`: return broker-visible derived text for one result.
- `mnema timeline --from RFC3339 --to RFC3339 [--limit n] [--app appOrBundleId] [--window-title text]`: return broker-visible activity intervals for a bounded window. Without app/window filters this is audio-oriented; with either filter it returns matching screen intervals.
- `mnema open <resultId>`: open Mnema to one result.

Global options:

- `--client <name>` sets the broker client identity. Prefer the CLI's auto-detected identity when the current agent appears in `mnema access known-clients` and `mnema access status` reports an inferred client. Use `--client` only when the current agent is not listed, status falls back to `mnema CLI (defaulted)`, or the user explicitly asks for a different client label. `MNEMA_CLI_CLIENT` and `AI_AGENT` can also supply the identity.
- `--format json|yaml|toon` changes output format for data commands only. Agents should prefer `--format toon` unless JSON/YAML is explicitly needed for tooling. Access commands reject `--format`.
- `--no-prompt` prevents data commands and `access request` from launching or waiting for Mnema approval. Do not use it for normal agent data access because agents must request a grant before data commands when no active grant exists.

Current CLI aliases that are no longer valid:

- Use `access status`, not `auth status`.
- Use `open`, not `open-in-mnema`.

## Output Shape

Data command output is an envelope:

```json
{
  "schemaVersion": 1,
  "command": "search",
  "client": { "label": "<detected-client>", "source": "inferred" },
  "data": {},
  "error": null
}
```

For `search`, `data.results[]` contains `id`, `kind`, `snippet`, `startedAt`, `endedAt`, and optional `context`; current kinds map to `screenText` and `audioTranscript`. For `timeline`, `data.intervals[]` contains `kind`, `startedAt`, `endedAt`, `summary`, and optional `context`. Screen `context` is allowlisted to `appBundleId`, `appName`, and `windowTitle`; it does not include browser URL, paths, or raw metadata snapshots. For `show-text`, `data.text` contains broker-visible derived text. For `open`, `data.opened` reports whether Mnema was opened.

Structured error codes include `authorization_required`, `authorization_timeout`, `authorization_denied`, `app_unavailable`, `outside_grant_scope`, and `broker_operation_failed`.

## Output Guidance

- Normalize `<mark>` tags from snippets into plain emphasis or remove them in final prose.
- Treat `context.appName`, `context.appBundleId`, and `context.windowTitle` as broker-visible search context. Use them to disambiguate results, but avoid over-reporting window titles when they are not relevant to the user's question.
- Do not expose config paths, grant file paths, raw database paths, or media paths in final answers unless directly relevant and requested.
- Result `startedAt` / `endedAt` are UTC (`Z`-suffixed). Convert them to the user's local timezone before describing time-of-day or reasoning about which record is "first", "earliest", "latest", "morning", or "evening"; the raw UTC clock can fall on a different local date.
- `search` and `timeline` results are not guaranteed to be in chronological order. For "first / earliest / last / latest" requests, sort the candidate results by `startedAt` and pick the extreme — never assume the first item in the response is the earliest. Widen the time window and raise `--limit` (and page with `nextCursor` when present) so the true earliest/latest is not cut off before you conclude.
- Cite timestamps and opaque IDs when they help the user verify a claim, for example `2026-05-21T09:42:10+05:30`, `screenText <id>`, or `audioTranscript <id>`.
- If a query is blocked by authorization, missing CLI installation, or an expired grant, stop and explain the exact next action.
