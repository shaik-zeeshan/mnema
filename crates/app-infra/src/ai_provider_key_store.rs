//! Vault-backed store for the bring-your-own AI provider API keys.
//!
//! Thin wrapper over the process [`crate::SecretVaultHandle`]: keys live in the
//! AEAD-encrypted `secrets.vault` file under one keychain master key, so reads
//! are in-memory lookups (no per-call keychain access). A denied vault unlock
//! surfaces as [`AppInfraError::SecretVaultDenied`] — programmatically distinct
//! from "no key stored" (`Ok(None)`).
//!
//! Accounts preserve the legacy keychain naming (service
//! `com.shaikzeeshan.mnema.ai-runtime`, account = provider instance id — incl.
//! `transcription.deepgram`) as `ai-runtime/<provider>` so legacy items map 1:1
//! during migration.

use crate::error::{AppInfraError, Result};
use crate::secret_vault_handle::{process_secret_vault, SecretVaultHandle};

/// The legacy per-key keychain service, consumed by the legacy-item migration
/// in [`crate::secret_vault_migration`].
pub(crate) const LEGACY_KEYCHAIN_SERVICE: &str = "com.shaikzeeshan.mnema.ai-runtime";

/// Vault account for a provider instance id — the 1:1 image of the legacy
/// keychain `(service, account)` pair.
pub fn ai_provider_vault_account(provider: &str) -> String {
    format!("ai-runtime/{provider}")
}

/// Store the bring-your-own provider API key, keyed by provider instance id.
pub fn store_ai_provider_key(provider: &str, key: &str) -> Result<()> {
    store_ai_provider_key_in(&process_secret_vault()?, provider, key)
}

/// Load the stored provider API key, or `None` when no key is stored.
pub fn load_ai_provider_key(provider: &str) -> Result<Option<String>> {
    load_ai_provider_key_in(&process_secret_vault()?, provider)
}

/// Delete the stored provider API key. A missing key is treated as success.
pub fn delete_ai_provider_key(provider: &str) -> Result<()> {
    delete_ai_provider_key_in(&process_secret_vault()?, provider)
}

/// Whether a provider API key is currently stored (in-memory vault lookup).
pub fn has_ai_provider_key(provider: &str) -> Result<bool> {
    Ok(load_ai_provider_key_in(&process_secret_vault()?, provider)?.is_some())
}

fn store_ai_provider_key_in(vault: &SecretVaultHandle, provider: &str, key: &str) -> Result<()> {
    if key.trim().is_empty() {
        return Err(AppInfraError::AiProviderKeyStore(
            "provider api key must not be empty".to_string(),
        ));
    }
    vault.set(&ai_provider_vault_account(provider), key)
}

fn load_ai_provider_key_in(vault: &SecretVaultHandle, provider: &str) -> Result<Option<String>> {
    Ok(vault
        .get(&ai_provider_vault_account(provider))?
        .map(|key| key.trim().to_string())
        .filter(|key| !key.is_empty()))
}

fn delete_ai_provider_key_in(vault: &SecretVaultHandle, provider: &str) -> Result<()> {
    vault.delete(&ai_provider_vault_account(provider))
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;
    use crate::secret_vault::{FileMasterKeySource, MasterKeySource};

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(label: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time should be after unix epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!("ai-provider-key-store-{label}-{unique}"));
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

    fn vault(dir: &TestDir) -> SecretVaultHandle {
        SecretVaultHandle::with_source(
            dir.path(),
            Arc::new(FileMasterKeySource::new(dir.path().join("master.key"))),
        )
    }

    struct DenyingMasterKeySource;

    impl MasterKeySource for DenyingMasterKeySource {
        fn load(&self) -> Result<Option<[u8; 32]>> {
            Err(AppInfraError::SecretVault(
                "user denied keychain access".to_string(),
            ))
        }

        fn store(&self, _key: &[u8; 32]) -> Result<()> {
            Err(AppInfraError::SecretVault(
                "user denied keychain access".to_string(),
            ))
        }
    }

    #[test]
    fn round_trips_store_and_load() {
        let dir = TestDir::new("round-trip");
        let vault = vault(&dir);

        assert!(load_ai_provider_key_in(&vault, "anthropic")
            .expect("load should succeed")
            .is_none());

        store_ai_provider_key_in(&vault, "anthropic", "sk-secret-key")
            .expect("store should succeed");

        assert_eq!(
            load_ai_provider_key_in(&vault, "anthropic").expect("load should succeed"),
            Some("sk-secret-key".to_string())
        );
    }

    #[test]
    fn treats_stored_whitespace_key_as_absent() {
        let dir = TestDir::new("empty");
        let vault = vault(&dir);
        vault
            .set(&ai_provider_vault_account("anthropic"), "   ")
            .expect("raw whitespace secret should be written");

        assert!(load_ai_provider_key_in(&vault, "anthropic")
            .expect("load should succeed")
            .is_none());
    }

    #[test]
    fn store_rejects_empty_or_whitespace_key() {
        let dir = TestDir::new("reject-empty");
        let vault = vault(&dir);

        assert!(store_ai_provider_key_in(&vault, "anthropic", "").is_err());
        assert!(store_ai_provider_key_in(&vault, "anthropic", "   ").is_err());
        assert!(load_ai_provider_key_in(&vault, "anthropic")
            .expect("load should succeed")
            .is_none());
    }

    #[test]
    fn trims_key_on_read() {
        let dir = TestDir::new("trim");
        let vault = vault(&dir);
        vault
            .set(&ai_provider_vault_account("anthropic"), "  sk-secret-key\n")
            .expect("padded key should be written");

        assert_eq!(
            load_ai_provider_key_in(&vault, "anthropic").expect("load should succeed"),
            Some("sk-secret-key".to_string())
        );
    }

    #[test]
    fn deletes_existing_key_and_delete_missing_is_noop() {
        let dir = TestDir::new("delete");
        let vault = vault(&dir);

        store_ai_provider_key_in(&vault, "anthropic", "sk-secret-key")
            .expect("store should succeed");
        delete_ai_provider_key_in(&vault, "anthropic").expect("delete should succeed");
        assert!(load_ai_provider_key_in(&vault, "anthropic")
            .expect("load should succeed")
            .is_none());

        delete_ai_provider_key_in(&vault, "anthropic")
            .expect("deleting an absent key should succeed");
    }

    /// Denied ≠ missing: a denied vault surfaces `SecretVaultDenied` from every
    /// operation, while an unlocked vault without the account stays `Ok(None)`.
    #[test]
    fn denied_vault_errors_while_missing_key_is_none() {
        let denied_dir = TestDir::new("denied");
        let denied =
            SecretVaultHandle::with_source(denied_dir.path(), Arc::new(DenyingMasterKeySource));
        assert!(matches!(
            load_ai_provider_key_in(&denied, "anthropic").expect_err("denied must error"),
            AppInfraError::SecretVaultDenied(_)
        ));
        assert!(matches!(
            store_ai_provider_key_in(&denied, "anthropic", "sk-x").expect_err("denied must error"),
            AppInfraError::SecretVaultDenied(_)
        ));

        let missing_dir = TestDir::new("missing");
        let unlocked = vault(&missing_dir);
        assert_eq!(
            load_ai_provider_key_in(&unlocked, "anthropic").expect("missing key is not an error"),
            None
        );
    }

    /// The public free functions read through the process-global vault slot.
    /// Uses provider ids no other test writes, since the slot is process-wide.
    #[test]
    fn public_api_reads_through_the_installed_process_vault() {
        crate::secret_vault_handle::install_shared_test_process_vault();

        assert!(!has_ai_provider_key("public-api-openai").expect("has should succeed"));

        store_ai_provider_key("public-api-openai", "sk-public").expect("store should succeed");
        assert!(has_ai_provider_key("public-api-openai").expect("has should succeed"));
        assert_eq!(
            load_ai_provider_key("public-api-openai").expect("load should succeed"),
            Some("sk-public".to_string())
        );

        delete_ai_provider_key("public-api-openai").expect("delete should succeed");
        assert!(!has_ai_provider_key("public-api-openai").expect("has should succeed"));
    }
}
