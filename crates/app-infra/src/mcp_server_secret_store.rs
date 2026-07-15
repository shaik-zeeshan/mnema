//! Vault-backed store for the single optional secret of an MCP tool connector.
//!
//! A near-literal sibling of [`crate::ai_provider_key_store`] (same thin
//! wrapper over the process [`crate::SecretVaultHandle`]), keyed by the MCP
//! server instance id. Delivery of the secret at runtime differs by transport
//! (HTTP → `Authorization: Bearer`; stdio → the env var the connector names) but
//! storage is identical: secrets live in the AEAD-encrypted vault, never in the
//! settings/config values. (An MCP server is a *tool connector*, never an
//! inference "provider" — that word is reserved for ADR 0034/0035.)
//!
//! Accounts preserve the legacy keychain naming (service
//! `com.shaikzeeshan.mnema.mcp-connectors`, account = server instance id) as
//! `mcp-connectors/<id>` so legacy items map 1:1 during migration. A denied
//! vault unlock surfaces as [`AppInfraError::SecretVaultDenied`] — distinct
//! from "no secret stored" (`Ok(None)`).

use crate::error::{AppInfraError, Result};
use crate::secret_vault_handle::{process_secret_vault, SecretVaultHandle};

/// The legacy per-secret keychain service, consumed by the legacy-item
/// migration in [`crate::secret_vault_migration`].
pub(crate) const LEGACY_KEYCHAIN_SERVICE: &str = "com.shaikzeeshan.mnema.mcp-connectors";

/// Vault account for a server instance id — the 1:1 image of the legacy
/// keychain `(service, account)` pair.
pub fn mcp_server_vault_account(id: &str) -> String {
    format!("mcp-connectors/{id}")
}

/// Store the connector's single secret, keyed by server instance id.
pub fn store_mcp_server_secret(id: &str, secret: &str) -> Result<()> {
    store_mcp_server_secret_in(&process_secret_vault()?, id, secret)
}

/// Load the stored connector secret, or `None` when none is stored for the id.
pub fn load_mcp_server_secret(id: &str) -> Result<Option<String>> {
    load_mcp_server_secret_in(&process_secret_vault()?, id)
}

/// Delete the stored connector secret. A missing secret is treated as success.
pub fn delete_mcp_server_secret(id: &str) -> Result<()> {
    delete_mcp_server_secret_in(&process_secret_vault()?, id)
}

/// Whether a connector secret is currently stored (in-memory vault lookup).
pub fn has_mcp_server_secret(id: &str) -> Result<bool> {
    Ok(load_mcp_server_secret_in(&process_secret_vault()?, id)?.is_some())
}

fn store_mcp_server_secret_in(vault: &SecretVaultHandle, id: &str, secret: &str) -> Result<()> {
    if secret.trim().is_empty() {
        return Err(AppInfraError::McpServerSecretStore(
            "mcp server secret must not be empty".to_string(),
        ));
    }
    vault.set(&mcp_server_vault_account(id), secret)
}

fn load_mcp_server_secret_in(vault: &SecretVaultHandle, id: &str) -> Result<Option<String>> {
    Ok(vault
        .get(&mcp_server_vault_account(id))?
        .map(|secret| secret.trim().to_string())
        .filter(|secret| !secret.is_empty()))
}

fn delete_mcp_server_secret_in(vault: &SecretVaultHandle, id: &str) -> Result<()> {
    vault.delete(&mcp_server_vault_account(id))
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
            let path =
                std::env::temp_dir().join(format!("mcp-server-secret-store-{label}-{unique}"));
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

        assert!(load_mcp_server_secret_in(&vault, "github")
            .expect("load should succeed")
            .is_none());

        store_mcp_server_secret_in(&vault, "github", "ghp_secret_token")
            .expect("store should succeed");

        assert_eq!(
            load_mcp_server_secret_in(&vault, "github").expect("load should succeed"),
            Some("ghp_secret_token".to_string())
        );
    }

    #[test]
    fn reports_presence_via_in_memory_lookup() {
        let dir = TestDir::new("has");
        let vault = vault(&dir);

        assert!(load_mcp_server_secret_in(&vault, "linear")
            .expect("load should succeed")
            .is_none());

        store_mcp_server_secret_in(&vault, "linear", "lin_secret").expect("store should succeed");

        assert!(load_mcp_server_secret_in(&vault, "linear")
            .expect("load should succeed")
            .is_some());
    }

    #[test]
    fn treats_stored_whitespace_secret_as_absent() {
        let dir = TestDir::new("empty");
        let vault = vault(&dir);
        vault
            .set(&mcp_server_vault_account("github"), "   ")
            .expect("raw whitespace secret should be written");

        assert!(load_mcp_server_secret_in(&vault, "github")
            .expect("load should succeed")
            .is_none());
    }

    #[test]
    fn store_rejects_empty_or_whitespace_secret() {
        let dir = TestDir::new("reject-empty");
        let vault = vault(&dir);

        assert!(store_mcp_server_secret_in(&vault, "github", "").is_err());
        assert!(store_mcp_server_secret_in(&vault, "github", "   ").is_err());
        assert!(load_mcp_server_secret_in(&vault, "github")
            .expect("load should succeed")
            .is_none());
    }

    #[test]
    fn trims_secret_on_read() {
        let dir = TestDir::new("trim");
        let vault = vault(&dir);
        vault
            .set(&mcp_server_vault_account("github"), "  ghp_secret_token\n")
            .expect("padded secret should be written");

        assert_eq!(
            load_mcp_server_secret_in(&vault, "github").expect("load should succeed"),
            Some("ghp_secret_token".to_string())
        );
    }

    #[test]
    fn deletes_existing_secret_and_delete_missing_is_noop() {
        let dir = TestDir::new("delete");
        let vault = vault(&dir);

        store_mcp_server_secret_in(&vault, "github", "ghp_secret_token")
            .expect("store should succeed");
        delete_mcp_server_secret_in(&vault, "github").expect("delete should succeed");
        assert!(load_mcp_server_secret_in(&vault, "github")
            .expect("load should succeed")
            .is_none());

        delete_mcp_server_secret_in(&vault, "github")
            .expect("deleting an absent secret should succeed");
    }

    /// Denied ≠ missing: a denied vault surfaces `SecretVaultDenied` from every
    /// operation, while an unlocked vault without the account stays `Ok(None)`.
    #[test]
    fn denied_vault_errors_while_missing_secret_is_none() {
        let denied_dir = TestDir::new("denied");
        let denied =
            SecretVaultHandle::with_source(denied_dir.path(), Arc::new(DenyingMasterKeySource));
        assert!(matches!(
            load_mcp_server_secret_in(&denied, "github").expect_err("denied must error"),
            AppInfraError::SecretVaultDenied(_)
        ));
        assert!(matches!(
            store_mcp_server_secret_in(&denied, "github", "ghp_x").expect_err("denied must error"),
            AppInfraError::SecretVaultDenied(_)
        ));

        let missing_dir = TestDir::new("missing");
        let unlocked = vault(&missing_dir);
        assert_eq!(
            load_mcp_server_secret_in(&unlocked, "github").expect("missing secret is not an error"),
            None
        );
    }

    /// The public free functions read through the process-global vault slot.
    /// Uses server ids no other test writes, since the slot is process-wide.
    #[test]
    fn public_api_reads_through_the_installed_process_vault() {
        crate::secret_vault_handle::install_shared_test_process_vault();

        assert!(!has_mcp_server_secret("public-api-linear").expect("has should succeed"));

        store_mcp_server_secret("public-api-linear", "lin_public").expect("store should succeed");
        assert!(has_mcp_server_secret("public-api-linear").expect("has should succeed"));
        assert_eq!(
            load_mcp_server_secret("public-api-linear").expect("load should succeed"),
            Some("lin_public".to_string())
        );

        delete_mcp_server_secret("public-api-linear").expect("delete should succeed");
        assert!(!has_mcp_server_secret("public-api-linear").expect("has should succeed"));
    }
}
