# Receipt Refresh is event-driven from unhealthy states

## Status

Proposed (2026-07-17). Amends
[ADR 0053](0053-licenses-activate-once-per-machine-via-a-signed-activation-receipt.md) (the
renewal model paragraph — renewals do **not** carry their own license id) and
[ADR 0054](0054-licensing-moves-onto-licensegate.md) (the "never phone home" wording and the
activation request contents).

## Context

An audit of the licensegate integration against the server's actual code settled which renewal
model exists: **a renewal extends the buyer's existing license** — same license id, same device
slots, same CRL entry. The server advances the `updates` entitlement dates on the existing row,
re-signs the same key in place, and re-emails it. Refunding a renewal rolls the extension back on
that license; it never revokes a separate one. ADR 0053's original model (renewals mint their own
key with their own slots, delivered by paste) was never implemented server-side.

That means the *only* way extended dates reach an already-activated machine is another
activation call — free and idempotent (a known machine hash consumes no slot; the response
receipt always carries current dates). licensegate's integration doc assumes an **unconditional
daily** receipt refresh and ships a `receipt_stale` helper to cue it. Mnema's adapter had **no**
refresh at all: `run_activation` early-returns forever once a receipt exists, so a renewed
customer's machine would report "out of the update window" indefinitely.

The doc's daily model conflicts with Mnema's published privacy commitment ("activate once per
machine, then never phone home" — ADR 0054, word-for-word): it would turn every healthy machine
into a daily authenticated ping carrying a license id and machine hash.

## Decision

**Receipt Refresh is event-driven. A healthy, in-window activated machine sends nothing, ever.
A machine only re-activates when something could improve its situation:**

- **On key install** (paste or claim) — the existing silent re-activation, unchanged.
- **On return from the in-app Renew checkout.** The Renew button's Polar checkout gains a
  success redirect via the `https://mnema.day/license/open?flow=renewal` bounce page, which fires the
  `mnema://license/renewed` deep link; the app then polls Receipt Refresh
  (~2 s, webhook lag) until the extended window lands, giving up politely after ~1 minute into
  the background cadence. **Renewals never ride the claim flow** — claim is key *delivery* to a
  machine without one, and server-side claim only resolves the original mint's checkout ref
  inside 30 days of mint anyway. The renewing machine already holds the key; it only needs
  fresh dates.
- **On user demand, from any state including healthy**: a visible "Refresh license status"
  button in Settings → licensing (the Read-Only screen's "Re-check license" button is the same
  action). User-initiated contact never counts against the no-phone-home promise — the promise
  governs what the app does unprompted. Nobody waits out the cadence: renewed elsewhere, click,
  done. Opening the licensing section of Settings while unhealthy also refreshes automatically.
- **Background cadence, scoped to one state only — a paid license whose Update Window has
  lapsed**: every 4 hours for the first 14 days after lapse (renewals cluster near lapse), then
  daily, stopping the moment a refresh reports the window open. Expired trials and Revoked
  licenses get **no** background cadence: those populations are unbounded (every abandoned
  trial, forever) and the server almost never changes them — the rare support gesture (comped
  trial days, an un-revoke) propagates via the Re-check button. Provisional keeps its
  pre-existing 7-day activation retry loop unchanged.

**Activation gains one field: a generic hardware model label** ("MacBook Air (M2, 2022)") so the
seller's support dashboard can tell a license's ≤3 devices apart. Never the personal computer
name — macOS defaults it to "⟨FirstName⟩'s MacBook Pro", which would attach the customer's real
name to the license row. The label surfaces only in the admin dashboard; Mnema still shows a
device *count*, never a list.

A failed refresh changes nothing: staleness never locks, and the stored receipt stays
offline-forever valid. Refresh can only ever *improve* a machine's state.

## Considered options

- **Unconditional daily refresh (the integration doc's model).** Simplest and propagates admin
  grants within a day — but rewrites "activate once, never phone home" into a daily heartbeat
  from every machine. Rejected: the privacy sentence is the product's strongest claim.
- **No refresh at all (paste-only propagation).** Keeps the promise perfectly but strands the
  main case: a customer who renews has sibling machines that stay "lapsed" until they dig the
  re-emailed key out and re-paste on each. Rejected as hostile to paying renewers.
- **Renewals through the claim flow.** Would need server changes (renewals don't update the
  claim ref, and the 30-day mint window has long passed) to deliver a key the machine already
  has. Rejected: wrong tool.

## Consequences

- A renewing customer: the machine they renew from confirms in seconds; their other machines
  heal within ≤4 hours unattended (instantly on opening Settings). The `receipt_stale` helper
  cues the cadence.
- Accepted residual — **early renewal blip**: someone who renews while still in-window has
  healthy sibling machines that stay silent and briefly read "lapsed" at the old expiry before
  the cadence heals them within hours.
- Accepted residual — a renewal-refunder whose machines never re-contact the server keeps the
  extended receipt offline; same shape as ADR 0053's existing "offline refunder keeps a frozen
  build" residual.
- Abandoned trials and revoked machines generate zero background traffic forever.
- Deliberate deviation from the licensegate integration doc's daily-refresh recommendation and
  its `Some(device_name)` (personal name) example — this ADR is the record of why.
- Implementation seam: the `already_activated` early-return in `licensing/activation.rs` becomes
  state-aware; the cadence can ride the existing daily CRL tick's plumbing but needs its own
  4-hour timer while lapsed.
- **Concurrent refresh triggers are deliberately uncoordinated** *(2026-07-18)*: the cadence
  timer, the manual button, and the renewed deep-link poll may overlap freely — safe because
  activation is idempotent per machine hash and the compute-generation guard defeats every
  stale-publish interleaving; single-flight is not required and not implemented. The one
  exception is the renewed deep-link poll itself, which is deduplicated (a second
  `mnema://license/renewed` while a poll loop is already running is a no-op) — that link is
  web-fireable at zero cost, and stacked loops could rate-limit the machine against the server
  right when its legitimate renewal refresh needs to land.
