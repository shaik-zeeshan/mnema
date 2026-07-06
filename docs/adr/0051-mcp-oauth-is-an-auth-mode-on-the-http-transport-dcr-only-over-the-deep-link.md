# MCP OAuth is an auth mode on the HTTP transport — DCR-only, over the existing deep link

## Status

Accepted. Implemented — OAuth is an auth mode on the `Http` transport
(`McpAuthMode`), tokens ride the existing keychain slot as an rmcp
`StoredCredentials` set, the browser flow completes over `mnema://oauth/callback`,
and the Settings connector list renders the authorization lifecycle.

Supersedes the "OAuth for HTTP servers" deferral in
[ADR 0048](0048-mcp-connectors-desktop-side-rmcp-client-trust-per-server.md) and
amends its "one optional secret per server" credential model (item 4) and its
"lazy connect, warm-on-open" lifecycle (item 5).

Amended 2026-07-06 (deferred deep-review findings): §5's "the auth mode tells
the reader which payload to expect" breaks when the mode changes without a
Disconnect, so three mitigations now apply — (1) the settings-save seam clears
the keychain slot whenever a connector's **effective** auth mode flips
(Bearer↔OAuth, including an http+OAuth→stdio transport edit, which keeps the
raw `auth_mode` but changes the expected payload; `auth_mode_flipped_ids`);
(2) both transports refuse a mismatched payload as a backstop — the http
bearer branch and the stdio env-var delivery never ship a payload that parses
as a `StoredCredentials` Token Set, mirroring the read-side gate that stops a
stale bearer string reading as OAuth-authorized; (3) the §4 *Needs reconnect*
flag carries a per-connector generation counter (`ReconnectState`) bumped by
authorize/callback/disconnect, so a warm task that resolves after newer truth
cannot resurrect the flag (the state converges under any interleaving). The
flip-clear deletes locally without §8's best-effort server-side revoke — an
accepted courtesy gap, matching revoke's non-blocking role.

## Context

[ADR 0048](0048-mcp-connectors-desktop-side-rmcp-client-trust-per-server.md) gave
Ask AI **tool connectors**: user-configured MCP servers whose tools join the
conversation, driven desktop-side via `rmcp`, each carrying **one optional
secret** delivered as `Authorization: Bearer` (HTTP) or an env var (stdio). It
explicitly **deferred OAuth**: "add when real users are confused by dead-server
behavior, not before."

That deferral now bites. A growing set of hosted MCP servers are **OAuth-only** —
`mcp.notion.com` is the immediate one: it advertises no static-token path, so
0048 shipped Notion as its *local* `npx @notionhq/notion-mcp-server` server with a
pasted integration token instead. That local path drags in a Node/`npx`
dependency (gated by `node_check.rs`) purely to avoid OAuth. Users who want the
hosted server — or any of the OAuth-only servers appearing across the ecosystem —
cannot connect at all.

`rmcp = "2.1"` ships a complete OAuth2 client behind its `auth` feature (currently
off): PKCE, RFC 7591 Dynamic Client Registration, `.well-known` metadata
discovery, code→token exchange, silent refresh, and 401/scope-upgrade handling.
Critically, `AuthClient<C>` **implements `rmcp`'s `StreamableHttpClient` trait**
and calls `get_access_token()` (refreshing when stale) on every request, and
`StreamableHttpClientTransport::with_client(auth_client, config)` accepts it — so
the OAuth machinery drops straight into the existing HTTP transport with no new
protocol code of our own. The hard, dangerous-to-hand-roll part is done upstream;
what remains is desktop plumbing.

## Decision

### 1. OAuth is an **auth mode on the `Http` transport**, not a new transport.

`McpTransport` stays `{ Stdio, Http }`. An OAuth connector still talks
streamable-HTTP over the workspace `reqwest`; only *authentication* differs. The
`Http` transport gains an auth axis — a **static bearer** (0048's pasted secret)
or **OAuth**. We do **not** add an `McpTransport::HttpOAuth`.

The credential model of 0048 (item 4) widens from "one optional secret" to **one
optional Connector Credential**, of two kinds:

- **Static Secret** — the user-pasted opaque string of 0048 (Bearer / env var).
- **OAuth Token Set** — a machine-managed bundle: access token, refresh token,
  expiry, and the DCR-registered `client_id` (plus a `client_secret` only when a
  server issues one — native apps are public clients, so usually none).

### 2. **Dynamic Client Registration only** (RFC 7591) for v1.

On first Connect, Mnema registers itself with the server (`rmcp`'s
`register_client`) and stores the returned `client_id` in the token set. No
per-provider `client_id`s are shipped in presets. This is the only scheme that
serves a **Custom** OAuth URL the user typed — 0048's first-class case — since we
have never pre-registered with an arbitrary host. If a specific wanted server
turns out to lack a registration endpoint, a narrow **manual `client_id`** field
on *that* connector is the escape hatch; we do not build pre-registration
machinery preemptively.

### 3. Redirect capture **reuses the existing `mnema://` deep link**.

Mnema already registers the `mnema://` scheme (dev: `mnema-dev`) via
`tauri-plugin-deep-link` + `tauri-plugin-single-instance`, handled at
`on_open_url` in `lib.rs` for the broker authorization channel. The OAuth callback
is one more routed path — `mnema://oauth/callback?code=…&state=…` — next to the
broker path. This reuses the scheme registration, the Info.plist entry, the
single-instance re-focus, and the handler, adding **zero new network
infrastructure**. A loopback `http://127.0.0.1:PORT/callback` listener is
explicitly *rejected* as the default: it is net-new in-process HTTP, and it trips
macOS's "accept incoming network connections?" firewall prompt every launch —
unacceptable noise for a privacy-first app. Loopback remains a **per-server
fallback**, built only if some server's DCR endpoint rejects a private-use URI
scheme. The registered `redirect_uri` is built from the **active build's scheme**
so dev (`mnema-dev`) and prod (`mnema`) each round-trip correctly.

### 4. **usable = `enabled` AND authorized** — two independent gates.

0048's invariant "enabled ⟹ usable" **breaks**: an OAuth connector can be
`enabled: true` yet unusable because it holds no token. A Static Secret connector
is authorized the moment its secret is present; an **OAuth Token Set** connector
moves through a lifecycle:

- **Needs authorization** — added, never connected; no token set.
- **Authorized** — token set held; 0048's lazy warm-on-open is **preserved
  unchanged** — `AuthorizationManager::initialize_from_store()` loads the token on
  reconnect and `get_access_token()` refreshes silently.
- **Needs reconnect** — refresh token expired or revoked server-side;
  `get_access_token()` fails.

Authorization is an **explicit foreground Connect action** — a browser
round-trip — because a browser cannot be popped mid-turn when warm-on-open
discovery hits a 401. The Connect click **is** the trust consent (0048's "enabling
is consent," relocated from the toggle to the button, since it now carries a real
auth ceremony); there is **no separate Mnema consent dialog** — the provider's own
OAuth consent screen is the gate, and data-egress carries the same posture every
hosted HTTP connector already has. A refresh failure surfaces exactly as 0048's
dead-server policy prescribes — **readable tool error text the model relays**
("the Notion connector needs reconnecting in Settings") plus a Settings status —
with **no auto-browser-popup**; the remedy is a **Reconnect** click. `enabled` and
authorization stay orthogonal: **disabling keeps the token set**.

### 5. Storage **reuses the one keychain slot**, polymorphic by auth mode.

The OAuth Token Set serializes to JSON and lives in the **same** per-instance-id
keychain slot the Static Secret uses (`mcp_server_secret_store.rs`, service
`com.shaikzeeshan.mnema.mcp-connectors`, account = instance id, opaque bytes). The
connector's auth mode tells the reader which payload to expect; `rmcp`'s
`CredentialStore` is a thin ~20-line adapter over `store/load_mcp_server_secret`;
`has_mcp_server_secret(id)` becomes the *authorized?* check for free. **No new
keychain service, no new table, no migration.** The transient PKCE verifier + CSRF
state (`rmcp`'s `StateStore`) live **in memory only** (`InMemoryStateStore`) — the
app stays alive across the deep-link hop, and an app quit mid-flow just means the
user re-clicks Connect.

### 6. Connect↔callback rendezvous via a CSRF-`state`-keyed pending map.

A new `mcp_oauth_begin(id)` Tauri command builds the `AuthorizationManager`, runs
DCR + discovery, opens `get_authorization_url(scopes)` with the opener plugin, and
inserts the in-flight authorization into a **pending map in `McpManager` keyed by
the CSRF `state`** (dropped on completion or a ~5-min timeout). The `on_open_url`
handler gains one branch: parse `mnema://oauth/callback`, look up by `state`,
`exchange_code_for_token(code)`, persist the token set, emit the existing
settings-refresh event so the row flips to *Authorized* live. Keying by `state`
(not connector id) makes overlapping Connect flows safe and reuses the value
`rmcp` already generates for CSRF protection. Scopes: request the server's
**advertised** scopes at Connect and lean on `rmcp`'s scope-upgrade path if a tool
later needs more — **no scope-picker UI**.

### 7. v1 surface: mechanism + Custom auth toggle + Notion flips to hosted OAuth.

Three things ship, nothing speculative:

1. The OAuth auth mode (1–6 above).
2. An **auth-mode selector on the Custom connector form** — `Bearer secret` vs
   `OAuth` — so a user can point at *any* OAuth MCP server they type. This is what
   keeps it general rather than Notion-only.
3. The **Notion preset flips from local to hosted OAuth** (`url:
   https://mcp.notion.com/mcp`); the local `npx` variant is deleted. The hosted
   path needs **no Node**, so it sidesteps `node_check.rs` entirely and is strictly
   simpler for the user (click Connect, approve — no token to mint, no Node to
   install).

GitHub / Linear / Stripe **stay on their static-token path** — they all support
PATs/API keys, which is *simpler* than OAuth; convert one only if it drops token
support.

### 8. Teardown revokes server-side, then drops locally.

A new **Disconnect** action drops the token set and returns the connector to
*Needs authorization* without removing it (switch accounts / revoke access without
re-adding). On **Disconnect and delete**, Mnema makes a **best-effort server-side
revocation** (RFC 7009 — POST the token to the discovered revocation endpoint) so
a deleted connector's refresh token is actually killed on the provider, not merely
forgotten — the privacy-correct behavior for this product. Local drop is
guaranteed; revocation is best-effort (not every server advertises the endpoint;
`rmcp` exposes no revoke helper, so this is ~15 hand-rolled lines) and **never
blocks** the Disconnect/delete. The existing `secret_may_ride_url` guard
(HTTPS-only, loopback exempt) **extends to every OAuth endpoint** — discovery,
registration, token, authorization — so a Custom OAuth URL cannot phish a token
over cleartext; PKCE S256 stays mandatory (`rmcp` default) and `rmcp` validates
`state`/CSRF and issuer.

## Considered options

- **A third `McpTransport::HttpOAuth` variant.** Rejected — OAuth is not a wire;
  it is the same streamable-HTTP transport authenticated differently. A transport
  variant would fork the transport-construction and fingerprint logic for no wire
  difference. Modeled as an auth mode instead.
- **Pre-registered OAuth clients** (ship a `client_id` per provider). Rejected for
  v1 — cannot serve a Custom OAuth URL the user typed (0048's first-class case),
  and forces the developer to register "Mnema" in every provider's dashboard
  before a connector can exist. DCR is zero-config and universal.
- **Loopback listener for redirect capture.** Rejected as default — net-new
  in-process HTTP and a recurring macOS firewall prompt, when the `mnema://` deep
  link already exists and does the job. Kept as a per-server fallback only.
- **A second keychain slot / DB table for OAuth tokens.** Rejected — the existing
  slot is byte-opaque and the config already records the auth mode, so a
  serialized JSON blob in the same slot costs nothing and needs no migration.
- **Persist PKCE/CSRF state.** Rejected — the flow completes within one app
  session across the deep-link hop; in-memory state is sufficient and a mid-flow
  quit is a harmless re-Connect.
- **Drop tokens locally only on delete.** Rejected — for a privacy-first product,
  leaving a live refresh token on the provider after "delete" is a real gap;
  best-effort server-side revocation is cheap insurance.
- **Separate Mnema consent dialog before the browser flow** (à la the Deepgram
  cloud-transcription gate, [ADR 0047](0047-cloud-transcription-is-a-provider-property-with-an-explicit-consent-gate.md)).
  Rejected — the provider's own OAuth consent screen already is the gate, and a
  hosted OAuth connector has the same data-egress posture as any hosted
  static-token connector, which carries no extra gate.

## Consequences

- **`crates/ai-runtime` stays untouched and MCP-ignorant** (0033/0048). All OAuth
  lives in the desktop `ask_ai/mcp/` module; the `AuthorizationManager` is held
  per-connector in `McpManager`. `rig`'s `rmcp` feature stays off.
- **`rmcp` gains the `auth` feature** (+ the `oauth2` 5.0 dependency,
  `default-features = false`, rustls path).
- **The Settings UI must show authorization state**, because enabled no longer
  implies usable. A connector can read *enabled but Needs authorization*, and the
  row must make that visible so a user does not think it works. New affordances:
  **Connect / Reconnect / Disconnect**, an *Authorized* badge distinct from the
  Static Secret's "secret in keychain," a *Needs reconnect* warn state, and the
  Custom-form auth-mode toggle. Design mockups: `docs/mockups/mcp-connectors/`.
- **The deep-link handler is now multi-purpose** — broker authorization *and* OAuth
  callback route through `on_open_url`; the router must dispatch by path and must
  not let one consumer swallow the other's URLs.
- **macOS-first** (SUPPORTS.md). The deep link, keychain, and opener are macOS on
  this branch; Windows/Linux need their scheme-registration and keychain siblings
  when those platforms become real.
- **Notion loses its Node dependency** but gains a browser round-trip; a user
  entirely offline cannot Connect (acceptable — the hosted server is remote
  anyway).

Reuses the per-instance identity of
[ADR 0035](0035-provider-identity-is-a-per-instance-id-not-the-kind.md), the shared
engine of [ADR 0033](0033-ask-ai-migrates-onto-shared-reasoning-engine.md), and the
connector model of [ADR 0048](0048-mcp-connectors-desktop-side-rmcp-client-trust-per-server.md).
