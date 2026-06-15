#[cfg(test)]
use std::path::PathBuf;

#[cfg(target_os = "macos")]
use std::process::Command;

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

#[cfg(target_os = "macos")]
fn load_platform_key(provider: &str) -> Result<Option<String>> {
    let lookup = Command::new("security")
        .args([
            "find-generic-password",
            "-s",
            KEYCHAIN_SERVICE,
            "-a",
            provider,
            "-w",
        ])
        .output()?;
    if !lookup.status.success() {
        return Ok(None);
    }

    let key = String::from_utf8_lossy(&lookup.stdout).trim().to_string();
    if key.is_empty() {
        return Ok(None);
    }
    Ok(Some(key))
}

#[cfg(target_os = "macos")]
fn store_platform_key(provider: &str, key: &str) -> Result<()> {
    use std::io::Write;
    use std::process::Stdio;

    // `-w` with no value keeps the secret off the subprocess argv (visible to
    // same-user `ps`), but it does NOT take a single value from stdin: `security`
    // runs an interactive "password" + "retype password" confirmation prompt and
    // reads BOTH from stdin. Feeding the key once leaves the retype empty, which
    // `security` resolves by silently storing an EMPTY password while still
    // exiting 0 — so the key must be written twice and the store verified.
    let mut child = Command::new("security")
        .args([
            "add-generic-password",
            "-U",
            "-s",
            KEYCHAIN_SERVICE,
            "-a",
            provider,
            "-w",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    {
        let mut stdin = child
            .stdin
            .take()
            .expect("stdin was requested via Stdio::piped");
        // Password line, then the retype confirmation line.
        writeln!(stdin, "{key}")?;
        writeln!(stdin, "{key}")?;
    }

    let add = child.wait_with_output()?;
    if !add.status.success() {
        return Err(AppInfraError::AiProviderKeyStore(
            String::from_utf8_lossy(&add.stderr).trim().to_string(),
        ));
    }

    // `security` reports success even when the prompts left it storing an empty
    // value, so confirm the key actually round-trips before declaring success.
    match load_platform_key(provider)? {
        Some(stored) if stored == key.trim() => Ok(()),
        _ => Err(AppInfraError::AiProviderKeyStore(
            "keychain reported success but did not store the provider key".to_string(),
        )),
    }
}

#[cfg(target_os = "macos")]
fn delete_platform_key(provider: &str) -> Result<()> {
    let delete = Command::new("security")
        .args([
            "delete-generic-password",
            "-s",
            KEYCHAIN_SERVICE,
            "-a",
            provider,
        ])
        .output()?;
    if !delete.status.success() {
        let stderr = String::from_utf8_lossy(&delete.stderr);
        // A missing entry is not an error: deleting an absent key is a no-op.
        if stderr.contains("could not be found") {
            return Ok(());
        }
        return Err(AppInfraError::AiProviderKeyStore(stderr.trim().to_string()));
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn load_platform_key(_provider: &str) -> Result<Option<String>> {
    Err(AppInfraError::AiProviderKeyStore(
        "ai provider key store is unsupported on this platform".to_string(),
    ))
}

#[cfg(not(target_os = "macos"))]
fn store_platform_key(_provider: &str, _key: &str) -> Result<()> {
    Err(AppInfraError::AiProviderKeyStore(
        "ai provider key store is unsupported on this platform".to_string(),
    ))
}

#[cfg(not(target_os = "macos"))]
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
