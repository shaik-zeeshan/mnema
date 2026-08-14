# ChatGPT subscription is a provider kind with a vault-held OAuth token set

## Status

Accepted (2026-08-13). Extends the provider model of
[ADR 0034](0034-ai-settings-are-provider-centric-with-one-global-default-model.md) /
[ADR 0035](0035-provider-identity-is-a-per-instance-id-not-the-kind.md) with a
third credential shape, next to keychain API keys (0034/0035) and the
consent-gated cloud-transcription key
([ADR 0047](0047-cloud-transcription-is-a-provider-property-with-an-explicit-consent-gate.md)).

## Context

Many Mnema users pay for ChatGPT Plus/Pro but hold no OpenAI API key. Their
subscription already includes model access through the Codex backend
(`chatgpt.com/backend-api/codex`), and OpenAI officially sanctions third-party
apps integrating on a bring-your-own-subscription basis (the Codex app-server
docs invite "deep integration inside your own product"; openai/codex#8338 has
an OpenAI engineer pointing at OpenCode doing exactly this). rig-core 0.41
ships a `providers::chatgpt` module for the route — but its OAuth support
persists tokens only as a plaintext `auth.json` file, and with no auth file it
re-runs the interactive login on every call.

The credential here is not a pasted key: it is an OAuth token set (access +
refresh token) obtained by a device-code login and rotated by refresh. That
shape collides with two Mnema invariants: secrets live in the AEAD vault, never
plaintext on disk; and background jobs must never block on interactive auth.

## Decision

`chatgpt` is a sixth `AiProviderKind` — a full peer cloud provider, not an
experimental flag — with the auth lifecycle split at the rig boundary:

- **Login and refresh are app-owned** (`apps/desktop/src-tauri/src/chatgpt_auth.rs`):
  the device-code flow (user code surfaced in-app, approval at
  `auth.openai.com/codex/device`, background poll) and the refresh grant, both
  mirroring rig 0.41 / Codex CLI semantics, including preserving the previous
  refresh token when a refresh response omits a rotated one.
- **The token set is one vault secret** — JSON in the same
  `app_infra` key-store slot (keyed by provider instance id) an API key would
  occupy, so presence probing, disconnect, and removal reuse the existing
  key-store commands unchanged. rig's `auth_file` persistence is deliberately
  unused.
- **Completions are rig-owned**: the engine crate builds the rig chatgpt
  client from a per-call `ChatGPTAuth::AccessToken`. rig derives the account id
  from the token *only* on its own OAuth/`auth.json` login path, which we do
  not use — on the `AccessToken` path it passes the field through verbatim and
  sends the `ChatGPT-Account-Id` header only when it is `Some`. So the engine
  crate reads the same `chatgpt_account_id` claim off the access token itself
  (`ai_runtime::chatgpt_account_id`); omitting it sends no header at all, which
  is wrong for multi-workspace ChatGPT accounts.
- **Freshness is a resolver concern**: the sync resolver
  (`resolve_engine_config`) stays sync/no-network and hands over the stored
  access token; every real call site goes through the async
  `resolve_engine_config_live`, which refreshes an expiring token (60 s skew,
  `exp` read off the JWT) and persists the rotation before the engine runs.
- **No unattended login, ever**: a missing/unreadable/unrefreshable token set
  collapses to the `needs_reconnect:<id>` reason code, rendered as "Reconnect
  ChatGPT in Settings" — the analog of rig's `allow_device_flow(false)`.
- **The model list is static** (`gpt-5.6-sol`, `gpt-5.6-terra`, `gpt-5.6-luna`,
  `gpt-5.5`, `gpt-5.2` — the `visibility: "list"` entries in openai/codex
  `codex-rs/models-manager/models.json`; rig's exported constants proved stale
  live), owned in
  `chatgpt_auth.rs`, and **gated on a stored token set** — model listing is the
  provider-verification proof (onboarding readiness, Settings lamps), so a
  never-connected instance must read "needs its login", never "live". Plan-tier
  mismatches (Plus user, Pro model) surface as provider errors at call time.

One shared `ChatgptConnect` component renders the flow in both Settings and
onboarding; disconnect is the existing clear-provider-key command behind a
confirm dialog.

## Consequences

- Usage bills to the user's own ChatGPT plan; each user brings their own
  login. No pooling, no reselling — the same bring-your-own-credential posture
  as every other provider.
- The route is unversioned and has no SLA, and the client id is OpenAI's own
  Codex app registration. Upstream breakage (endpoint, auth, per-app
  registration requirements) is absorbed as a rig-core bump plus, if needed,
  constant updates in `chatgpt_auth.rs` — not defended against in advance.
- The static model list goes stale until touched; updating it is a one-line
  diff that waits on no rig release.
- A second `chatgpt` instance is possible (per ADR 0035) but pointless today —
  nothing distinguishes two logins to the same backend except the account.
