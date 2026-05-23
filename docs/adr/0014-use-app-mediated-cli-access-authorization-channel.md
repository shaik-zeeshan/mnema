# Use app-mediated CLI access authorization channel

Mnema will authorize first-time or expanded CLI access through an app-owned **Broker Authorization Channel** instead of using Settings navigation as the primary flow. The desktop app owns the local authorization server and user prompts, `crates/cli` owns CLI parsing, output, app-launch behavior, and channel client transport, and app-infra remains the shared **Brokered Capture Access** policy, grant, audit, and data-access boundary.

The channel is auth-only, not a transport for brokered capture commands: authorized search, timeline, show-text, and the CLI `open` action continue to use app-infra policy/query code and may run without the desktop app except where app UI is inherently required. The default interactive approval creates a reusable, identity-scoped **CLI Access Grant** for last-day, redacted broker-visible history for 24 hours; all-retained history or non-default duration requires a dedicated **CLI Access Request** surface rather than the default native prompt. The existing broker authorization request file remains only a compatibility fallback while the channel is introduced.

**Consequences**

**Broker Client Identity** is explicit when supplied, may be inferred only from non-sensitive local agent markers, and is disclosed as locally declared or inferred rather than cryptographically verified. CLI data commands keep structured stdout output by default, while authorization progress stays on stderr and non-data access commands may use human-readable output.
