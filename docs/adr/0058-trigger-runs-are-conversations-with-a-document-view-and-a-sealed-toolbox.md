# Trigger runs are conversations with a document view and a sealed toolbox

Status: accepted

A Trigger Run (docs/triggers/CONTEXT.md) persists as a normal conversation in the existing conversation store — `origin = trigger` plus trigger id/name on the row — not in a separate results system. The same Ask AI machinery runs the turn; the rail filters by origin; follow-ups continue the same conversation. Two contract points ride on this:

1. **Document view**: `origin = trigger` conversations render as a titled full-width markdown page (no question bubble) with the existing typed AnswerBlocks inline; the trigger preamble asks the model for a document, not chat. Follow-up turns render as normal chat beneath.
2. **Sealed toolbox**: trigger runs get read-only, inward-facing tools only (capture search, timeline, `recall_context`, past runs of the same trigger) — no web fetch, no MCP connectors, no app-control, no per-trigger model pin, unconditionally in v1. Triggers are shareable as pasted JSON and run unattended, so an outward-reaching tool would turn a pasted prompt into a standing exfiltration channel; sealing the toolbox makes that class of attack structurally impossible instead of review-dependent. Outward delivery (e.g. "post to Slack") must arrive as a Delivery option with its own consent design, never as an open outbound tool.

## Considered options

- **Dedicated trigger-results surface/table** — rejected: a second results system competing with Chat, duplicating rendering, search, and history for no user benefit.
- **Allowing connectors/web tools behind an advanced toggle** — rejected: the flagship use cases need none of them, and a toggle converts a structural guarantee into a skimmed-past warning.

## Consequences

- Storage split is deliberate: trigger *definitions* are config (`triggers.json` in the app config dir, plaintext — prompts included); the firing *ledger* (`trigger_firings`: outcome, reason, conversation link) and runs are data in the encrypted DB. The ledger references triggers by id string across the file/DB boundary — no FK.
- Deliveries are good-news-only: a macOS notification fires only for a completed run; skips and failures surface quietly as last-run status in the Triggers page.
