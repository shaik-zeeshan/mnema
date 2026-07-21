# Capture Index Key — Migration Drills

The real-keychain half of [ADR 0057](../adr/0057-capture-index-key-moves-to-a-shared-keychain-access-group.md)'s verification. The migration ordering, failure branches, and reader fallback are unit-tested through the adapter trait (`cargo test -p app-infra capture_index_key_store`); everything below needs a real Mac, a real keychain, and — critically — a build whose access-group entitlement actually validates. Nothing here is automated.

> **Current status: BLOCKED.** These drills cannot pass on any build we currently produce. Release CI still signs ad-hoc (`APPLE_SIGNING_IDENTITY: "-"`), and the spike (ADR 0057, "Spike findings") proved local Apple Development signing without a provisioning profile cannot claim the group either. Run the drills once Developer ID signing lands in `macos-release.yml`; until then ADR 0057 stays Proposed.

---

## Prerequisite: a build whose entitlement validates

The shared group (`RJYMY4RR97.day.mnema.capture-index`) only works when secd accepts the signing identity behind the `com.apple.security.application-groups` entitlement. That means:

- **Works:** a **Developer ID**–signed build, or an Apple Development build carrying an **embedded provisioning profile** (Xcode team profile in a `.app` bundle). Both binaries — the app *and* the bundled `mnema-cli` sidecar (`mnema.app/Contents/MacOS/mnema-cli`, re-signed with `Entitlements.mnema-cli.plist` by `build-macos-local-sign.sh` / `macos-release.yml`) — must be signed this way with Team ID `RJYMY4RR97`.
- **Does NOT work — plain local Apple Development** (`scripts/build-macos-local-sign.sh` today): the spike showed secd ignores the app-group entitlement without a profile — every SecItem call returns **−34018** (`errSecMissingEntitlement`), and secd logs the entitlement "is ignored because of invalid application signature or incorrect provisioning profile". (The `keychain-access-groups` flavor is worse: it is a restricted entitlement, and AMFI SIGKILLs the binary at exec — error −413, "No matching profile found". That is why the shipped flavor is app-groups.)
- **Does NOT work — ad-hoc** (release CI today, and every `tauri dev` rebuild): no team ID, so the group cannot be claimed. Same −34018, flat denial, no prompt.

On a build that cannot claim the group, both binaries silently fall back to the old `/usr/bin/security` item (logged as `shared group unavailable … falling back to old key store`), so the app *works* — but the drill proves nothing. Verify before starting:

```sh
codesign -d --entitlements - mnema.app
codesign -d --entitlements - mnema.app/Contents/MacOS/mnema-cli
```

Both must list `com.apple.security.application-groups` containing the group, **and** the identity must be Developer ID or profile-backed — the entitlement being *printed* is not the entitlement being *honored*.

Know your log: `rust.log` at `~/Library/Logs/day.mnema/rust.log` (`.dev` suffix for dev builds), **timestamps are UTC** (IST: +5:30). All migration lines share one prefix and are Info/Warn level, so they appear without developer options:

```sh
grep 'capture-index-key:' ~/Library/Logs/day.mnema/rust.log
```

---

## Drill 1 — Owner migration (the acceptance drill)

The migrate-and-delete path, gated on proof: read old item → write group item → read back → open the database → only then delete the old item.

1. Start from an install with **pre-existing data**: an encrypted index and a key in the old silent store. Verify the old item exists:
   ```sh
   security find-generic-password -s day.mnema.capture-index -w
   ```
   (prints the key — that promiscuous read *is* the problem being fixed).
2. Launch the entitled app build. Let it open the dashboard.
3. Check the log for the two migration lines, in order:
   ```
   capture-index-key: migrated key for <index_id> to shared group; old item is deleted after the database opens
   capture-index-key: deleted old silent keychain item for <index_id> after successful database open
   ```
4. Run a search through the bundled CLI (entitled sidecar) with the app closed or open — either way it must succeed with **no prompt**:
   ```sh
   mnema.app/Contents/MacOS/mnema-cli search "anything"
   ```
5. Confirm the old item is gone:
   ```sh
   security find-generic-password -s day.mnema.capture-index -w
   ```
   **Expected: nothing** (`could not be found`). The group item is invisible to `security` by design — an out-of-group process gets flat "not found", which is the whole point.

**Failure is safe by construction:** any migration failure logs a `capture-index-key: … staying on old key store` warning and leaves the old item intact; the app keeps running on the old path and retries next launch. A drill run that ends in step 5 still showing the key means migration did not complete — read the warnings, do not delete the item by hand.

---

## Drill 2 — Reader before owner migration

An updated CLI may run before the updated app has migrated (auto-update, agent query overnight). The reader resolves new-then-old and **never writes**.

1. Start from an install where the old silent item still exists and the entitled app build has **not** been launched (no group item yet). Verify the old item is present (step 1 above).
2. Run the entitled CLI:
   ```sh
   mnema.app/Contents/MacOS/mnema-cli search "anything"
   ```
   **Expected:** the search succeeds via the old-item fallback, headless, no prompt.
3. Confirm the reader changed nothing: the old item still answers `security find-generic-password … -w`, and no `capture-index-key:` migration/store/delete lines appear from the CLI run. Migration remains the app's job (ADR 0041: readers never migrate).

---

## Failure diagnosis

- **`−34018` / `errSecMissingEntitlement` in the log** (inside a `shared group unavailable (…); falling back to old key store` warning): this build's signature cannot claim the group — unsigned, ad-hoc, or dev-signed without a provisioning profile. It is **not** a lost key and not a bug in the store; the binary is running on the old path. Fix the signing, not the keychain.
- **"this build cannot access Mnema's keychain group — set MNEMA_CAPTURE_INDEX_KEY_DIR or use a signed build"**: the fail-closed error against an *existing* index once the old item is gone (post-migration) and the build lacks the entitlement. The DP keychain cannot distinguish denied from missing, so the binary checks its own entitlements before accusing the key of being missing — this message means the key is (almost certainly) fine and the *build* is wrong. Typical trigger: running a dev/ad-hoc build against a database an entitled build already migrated.
- **Dev escape hatch:** `MNEMA_CAPTURE_INDEX_KEY_DIR` switches key storage to a plain file directory, bypassing every keychain path. `scripts/dev-app.sh` exports it, so `bun run dev` never touches the keychain. Note it is a *different store* — a dev build pointed at a keychain-keyed database still cannot open it; use a dev-dedicated save directory (which `dev-app.sh` also sets up).
- **A truly lost key** is unchanged from ADR 0012: the index is undecryptable; reset it.
