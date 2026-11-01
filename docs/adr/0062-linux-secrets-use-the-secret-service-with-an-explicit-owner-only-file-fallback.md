# Linux secrets use the Secret Service, with an explicit, never-silent owner-only file fallback

## Status

Accepted (2026-11-01) for the Linux port; not yet implemented.
Related: [ADR 0012](0012-encrypted-capture-index-and-brokered-access.md) (key stores live outside
`saveDirectory`; fallback keys must never live there) and
[ADR 0057](0057-capture-index-key-moves-to-a-shared-keychain-access-group.md) (how the CLI reads
the database key without prompts on macOS). Source: `docs/linux-port-report.md` §4.8, §10
decision 6.

## Context

Mnema keeps three kinds of secret in the macOS keychain: the database key (service
`day.mnema.capture-index`), the vault master key (`day.mnema.vault` / `master-key`), and the
licence artifacts (`day.mnema.licensing`). Provider keys, Deepgram and MCP secrets sit in the
encrypted vault file, not the keychain.

Off macOS every one of those paths fails closed today, so nothing records on Linux until a store
exists. Three file-based stores already exist for development:

- `FileCaptureIndexKeyStoreAdapter` (`crates/app-infra/src/capture_index_key_store.rs:127`), chosen
  by `MNEMA_CAPTURE_INDEX_KEY_DIR` (`:242`).
- `FileMasterKeySource` (`crates/app-infra/src/secret_vault.rs:101`), read only in debug builds
  (`:321-322`).
- `FileLicenseTokenStoreAdapter` (`crates/app-infra/src/license_token_store.rs:54-56`), compiled only
  under `cfg(any(test, debug_assertions))`.

Linux's standard store is the freedesktop Secret Service (gnome-keyring, KDE's ksecretd, KeePassXC).
It is unlocked at login on most desktops and readable by any process running as the same user, with
no per-app access control. Some systems have no Secret Service at all (minimal window managers,
servers, some autologin setups).

The CLI reads the database key through `Database::initialize_brokered_reader`
(`crates/app-infra/src/db.rs:156`) as a reader that never writes. On Linux the app and the CLI run
as the same user on the same session bus, so they can read the same item with no sharing step.

## Decision

**1. The Secret Service is the Linux key store.** Use a pure-Rust zbus client (`oo7`, or `keyring`
v4 with its zbus store; pick the one that does not add a second zbus major version). Items keep the
same service/account identities as macOS, so `secret-tool lookup service day.mnema.capture-index
account <index_id>` finds them. Stores update in place; they never delete and recreate.

**2. When no Secret Service answers, Mnema asks, once, and never falls back silently.** A blocking
choice:

- **"Set up a keyring"**, with a docs link for gnome-keyring and KWallet.
- **"Store keys in a file only you can read (less secure)"**, which states plainly: "These keys are
  not encrypted on disk. They are only as safe as your disk encryption."

The choice is saved. Until the user picks, recording cannot start (ADR 0012's refuse rule).

**3. File mode reuses the existing file adapters** with these rules:

- Keys live in `$XDG_DATA_HOME/day.mnema/keys/`, directory `0700`, files `0600`. Never in
  `saveDirectory` (ADR 0012).
- The licensing file adapter moves out of `cfg(any(test, debug_assertions))` so release builds can
  use it. The vault master-key file source likewise becomes available in release builds, but only
  through this explicit choice, never through the dev environment variable.
- Switching between modes later never deletes the old copy until the database has been opened with
  the new one (ADR 0057's migrate-then-delete-after-proof rule).

**4. TPM sealing of the key file (`systemd-creds encrypt --user`) is deferred.** Unverified whether
user-scope decrypt works without polkit on stock distros.

## Considered options

- **Refuse to run without a Secret Service.** Matches ADR 0012 literally, but locks out every
  minimal-window-manager and server-style setup for a protection those users mostly already get
  from full-disk encryption.
- **Fall back to a file automatically.** Rejected: users would never learn their keys sit
  unencrypted on disk.
- **Linux kernel keyring (keyutils).** Rejected: it does not persist across reboots, which would
  lose the database key and make the database unreadable.

## Consequences

- On Linux any same-user program can read Mnema's keys from an unlocked keyring, unlike macOS where
  only the app and the CLI can. Mnema already accepts the same-user boundary elsewhere: the CLI grant
  file and the opaque-id MAC secret are plain `0600` files
  (`crates/app-infra/src/brokered_access.rs:1509-1533`).
- A copied or synced `saveDirectory` stays undecryptable on Linux, in both modes.
- A locked keyring (autologin, or a keyring password different from the login password) raises an
  unlock prompt in the app. A headless CLI call can't answer one, so agents over SSH or cron fail
  until the desktop session unlocks the keyring. KeePassXC with per-access confirmation breaks the
  CLI's no-prompt rule the same way. Both go in the Linux docs.
- File mode is the one place a Mnema key sits unencrypted at rest. The prompt says so; the setting
  shows which mode is active.
