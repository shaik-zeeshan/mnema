# Monetize Mnema as a one-time purchase with a paid update window

## Status

Accepted. Implemented on PR #162 (2026-07-18).

## Context

Mnema needs a revenue model. The two direct pure-recall competitors, **Rewind** and
**Limitless**, were both subscription + cloud — and both are gone (Meta acquired Rewind and
remotely killed the Mac app in Dec 2025). Mnema's sharpest differentiator is the exact inverse:
**local-first, we cannot be switched off.** A subscription undercuts that promise; a perpetual
license reinforces it.

Two constraints shape the model:

- **Cloud AI is BYOK.** Users supply their own provider API keys (stored in the OS keychain), so
  AI has **zero marginal cost** to Mnema. There is no need — today — for a metered AI tier,
  accounts, or a server. A hosted-AI paid tier is explicitly deferred, not rejected.
- **The app is continuously developed.** A *pure* one-time perpetual license is the model
  actively-developed indie apps regret and abandon (Sketch, Sublime, Kaleidoscope all switched
  away) because it starves ongoing development of recurring revenue.

## Decision

Sell Mnema as a **one-time purchase with a paid Update Window and Perpetual Fallback** — the
CleanShot X / Sketch model. Launch parameters (revisable policy values, not architecture):

- **$69 one-time, single SKU.** Priced above screenshot utilities, below the ~$19/mo cloud-recall
  subscriptions it replaces. The license payload reserves a `tier` field for a future
  (e.g. hosted-AI) tier; launch ships **one** tier.
- **1-year Update Window.** The License covers all builds released within one year of purchase.
- **$29 renewal** for another year — flat, no penalty for lapsing, decoupled from the base price
  (a renewal buys another year of *new work*; the owned build is kept regardless).
- **Perpetual Fallback.** An owner permanently keeps the right to *run* the newest build released
  within their window. "Perpetual" = perpetual **use**, not perpetual **updates**.
- **30-day Trial**, no account, no card. The clock starts at **first successful Capture** (not
  first launch), so setup/permission time never burns trial days. *(Amended 2026-07-16, ADR
  0054: trials are server-issued — the clock starts at issuance, which is requested at first
  Capture and may lag it by up to 7 offline days; capture never blocks on the request.)*
- **Read-Only Mode on Trial expiry.** The paid product is fundamentally **forward Capture**. When
  a trial lapses, new Capture stops but all already-recorded history stays fully browsable,
  searchable, and Ask-AI-able. Trial expiry never holds a user's own recorded history hostage —
  doing so would reproduce the "switched off" betrayal Mnema markets against.

Domain language and relationships live in [`docs/licensing/CONTEXT.md`](../licensing/CONTEXT.md).
The offline verification and enforcement mechanism is [ADR 0045](0045-licenses-verified-offline-ed25519-polar-merchant-of-record-only.md).

## Considered options

- **Pure subscription.** Rejected: it is precisely the model of the two dead competitors and
  directly undercuts the "local-first, can't be switched off" differentiator that is Mnema's
  reason to exist.
- **Pure one-time perpetual (no update window).** Rejected: the model actively-developed indie
  apps regret; it starves continuous development of recurring revenue and forces an eventual,
  more painful pivot (as Sketch/Sublime/Kaleidoscope all did).
- **Metered / hosted-AI paid tier now.** Deferred, not rejected: BYOK makes AI zero-marginal-cost
  today, so a metered tier would add accounts + a server for no present benefit. The reserved
  `tier` field keeps the door open.
- **Hard-lock on trial expiry (stronger conversion pressure).** Rejected: on-brand-wrong — it
  betrays the anti-"switched off" promise on a user's own data. Read-Only Mode keeps conversion
  pressure (you can see what you'd lose) without the betrayal.

## Consequences

- Renewals provide recurring-ish revenue **without** subscription lock-in.
- Read-Only Mode creates a permanent free read-only tier (record 30 days, mine it forever). This
  is an accepted funnel, not a leak.
- All prices/lengths above are launch policy values and can change without architectural churn.
- "Keep your version forever" carries real enforcement mechanics (the key must encode when the
  window ends; the updater — not a lock — enforces it; old covered builds must stay downloadable).
  Those live in [ADR 0045](0045-licenses-verified-offline-ed25519-polar-merchant-of-record-only.md).
