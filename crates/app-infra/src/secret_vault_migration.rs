//! One-way migration of the legacy per-secret keychain items into the vault.
//!
//! Runs once per process, right after the first successful vault unlock (see
//! [`crate::secret_vault_handle`]). For every generic-password item under the
//! two legacy services (`com.shaikzeeshan.mnema.ai-runtime`, `.mcp-connectors`)
//! the item is copied into the vault under its 1:1 mapped account
//! (`ai-runtime/<account>`, `mcp-connectors/<account>`), the copy is verified
//! by reading it back, and only then is the legacy keychain item deleted.
//! Anything that can't be copied-and-verified is left in place and retried on
//! the next launch — migration is idempotent, and the steady state is simply
//! "no legacy items remain".
//!
//! No "migration completed" marker is needed: enumeration is a
//! metadata-only keychain search (`kSecReturnAttributes`, never
//! `kSecReturnData`), which macOS does not gate behind the item ACL — it can
//! never prompt. Only per-item secret *reads* can prompt/deny, and those only
//! happen while legacy items still exist. Once migration finishes, the search
//! matches zero items and every subsequent launch is a silent no-op.

use crate::error::Result;
use crate::secret_vault::SecretVault;

/// The legacy per-secret keychain, injectable so tests never touch the real
/// keychain.
pub(crate) trait LegacyKeychain {
    /// Account names stored under `service`. Metadata-only — must not read
    /// secret data (the real impl is ACL-silent for exactly that reason).
    fn list_accounts(&self, service: &str) -> Result<Vec<String>>;
    /// The secret for one item. `Err` = denied/unreadable; the item is left
    /// in place and retried next launch.
    fn read_secret(&self, service: &str, account: &str) -> Result<String>;
    fn delete(&self, service: &str, account: &str) -> Result<()>;
}

/// The vault side of the migration, faked in tests to prove verify-before-delete.
pub(crate) trait MigrationVault {
    fn get(&self, account: &str) -> Option<String>;
    fn set(&mut self, account: &str, secret: &str) -> Result<()>;
}

impl MigrationVault for SecretVault {
    fn get(&self, account: &str) -> Option<String> {
        SecretVault::get(self, account).map(str::to_string)
    }

    fn set(&mut self, account: &str, secret: &str) -> Result<()> {
        SecretVault::set(self, account, secret)
    }
}

/// Migrate the real legacy keychain items into `vault`. Called from the vault
/// handle after the once-per-process unlock; a no-op off macOS and under the
/// dev master-key file knob (a disposable dev vault must never drain the real
/// keychain).
pub(crate) fn run_default_migration(vault: &mut SecretVault) {
    #[cfg(target_os = "macos")]
    {
        #[cfg(debug_assertions)]
        if std::env::var_os("MNEMA_DEV_MASTER_KEY_FILE").is_some() {
            return;
        }
        migrate_legacy_secrets(vault, &RealLegacyKeychain);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = vault;
}

/// The two legacy services and the account mapping into the vault.
fn legacy_services() -> [(&'static str, fn(&str) -> String); 2] {
    [
        (
            crate::ai_provider_key_store::LEGACY_KEYCHAIN_SERVICE,
            crate::ai_provider_key_store::ai_provider_vault_account,
        ),
        (
            crate::mcp_server_secret_store::LEGACY_KEYCHAIN_SERVICE,
            crate::mcp_server_secret_store::mcp_server_vault_account,
        ),
    ]
}

pub(crate) fn migrate_legacy_secrets(vault: &mut dyn MigrationVault, legacy: &dyn LegacyKeychain) {
    for (service, map_account) in legacy_services() {
        let accounts = match legacy.list_accounts(service) {
            Ok(accounts) => accounts,
            Err(error) => {
                log::warn!("secret-vault migration: listing legacy items under {service} failed, retrying next launch: {error}");
                continue;
            }
        };
        for account in accounts {
            migrate_item(vault, legacy, service, &account, map_account);
        }
    }
}

fn migrate_item(
    vault: &mut dyn MigrationVault,
    legacy: &dyn LegacyKeychain,
    service: &str,
    account: &str,
    map_account: fn(&str) -> String,
) {
    let target = map_account(account);
    let secret = match legacy.read_secret(service, account) {
        Ok(secret) => secret,
        Err(error) => {
            log::warn!("secret-vault migration: reading legacy item {service}/{account} failed (denied?), leaving it for next launch: {error}");
            return;
        }
    };

    match vault.get(&target) {
        // Vault already holds the same value (e.g. a previous run copied but
        // failed to delete): safe to delete the legacy item.
        Some(existing) if existing == secret => {}
        // Vault holds a DIFFERENT value: vault wins, never overwrite it — but
        // the legacy item isn't a verified copy either, so leave it.
        Some(_) => {
            log::warn!("secret-vault migration: vault already holds a different value for {target}; leaving legacy item {service}/{account} untouched");
            return;
        }
        None => {
            if let Err(error) = vault.set(&target, &secret) {
                log::warn!("secret-vault migration: writing {target} into the vault failed, leaving legacy item {service}/{account}: {error}");
                return;
            }
            // Verify the copy landed before deleting anything.
            if vault.get(&target).as_deref() != Some(secret.as_str()) {
                log::warn!("secret-vault migration: read-back of {target} did not match, leaving legacy item {service}/{account}");
                return;
            }
        }
    }

    if let Err(error) = legacy.delete(service, account) {
        // The copy is verified in the vault; a failed delete just means the
        // (matching) legacy item is retried — and deleted — next launch.
        log::warn!("secret-vault migration: deleting legacy item {service}/{account} failed, retrying next launch: {error}");
    } else {
        log::info!("secret-vault migration: migrated legacy item {service}/{account} into the vault as {target}");
    }
}

#[cfg(target_os = "macos")]
struct RealLegacyKeychain;

#[cfg(target_os = "macos")]
impl LegacyKeychain for RealLegacyKeychain {
    fn list_accounts(&self, service: &str) -> Result<Vec<String>> {
        use security_framework::item::{ItemClass, ItemSearchOptions, Limit};

        // Attributes-only search: no kSecReturnData, so no ACL check and no
        // prompt — see the module docs for why this needs no completed-marker.
        match ItemSearchOptions::new()
            .class(ItemClass::generic_password())
            .service(service)
            .load_attributes(true)
            .limit(Limit::All)
            .search()
        {
            Ok(results) => Ok(results
                .iter()
                .filter_map(|result| result.simplify_dict()?.remove("acct"))
                .collect()),
            Err(error) if error.code() == crate::secret_vault::ERR_SEC_ITEM_NOT_FOUND => {
                Ok(Vec::new())
            }
            Err(error) => Err(crate::error::AppInfraError::SecretVault(format!(
                "keychain search under {service} failed: {error}"
            ))),
        }
    }

    fn read_secret(&self, service: &str, account: &str) -> Result<String> {
        let bytes = security_framework::passwords::get_generic_password(service, account).map_err(
            |error| {
                crate::error::AppInfraError::SecretVault(format!(
                    "keychain read of {service}/{account} failed: {error}"
                ))
            },
        )?;
        String::from_utf8(bytes).map_err(|_| {
            crate::error::AppInfraError::SecretVault(format!(
                "legacy keychain item {service}/{account} is not valid utf-8"
            ))
        })
    }

    fn delete(&self, service: &str, account: &str) -> Result<()> {
        security_framework::passwords::delete_generic_password(service, account).map_err(|error| {
            crate::error::AppInfraError::SecretVault(format!(
                "keychain delete of {service}/{account} failed: {error}"
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;
    use crate::error::AppInfraError;
    use crate::secret_vault::{
        unlock_secret_vault_with_source, FileMasterKeySource, SecretVaultUnlock,
    };

    const AI_SERVICE: &str = crate::ai_provider_key_store::LEGACY_KEYCHAIN_SERVICE;
    const MCP_SERVICE: &str = crate::mcp_server_secret_store::LEGACY_KEYCHAIN_SERVICE;

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(label: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time should be after unix epoch")
                .as_nanos();
            let path =
                std::env::temp_dir().join(format!("secret-vault-migration-{label}-{unique}"));
            std::fs::create_dir_all(&path).expect("test directory should be created");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn open_vault(dir: &TestDir) -> SecretVault {
        let source = FileMasterKeySource::new(dir.path().join("master.key"));
        match unlock_secret_vault_with_source(dir.path(), &source).expect("unlock should succeed") {
            SecretVaultUnlock::Unlocked(vault) | SecretVaultUnlock::Missing(vault) => vault,
            SecretVaultUnlock::Denied(reason) => panic!("vault denied in test: {reason}"),
        }
    }

    /// Fake legacy keychain: `(service, account) -> secret`, with an optional
    /// set of accounts whose reads are denied.
    struct FakeLegacyKeychain {
        items: RefCell<BTreeMap<(String, String), String>>,
        denied_reads: Vec<(String, String)>,
    }

    impl FakeLegacyKeychain {
        fn new(items: &[(&str, &str, &str)]) -> Self {
            Self {
                items: RefCell::new(
                    items
                        .iter()
                        .map(|(service, account, secret)| {
                            (
                                (service.to_string(), account.to_string()),
                                secret.to_string(),
                            )
                        })
                        .collect(),
                ),
                denied_reads: Vec::new(),
            }
        }

        fn deny_read(mut self, service: &str, account: &str) -> Self {
            self.denied_reads
                .push((service.to_string(), account.to_string()));
            self
        }

        fn contains(&self, service: &str, account: &str) -> bool {
            self.items
                .borrow()
                .contains_key(&(service.to_string(), account.to_string()))
        }

        fn len(&self) -> usize {
            self.items.borrow().len()
        }
    }

    impl LegacyKeychain for FakeLegacyKeychain {
        fn list_accounts(&self, service: &str) -> Result<Vec<String>> {
            Ok(self
                .items
                .borrow()
                .keys()
                .filter(|(item_service, _)| item_service == service)
                .map(|(_, account)| account.clone())
                .collect())
        }

        fn read_secret(&self, service: &str, account: &str) -> Result<String> {
            if self
                .denied_reads
                .contains(&(service.to_string(), account.to_string()))
            {
                return Err(AppInfraError::SecretVault(
                    "user denied keychain access".to_string(),
                ));
            }
            self.items
                .borrow()
                .get(&(service.to_string(), account.to_string()))
                .cloned()
                .ok_or_else(|| AppInfraError::SecretVault("item not found".to_string()))
        }

        fn delete(&self, service: &str, account: &str) -> Result<()> {
            self.items
                .borrow_mut()
                .remove(&(service.to_string(), account.to_string()));
            Ok(())
        }
    }

    /// Fake vault whose `set` reports success but stores nothing — the
    /// "silently failed persist" a verify-before-delete must catch.
    struct SilentlyFailingVault;

    impl MigrationVault for SilentlyFailingVault {
        fn get(&self, _account: &str) -> Option<String> {
            None
        }

        fn set(&mut self, _account: &str, _secret: &str) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn copies_legacy_items_into_the_vault_then_deletes_them() {
        let dir = TestDir::new("copies");
        let mut vault = open_vault(&dir);
        let legacy = FakeLegacyKeychain::new(&[
            (AI_SERVICE, "anthropic", "sk-ant-legacy"),
            (AI_SERVICE, "transcription.deepgram", "dg-legacy"),
            (MCP_SERVICE, "github", "ghp-legacy"),
        ]);

        migrate_legacy_secrets(&mut vault, &legacy);

        assert_eq!(vault.get("ai-runtime/anthropic"), Some("sk-ant-legacy"));
        assert_eq!(
            vault.get("ai-runtime/transcription.deepgram"),
            Some("dg-legacy")
        );
        assert_eq!(vault.get("mcp-connectors/github"), Some("ghp-legacy"));
        assert_eq!(legacy.len(), 0, "all legacy items should be deleted");

        // The copies really persisted: reopen the vault from disk.
        let reopened = open_vault(&dir);
        assert_eq!(reopened.get("ai-runtime/anthropic"), Some("sk-ant-legacy"));
        assert_eq!(reopened.get("mcp-connectors/github"), Some("ghp-legacy"));
    }

    #[test]
    fn denied_legacy_read_leaves_the_item_and_migrates_the_rest() {
        let dir = TestDir::new("denied");
        let mut vault = open_vault(&dir);
        let legacy = FakeLegacyKeychain::new(&[
            (AI_SERVICE, "anthropic", "sk-ant-legacy"),
            (AI_SERVICE, "openai", "sk-oai-legacy"),
        ])
        .deny_read(AI_SERVICE, "anthropic");

        migrate_legacy_secrets(&mut vault, &legacy);

        assert!(
            legacy.contains(AI_SERVICE, "anthropic"),
            "a denied legacy read must leave the legacy item intact"
        );
        assert_eq!(vault.get("ai-runtime/anthropic"), None);
        // The denial did not block the sibling item.
        assert_eq!(vault.get("ai-runtime/openai"), Some("sk-oai-legacy"));
        assert!(!legacy.contains(AI_SERVICE, "openai"));
    }

    #[test]
    fn vault_wins_matching_value_deletes_legacy_differing_value_keeps_it() {
        let dir = TestDir::new("vault-wins");
        let mut vault = open_vault(&dir);
        vault
            .set("ai-runtime/anthropic", "sk-ant-vault")
            .expect("seed should succeed");
        vault
            .set("mcp-connectors/github", "ghp-same")
            .expect("seed should succeed");
        let legacy = FakeLegacyKeychain::new(&[
            // Differs from the vault: vault wins, legacy item stays.
            (AI_SERVICE, "anthropic", "sk-ant-legacy"),
            // Matches the vault: legacy item is safe to delete.
            (MCP_SERVICE, "github", "ghp-same"),
        ]);

        migrate_legacy_secrets(&mut vault, &legacy);

        assert_eq!(
            vault.get("ai-runtime/anthropic"),
            Some("sk-ant-vault"),
            "the vault value must never be overwritten"
        );
        assert!(
            legacy.contains(AI_SERVICE, "anthropic"),
            "a legacy item that differs from the vault must be left in place"
        );
        assert!(
            !legacy.contains(MCP_SERVICE, "github"),
            "a legacy item matching the vault should be deleted"
        );
    }

    #[test]
    fn silently_failed_vault_write_never_deletes_the_legacy_item() {
        let legacy = FakeLegacyKeychain::new(&[(AI_SERVICE, "anthropic", "sk-ant-legacy")]);

        migrate_legacy_secrets(&mut SilentlyFailingVault, &legacy);

        assert!(
            legacy.contains(AI_SERVICE, "anthropic"),
            "verify-before-delete: an unverified copy must never delete the legacy item"
        );
    }

    #[test]
    fn migration_is_idempotent_across_runs() {
        let dir = TestDir::new("idempotent");
        let mut vault = open_vault(&dir);
        // First run: the read is denied, item stays behind.
        let legacy = FakeLegacyKeychain::new(&[(AI_SERVICE, "anthropic", "sk-ant-legacy")])
            .deny_read(AI_SERVICE, "anthropic");
        migrate_legacy_secrets(&mut vault, &legacy);
        assert!(legacy.contains(AI_SERVICE, "anthropic"));

        // "Next launch": same item, read now allowed.
        let legacy = FakeLegacyKeychain::new(&[(AI_SERVICE, "anthropic", "sk-ant-legacy")]);
        migrate_legacy_secrets(&mut vault, &legacy);
        assert_eq!(vault.get("ai-runtime/anthropic"), Some("sk-ant-legacy"));
        assert_eq!(legacy.len(), 0);

        // Steady state: nothing left to migrate, nothing changes.
        let legacy = FakeLegacyKeychain::new(&[]);
        migrate_legacy_secrets(&mut vault, &legacy);
        assert_eq!(vault.get("ai-runtime/anthropic"), Some("sk-ant-legacy"));
    }
}
