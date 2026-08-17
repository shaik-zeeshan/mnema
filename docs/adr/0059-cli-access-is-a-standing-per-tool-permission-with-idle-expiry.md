# CLI Access is a standing per-tool permission with idle expiry

## Status

Accepted (2026-08-17). Supersedes the consent model of
[ADR 0014](0014-use-app-mediated-cli-access-authorization-channel.md) — the
app-owned Broker Authorization Channel survives, the 24-hour reusable grant does
not. Amends the grant shape of
[ADR 0012](0012-encrypted-capture-index-and-brokered-access.md). Leaves
[ADR 0041](0041-single-writer-owner-and-read-only-brokered-readers.md)'s
in-process brokered reader intact.

## Context

ADR 0014 made a CLI Access Grant a time-boxed ticket: an identity-scoped,
reusable grant for last-day redacted history, 24 hours, minted by a native
prompt. In practice the ticket model produced four compounding failures.

**Grants accumulate without bound.** `broker-grants.json` is append-only. There
is no GC for expired or revoked rows, and the whole file is parsed on every
broker call (`execute_for_identity`, `auth_status_for_config`,
`authorize_active_opaque_capture_reference`). `mnema access request` never
checks for an existing active grant, so every agent session that skips the
documented `access status` preamble mints another. Scopes union rather than
replace, so a `lastDay` and an `allRetained` row for one tool coexist and the
widest wins — a security property nobody can reason about across fourteen
overlapping rows. Settings renders the entire list, forever.

**Headless agents cannot reach the prompt at all.**
`can_prompt_for_authorization` requires stdin, stdout, *and* stderr to all be
TTYs. Every agent harness pipes stdout, so the just-in-time approval never
fires and the agent gets `authorization_required` with no path forward. The MCP
path already contradicts this, passing `allow_prompt = true` unconditionally
because "the user is present."

**The prompt grants something other than what it says.** Quick Allow always
minted `lastDay`/24h regardless of the request, and the CLI drops the `grant`
field from the channel response — so `access request --scope all-retained
--duration 7d` prints "approved" and the caller never learns it was downgraded.
An entire disclosure paragraph existed only to apologise for this.

**Expiry destroys the agent's working set.** Opaque ids are HMAC-signed against
the issuing grant id. When that grant expires, every id already handed to the
agent fails re-authorization, mid-task, even when a fresh grant covers the same
window.

Underneath all four is one mistake: consent was modelled as a disposable
session ticket when what the user actually wants to express is a standing
decision about a tool. The daily dialog that model requires does not buy
security — a prompt seen every day is a prompt clicked reflexively.

## Decision

**A CLI Access permission is per-tool and standing.** One row per normalized
Broker Client Identity. No calendar expiry. The row dies 30 days after its last
use.

- **Idle expiry, not TTL.** `last_used_at_unix_ms` is stamped coarsely —
  rewritten only when the stored value is more than an hour stale — so a read
  path stays a read path under ADR 0041 rather than paying a flocked file
  rewrite per brokered read. One-hour resolution against a 30-day threshold.
- **Scope is `lastDay | last7Days | allRetained` on the row.** A tool needing
  more **upgrades the row in place, preserving its id**, because opaque ids are
  signed against the issuing grant id and a new id would kill the agent's
  working set. Grants no longer union: `effective_scope_start`, `scope_class`,
  and `opaque_issuing_grant` collapse from set-reductions to single-row reads.
- **Idle-expired and revoked are different states.** Idle expiry is benign
  disuse: the row is deleted and the next call prompts fresh. Revocation is a
  standing rejection: the row becomes **blocked** — visible in Settings, denied
  *without* prompting, never idle-expiring, one click to re-enable. A rejection
  the user must re-issue every time the tool runs is not a rejection.

**Approval always opens the CLI Access Request window.** The native message box
is removed, along with the fixed quick-approval policy and its disclosure
paragraph. Approval fires a few times in a tool's life, so a fast path buys
nothing and costs the two things only a real window can carry: the identity
**provenance chip** (explicit / env / inferred / defaulted, warn-tinted when
weak) and the anti-reflex affordances — focus lands on Deny, Enter is not bound
to approve, Esc denies. The window is scope-only; idle expiry leaves no
duration to choose.

**Consent is the user's business, not the agent's.** Agents run data commands
directly. The TTY gate is deleted — it inspected the caller's file descriptors
to decide whether a human was at the Mac, which they carry no information
about; `--no-prompt` is the explicit control, and an unanswered window already
dies on the channel read timeout. `access request` survives as a human
pre-authorization, and the agent-facing skill's `status` → `request` → `status`
preamble is deleted.

**Narrowing is never silent.** `scoped_date_range` clamping a requested range
to the permission's scope and returning success is a correctness bug: for a
recall product, a confidently incomplete answer is the most damaging failure
mode there is. The broker marks a clamped range, the CLI derives the scope a
command actually needs and sends it as the channel `minimum`, and a clamp
triggers the widen prompt. The CLI also reads the `grant` field it currently
drops, verifies it covers the request, and maps each channel reason code
(`blocked`, `onboardingRequired`, `busy`, `userCancelled`, …) to its own
message and exit code instead of collapsing all of them into "Mnema app is
unavailable."

**Identity remains self-declared, and this ADR says so out loud.** There is no
peer-credential check and there cannot be a cheap one: under ADR 0041 the CLI
reads the permission file and the capture index in-process, so the
authorization socket is not in the read path. Any process that can exec `mnema`
can pass `--client "Claude Code"` and inherit that tool's permission. Standing
permissions widen this from "must land inside a live 24-hour window" to
"always." **We accept it.** An attacker with local code execution can already
watch the live screen, read files, and keylog; historical access is more, but
not categorically more. Binding permissions to the calling binary would require
routing reads through the socket, undoing ADR 0041, to defend against an
attacker who has already won. The provenance chip is disclosure, not a
boundary, and is documented as such.

## Consequences

**Nothing dead sits in the access list.** Live rows and blocked rows only.
History moves to `broker-audit.json`, which was already written and capped at
500 events and had no UI at all — `list_cli_access_history` shipped registered
with zero callers. Its `outcome` field starts recording denials and
scope rejections rather than being hardcoded `"success"`. Blocking and
re-enabling a client are permission-file edits and are not audited.

**Ask AI stops writing to the audit file.** `execute_for_ask_ai` wrote one
event per tool call and Ask AI runs an agent loop, so a couple of dozen
conversations evicted every CLI event from the FIFO. Ask AI is not an external
tool and cannot appear as a permission row — it is authorized by the Ask AI
setting, so a Revoke button there would have to lie — and it already shows its
sources per answer in `AnswerSourceCard`, which is better evidence than an
audit line.

**Permission ids become random.** The `{now:x}-{len:x}` scheme is only sound on
an append-only vector, and rows are now deleted.

**No migration.** `broker-grants.json` is a config file, not a `sqlx::migrate!`
DB migration, and `CLAUDE.md` records that there are no installed users. The
new shape is written and any old file is ignored.

**Windows still has no approval path.** The channel remains `#[cfg(unix)]`;
Linux inherits this work, Windows is unchanged and tracked in `SUPPORTS.md`.

**Deleted by this decision:** the native prompt and its downgrade-disclosure
body, the quick-approval policy and constants, the three-way authorization
decision branch, the window's duration control, the TTY gate, the grant-set
reductions, the unreachable `BrokerGrantCreateRequest` path, and the agent
preamble in the `mnema-data` skill.
