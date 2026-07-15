//! Shared, process-cached handle over the [`crate::secret_vault`].
//!
//! The handle is created cheaply (no keychain touch) when [`crate::AppInfra`]
//! is built and unlocks the vault at most **once** per process: either eagerly
//! via [`SecretVaultHandle::unlock_now`] (the desktop app calls this once at
//! startup) or lazily on the first secret access. The unlock outcome —
//! including a *Denied* one — is cached for the process lifetime, so a denied
//! keychain prompt never re-prompts: every subsequent store read/write returns
//! [`AppInfraError::SecretVaultDenied`] immediately.
//!
//! A process-global slot lets the existing free-function store APIs
//! (`load_ai_provider_key`, `load_mcp_server_secret`, …) keep their signatures:
//! `AppInfra` installs its handle into the slot at build time, and the store
//! wrappers read through it.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError, RwLock};

use crate::error::{AppInfraError, Result};
use crate::secret_vault::{
    unlock_secret_vault, unlock_secret_vault_with_source, MasterKeySource, SecretVault,
    SecretVaultUnlock,
};

/// Cloneable handle over the vault. All clones share one unlock state and one
/// in-memory secret map; `set`/`delete` rewrite the vault file atomically under
/// the shared lock.
#[derive(Clone)]
pub struct SecretVaultHandle {
    inner: Arc<Mutex<HandleInner>>,
}

struct HandleInner {
    save_dir: PathBuf,
    /// Injected master-key source (tests/dev). `None` = the default chain in
    /// [`unlock_secret_vault`] (keychain, or `MNEMA_DEV_MASTER_KEY_FILE` in
    /// debug builds).
    source: Option<Arc<dyn MasterKeySource + Send + Sync>>,
    state: VaultState,
}

enum VaultState {
    /// Not yet unlocked; no keychain access has happened.
    Locked,
    Ready(SecretVault),
    /// Cached for the process lifetime — never re-attempted per call.
    Denied(String),
}

impl SecretVaultHandle {
    /// Handle over the vault in `save_dir`, using the default master-key source.
    pub fn new(save_dir: impl Into<PathBuf>) -> Self {
        Self::build(save_dir, None)
    }

    /// Handle with an injected master-key source (tests, dev harnesses).
    pub fn with_source(
        save_dir: impl Into<PathBuf>,
        source: Arc<dyn MasterKeySource + Send + Sync>,
    ) -> Self {
        Self::build(save_dir, Some(source))
    }

    fn build(
        save_dir: impl Into<PathBuf>,
        source: Option<Arc<dyn MasterKeySource + Send + Sync>>,
    ) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HandleInner {
                save_dir: save_dir.into(),
                source,
                state: VaultState::Locked,
            })),
        }
    }

    /// Perform the once-per-process unlock now (idempotent). Returns
    /// [`AppInfraError::SecretVaultDenied`] when the vault is (now cached as)
    /// denied. The desktop app calls this once at startup so the single
    /// keychain prompt happens at a predictable moment.
    pub fn unlock_now(&self) -> Result<()> {
        self.with_inner(|inner| match &inner.state {
            VaultState::Ready(_) => Ok(()),
            VaultState::Denied(reason) => Err(AppInfraError::SecretVaultDenied(reason.clone())),
            VaultState::Locked => unreachable!("ensure_unlocked leaves no Locked state"),
        })
    }

    /// Read a secret. `Ok(None)` = no secret stored (distinct from denied).
    pub fn get(&self, account: &str) -> Result<Option<String>> {
        self.with_vault(|vault| Ok(vault.get(account).map(str::to_string)))
    }

    pub fn set(&self, account: &str, secret: &str) -> Result<()> {
        self.with_vault(|vault| vault.set(account, secret))
    }

    pub fn delete(&self, account: &str) -> Result<()> {
        self.with_vault(|vault| vault.delete(account))
    }

    fn with_vault<T>(&self, action: impl FnOnce(&mut SecretVault) -> Result<T>) -> Result<T> {
        self.with_inner(|inner| match &mut inner.state {
            VaultState::Ready(vault) => action(vault),
            VaultState::Denied(reason) => Err(AppInfraError::SecretVaultDenied(reason.clone())),
            VaultState::Locked => unreachable!("ensure_unlocked leaves no Locked state"),
        })
    }

    fn with_inner<T>(&self, action: impl FnOnce(&mut HandleInner) -> Result<T>) -> Result<T> {
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        inner.ensure_unlocked();
        action(&mut inner)
    }
}

impl std::fmt::Debug for SecretVaultHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SecretVaultHandle")
            .finish_non_exhaustive()
    }
}

impl HandleInner {
    fn ensure_unlocked(&mut self) {
        if !matches!(self.state, VaultState::Locked) {
            return;
        }
        let default_source = self.source.is_none();
        let unlock = match &self.source {
            Some(source) => unlock_secret_vault_with_source(&self.save_dir, source.as_ref()),
            None => unlock_secret_vault(&self.save_dir),
        };
        self.state = match unlock {
            // `Missing` is first-run: a fresh empty vault, usable like Unlocked.
            Ok(SecretVaultUnlock::Unlocked(vault)) | Ok(SecretVaultUnlock::Missing(vault)) => {
                VaultState::Ready(vault)
            }
            Ok(SecretVaultUnlock::Denied(reason)) => VaultState::Denied(reason),
            // ponytail: hard unlock errors (corrupt/undecryptable vault file, io)
            // cache as Denied too — same "vault unavailable, do not retry per
            // call, never conflate with 'no key stored'" surface slice 5 renders.
            Err(error) => VaultState::Denied(error.to_string()),
        };
        // Legacy keychain → vault migration runs exactly once per process, on
        // ANY unlock path (eager `unlock_now` or lazy first access), because
        // this Locked→Ready transition happens exactly once. Only handles on
        // the default source migrate: injected-source handles (tests, dev
        // harnesses) must never touch the real keychain.
        if default_source {
            if let VaultState::Ready(vault) = &mut self.state {
                crate::secret_vault_migration::run_default_migration(vault);
            }
        }
    }
}

/// The process-global vault slot the free-function store wrappers read through.
static PROCESS_SECRET_VAULT: RwLock<Option<SecretVaultHandle>> = RwLock::new(None);

/// Install (or replace) the process-global vault handle. `AppInfra` installs
/// its handle via [`install_process_secret_vault_if_absent`]; this overwriting
/// variant exists for test harnesses that must pin the slot to a scratch vault.
pub fn install_process_secret_vault(handle: SecretVaultHandle) {
    *PROCESS_SECRET_VAULT
        .write()
        .unwrap_or_else(PoisonError::into_inner) = Some(handle);
}

/// Install the handle only when no handle is installed yet (first `AppInfra`
/// in the process wins; a test-installed handle is never clobbered).
pub(crate) fn install_process_secret_vault_if_absent(handle: &SecretVaultHandle) {
    let mut slot = PROCESS_SECRET_VAULT
        .write()
        .unwrap_or_else(PoisonError::into_inner);
    if slot.is_none() {
        *slot = Some(handle.clone());
    }
}

/// The installed process-global handle, or a `SecretVault` error when no
/// `AppInfra` has been initialized in this process.
pub(crate) fn process_secret_vault() -> Result<SecretVaultHandle> {
    PROCESS_SECRET_VAULT
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
        .ok_or_else(|| {
            AppInfraError::SecretVault(
                "secret vault is not initialized (no AppInfra in this process)".to_string(),
            )
        })
}

/// Install a single shared, file-key-backed process vault for this test binary
/// (idempotent). Tests exercising the free-function store APIs share it — the
/// slot is process-wide, so per-test installs would race under parallel tests.
#[cfg(test)]
pub(crate) fn install_shared_test_process_vault() {
    use std::sync::OnceLock;
    static INSTALLED: OnceLock<()> = OnceLock::new();
    INSTALLED.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!(
            "app-infra-shared-test-vault-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("shared test vault dir should be created");
        install_process_secret_vault(SecretVaultHandle::with_source(
            &dir,
            Arc::new(crate::secret_vault::FileMasterKeySource::new(
                dir.join("master.key"),
            )),
        ));
    });
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;
    use crate::secret_vault::FileMasterKeySource;

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(label: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time should be after unix epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!("secret-vault-handle-{label}-{unique}"));
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

    fn file_handle(dir: &TestDir) -> SecretVaultHandle {
        SecretVaultHandle::with_source(
            dir.path(),
            Arc::new(FileMasterKeySource::new(dir.path().join("master.key"))),
        )
    }

    #[test]
    fn handle_round_trips_and_clones_share_state() {
        let dir = TestDir::new("roundtrip");
        let handle = file_handle(&dir);
        assert_eq!(handle.get("acct").expect("get"), None);

        handle.set("acct", "secret").expect("set");
        let clone = handle.clone();
        assert_eq!(clone.get("acct").expect("get"), Some("secret".to_string()));

        clone.delete("acct").expect("delete");
        assert_eq!(handle.get("acct").expect("get"), None);
    }

    #[test]
    fn denied_unlock_is_cached_and_returns_the_denied_error() {
        let dir = TestDir::new("denied");
        let handle = SecretVaultHandle::with_source(dir.path(), Arc::new(DenyingMasterKeySource));

        for _ in 0..2 {
            let error = handle.get("acct").expect_err("denied vault must error");
            assert!(
                matches!(error, AppInfraError::SecretVaultDenied(_)),
                "expected SecretVaultDenied, got {error:?}"
            );
        }
        assert!(matches!(
            handle.unlock_now().expect_err("unlock_now reports denied"),
            AppInfraError::SecretVaultDenied(_)
        ));
        // The denial was cached: the source was consulted once, no vault file
        // was created, and no retry happened per call (behavioral: the calls
        // above would each error identically regardless).
        assert!(!dir.path().join(crate::SECRET_VAULT_FILE_NAME).exists());
    }

    #[test]
    fn unlock_now_is_idempotent_and_primes_the_vault() {
        let dir = TestDir::new("unlock-now");
        let handle = file_handle(&dir);
        handle.unlock_now().expect("first unlock");
        handle.unlock_now().expect("second unlock is a no-op");
        assert!(dir.path().join(crate::SECRET_VAULT_FILE_NAME).exists());
    }
}
