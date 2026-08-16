---
name: mnema-data
description: Access Mnema user activity data through the brokered mnema CLI. Use when the user asks an AI agent to search, summarize, reconstruct, audit, or open local Mnema activity from recordings, OCR text, captured frames, audio transcripts, speaker turns, timeline windows, or saved app data, especially for requests about user activity, Mnema recordings, brokered access, or agent access to Mnema.
---

# Mnema Data

Use this skill to answer user questions from Mnema's local personal record through the brokered `mnema` CLI. Treat the data as private user data: read only what is needed, summarize narrowly, and avoid dumping long derived text unless the user explicitly asks.

## Safety Rules

- Use the brokered `mnema` CLI as the agent contract. Do not query Mnema SQLite directly, inspect media paths, read raw frame/audio files, edit broker grant JSON, or call app-internal Tauri commands for agent data access.
- Keep data access read-only. The data commands are `search`, `timeline`, `speakers`, `show-text`, and `open`. Access-management commands live under `access`; run `access request` or `access revoke` **only when the user explicitly asks you to manage CLI Access**. Consent is theirs to give, not yours to arrange on their behalf.
- **Do not gate yourself on authorization — just run the data command.** Access to Mnema is a standing per-tool permission, not a session ticket. If this client has no permission yet, Mnema opens its own approval window, and the command continues by itself once the user answers. Never run `access request` first to "prepare" access, and never re-run it in a loop.
- If a command still fails, read `error.code` from the output envelope and act on that specific code — see **Failure Codes**. They are not interchangeable: telling the user "Mnema is unavailable" when the real answer is "finish Mnema onboarding" or "you blocked this tool" sends them to the wrong place. Never edit the access files yourself.
- Prefer search snippets and concise synthesis. Use `show-text` only for a specific signed opaque result ID returned by `search` when the snippet is insufficient, and avoid pasting long OCR/transcript text unless requested.
- Use `open` when the user wants to inspect the original record in Mnema. Do not open media files or export frame images yourself unless the user explicitly asks.
- You never see a result's raw captured browser URL — only the guarded `context.url` (a sanitized host+path). There is no agent or CLI command that opens a captured URL: the broker never opens one. Revisiting the original page in a browser is a user-only action inside the Mnema app, so if the user wants that, tell them to use the open-in-browser button in Mnema rather than attempting it yourself.
- Use project terms from `CONTEXT.md`: **Captured Frame**, **Audio Segment**, **Audio Transcription**, **Speaker Turn**, **Capture Session**, **Capture Segment**, and **Managed Storage Layout**.
- Remember that **Scrub Preview** is not source-of-truth. For exact inspection, open the broker result in Mnema rather than relying on preview cache artifacts.

## Quick Start

Check that the CLI is installed, then go straight to the data command — there is no authorization preamble:

```bash
command -v mnema
mnema access known-clients
```

`mnema access known-clients` is the CLI-owned source of truth for agent harnesses it can auto-detect. If the current agent is listed, do not pass `--client`; let the CLI infer it. If the current agent is not listed, use `--client <name>` or `MNEMA_CLI_CLIENT=<name>` for that session. Whichever identity the first command resolves, keep using it — the permission is keyed to it, and switching labels mid-session asks the user to approve a second tool.

`mnema access status` prints the resolved client label and identity source, plus this client's standing permission if it has one. It is a diagnostic, not a gate: run it when you need to know which identity you resolved to (`inferred` means auto-detect worked; `mnema CLI (defaulted)` means the current process was not recognized) or to explain the current state to the user. Do not run it before every data command.

The first data command from an unapproved client opens Mnema's approval window and waits up to 120 seconds for the user. Say so if you are about to block on it — the user has to look at their screen.

Typical queries:

```bash
mnema --format toon search --query "invoice" --limit 10
mnema search --query "invoice" --limit 10
mnema search --query "standup" --from 2026-05-21T09:00:00+05:30 --to 2026-05-21T18:00:00+05:30 --limit 20
mnema search --query "roadmap" --app Linear --window-title "Grooming" --limit 10
mnema search --query "review" --url github.com --limit 10
mnema search --query "invoice" --limit 100 --cursor v1:4821:100:0
mnema search --query "review" --url-regex '(?i)^github\.com/[^/]+/[^/]+/pull/' --limit 10
mnema timeline --from 2026-05-21T09:00:00+05:30 --to 2026-05-21T10:00:00+05:30 --limit 50
mnema timeline --from 2026-05-21T09:00:00+05:30 --to 2026-05-21T10:00:00+05:30 --app Linear --window-title "Grooming" --limit 50
mnema speakers --limit 20
mnema speakers --name skywalker
mnema search --query "roadmap" --speaker '<handle-from-speakers>' --limit 10
mnema timeline --from 2026-05-21T09:00:00+05:30 --to 2026-05-21T18:00:00+05:30 --speaker '<handle-from-speakers>'
mnema show-text '<id-from-search>'
mnema open '<id-from-search>'
```

Data commands print JSON by default, but agents should prefer `--format toon` over JSON for compact structured output. Preserve useful anchors such as result `id`, `kind`, `startedAt`, `endedAt`, and allowlisted `context` in notes, but cite them sparingly in final answers.

When the user asks to pre-authorize a tool from the terminal (only then), `access request` opens the same approval window without running a query:

```bash
mnema access request --scope last-day
mnema access request --scope last-7-days
mnema access request --scope all-retained
```

Supported scopes are `last-day`, `last-7-days`, and `all-retained`. There is no `--duration`: a permission stands until the user blocks it in Mnema, and lapses on its own 30 days after its last use. A data command asks for exactly the scope its `--from` needs, so an unadorned query asks for `last-day` and a query reaching back three weeks asks for `all-retained`.

If `mnema` is not on `PATH` and you are working from the Mnema repo, use the development fallback:

```bash
cargo run -p cli -- access status
cargo run -p cli -- search --query "invoice" --limit 10
```

Use the fallback only as a way to run the same brokered CLI during development. Do not replace it with direct database access.

The bundled sidecar binary is named `mnema-cli`, but the user-facing installed command is `mnema`. The Mnema app installs it from Settings, Access.

## Workflow

1. Convert the user's time wording into concrete RFC3339 timestamps **in the user's local timezone**. "Today" means local midnight to now in that timezone (for example `2026-05-26T00:00:00+05:30` to now), which maps to a UTC range that begins on the *previous* UTC calendar date for east-of-UTC offsets — do not assume the local date equals the UTC date, or you will silently drop early-morning local activity. Apply the same care to "this morning", "last night", "yesterday", and other day-relative wording.
2. Run `mnema access known-clients` when deciding whether the current agent should rely on auto-detect or pass `--client`. Use the same identity for every command in the session.
3. Run the data command you need. Do not check or request access first — if this client has no permission, Mnema opens the approval window and the command continues once the user answers.
4. Check `scopeClamped` on every `search` and `timeline` response before you read the results (see **Scope Clamping**). A clamped page covers less than you asked for, and reporting it as the whole window is the worst thing this skill can do.
5. On a failure, read `error.code` and follow **Failure Codes** — never collapse them into one "Mnema is unavailable" message, and never retry a code marked not retryable.
6. Use `mnema search --query ...` for keyword and semantic reconstruction from broker-visible OCR/transcript search results. Add `--from`, `--to`, `--limit`, `--app`, `--window-title`, or a URL filter when the request implies a time window or screen context. `--app` matches app bundle ID or app name; `--window-title` is a case-insensitive substring filter. Context-filtered search is frame-only.
7. Use `mnema timeline --from ... --to ...` for coarse activity intervals in a known window. Without context filters, timeline returns broker-visible audio activity intervals. With `--app`, `--window-title`, or a URL filter, timeline returns matching screen intervals from broker-visible searchable frame projections.
8. Filter by site with `--url <substring>` (case-insensitive) or `--url-regex <pattern>` (case-sensitive; prefix `(?i)` to ignore case). The two are mutually exclusive. **Both match only the sanitized `host[:port]/path` form**: query strings and fragments are never indexed, so a filter containing `?` or `#` matches nothing, and high-entropy path segments may appear redacted. Filter on host and readable path segments, never on query parameters. Prefer `--url` unless the pattern genuinely needs alternation or anchoring.
9. For "what did *someone* say" questions, use the two-call workflow: `mnema speakers` (add `--name <fragment>` when the user named a person) to get that person's `handle`, then ONE `mnema search --query <text> --speaker <handle>` or `mnema timeline --from ... --to ... --speaker <handle>` — the filtered response already carries their words in `turns`, so no `show-text` follow-up per result is needed. `search` still requires `--query`, so a question with no keyword in it ("what did Priya say yesterday", "when was Priya talking") is answerable only through `timeline --speaker`; do not invent a keyword to force it through `search`. A speaker filter cannot be combined with `--app`, `--window-title`, `--url`, or `--url-regex`; "what did Priya say in Zoom" is not answerable in one call, because the app is recorded on screen frames and the voice on audio and nothing joins them. Ask for the speaker first, then search the screen filters over the times that come back.
10. Use `mnema show-text <resultId>` only after a search result needs more context.
11. Use `mnema open <resultId>` when the user asks to inspect the source in the app.
12. Answer with concise synthesized findings. Mention uncertainty when the broker returns only snippets, no hits, or `scopeClamped` says the window you searched was narrower than the one you were asked about.

## Helper Commands

- `mnema access status [--all-clients]`: print the resolved client identity and this client's standing permission — scope, when it was last used, and whether it is blocked. Human-readable only; access commands reject `--format`. There is no expiry to report.
- `mnema access known-clients`: report the CLI-owned list of agent client labels that can be auto-detected from known harness markers.
- `mnema access request [--scope last-day|last-7-days|all-retained]`: open Mnema's approval window without running a query. A human pre-authorization step — run it only when the user asks, never as a preamble to your own data command. Defaults to `last-day`. No `--duration`; permissions do not expire on a clock.
- `mnema access revoke <client>`: **block** that client's standing access when the user asks. It stays blocked — further commands from it are denied without any window opening — until the user re-enables it in Mnema under Settings, Data, Access. There is no CLI unblock.
- `mnema search --query <text> [--from RFC3339] [--to RFC3339] [--limit n] [--app appOrBundleId] [--window-title text] [--url text | --url-regex pattern] [--speaker handle] [--cursor nextCursor]`: search broker-visible redacted derived text and return snippets plus signed opaque result IDs. Context filters apply to screen results; `--app` matches bundle ID or app name, `--window-title` is a case-insensitive substring, and the URL filters match the sanitized `host[:port]/path` form only (see step 8). `--speaker` narrows to audio and cannot be combined with any of them.
- `mnema show-text <resultId>`: return broker-visible derived text for one result.
- `mnema timeline --from RFC3339 --to RFC3339 [--limit n] [--app appOrBundleId] [--window-title text] [--url text | --url-regex pattern] [--speaker handle]`: return broker-visible activity intervals for a bounded window. Without context filters this is audio-oriented; with any of them it returns matching screen intervals. `--speaker` narrows to audio and cannot be combined with them.
- `mnema speakers [--name fragment] [--limit n]`: list who was heard inside the active grant's time scope, longest-speaking first, each with the opaque `handle` that `--speaker` takes. `--name` is a case-insensitive substring over named people only; use it when a person is quiet enough to rank below the limit. This is the only way to obtain a speaker handle apart from a `show-text` speaker.
- `mnema open <resultId>`: open Mnema to one result.

Global options:

- `--client <name>` sets the broker client identity. Prefer the CLI's auto-detected identity when the current agent appears in `mnema access known-clients` and `mnema access status` reports an inferred client. Use `--client` only when the current agent is not listed, status falls back to `mnema CLI (defaulted)`, or the user explicitly asks for a different client label. `MNEMA_CLI_CLIENT` and `AI_AGENT` can also supply the identity.
- `--format json|yaml|toon` changes output format for data commands only. Agents should prefer `--format toon` unless JSON/YAML is explicitly needed for tooling. Access commands reject `--format`.
- `--no-prompt` stops a command from opening or waiting on Mnema's approval window. Use it only when the user has asked you not to interrupt them: without a permission the command fails `authorization_required` instead of asking, and a request that would have been narrowed comes back clamped (with `scopeClamped` set) instead of prompting to widen. Normal agent data access should leave it off — the window is how the user gives consent.

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

For `search`, `data.results[]` contains `id`, `kind`, `snippet`, `startedAt`, `endedAt`, and optional `context`. An audio result also carries `spanStartMs`/`spanEndMs` — where inside that recording the match actually falls, in ms from its start. A recording runs up to five minutes, so `startedAt`/`endedAt` on an audio result already bound the matched span, NOT the whole recording: the broker has already added `spanStartMs` for you. Cite `startedAt` as-is and never add `spanStartMs` to it — doing so double-counts the offset and cites a moment that can be minutes late. The span fields are for locating the match inside the recording (a player offset, or the input to a second call), not for timestamp arithmetic. They are absent on screen results, which have no sub-segment span. They are also media-relative while `startedAt`/`endedAt` are wall-clock, so a span may end a few hundred ms past `endedAt` − `startedAt`; that is clock skew, not an error. For `timeline`, `data.intervals[]` contains `kind`, `startedAt`, `endedAt`, an optional `opaqueId`, and optional `context`; pass `opaqueId` straight to `show-text` to read what an interval covers. It is omitted, never null, when a screen interval has no representative frame to point at. `search`, `timeline`, and `show-text` all report the same three kinds: `frame` for captured screen text, `audio_microphone` for the microphone, and `audio_system` for sound that played through the speakers. Screen `context` is allowlisted to `appBundleId`, `appName`, `windowTitle`, and an optional guarded `url`; it does not include raw paths or raw metadata snapshots. The `url` is a sanitized host+path only — the broker strips the query string and fragment and redacts secret/token-shaped path segments before returning it, so it is safe to read but is **not** the raw captured URL. It appears only on screen results/intervals (audio carries no URL) and only when the underlying frame actually captured a URL. For `show-text`, `data.text` contains broker-visible derived text, and audio results also carry `data.speakers[]` — one entry per person heard, ordered by first turn, each with `attribution` (`assigned` when the user said who this voice is, `recognized` when the voice was matched automatically, `unknown` when it was not), an optional `name` (absent for `unknown`), and an optional `confidence` (`high`/`medium`/`low`, only on `recognized`). It is omitted for frames and for audio with no speaker analysis. Every speaker also carries a `handle` (`{ id, kind }`, plus `startMs`/`endMs` on a `voice` handle) — the value `--speaker` takes. Audio `show-text` may also carry `data.turns[]`, each turn being `speaker` (an index into `data.speakers[]`), `startMs`, `endMs`, and `text`; it is an attribution overlay on `data.text`, not a decomposition of it, so it may cover only part of the recording. For `speakers`, `data.speakers[]` contains `name` (absent for an unnamed voice), `handle`, `speakingMs`, `assignedTurns`, and `recognizedTurns`, alongside `data.limit` and `data.truncated`. On a `--speaker` search or timeline, each returned result/interval carries `turns[]` holding only that speaker's words (no `speaker` index — every turn is theirs), and the response carries `data.speakerCoverage` with `recordingsWithUnnamedVoices` and `recordingsWithoutSpeakerData`. For `open`, `data.opened` reports whether Mnema was opened.

`search` and `timeline` responses also carry `data.scopeClamped` (always present) and `data.requiredScope` (only when clamped) — see **Scope Clamping**.

## Scope Clamping

`data.scopeClamped: true` means **this client's permission does not reach as far back as your `--from` asked, so the results cover less time than the question did.** The page is real data, not an error — but a thin page under a clamp is the one thing you must never read as "nothing happened then".

Normally you will not see it: a clamp makes the CLI open the approval window asking to widen the permission, then re-run the query on the wider scope. It reaches you mainly when you passed `--no-prompt`, which trades the prompt for a marked-but-incomplete page. (If the user approves something narrower than the command needs, you get `scope_not_granted` instead — the CLI refuses to retry into an incomplete answer.)

When it is set:

- Never report the requested window as empty, quiet, or covered. Say which slice you could actually see.
- `data.requiredScope` names the scope that would have covered it, in wire spelling: `lastDay`, `last7Days`, or `allRetained`. Tell the user to widen the permission to that scope — in Mnema under Settings, Data, Access, or by running `mnema access request --scope <last-day|last-7-days|all-retained>` themselves.
- Do not silently retry with a narrower `--from` and present that as the answer.

## Failure Codes

A failed data command still prints the envelope, with `data: null` and an `error` object carrying `code`, `message`, and `retryable`. Each code has its own exit status and its own next action. Do not collapse them.

| `error.code` | Exit | What happened, and what to do |
| --- | --- | --- |
| `usage` | 2 | Bad arguments. Fix the command. |
| `authorization_required` | 10 | This client has no access and nothing asked for it — normally because you passed `--no-prompt`. Tell the user; do not loop. |
| `authorization_denied` | 10 | The user saw the request and said no. **Not retryable — a denial is an answer.** Stop and report it. Do not re-run to "try again". |
| `authorization_timeout` | 11 | The approval window went unanswered for 120 seconds. Tell the user it is waiting for them; retry only if they ask. |
| `app_unavailable` | 12 | Mnema is not running or not reachable. Ask the user to open Mnema. |
| `outside_grant_scope` | 13 | The requested window, or that result id, falls entirely outside what this client may read. Narrow the window, or ask the user to widen the permission. |
| `authorization_window_closed` | 14 | The approval window was closed without a decision. Retryable — but ask the user first. |
| `access_blocked` | 15 | The user has **blocked** this tool. It stays blocked; no window will open. **Only they can lift it**, in Mnema under Settings, Data, Access. Never retry, and never try another `--client` label to get around it. |
| `authorization_busy` | 16 | Mnema is already showing a different approval. Ask the user to answer that one, then retry. Mnema is running — do not report it as unavailable. |
| `onboarding_required` | 17 | Mnema's setup is unfinished. Tell the user to open Mnema and complete onboarding. **This is not "Mnema is unavailable"** — reporting it that way sends them to the wrong place. |
| `authorization_invalid_request` | 18 | Mnema rejected the request as malformed. Report it; not something you can fix by retrying. |
| `authorization_unsupported_version` | 19 | The `mnema` CLI and the Mnema app are on different releases. Tell the user to update both. |
| `broker_operation_failed` | 20 | The query itself failed inside Mnema. Report the message. |
| `output_serialization_failed` | 21 | The CLI could not render the output. Try another `--format`. |
| `scope_not_granted` | 22 | Approval came back **narrower** than this command needs, so it was not retried against an incomplete window. The message names the scope granted and the one needed; ask the user to widen it rather than re-running. |

## Output Guidance

- Normalize `<mark>` tags from snippets into plain emphasis or remove them in final prose.
- Treat `context.appName`, `context.appBundleId`, `context.windowTitle`, and `context.url` as broker-visible search context. Use them to disambiguate results, but avoid over-reporting window titles when they are not relevant to the user's question. `context.url` is a guarded host+path (query/fragment stripped, secrets/tokens redacted), not the raw captured URL; cite it as a hint about where the user was, and never present it as a clickable or complete link.
- A result `id` names the RECORDING, not the match, so the same `id` can appear more than once in one page — a recording answers the query at two moments far enough apart that the broker kept them separate, each with its own `spanStartMs`/`spanEndMs`, snippet, and `turns`. Treat them as distinct findings. If you deduplicate results, key on (`id`, `spanStartMs`); collapsing on `id` alone silently discards a real match and can make a recording look like it mentioned something once when it did so repeatedly.
- Treat `data.speakers[]` as the only names you may attach to audio. Never invent, guess, or infer a name for an `unknown` speaker — say "an unidentified speaker". Report a `recognized` name as a match the app made rather than a fact ("recognized as Priya, medium confidence"); only `assigned` names are user-confirmed. The list has one entry per person, so do not describe it as a participant count when unnamed voices are present: one person's voice can appear as several `unknown` entries.
- **An absent `turns` means the words could NOT be attributed to anyone — it never means nobody spoke.** Speaker detection produces nothing at all for plenty of audio the transcriber handled fine, so a recording with words in `text` and no `turns` is normal and is not silence. Never report such a recording as empty, as nobody speaking, or as no one being present: read `text` and say who spoke is unknown. The same rule applies to a `--speaker` result whose `turns` are missing.
- A `handle` is how you address a person, and its `kind` decides what it means. A `person` handle is one human: stable across recordings and channels, and it survives a rename. A `voice` handle is ONE voice inside ONE capture session and is **not a person** — a session is a continuous sitting, and recordings are capped at 5 minutes, so filtering on a single `voice` handle returns **every consecutive recording in that sitting**, not one. Across two sittings the same human gets two unrelated `voice` handles, often several within one, and the handle dies when that session is re-analyzed. Its `startMs`/`endMs` describe only the recording it was published for, never how far it reaches — do not report that span as how long the voice was heard. Never store a `voice` handle for later, never merge two of them, and never present one as an identity. A saved view, scheduled digest, or trigger holding a `voice` handle will silently return nothing.
- Read `data.speakerCoverage` on every `--speaker` answer and say what it implies. `recordingsWithUnnamedVoices` counts recordings in range holding a voice nobody has named — any of them could be the person asked about, and this is fixable: tell the user labeling that voice in Mnema brings the recording into reach. `recordingsWithoutSpeakerData` counts recordings where speaker detection produced nothing at all — no filter can ever reach that audio, and the user cannot fix it. Either count being non-zero makes the answer partial; present it as "here is what I could attribute", not as everything the person said.
- The speaker filter matches voices the user assigned AND voices the app merely recognized, so results can include guesses the user never confirmed. `assignedTurns` versus `recognizedTurns` in the `speakers` output says how much of a handle's identity is confirmed; when a handle is mostly recognized, say that the match is the app's, not the user's.
- `data.truncated` from `speakers` means the ranked list is **not** everyone heard. Do not read it as the full roster — narrow with `--name` instead.
- `audio_system` is sound the user's speakers played — a video, a call's far side, a podcast — **not** the user speaking. Never quote it as something the user said; attribute it to what was playing ("a video the user was watching said …"). Only `audio_microphone` carries the room the user was in, and even there `data.speakers[]` decides who spoke.
- Do not expose config paths, grant file paths, raw database paths, or media paths in final answers unless directly relevant and requested.
- Result `startedAt` / `endedAt` are UTC (`Z`-suffixed). Convert them to the user's local timezone before describing time-of-day or reasoning about which record is "first", "earliest", "latest", "morning", or "evening"; the raw UTC clock can fall on a different local date.
- `search` results are one relevance-ranked list, best match first, mixing screen and audio hits by score — a page may legitimately be all screen or all audio. Ranked order is NOT chronological, and `timeline` intervals are not either. For "first / earliest / last / latest" requests, sort the candidates by `startedAt` and pick the extreme — never assume the first item in the response is the earliest.
- **`--limit` is clamped to 100 — page with `--cursor`, do not raise the limit.** `data.limit` is the page-size *ceiling* that was applied, not a count. On `search`, `data.nextCursor` is present whenever more matches remain: re-run the identical query and filters with `--cursor <value>`, and repeat until `nextCursor` is absent. **A short page is not the end of the results** — screen and audio matches are each capped at 50 per page, so a 50-row response to `--limit 100` is normal and will still carry a cursor. `nextCursor`, never the row count, decides whether you have seen everything. On `timeline` there is no cursor — `data.truncated: true` means intervals were dropped, and the only way past it is a narrower `--from`/`--to` window. Because results come back newest-first, a truncated timeline biases toward the recent end, which is exactly wrong for "earliest / first" questions.
- Cite timestamps and opaque IDs when they help the user verify a claim, for example `2026-05-21T09:42:10+05:30`, `frame <id>`, or `audio_microphone <id>`.
- If a query is blocked by authorization, a missing CLI installation, or a blocked or lapsed permission, stop and explain the exact next action — the one that matches `error.code`, not a generic "Mnema is unavailable".
