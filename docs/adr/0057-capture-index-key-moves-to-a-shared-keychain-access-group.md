# Capture index key moves to a team-shared data-protection keychain access group

## Status

Proposed (2026-07-20). Amends the **Capture Index Key Store** mechanics of
[ADR 0012](0012-encrypted-capture-index-and-brokered-access.md); its policy pins
(fail closed, keys outside `saveDirectory`, brokered CLI runs without the app) are
unchanged.

Implementation landed 2026-07-20; acceptance (the validation drill,
`docs/agents/capture-index-key-drills.md`) is blocked pending Developer ID signing
in release CI.

## Context

The macOS Capture Index Key Store stores the SQLCipher passphrase via
`/usr/bin/security add-generic-password`. Items created that way carry an ACL that
trusts the `security` tool itself, so **any process running as the user can read the
key back silently** (`security find-generic-password -w`). The encryption therefore
protects an offline copy of `saveDirectory` (stolen backup, cloud sync, other user
account) but is worthless against software already running on the machine.

The silent mechanism was not an accident: two differently-signed binaries need the
key without prompting — the desktop app, and `mnema-cli`, whose Brokered Reader opens
the encrypted index directly (ADR 0012: "may run without the Mnema app"; ADR 0041:
read-only, `query_only=ON`), headless, invoked by third-party agents where a keychain
prompt is a broken feature, not an inconvenience. A conventional signature-bound
keychain item (the secret vault's model) trusts exactly one code identity, so it
would lock out the CLI.

## Decision

The key moves to the **data-protection keychain** under a **team-prefixed shared
keychain access group** (`$(TeamID).com.shaikzeeshan.mnema.capture-index`). Both the
app and the bundled `mnema-cli` sidecar carry the access-group entitlement and read
the item silently; every other process gets "not found" — flatly denied, with no
click-through-able prompt to social-engineer past.

Mechanics:

- **Migration is migrate-and-delete, gated on proof.** On first launch the app (the
  Capture Index Owner — readers never migrate, per ADR 0041) reads the old silent
  item, writes the group item, reads it back through the new path, opens the
  database with it, and only then deletes the old item. Any failure leaves the old
  item in place and the app running on the old path with a logged warning.
- **The CLI reads new-then-old, and never writes.** An updated CLI may run before
  the updated app has migrated (auto-update, agent query overnight), so it falls
  back to reading the old silent item if the group item is absent. The fallback
  goes dead once the app migrates.
- **Failure is fail-closed with a self-diagnosing message.** The data-protection
  keychain makes "denied" indistinguishable from "missing" (out-of-group items are
  invisible), so before reporting a missing key against an existing index, the
  binary checks its own entitlements: if it lacks the access group, the error says
  "this build cannot access Mnema's keychain group" (unsigned/dev build) instead of
  the misleading "key is missing". Recovery for a truly lost key is unchanged from
  ADR 0012: the index is undecryptable; reset it.
- **Unsigned dev builds use the existing file knob.** Ad-hoc signatures carry no
  team ID and cannot claim the group; `MNEMA_CAPTURE_INDEX_KEY_DIR` (already in
  `turbo.json`) becomes the standard dev path, exported by `scripts/dev-app.sh`.
- The item uses `kSecAttrAccessibleAfterFirstUnlock` so a login-item launch can
  open the index before any user interaction.

## Considered options

- **Fold the key into the secret vault** (`com.shaikzeeshan.mnema.vault`): the vault
  master item is bound to the app's code identity only, and the Brokered Reader path
  never touches the vault by invariant — the CLI would prompt or fail.
- **App-served data plane** (CLI never opens the DB; queries the running app over
  IPC): cleanest key story, but reverses ADR 0012's "runs without the app" pin and
  requires building a query transport that does not exist. Rejected as a key-storage
  fix priced like an architecture change.
- **Legacy file-keychain ACL with two trusted apps**: deprecated API surface, thin
  Rust support, and still produces promptable dialogs rather than flat denial.
- **Grant-carried key** (authorization flow hands the CLI a sealed key): anything
  the CLI can unseal offline, malware can too — it degrades the key to file-system
  protection whenever a grant is active.

## Consequences

- Release builds must be signed with a team identity and both binaries must carry
  the entitlement; CI needs an explicit sidecar re-sign step (Tauri does not entitle
  sidecars). Local `build-macos-local-sign.sh` (Apple Development) works iff it uses
  the same Team ID as the Developer ID release identity.
- Downgrading past the migration bricks the index ("key missing" on the old path).
  Downgrade is already unsupported (in-place migration edits), so this adds no new
  promise.
- The validation drill doubles as the acceptance test: signed build → app launch
  migrates → `mnema-cli` search succeeds → `security find-generic-password -s
  com.shaikzeeshan.mnema.capture-index -w` returns nothing.

## Spike findings (2026-07-20)

Tested on macOS 26.5.1 with "Apple Development" signing, **no provisioning profile**
(scratch project: two bins, raw `SecItemAdd/CopyMatching` with
`kSecUseDataProtectionKeychain` + explicit `kSecAttrAccessGroup`). **Both entitlement
flavors fail without a profile — neither works for local dev signing:**

- **Flavor A** — `keychain-access-groups` = `[RJYMY4RR97.com.shaikzeeshan.mnema.capture-index]`
  (+ `com.apple.application-identifier`): process is SIGKILLed at exec (exit 137);
  amfid logs "Restricted entitlements not validated, bailing out … No matching
  profile found" (AMFI error −413). Restricted entitlements require an embedded
  provisioning profile, full stop.
- **Flavor B** — `com.apple.security.application-groups` with the same group: binary
  runs (unrestricted at exec) but every SecItem call returns −34018
  (errSecMissingEntitlement); secd logs the entitlement "is ignored because of
  invalid application signature or incorrect provisioning profile". Same result
  without hardened runtime and inside a minimal .app bundle.
- Ad-hoc–signed reader: −34018 (flat denial, no prompt, no item leak).

Consequence: the "local Apple Development build works iff same Team ID" line above is
wrong as written — dev-signed builds need an embedded provisioning profile (Xcode
team profile in an .app bundle), so `MNEMA_CAPTURE_INDEX_KEY_DIR` is the dev path,
period. Developer ID release signing is the expected working path (untestable on
this machine: no Developer ID cert, no profiles present); verify in CI before ship.
