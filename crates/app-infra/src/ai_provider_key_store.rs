#[cfg(test)]
use std::path::PathBuf;

use crate::error::{AppInfraError, Result};

const KEYCHAIN_SERVICE: &str = "com.shaikzeeshan.mnema.ai-runtime";

trait AiProviderKeyStoreAdapter {
    fn load_key(&self, provider: &str) -> Result<Option<String>>;
    fn store_key(&self, provider: &str, key: &str) -> Result<()>;
    fn delete_key(&self, provider: &str) -> Result<()>;
}

struct AiProviderKeyStore<A> {
    adapter: A,
}

impl<A> AiProviderKeyStore<A>
where
    A: AiProviderKeyStoreAdapter,
{
    fn new(adapter: A) -> Self {
        Self { adapter }
    }

    fn store(&self, provider: &str, key: &str) -> Result<()> {
        if key.trim().is_empty() {
            return Err(AppInfraError::AiProviderKeyStore(
                "provider api key must not be empty".to_string(),
            ));
        }
        self.adapter.store_key(provider, key)
    }

    fn load(&self, provider: &str) -> Result<Option<String>> {
        self.adapter.load_key(provider)
    }

    fn delete(&self, provider: &str) -> Result<()> {
        self.adapter.delete_key(provider)
    }

    fn has(&self, provider: &str) -> Result<bool> {
        Ok(self.adapter.load_key(provider)?.is_some())
    }
}

// The plaintext file-backed key store exists only for tests; release builds
// must route every key through the OS keychain (the "keys live ONLY in the
// keychain" invariant), so it is compiled out entirely outside `cfg(test)`.
#[cfg(test)]
#[derive(Debug, Clone)]
struct FileAiProviderKeyStoreAdapter {
    key_dir: PathBuf,
}

#[cfg(test)]
impl FileAiProviderKeyStoreAdapter {
    fn new(key_dir: impl Into<PathBuf>) -> Self {
        Self {
            key_dir: key_dir.into(),
        }
    }

    fn key_path(&self, provider: &str) -> PathBuf {
        self.key_dir.join(format!("{provider}.key"))
    }
}

#[cfg(test)]
impl AiProviderKeyStoreAdapter for FileAiProviderKeyStoreAdapter {
    fn load_key(&self, provider: &str) -> Result<Option<String>> {
        std::fs::create_dir_all(&self.key_dir)?;
        let path = self.key_path(provider);
        if !path.exists() {
            return Ok(None);
        }

        let key = std::fs::read_to_string(path)?;
        let key = key.trim();
        if key.is_empty() {
            return Ok(None);
        }
        Ok(Some(key.to_string()))
    }

    fn store_key(&self, provider: &str, key: &str) -> Result<()> {
        std::fs::create_dir_all(&self.key_dir)?;
        std::fs::write(self.key_path(provider), key)?;
        Ok(())
    }

    fn delete_key(&self, provider: &str) -> Result<()> {
        let path = self.key_path(provider);
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct PlatformKeychainAiProviderKeyStoreAdapter;

impl AiProviderKeyStoreAdapter for PlatformKeychainAiProviderKeyStoreAdapter {
    fn load_key(&self, provider: &str) -> Result<Option<String>> {
        load_platform_key(provider)
    }

    fn store_key(&self, provider: &str, key: &str) -> Result<()> {
        store_platform_key(provider, key)
    }

    fn delete_key(&self, provider: &str) -> Result<()> {
        delete_platform_key(provider)
    }
}

/// Store the bring-your-own provider API key in the OS keychain, keyed by provider id.
pub fn store_ai_provider_key(provider: &str, key: &str) -> Result<()> {
    #[cfg(test)]
    if let Ok(key_dir) = std::env::var("MNEMA_AI_PROVIDER_KEY_DIR") {
        return AiProviderKeyStore::new(FileAiProviderKeyStoreAdapter::new(key_dir))
            .store(provider, key);
    }

    AiProviderKeyStore::new(PlatformKeychainAiProviderKeyStoreAdapter).store(provider, key)
}

/// Load the stored provider API key, or `None` when no key is stored for the provider.
pub fn load_ai_provider_key(provider: &str) -> Result<Option<String>> {
    #[cfg(test)]
    if let Ok(key_dir) = std::env::var("MNEMA_AI_PROVIDER_KEY_DIR") {
        return AiProviderKeyStore::new(FileAiProviderKeyStoreAdapter::new(key_dir)).load(provider);
    }

    AiProviderKeyStore::new(PlatformKeychainAiProviderKeyStoreAdapter).load(provider)
}

/// Delete the stored provider API key. A missing key is treated as success.
pub fn delete_ai_provider_key(provider: &str) -> Result<()> {
    #[cfg(test)]
    if let Ok(key_dir) = std::env::var("MNEMA_AI_PROVIDER_KEY_DIR") {
        return AiProviderKeyStore::new(FileAiProviderKeyStoreAdapter::new(key_dir))
            .delete(provider);
    }

    AiProviderKeyStore::new(PlatformKeychainAiProviderKeyStoreAdapter).delete(provider)
}

/// Whether a provider API key is currently stored.
pub fn has_ai_provider_key(provider: &str) -> Result<bool> {
    #[cfg(test)]
    if let Ok(key_dir) = std::env::var("MNEMA_AI_PROVIDER_KEY_DIR") {
        return AiProviderKeyStore::new(FileAiProviderKeyStoreAdapter::new(key_dir)).has(provider);
    }

    AiProviderKeyStore::new(PlatformKeychainAiProviderKeyStoreAdapter).has(provider)
}

// errSecItemNotFound: the keychain has no entry for this service/account.
// Treated as "absent", not an error, on every read/delete path.
#[cfg(target_os = "macos")]
const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;

// Use the Keychain Services API directly rather than shelling out to
// `/usr/bin/security`: the CLI's interactive `-w` prompt reads from `/dev/tty`
// when one is present (dev launched from a terminal), so a piped stdin is
// ignored and `add-generic-password` hangs forever. The API also adds THIS app
// to the item's ACL, so reads don't depend on a separate binary's trust grant.
#[cfg(target_os = "macos")]
fn load_platform_key(provider: &str) -> Result<Option<String>> {
    match security_framework::passwords::get_generic_password(KEYCHAIN_SERVICE, provider) {
        Ok(bytes) => {
            let key = String::from_utf8_lossy(&bytes).trim().to_string();
            Ok((!key.is_empty()).then_some(key))
        }
        Err(error) if error.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(None),
        Err(error) => Err(AppInfraError::AiProviderKeyStore(error.to_string())),
    }
}

#[cfg(target_os = "macos")]
fn store_platform_key(provider: &str, key: &str) -> Result<()> {
    // `set_generic_password` creates or updates the entry, writing exactly these
    // bytes — no empty-store hazard, so no round-trip verification needed.
    security_framework::passwords::set_generic_password(KEYCHAIN_SERVICE, provider, key.as_bytes())
        .map_err(|error| AppInfraError::AiProviderKeyStore(error.to_string()))
}

#[cfg(target_os = "macos")]
fn delete_platform_key(provider: &str) -> Result<()> {
    match security_framework::passwords::delete_generic_password(KEYCHAIN_SERVICE, provider) {
        Ok(()) => Ok(()),
        // Deleting an absent key is a no-op.
        Err(error) if error.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(()),
        Err(error) => Err(AppInfraError::AiProviderKeyStore(error.to_string())),
    }
}

// Windows Credential Manager backend, sharing the generic-credential helpers
// with `mcp_server_secret_store` (same "{service}:{account}" target-name and
// app-id user-name conventions as `capture_index_key_store`).
#[cfg(target_os = "windows")]
fn load_platform_key(provider: &str) -> Result<Option<String>> {
    crate::windows_credential_store::load_credential(
        KEYCHAIN_SERVICE,
        provider,
        "ai provider key",
        AppInfraError::AiProviderKeyStore,
    )
}

#[cfg(target_os = "windows")]
fn store_platform_key(provider: &str, key: &str) -> Result<()> {
    crate::windows_credential_store::store_credential(
        KEYCHAIN_SERVICE,
        provider,
        key,
        "ai provider key",
        AppInfraError::AiProviderKeyStore,
    )
}

#[cfg(target_os = "windows")]
fn delete_platform_key(provider: &str) -> Result<()> {
    crate::windows_credential_store::delete_credential(
        KEYCHAIN_SERVICE,
        provider,
        "ai provider key",
        AppInfraError::AiProviderKeyStore,
    )
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn load_platform_key(_provider: &str) -> Result<Option<String>> {
    Err(AppInfraError::AiProviderKeyStore(
        "ai provider key store is unsupported on this platform".to_string(),
    ))
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn store_platform_key(_provider: &str, _key: &str) -> Result<()> {
    Err(AppInfraError::AiProviderKeyStore(
        "ai provider key store is unsupported on this platform".to_string(),
    ))
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn delete_platform_key(_provider: &str) -> Result<()> {
    Err(AppInfraError::AiProviderKeyStore(
        "ai provider key store is unsupported on this platform".to_string(),
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
                std::env::temp_dir().join(format!("ai-provider-key-store-{label}-{unique}"));

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

    fn file_store(key_dir: impl Into<PathBuf>) -> AiProviderKeyStore<FileAiProviderKeyStoreAdapter> {
        AiProviderKeyStore::new(FileAiProviderKeyStoreAdapter::new(key_dir))
    }

    #[test]
    fn file_store_round_trips_store_and_load() {
        let key_dir = TestDir::new("round-trip");
        let store = file_store(key_dir.path());

        assert!(store.load("anthropic").expect("load should succeed").is_none());

        store
            .store("anthropic", "sk-secret-key")
            .expect("store should succeed");

        assert_eq!(
            store.load("anthropic").expect("load should succeed"),
            Some("sk-secret-key".to_string())
        );

        let key_path = key_dir.path().join("anthropic.key");
        assert!(key_path.exists());
    }

    #[test]
    fn file_store_reports_presence() {
        let key_dir = TestDir::new("has");
        let store = file_store(key_dir.path());

        assert!(!store.has("openai").expect("has should succeed"));

        store
            .store("openai", "sk-openai")
            .expect("store should succeed");

        assert!(store.has("openai").expect("has should succeed"));
    }

    #[test]
    fn file_store_treats_empty_key_as_absent() {
        let key_dir = TestDir::new("empty");
        let store = file_store(key_dir.path());
        std::fs::write(key_dir.path().join("anthropic.key"), "   ")
            .expect("empty key should be written");

        assert!(store.load("anthropic").expect("load should succeed").is_none());
        assert!(!store.has("anthropic").expect("has should succeed"));
    }

    #[test]
    fn store_rejects_empty_or_whitespace_key() {
        let key_dir = TestDir::new("reject-empty");
        let store = file_store(key_dir.path());

        assert!(store.store("anthropic", "").is_err());
        assert!(store.store("anthropic", "   ").is_err());
        assert!(store.load("anthropic").expect("load should succeed").is_none());
    }

    #[test]
    fn file_store_trims_key_on_read() {
        let key_dir = TestDir::new("trim");
        let store = file_store(key_dir.path());
        std::fs::write(key_dir.path().join("anthropic.key"), "  sk-secret-key\n")
            .expect("padded key should be written");

        assert_eq!(
            store.load("anthropic").expect("load should succeed"),
            Some("sk-secret-key".to_string())
        );
    }

    #[test]
    fn file_store_deletes_existing_key() {
        let key_dir = TestDir::new("delete");
        let store = file_store(key_dir.path());

        store
            .store("anthropic", "sk-secret-key")
            .expect("store should succeed");
        assert!(store.has("anthropic").expect("has should succeed"));

        store.delete("anthropic").expect("delete should succeed");

        assert!(!store.has("anthropic").expect("has should succeed"));
        assert!(!key_dir.path().join("anthropic.key").exists());
    }

    #[test]
    fn file_store_delete_missing_key_is_noop() {
        let key_dir = TestDir::new("delete-missing");
        let store = file_store(key_dir.path());

        store
            .delete("anthropic")
            .expect("deleting an absent key should succeed");
    }

    #[test]
    fn public_api_uses_env_file_fallback() {
        let key_dir = TestDir::new("public-api");
        std::env::set_var("MNEMA_AI_PROVIDER_KEY_DIR", key_dir.path());

        assert!(!has_ai_provider_key("openai").expect("has should succeed"));

        store_ai_provider_key("openai", "sk-public").expect("store should succeed");
        assert!(has_ai_provider_key("openai").expect("has should succeed"));
        assert_eq!(
            load_ai_provider_key("openai").expect("load should succeed"),
            Some("sk-public".to_string())
        );

        delete_ai_provider_key("openai").expect("delete should succeed");
        assert!(!has_ai_provider_key("openai").expect("has should succeed"));

        std::env::remove_var("MNEMA_AI_PROVIDER_KEY_DIR");
    }
}
