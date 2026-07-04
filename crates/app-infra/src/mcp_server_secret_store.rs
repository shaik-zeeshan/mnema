//! Keychain store for the single optional secret of an MCP tool connector.
//!
//! A near-literal sibling of [`crate::ai_provider_key_store`] (same adapter
//! trait, same file-backed test fallback + platform keychain), keyed by the MCP
//! server instance id. Delivery of the secret at runtime differs by transport
//! (HTTP → `Authorization: Bearer`; stdio → the env var the connector names) but
//! storage is identical: the secret lives ONLY in the OS keychain, never in the
//! settings/config values. (An MCP server is a *tool connector*, never an
//! inference "provider" — that word is reserved for ADR 0034/0035.)

#[cfg(test)]
use std::path::PathBuf;

use crate::error::{AppInfraError, Result};

const KEYCHAIN_SERVICE: &str = "com.shaikzeeshan.mnema.mcp-connectors";

trait McpServerSecretStoreAdapter {
    fn load_secret(&self, id: &str) -> Result<Option<String>>;
    fn store_secret(&self, id: &str, secret: &str) -> Result<()>;
    fn delete_secret(&self, id: &str) -> Result<()>;
}

struct McpServerSecretStore<A> {
    adapter: A,
}

impl<A> McpServerSecretStore<A>
where
    A: McpServerSecretStoreAdapter,
{
    fn new(adapter: A) -> Self {
        Self { adapter }
    }

    fn store(&self, id: &str, secret: &str) -> Result<()> {
        if secret.trim().is_empty() {
            return Err(AppInfraError::McpServerSecretStore(
                "mcp server secret must not be empty".to_string(),
            ));
        }
        self.adapter.store_secret(id, secret)
    }

    fn load(&self, id: &str) -> Result<Option<String>> {
        self.adapter.load_secret(id)
    }

    fn delete(&self, id: &str) -> Result<()> {
        self.adapter.delete_secret(id)
    }

    fn has(&self, id: &str) -> Result<bool> {
        Ok(self.adapter.load_secret(id)?.is_some())
    }
}

// The plaintext file-backed secret store exists only for tests; release builds
// must route every secret through the OS keychain (the "secrets live ONLY in the
// keychain" invariant), so it is compiled out entirely outside `cfg(test)`.
#[cfg(test)]
#[derive(Debug, Clone)]
struct FileMcpServerSecretStoreAdapter {
    secret_dir: PathBuf,
}

#[cfg(test)]
impl FileMcpServerSecretStoreAdapter {
    fn new(secret_dir: impl Into<PathBuf>) -> Self {
        Self {
            secret_dir: secret_dir.into(),
        }
    }

    fn secret_path(&self, id: &str) -> PathBuf {
        self.secret_dir.join(format!("{id}.secret"))
    }
}

#[cfg(test)]
impl McpServerSecretStoreAdapter for FileMcpServerSecretStoreAdapter {
    fn load_secret(&self, id: &str) -> Result<Option<String>> {
        std::fs::create_dir_all(&self.secret_dir)?;
        let path = self.secret_path(id);
        if !path.exists() {
            return Ok(None);
        }

        let secret = std::fs::read_to_string(path)?;
        let secret = secret.trim();
        if secret.is_empty() {
            return Ok(None);
        }
        Ok(Some(secret.to_string()))
    }

    fn store_secret(&self, id: &str, secret: &str) -> Result<()> {
        std::fs::create_dir_all(&self.secret_dir)?;
        std::fs::write(self.secret_path(id), secret)?;
        Ok(())
    }

    fn delete_secret(&self, id: &str) -> Result<()> {
        let path = self.secret_path(id);
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct PlatformKeychainMcpServerSecretStoreAdapter;

impl McpServerSecretStoreAdapter for PlatformKeychainMcpServerSecretStoreAdapter {
    fn load_secret(&self, id: &str) -> Result<Option<String>> {
        load_platform_secret(id)
    }

    fn store_secret(&self, id: &str, secret: &str) -> Result<()> {
        store_platform_secret(id, secret)
    }

    fn delete_secret(&self, id: &str) -> Result<()> {
        delete_platform_secret(id)
    }
}

/// Store the connector's single secret in the OS keychain, keyed by server id.
pub fn store_mcp_server_secret(id: &str, secret: &str) -> Result<()> {
    #[cfg(test)]
    if let Ok(secret_dir) = std::env::var("MNEMA_MCP_SERVER_SECRET_DIR") {
        return McpServerSecretStore::new(FileMcpServerSecretStoreAdapter::new(secret_dir))
            .store(id, secret);
    }

    McpServerSecretStore::new(PlatformKeychainMcpServerSecretStoreAdapter).store(id, secret)
}

/// Load the stored connector secret, or `None` when none is stored for the id.
pub fn load_mcp_server_secret(id: &str) -> Result<Option<String>> {
    #[cfg(test)]
    if let Ok(secret_dir) = std::env::var("MNEMA_MCP_SERVER_SECRET_DIR") {
        return McpServerSecretStore::new(FileMcpServerSecretStoreAdapter::new(secret_dir)).load(id);
    }

    McpServerSecretStore::new(PlatformKeychainMcpServerSecretStoreAdapter).load(id)
}

/// Delete the stored connector secret. A missing secret is treated as success.
pub fn delete_mcp_server_secret(id: &str) -> Result<()> {
    #[cfg(test)]
    if let Ok(secret_dir) = std::env::var("MNEMA_MCP_SERVER_SECRET_DIR") {
        return McpServerSecretStore::new(FileMcpServerSecretStoreAdapter::new(secret_dir))
            .delete(id);
    }

    McpServerSecretStore::new(PlatformKeychainMcpServerSecretStoreAdapter).delete(id)
}

/// Whether a connector secret is currently stored.
pub fn has_mcp_server_secret(id: &str) -> Result<bool> {
    #[cfg(test)]
    if let Ok(secret_dir) = std::env::var("MNEMA_MCP_SERVER_SECRET_DIR") {
        return McpServerSecretStore::new(FileMcpServerSecretStoreAdapter::new(secret_dir)).has(id);
    }

    McpServerSecretStore::new(PlatformKeychainMcpServerSecretStoreAdapter).has(id)
}

// errSecItemNotFound: the keychain has no entry for this service/account.
// Treated as "absent", not an error, on every read/delete path.
#[cfg(target_os = "macos")]
const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;

// Use the Keychain Services API directly rather than shelling out to
// `/usr/bin/security` — see `ai_provider_key_store` for the interactive-prompt /
// ACL rationale.
#[cfg(target_os = "macos")]
fn load_platform_secret(id: &str) -> Result<Option<String>> {
    match security_framework::passwords::get_generic_password(KEYCHAIN_SERVICE, id) {
        Ok(bytes) => {
            let secret = String::from_utf8_lossy(&bytes).trim().to_string();
            Ok((!secret.is_empty()).then_some(secret))
        }
        Err(error) if error.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(None),
        Err(error) => Err(AppInfraError::McpServerSecretStore(error.to_string())),
    }
}

#[cfg(target_os = "macos")]
fn store_platform_secret(id: &str, secret: &str) -> Result<()> {
    // `set_generic_password` creates or updates the entry, writing exactly these
    // bytes — no empty-store hazard, so no round-trip verification needed.
    security_framework::passwords::set_generic_password(KEYCHAIN_SERVICE, id, secret.as_bytes())
        .map_err(|error| AppInfraError::McpServerSecretStore(error.to_string()))
}

#[cfg(target_os = "macos")]
fn delete_platform_secret(id: &str) -> Result<()> {
    match security_framework::passwords::delete_generic_password(KEYCHAIN_SERVICE, id) {
        Ok(()) => Ok(()),
        // Deleting an absent secret is a no-op.
        Err(error) if error.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(()),
        Err(error) => Err(AppInfraError::McpServerSecretStore(error.to_string())),
    }
}

#[cfg(not(target_os = "macos"))]
fn load_platform_secret(_id: &str) -> Result<Option<String>> {
    Err(AppInfraError::McpServerSecretStore(
        "mcp server secret store is unsupported on this platform".to_string(),
    ))
}

#[cfg(not(target_os = "macos"))]
fn store_platform_secret(_id: &str, _secret: &str) -> Result<()> {
    Err(AppInfraError::McpServerSecretStore(
        "mcp server secret store is unsupported on this platform".to_string(),
    ))
}

#[cfg(not(target_os = "macos"))]
fn delete_platform_secret(_id: &str) -> Result<()> {
    Err(AppInfraError::McpServerSecretStore(
        "mcp server secret store is unsupported on this platform".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

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

    fn file_store(
        secret_dir: impl Into<PathBuf>,
    ) -> McpServerSecretStore<FileMcpServerSecretStoreAdapter> {
        McpServerSecretStore::new(FileMcpServerSecretStoreAdapter::new(secret_dir))
    }

    #[test]
    fn file_store_round_trips_store_and_load() {
        let secret_dir = TestDir::new("round-trip");
        let store = file_store(secret_dir.path());

        assert!(store.load("github").expect("load should succeed").is_none());

        store
            .store("github", "ghp_secret_token")
            .expect("store should succeed");

        assert_eq!(
            store.load("github").expect("load should succeed"),
            Some("ghp_secret_token".to_string())
        );

        let secret_path = secret_dir.path().join("github.secret");
        assert!(secret_path.exists());
    }

    #[test]
    fn file_store_reports_presence() {
        let secret_dir = TestDir::new("has");
        let store = file_store(secret_dir.path());

        assert!(!store.has("linear").expect("has should succeed"));

        store
            .store("linear", "lin_secret")
            .expect("store should succeed");

        assert!(store.has("linear").expect("has should succeed"));
    }

    #[test]
    fn file_store_treats_empty_secret_as_absent() {
        let secret_dir = TestDir::new("empty");
        let store = file_store(secret_dir.path());
        std::fs::write(secret_dir.path().join("github.secret"), "   ")
            .expect("empty secret should be written");

        assert!(store.load("github").expect("load should succeed").is_none());
        assert!(!store.has("github").expect("has should succeed"));
    }

    #[test]
    fn store_rejects_empty_or_whitespace_secret() {
        let secret_dir = TestDir::new("reject-empty");
        let store = file_store(secret_dir.path());

        assert!(store.store("github", "").is_err());
        assert!(store.store("github", "   ").is_err());
        assert!(store.load("github").expect("load should succeed").is_none());
    }

    #[test]
    fn file_store_trims_secret_on_read() {
        let secret_dir = TestDir::new("trim");
        let store = file_store(secret_dir.path());
        std::fs::write(secret_dir.path().join("github.secret"), "  ghp_secret_token\n")
            .expect("padded secret should be written");

        assert_eq!(
            store.load("github").expect("load should succeed"),
            Some("ghp_secret_token".to_string())
        );
    }

    #[test]
    fn file_store_deletes_existing_secret() {
        let secret_dir = TestDir::new("delete");
        let store = file_store(secret_dir.path());

        store
            .store("github", "ghp_secret_token")
            .expect("store should succeed");
        assert!(store.has("github").expect("has should succeed"));

        store.delete("github").expect("delete should succeed");

        assert!(!store.has("github").expect("has should succeed"));
        assert!(!secret_dir.path().join("github.secret").exists());
    }

    #[test]
    fn file_store_delete_missing_secret_is_noop() {
        let secret_dir = TestDir::new("delete-missing");
        let store = file_store(secret_dir.path());

        store
            .delete("github")
            .expect("deleting an absent secret should succeed");
    }

    #[test]
    fn public_api_uses_env_file_fallback() {
        let secret_dir = TestDir::new("public-api");
        std::env::set_var("MNEMA_MCP_SERVER_SECRET_DIR", secret_dir.path());

        assert!(!has_mcp_server_secret("linear").expect("has should succeed"));

        store_mcp_server_secret("linear", "lin_public").expect("store should succeed");
        assert!(has_mcp_server_secret("linear").expect("has should succeed"));
        assert_eq!(
            load_mcp_server_secret("linear").expect("load should succeed"),
            Some("lin_public".to_string())
        );

        delete_mcp_server_secret("linear").expect("delete should succeed");
        assert!(!has_mcp_server_secret("linear").expect("has should succeed"));

        std::env::remove_var("MNEMA_MCP_SERVER_SECRET_DIR");
    }
}
