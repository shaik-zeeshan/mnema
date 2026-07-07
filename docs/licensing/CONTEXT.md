# Licensing Context

How Mnema is monetized: a one-time **License** with a paid **Update Window**, a time-limited **Trial**, and the offline, account-less machinery that verifies both. This context is cross-cutting — license/trial state and its keychain store live in `crates/app-infra`, the Tauri commands and gate live in `apps/desktop/src-tauri`, and the buy/trial UI lives in `apps/desktop/src`.

Everything here is **local-first and account-less** *for the app and the user*: at runtime the desktop app and the customer never phone home. The payment platform (Polar, as Merchant of Record) only takes the money; a **minimal seller-side Fulfillment step** mints the signed key and emails it (Polar cannot carry our dynamically-minted key). The app verifies the key entirely on-device against a hardcoded public key.

## Language

**Capture** (existing capture-pipeline term):
Forward recording of screen/audio into Mnema. This is the *paid capability* — the thing a License unlocks and the thing that stops when unlicensed.

**License**:
A one-time purchase that grants permanent **Capture** rights on the builds it covers, plus updates released within its **Update Window**.
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
The minimal seller-side, automated-serverless step that turns a paid Polar `order.paid` webhook into a delivered key: verify the webhook signature, mint the Ed25519-signed payload with the **private key** (held as a cloud secret, not on the build machine), and email it to the buyer via a mail provider (Resend). The desktop app never touches this — it only verifies the resulting key offline.
_Avoid_: license server, activation server (Fulfillment mints and emails; it never validates at runtime).

**Renewal**:
A separate one-time Polar SKU that, on purchase, has **Fulfillment** mint a *fresh* key with `update_through = renewal_date + 1 year`, emailed to the same buyer; the owner pastes it into the app, which keeps whichever key has the latest `update_through`. Stateless — Fulfillment stores no prior-license record.

## Relationships

- A **Trial** grants **Capture** for a fixed window, then transitions the app to **Read-Only Mode**.
- A **License** grants **Capture** permanently and supersedes both **Trial** and **Read-Only Mode**.
- **Read-Only Mode** disables **Capture** but never restricts reading already-recorded history.
- An **Update Window** lapse enforces **Perpetual Fallback** via the **auto-updater** (it declines builds dated after `update_through`), *not* via any runtime lock — a lapsed owner keeps full **Capture** on their covered build.
- A Polar `order.paid` webhook triggers **Fulfillment**, which mints and emails the key; a **Renewal** SKU triggers the same path with an extended `update_through`.
- Keys are **non-revocable** by design (offline, no phone-home): a refunded/charged-back buyer keeps a working key — accepted as "keep honest people honest." Polar handles the money side; the app does nothing on `order.refunded`.

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
