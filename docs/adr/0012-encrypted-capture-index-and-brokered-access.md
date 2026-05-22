# Add encrypted capture index and brokered capture access

Mnema will add an **Encrypted Capture Index** for SQLite-backed searchable and contextual capture data, paired with **Brokered Capture Access** as the supported path for AI agents and downstream tools. This protects high-context derived data such as OCR text, transcripts, search projections, app/window metadata, URLs, timing relationships, deletion/tombstone state, and speaker-derived rows, while original frame, video, and audio media encryption remains out of scope for this phase.

The encrypted index should use maintained page-level SQLite encryption rather than hand-rolled field encryption, so app-infra can preserve normal SQL/query behavior after opening the database with a key from the **Capture Index Key Store**.

## Consequences

Encrypted index keys belong in a platform-owned **Capture Index Key Store** outside `saveDirectory`; macOS should use Keychain through that abstraction. Brokered CLI access may run without the Mnema app, but it must share the app's policy, redaction, retention, tombstone, and key-store paths, returning redacted derived content and opaque identifiers by default rather than raw SQLite rows or media file paths. Direct SQLite or media-file access by agents is outside Mnema's privacy guarantee.

Brokered access requires user authorization before an agent or downstream tool can query capture data. V1 grants are read-only, redacted, time-bounded, revocable, and limited to searchable-content commands such as search, show-text, timeline, and open-in-Mnema. Grants may be time-scoped, and all-retained-history access requires an explicit user choice. Audit history stores non-content events such as tool identity, command type, timestamp, and result count, not raw query text, returned snippets, OCR text, transcripts, app/window titles, or media paths.

The first phase targets fresh installs and does not require plaintext database migration.

New capture index databases are encrypted by default and do not expose a user-facing plaintext mode. If the platform key store cannot create or retrieve the required key, Mnema should fail setup/storage clearly rather than silently creating a plaintext database.

If a key-store entry is later missing or inaccessible, Mnema treats the index as undecryptable unless an explicit future backup/export key flow exists. Fallback keys must not live in `saveDirectory`; recovery should be limited to reconnecting the original platform key store context, choosing a different save directory, or resetting the encrypted index.
