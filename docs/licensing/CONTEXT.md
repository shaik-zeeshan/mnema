# Licensing Context

How Mnema is monetized: a one-time **License** with a paid **Update Window**, a time-limited **Trial**, and the offline, account-less machinery that verifies both. This context is cross-cutting — license/trial state and its keychain store live in `crates/app-infra`, the Tauri commands and gate live in `apps/desktop/src-tauri`, and the buy/trial UI lives in `apps/desktop/src`.

Everything here is **local-first and account-less** *for the app and the user*: the app **activates once per machine, then never phones home**. The payment platform (Polar, as Merchant of Record) only takes the money; a **minimal seller-side Fulfillment step** mints the signed key, emails it (Polar cannot carry our dynamically-minted key), and answers one **Activation** request per machine with a signed **Activation Receipt**. After that single exchange, the app verifies key + receipt entirely on-device against a hardcoded public key — no heartbeat, no expiry, no runtime calls.

**Privacy commitment (public):** the activation request contains exactly two things — the license id and a salted, irreversible hash of a hardware identifier. No name, no email, no device name, no OS version, no telemetry. The server stores, per license: the set of those hashes (max 3), a lifetime count, and the last reset time; no IP logs retained. After activation succeeds, the app never contacts the activation endpoint again for that machine. (The CRL fetch remains an anonymous GET of a public file.)

## Language

**Capture** (existing capture-pipeline term):
Forward recording of screen/audio into Mnema. This is the *paid capability* — the thing a License unlocks and the thing that stops when unlicensed.

**License**:
A one-time purchase that grants permanent **Capture** rights on the builds it covers, plus updates released within its **Update Window**. On any given machine those rights are unlocked by an **Activation Receipt** (or, briefly, the **Provisional Window**). The signed payload carries the buyer's `name` and `email`, shown as "Licensed to ⟨name, falling back to email⟩" in Settings — ink on the certificate as a social deterrent, never verified.
_Avoid_: subscription, seat.

**Update Window**:
The dated period after purchase during which newly-released builds are covered by a **License** (launch length: 1 year). Encoded in the signed key as an `update_through` date. Renewing extends it.
_Avoid_: expiry, license expiry (the *app* never expires — only new-build coverage does).

**Perpetual Fallback**:
The rule that an owner permanently keeps the right to *run* the newest build released within their **Update Window**, even after it lapses — they simply stop receiving newer builds. "Perpetual" means perpetual **use**, not perpetual **updates**.

**Trial**:
A time-limited grant of full **Capture** rights for evaluating Mnema before any purchase (launch length: 30 days). No account, no card. The clock starts at **first successful Capture**, not first launch — so every trial day is a day that built recall history.

**Read-Only Mode**:
The state after **Trial** expiry (and before purchase) where all already-recorded history stays fully browsable, searchable, and Ask-AI-able, but new **Capture** is disabled. Buying a **License** re-enables **Capture**.
_Avoid_: hard-lock, paywall (Read-Only Mode is explicitly *not* a lock-out).

**Fulfillment**:
The minimal seller-side, automated-serverless step that turns a paid Polar `order.paid` webhook into a delivered key: verify the webhook signature, mint the Ed25519-signed payload with the **private key** (held as a cloud secret, not on the build machine), and email it to the buyer via a mail provider (Resend). It also serves the one **Activation** endpoint and the CRL. The desktop app touches it exactly once per machine (activation); after that it only verifies signatures offline.
_Avoid_: license server (there is no per-check runtime validation — **Activation** is once per machine, ever, not a recurring check).

**Renewal**:
A separate one-time Polar SKU that, on purchase, has **Fulfillment** mint a *fresh* key with `update_through = renewal_date + 1 year`, emailed to the same buyer; the owner pastes it into the app, which keeps whichever key has the latest `update_through`. Fulfillment holds no ownership record of its own, but a renewal is **only honored for an existing owner**: before minting, it queries the Polar API for the buyer's license orders. A renewal mints the same full `tier="license"` grant as the license SKU (just cheaper), so a renewal from a non-owner would be a full-price bypass — it is **auto-refunded** with an explanatory email instead of minting. Ownership truth stays in Polar (the Merchant of Record), not in a Fulfillment-side database.

**Activation**:
The single online exchange that binds a **License** to a machine: the app sends the license id plus a salted, irreversible hash of a hardware-stable machine identifier (macOS Hardware UUID — survives factory reset; salt derived from the license id so the same machine always hashes the same) to the Fulfillment worker, which — if the license is not **Revoked** and under its **Device Cap** of 3 machines — returns an **Activation Receipt**. Activation is **mandatory** (a key without a receipt ends in **Read-Only Mode** after the **Provisional Window**) and **idempotent** (re-activating a known machine returns a fresh receipt and consumes nothing — factory reset, reinstall, and re-paste are free). **Comp Keys are not exempt.** The **Trial** never activates — the activation request is the first byte Mnema's licensing ever sends anywhere.
_Avoid_: registration, sign-in, heartbeat (there is no account and no recurring check).

**Activation Receipt**:
An Ed25519-signed, domain-separated document binding one **License** to one machine hash, minted by the **Activation** endpoint and verified offline against the same hardcoded public key **forever** — no TTL, no re-check, no expiry. A receipt never outranks the **Revocation List**: receipt says "this machine may use this license," CRL says "this license is dead" — CRL wins.

**Provisional Window**:
Up to 7 days of **actual server unreachability** (not calendar days since paste) during which a pasted, signature-valid key grants **Capture** while the app retries **Activation** quietly in the background. Consumed **per license id**, recorded in the OS keychain with the same max-timestamp-ever-seen rollback guard as the **Trial** — re-pasting the same key grants no new window; a genuinely different purchased key gets its own. If the window runs out unactivated, the app drops to **Read-Only Mode** ("couldn't confirm your license — connect once to finish activation") until an activation succeeds.
_Avoid_: grace period (it is unreachability-metered, not a calendar grace), offline mode.

**Device Cap / Reset**:
A **License** activates at most **3 machines** (a lifetime *set* of machine hashes, not a counter). At-cap refusal is never a dead end: the refusal carries a self-service **Reset** link and a buy-another-license link. Reset — authorized by **possession of the key** (pasted on a seller web page, signature-verified), rate-limited to once per 30 days — empties the slot set so new machines can activate; it **cannot** kill already-issued receipts (they verify offline forever). Egregious leaks are caught instead by the lifetime distinct-machine count — the abuse telemetry that feeds the **Revocation List**. No per-device dashboard, no device names stored.

**Revocation List (CRL)**:
A small signed document listing the license ids of fully-refunded orders **and seller-revoked leaked keys** (e.g. a license whose lifetime activation count betrays public sharing, or a leaked **Comp Key**), published by **Fulfillment** and fetched anonymously by the app (no license id or identifier is ever sent). A key on the list is **Revoked**: authentic but no longer valid. The CRL is the only channel that reaches **already-activated** machines; the **Activation** endpoint additionally refuses new activations of Revoked ids, so a dead key can never plant itself on a new machine.
_Avoid_: license server, blacklist, deactivation.

**Revoked**:
The state of a **License** whose order was fully refunded, whose key demonstrably leaked (activation telemetry), or a **Comp Key** the seller withdrew. A Revoked key is rejected exactly like **Trial** expiry — the app enters **Read-Only Mode**, live if the key is in active use. User-facing copy says "revoked", never "refunded". Partial refunds never revoke.

**Comp Key**:
A **License** the seller gifts outside Polar (press, friends, self) — minted locally with the private key, no order behind it. It is a *real* License: **Capture** is permanent, only its **Update Window** is set short (e.g. 90 days). Its license id is seller-chosen (`comp:<slug>`), making it identifiable and hand-revocable via the **Revocation List** if it leaks. Strangers evaluating Mnema use the **Trial**, not a Comp Key.
_Avoid_: trial key, demo key (a Comp Key never expires back to Read-Only Mode — it is a gift, not an evaluation window).

## Relationships

- A **Trial** grants **Capture** for a fixed window, then transitions the app to **Read-Only Mode**.
- A **License** grants **Capture** permanently and supersedes both **Trial** and **Read-Only Mode**.
- **Read-Only Mode** disables **Capture** but never restricts reading already-recorded history.
- An **Update Window** lapse enforces **Perpetual Fallback** via the **auto-updater** (it declines builds dated after `update_through`), *not* via any runtime lock — a lapsed owner keeps full **Capture** on their covered build.
- A Polar `order.paid` webhook triggers **Fulfillment**, which mints and emails the key; a **Renewal** SKU triggers the same path with an extended `update_through`.
- A **full** refund (Polar order status `refunded`) puts the order's license id on the **Revocation List**; a **Revoked** key drops the app into **Read-Only Mode** through the same seam as **Trial** expiry. Partial refunds (`partially_refunded`) never revoke — they are goodwill, not an unwound sale. *(Amends the original "keys are non-revocable" decision in ADR 0045.)*
- A **License** grants **Capture** on a machine only through an **Activation Receipt** or its **Provisional Window**; no receipt and no window → the app behaves as unlicensed.
- **Activation** state is one KV record per license id on the Fulfillment worker: `{ machine hashes (≤3), lifetime count, last reset }`. Because license ids derive from the order id, a lost-key **re-mint shares its activation set** (an already-activated machine re-pastes and consumes nothing); a **Renewal** has its *own* license id and therefore its own slots — accepted, since only verified owners can renew and every slot-set costs money. Pasting a renewal key triggers one silent re-activation.
- License ids are **derived from the Polar order id**: revocation needs no mint-time record, and a lost-key re-mint of the same order yields the same license id (a revocation always covers re-mints).
- **Staleness never locks — scoped to *after* activation**: a missing, unreachable, or stale CRL means the license stands, and a machine holding an **Activation Receipt** is offline-forever safe. A machine that never completed **Activation** is a different case — its **Provisional Window** ends in **Read-Only Mode**, because the license isn't fully established yet. The accepted residual: a refunder who keeps an *activated* machine offline keeps a frozen build.
- The **Trial** is 100% serverless: keychain state only, no activation, no server contact of any kind before a key is pasted.

## Documented edge case — fresh install after Update Window lapse

Enforcement normally lives entirely in the auto-updater: a non-renewing owner is simply never offered a newer build, so they never hit a wall. The exception is a **clean install on a new machine after the Update Window has lapsed** — the download site serves the *latest* build, which the owner's key does not cover.

Resolution (for the plan): the app compares its own build date against the key's `update_through` at launch. If the installed build is newer than the window (owner, not trial), it does **not** hard-lock — instead it directs the owner to the newest build their **License** covers (kept downloadable), or to renew. Old covered builds must therefore remain downloadable indefinitely. This is the single place Perpetual Fallback needs a runtime check rather than an updater gate; it must never degrade already-recorded history, matching **Read-Only Mode**'s never-lock-existing-data rule.

## Flagged ambiguities

- **"stop capture" is overloaded.** Capture can stop for two unrelated reasons: (1) **Capture Suspension** — a *transient liveness* condition (display unavailable, low disk) that self-heals when the condition clears (see ADR 0021, 0040); (2) **Read-Only Mode** — a *licensing* state that does not self-heal and is cleared only by purchasing a **License**. These are distinct concepts and must not share a code path or a user-facing message.
- **Two different lapse gates.** "Unlicensed" conflates (A) **Trial expiry** → Read-Only Mode, and (B) **Update Window** lapse for a paying owner → *nothing is taken away*; they keep their owned version fully functional and only stop receiving new builds. Only (A) triggers Read-Only Mode.
- **"Trial start" resolved.** Ambiguous between first launch and first capture — resolved: the **Trial** clock starts at **first successful Capture**. Setup/permission time before any recording does not consume trial days.

## Decisions

- [ADR 0044](../adr/0044-monetize-as-one-time-purchase-with-paid-update-window.md) — the business model (one-time purchase + paid Update Window + Trial → Read-Only Mode).
- [ADR 0045](../adr/0045-licenses-verified-offline-ed25519-polar-merchant-of-record-only.md) — offline Ed25519 verification, updater-gated enforcement, and Polar-as-Merchant-of-Record-only Fulfillment.
- [ADR 0052](../adr/0052-refunded-licenses-die-via-a-signed-revocation-list.md) — refunded licenses die via a signed **Revocation List**; staleness never locks (amends 0045's "keys are non-revocable").
- [ADR 0053](../adr/0053-licenses-activate-once-per-machine-via-a-signed-activation-receipt.md) — mandatory one-time **Activation** per machine with an offline-forever **Activation Receipt**, 3-device cap, self-service reset (supersedes 0045's "no activation server"; widens 0052's CRL charter to leaked keys).
