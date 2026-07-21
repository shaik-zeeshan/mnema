//! Owner/reader key resolution over two key stores (ADR 0057): the new
//! shared-access-group keychain ("new") and the old silent `/usr/bin/security`
//! item ("old"). The Owner migrates old → new, gated on proof; the Reader
//! resolves new-then-old and never writes.

use std::cell::RefCell;

use super::CaptureIndexKeyStoreAdapter;
use crate::error::{AppInfraError, Result};

/// Owner-side composite: prefers the shared-group item, migrates an old-only
/// key into the group (write → read back → defer old delete until the database
/// has opened), and falls back to the old path whenever the group is
/// unavailable — e.g. `errSecMissingEntitlement` on dev-signed builds — leaving
/// the old item untouched.
pub(super) struct OwnerMigratingAdapter<N, O> {
    new: N,
    old: O,
    /// Set when the old item may be deleted once the database has successfully
    /// opened with the key resolved through the new path.
    pending_old_delete: RefCell<Option<String>>,
}

impl<N, O> OwnerMigratingAdapter<N, O>
where
    N: CaptureIndexKeyStoreAdapter,
    O: CaptureIndexKeyStoreAdapter,
{
    pub(super) fn new(new: N, old: O) -> Self {
        Self {
            new,
            old,
            pending_old_delete: RefCell::new(None),
        }
    }

    pub(super) fn has_pending_old_delete(&self) -> bool {
        self.pending_old_delete.borrow().is_some()
    }

    /// Deletes the old silent item. Only call after the database has opened
    /// with the key served through the new path (migrate-and-delete, gated on
    /// proof). A failed delete is retried on the next launch's migration pass.
    pub(super) fn delete_old_item_after_open(&self) {
        let Some(index_id) = self.pending_old_delete.borrow_mut().take() else {
            return;
        };
        match self.old.delete_key(&index_id) {
            Ok(()) => log::info!(
                "capture-index-key: deleted old silent keychain item for {index_id} after successful database open"
            ),
            Err(error) => log::warn!(
                "capture-index-key: deleting old silent keychain item for {index_id} failed (will retry next launch): {error}"
            ),
        }
    }
}

impl<N, O> CaptureIndexKeyStoreAdapter for OwnerMigratingAdapter<N, O>
where
    N: CaptureIndexKeyStoreAdapter,
    O: CaptureIndexKeyStoreAdapter,
{
    fn load_key(&self, index_id: &str) -> Result<Option<String>> {
        match self.new.load_key(index_id) {
            Ok(Some(key)) => {
                // The group item is live; a leftover old item only goes away
                // once the database has opened with the group key.
                if matches!(self.old.load_key(index_id), Ok(Some(_))) {
                    log::info!(
                        "capture-index-key: shared-group item present alongside old silent item; old item is deleted after the database opens"
                    );
                    self.pending_old_delete.replace(Some(index_id.to_string()));
                }
                Ok(Some(key))
            }
            Ok(None) => {
                let Some(old_key) = self.old.load_key(index_id)? else {
                    return Ok(None);
                };
                // Migrate: write the group item, prove the new path serves it
                // back, and only then schedule the old item's deletion for
                // after the database open. Any failure stays on the old path
                // with the old item untouched.
                if let Err(error) = self.new.store_key(index_id, &old_key) {
                    log::warn!(
                        "capture-index-key: migration write to shared group failed; staying on old key store: {error}"
                    );
                    return Ok(Some(old_key));
                }
                match self.new.load_key(index_id) {
                    Ok(Some(read_back)) if read_back == old_key => {
                        log::info!(
                            "capture-index-key: migrated key for {index_id} to shared group; old item is deleted after the database opens"
                        );
                        self.pending_old_delete.replace(Some(index_id.to_string()));
                    }
                    Ok(_) => log::warn!(
                        "capture-index-key: shared-group read-back mismatch; staying on old key store"
                    ),
                    Err(error) => log::warn!(
                        "capture-index-key: shared-group read-back failed; staying on old key store: {error}"
                    ),
                }
                Ok(Some(old_key))
            }
            Err(error) => {
                // Dev-signed/ad-hoc builds land here (errSecMissingEntitlement):
                // this build cannot access the group, so stay on the old path.
                log::warn!(
                    "capture-index-key: shared group unavailable ({error}); falling back to old key store"
                );
                self.old.load_key(index_id)
            }
        }
    }

    fn store_key(&self, index_id: &str, key: &str) -> Result<()> {
        // New install: prefer the group; a build that cannot access it (dev
        // signing) still works end-to-end on the old silent store.
        match self.new.store_key(index_id, key) {
            Ok(()) => {
                log::info!("capture-index-key: new key for {index_id} stored in shared keychain group");
                Ok(())
            }
            Err(error) => {
                log::warn!(
                    "capture-index-key: shared-group store failed ({error}); storing key in old key store"
                );
                self.old.store_key(index_id, key)
            }
        }
    }

    fn missing_key_error(&self, index_id: &str) -> AppInfraError {
        // The new adapter's error is entitlement-aware (an unsigned build
        // cannot tell "denied" from "missing" on the data-protection keychain).
        self.new.missing_key_error(index_id)
    }
}

/// Reader-side composite (the `mnema` CLI): group item first, then the old
/// silent item — covering the window where an updated CLI runs before the
/// updated app has migrated. Resolution never writes, deletes, or migrates.
pub(super) struct ReaderFallbackAdapter<N, O> {
    new: N,
    old: O,
}

impl<N, O> ReaderFallbackAdapter<N, O>
where
    N: CaptureIndexKeyStoreAdapter,
    O: CaptureIndexKeyStoreAdapter,
{
    pub(super) fn new(new: N, old: O) -> Self {
        Self { new, old }
    }
}

impl<N, O> CaptureIndexKeyStoreAdapter for ReaderFallbackAdapter<N, O>
where
    N: CaptureIndexKeyStoreAdapter,
    O: CaptureIndexKeyStoreAdapter,
{
    fn load_key(&self, index_id: &str) -> Result<Option<String>> {
        match self.new.load_key(index_id) {
            Ok(Some(key)) => Ok(Some(key)),
            Ok(None) => match self.old.load_key(index_id)? {
                Some(key) => Ok(Some(key)),
                // Both items missed. The owner only deletes the old item after
                // its group write proved out (delete is gated on the database
                // opening with the migrated key), so if the old item vanished
                // in the gap between our two reads, a group re-read resolves
                // it. Closes the migration-window TOCTOU (ADR 0057).
                None => self.new.load_key(index_id),
            },
            Err(error) => {
                log::warn!(
                    "capture-index-key: shared group unavailable for reader ({error}); falling back to old key store"
                );
                self.old.load_key(index_id)
            }
        }
    }

    fn store_key(&self, index_id: &str, key: &str) -> Result<()> {
        // Only reachable when no database exists yet (fresh key creation);
        // resolving an existing index never writes. Delegating to the old
        // silent store preserves the pre-ADR-0057 CLI-first-run behavior
        // without the reader ever touching the shared group.
        self.old.store_key(index_id, key)
    }

    fn missing_key_error(&self, index_id: &str) -> AppInfraError {
        self.new.missing_key_error(index_id)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::collections::HashMap;

    use super::super::CaptureIndexKeyStore;
    use super::*;

    #[derive(Default)]
    struct FakeKeyStore {
        keys: RefCell<HashMap<String, String>>,
        writes: Cell<usize>,
        deletes: Cell<usize>,
        fail_store: Cell<bool>,
        fail_load: Cell<bool>,
        /// Fails loads only once a store has happened: lets the migration's
        /// first load return `Ok(None)` while the read-back errors.
        fail_load_after_store: Cell<bool>,
        fail_delete: Cell<bool>,
        /// Simulates a corrupted read-back: loads return the stored value mangled.
        mangle_loads: Cell<bool>,
    }

    impl FakeKeyStore {
        fn with_key(index_id: &str, key: &str) -> Self {
            let fake = Self::default();
            fake.keys
                .borrow_mut()
                .insert(index_id.to_string(), key.to_string());
            fake
        }

        fn key(&self, index_id: &str) -> Option<String> {
            self.keys.borrow().get(index_id).cloned()
        }
    }

    impl CaptureIndexKeyStoreAdapter for &FakeKeyStore {
        fn load_key(&self, index_id: &str) -> Result<Option<String>> {
            if self.fail_load.get()
                || (self.fail_load_after_store.get() && self.writes.get() > 0)
            {
                return Err(AppInfraError::CaptureIndexEncryption(
                    "this build cannot access Mnema's keychain group (errSecMissingEntitlement)"
                        .to_string(),
                ));
            }
            let key = self.keys.borrow().get(index_id).cloned();
            if self.mangle_loads.get() {
                return Ok(key.map(|key| format!("{key}-mangled")));
            }
            Ok(key)
        }

        fn store_key(&self, index_id: &str, key: &str) -> Result<()> {
            if self.fail_store.get() {
                return Err(AppInfraError::CaptureIndexEncryption(
                    "shared-group keychain write failed".to_string(),
                ));
            }
            self.writes.set(self.writes.get() + 1);
            self.keys
                .borrow_mut()
                .insert(index_id.to_string(), key.to_string());
            Ok(())
        }

        fn missing_key_error(&self, index_id: &str) -> AppInfraError {
            AppInfraError::CaptureIndexEncryption(format!(
                "capture index key for {index_id} is missing from Keychain"
            ))
        }

        fn delete_key(&self, index_id: &str) -> Result<()> {
            if self.fail_delete.get() {
                return Err(AppInfraError::CaptureIndexEncryption(
                    "keychain delete failed".to_string(),
                ));
            }
            self.deletes.set(self.deletes.get() + 1);
            self.keys.borrow_mut().remove(index_id);
            Ok(())
        }
    }

    const INDEX_ID: &str = "mnema-index-migration";
    const KEY: &str = "deadbeefdeadbeefdeadbeefdeadbeef";

    #[test]
    fn owner_migrates_old_only_key_and_deletes_old_only_after_open_success() {
        let new = FakeKeyStore::default();
        let old = FakeKeyStore::with_key(INDEX_ID, KEY);
        let owner = OwnerMigratingAdapter::new(&new, &old);

        let key = owner
            .load_key(INDEX_ID)
            .expect("resolution should succeed")
            .expect("old key should resolve");

        assert_eq!(key, KEY);
        assert_eq!(new.key(INDEX_ID).as_deref(), Some(KEY));
        // The old item survives until the database has opened with the key.
        assert_eq!(old.key(INDEX_ID).as_deref(), Some(KEY));
        assert_eq!(old.deletes.get(), 0);
        assert!(owner.has_pending_old_delete());

        owner.delete_old_item_after_open();
        assert_eq!(old.deletes.get(), 1);
        assert!(old.key(INDEX_ID).is_none());
        assert!(!owner.has_pending_old_delete());
    }

    #[test]
    fn owner_prefers_new_when_both_present_and_deletes_old_only_after_open_success() {
        let new = FakeKeyStore::with_key(INDEX_ID, KEY);
        let old = FakeKeyStore::with_key(INDEX_ID, "stale-old-key");
        let owner = OwnerMigratingAdapter::new(&new, &old);

        let key = owner
            .load_key(INDEX_ID)
            .expect("resolution should succeed")
            .expect("new key should resolve");

        assert_eq!(key, KEY);
        assert_eq!(new.writes.get(), 0, "no re-write of the group item");
        assert_eq!(old.deletes.get(), 0);
        assert!(owner.has_pending_old_delete());

        owner.delete_old_item_after_open();
        assert_eq!(old.deletes.get(), 1);
    }

    #[test]
    fn owner_stays_on_old_path_when_group_write_fails() {
        let new = FakeKeyStore::default();
        new.fail_store.set(true);
        let old = FakeKeyStore::with_key(INDEX_ID, KEY);
        let owner = OwnerMigratingAdapter::new(&new, &old);

        let key = owner
            .load_key(INDEX_ID)
            .expect("resolution should succeed")
            .expect("old key should still resolve");

        assert_eq!(key, KEY);
        assert!(new.key(INDEX_ID).is_none());
        assert_eq!(old.key(INDEX_ID).as_deref(), Some(KEY));
        assert_eq!(old.deletes.get(), 0);
        assert!(!owner.has_pending_old_delete());
    }

    #[test]
    fn owner_stays_on_old_path_when_read_back_mismatches() {
        let new = FakeKeyStore::default();
        new.mangle_loads.set(true);
        let old = FakeKeyStore::with_key(INDEX_ID, KEY);
        let owner = OwnerMigratingAdapter::new(&new, &old);

        let key = owner
            .load_key(INDEX_ID)
            .expect("resolution should succeed")
            .expect("old key should still resolve");

        assert_eq!(key, KEY);
        assert_eq!(old.key(INDEX_ID).as_deref(), Some(KEY));
        assert_eq!(old.deletes.get(), 0);
        assert!(!owner.has_pending_old_delete());
    }

    #[test]
    fn owner_falls_back_when_group_is_entitlement_denied() {
        let new = FakeKeyStore::default();
        new.fail_load.set(true);
        let old = FakeKeyStore::with_key(INDEX_ID, KEY);
        let owner = OwnerMigratingAdapter::new(&new, &old);

        let key = owner
            .load_key(INDEX_ID)
            .expect("resolution should fall back to the old key store")
            .expect("old key should resolve");

        assert_eq!(key, KEY);
        assert_eq!(old.key(INDEX_ID).as_deref(), Some(KEY));
        assert_eq!(old.deletes.get(), 0);
        assert_eq!(new.writes.get(), 0);
        assert!(!owner.has_pending_old_delete());
    }

    #[test]
    fn owner_new_install_falls_back_to_old_store_when_group_store_fails() {
        let new = FakeKeyStore::default();
        new.fail_store.set(true);
        let old = FakeKeyStore::default();
        let owner = OwnerMigratingAdapter::new(&new, &old);

        owner
            .store_key(INDEX_ID, KEY)
            .expect("fallback store should succeed");

        assert!(new.key(INDEX_ID).is_none());
        assert_eq!(old.key(INDEX_ID).as_deref(), Some(KEY));
    }

    #[test]
    fn owner_migrates_through_full_key_store_resolution() {
        let save_dir = super::super::tests::TestDir::new("owner-migration-save");
        let database_path = super::super::tests::database_path(save_dir.path());
        super::super::tests::write_identity(save_dir.path(), INDEX_ID);
        std::fs::write(&database_path, b"not-sqlite-ciphertext").expect("database should exist");

        let new = FakeKeyStore::default();
        let old = FakeKeyStore::with_key(INDEX_ID, KEY);
        let store = CaptureIndexKeyStore::new(OwnerMigratingAdapter::new(&new, &old));

        let key = store
            .resolve_database_key(save_dir.path(), &database_path)
            .expect("key should resolve")
            .expect("encrypted index should have a key");

        assert!(key.sqlcipher_pragma_value().contains(KEY));
        assert_eq!(new.key(INDEX_ID).as_deref(), Some(KEY));
        assert_eq!(old.deletes.get(), 0);
        assert!(store.adapter.has_pending_old_delete());
    }

    #[test]
    fn reader_resolves_new_only_without_writes_or_deletes() {
        let new = FakeKeyStore::with_key(INDEX_ID, KEY);
        let old = FakeKeyStore::default();
        let reader = ReaderFallbackAdapter::new(&new, &old);

        let key = reader
            .load_key(INDEX_ID)
            .expect("resolution should succeed")
            .expect("new key should resolve");

        assert_eq!(key, KEY);
        assert_eq!(new.writes.get() + old.writes.get(), 0);
        assert_eq!(new.deletes.get() + old.deletes.get(), 0);
    }

    #[test]
    fn reader_resolves_old_only_without_writes_or_deletes() {
        let new = FakeKeyStore::default();
        let old = FakeKeyStore::with_key(INDEX_ID, KEY);
        let reader = ReaderFallbackAdapter::new(&new, &old);

        let key = reader
            .load_key(INDEX_ID)
            .expect("resolution should succeed")
            .expect("old key should resolve");

        assert_eq!(key, KEY);
        assert_eq!(new.writes.get() + old.writes.get(), 0);
        assert_eq!(new.deletes.get() + old.deletes.get(), 0);
    }

    #[test]
    fn reader_prefers_new_when_both_present_without_writes_or_deletes() {
        let new = FakeKeyStore::with_key(INDEX_ID, KEY);
        let old = FakeKeyStore::with_key(INDEX_ID, "stale-old-key");
        let reader = ReaderFallbackAdapter::new(&new, &old);

        let key = reader
            .load_key(INDEX_ID)
            .expect("resolution should succeed")
            .expect("new key should resolve");

        assert_eq!(key, KEY);
        assert_eq!(old.key(INDEX_ID).as_deref(), Some("stale-old-key"));
        assert_eq!(new.writes.get() + old.writes.get(), 0);
        assert_eq!(new.deletes.get() + old.deletes.get(), 0);
    }

    #[test]
    fn reader_falls_back_when_group_is_entitlement_denied() {
        let new = FakeKeyStore::default();
        new.fail_load.set(true);
        let old = FakeKeyStore::with_key(INDEX_ID, KEY);
        let reader = ReaderFallbackAdapter::new(&new, &old);

        let key = reader
            .load_key(INDEX_ID)
            .expect("resolution should fall back to the old key store")
            .expect("old key should resolve");

        assert_eq!(key, KEY);
        assert_eq!(new.writes.get() + old.writes.get(), 0);
        assert_eq!(new.deletes.get() + old.deletes.get(), 0);
    }

    #[test]
    fn reader_rechecks_new_after_old_miss_during_owner_migration_window() {
        // Migration-window interleaving (ADR 0057): the reader reads the group
        // item absent, is preempted, and the owner completes migration in that
        // gap — writes the group item, opens the database, deletes the old
        // silent item. The reader's old-read then misses too, even though the
        // key exists in the group. The old item can only vanish after the
        // owner's group write, so a group re-read must resolve it.
        struct MigratingOldStore<'a> {
            new: &'a FakeKeyStore,
            migrated: Cell<bool>,
        }
        impl CaptureIndexKeyStoreAdapter for MigratingOldStore<'_> {
            fn load_key(&self, index_id: &str) -> Result<Option<String>> {
                if !self.migrated.get() {
                    self.migrated.set(true);
                    self.new
                        .store_key(index_id, KEY)
                        .expect("owner writes the group item during the window");
                }
                Ok(None)
            }
            fn store_key(&self, _index_id: &str, _key: &str) -> Result<()> {
                Ok(())
            }
            fn missing_key_error(&self, index_id: &str) -> AppInfraError {
                AppInfraError::CaptureIndexEncryption(format!("missing {index_id}"))
            }
        }

        let new = FakeKeyStore::default();
        let old = MigratingOldStore {
            new: &new,
            migrated: Cell::new(false),
        };
        let reader = ReaderFallbackAdapter::new(&new, old);

        let key = reader
            .load_key(INDEX_ID)
            .expect("resolution should succeed")
            .expect("reader must resolve the key the owner just migrated in the window");

        assert_eq!(key, KEY);
    }

    #[test]
    fn owner_new_install_stores_in_group_and_never_touches_old() {
        let new = FakeKeyStore::default();
        let old = FakeKeyStore::default();
        let owner = OwnerMigratingAdapter::new(&new, &old);

        owner
            .store_key(INDEX_ID, KEY)
            .expect("group store should succeed");

        assert_eq!(new.key(INDEX_ID).as_deref(), Some(KEY));
        assert!(old.key(INDEX_ID).is_none());
        assert_eq!(old.writes.get(), 0);
        assert_eq!(old.deletes.get(), 0);
    }

    #[test]
    fn owner_new_only_resolves_without_scheduling_delete() {
        let new = FakeKeyStore::with_key(INDEX_ID, KEY);
        let old = FakeKeyStore::default();
        let owner = OwnerMigratingAdapter::new(&new, &old);

        let key = owner
            .load_key(INDEX_ID)
            .expect("resolution should succeed")
            .expect("new key should resolve");

        assert_eq!(key, KEY);
        assert_eq!(new.writes.get(), 0);
        assert!(!owner.has_pending_old_delete());

        // Post-migration steady state must not attempt a phantom old delete.
        owner.delete_old_item_after_open();
        assert_eq!(old.deletes.get(), 0);
    }

    #[test]
    fn owner_delete_old_failure_is_non_fatal_and_clears_pending() {
        let new = FakeKeyStore::default();
        let old = FakeKeyStore::with_key(INDEX_ID, KEY);
        old.fail_delete.set(true);
        let owner = OwnerMigratingAdapter::new(&new, &old);

        owner
            .load_key(INDEX_ID)
            .expect("resolution should succeed")
            .expect("old key should resolve");
        assert!(owner.has_pending_old_delete());

        // A failed delete must not panic or propagate; the pending marker is
        // cleared and the retry happens on the next launch's migration pass,
        // which re-detects both items present.
        owner.delete_old_item_after_open();
        assert!(!owner.has_pending_old_delete());
        assert_eq!(old.key(INDEX_ID).as_deref(), Some(KEY));
    }

    #[test]
    fn owner_stays_on_old_path_when_read_back_errors() {
        let new = FakeKeyStore::default();
        new.fail_load_after_store.set(true);
        let old = FakeKeyStore::with_key(INDEX_ID, KEY);
        let owner = OwnerMigratingAdapter::new(&new, &old);

        let key = owner
            .load_key(INDEX_ID)
            .expect("resolution should succeed")
            .expect("old key should still resolve");

        assert_eq!(key, KEY);
        assert_eq!(old.deletes.get(), 0);
        assert!(!owner.has_pending_old_delete());
        // The freshly-written group item is left in place; the next launch's
        // migration pass re-verifies it via the both-present path.
        assert_eq!(new.key(INDEX_ID).as_deref(), Some(KEY));
    }

    #[test]
    fn reader_stores_fresh_key_in_old_store_and_never_writes_group() {
        let new = FakeKeyStore::default();
        let old = FakeKeyStore::default();
        let reader = ReaderFallbackAdapter::new(&new, &old);

        reader
            .store_key(INDEX_ID, KEY)
            .expect("fresh key creation should succeed");

        assert_eq!(old.key(INDEX_ID).as_deref(), Some(KEY));
        assert!(new.key(INDEX_ID).is_none());
        assert_eq!(new.writes.get(), 0);
    }
}
