#[cfg(test)]
use std::path::PathBuf;

use crate::error::{AppInfraError, Result};

const KEYCHAIN_SERVICE: &str = "com.shaikzeeshan.mnema.licensing";

// The two logical entries this store holds, keyed by keychain account string.
// The raw signed license key (present only when activated) and the signed trial
// record (written at first successful Capture). Keychain survives uninstall,
// defeating a casual reinstall-reset of the trial (ADR 0045).
const LICENSE_KEY_ACCOUNT: &str = "license_key";
const TRIAL_RECORD_ACCOUNT: &str = "trial_record";
// Once-per-machine activation (ADR 0053): the signed receipt (wire string) and
// the provisional-window record (JSON). Both survive uninstall like the others.
const ACTIVATION_RECEIPT_ACCOUNT: &str = "activation_receipt";
const ACTIVATION_STATE_ACCOUNT: &str = "activation_state";

trait LicenseTokenStoreAdapter {
    fn load_token(&self, account: &str) -> Result<Option<String>>;
    fn store_token(&self, account: &str, token: &str) -> Result<()>;
    fn delete_token(&self, account: &str) -> Result<()>;
}

struct LicenseTokenStore<A> {
    adapter: A,
}

impl<A> LicenseTokenStore<A>
where
    A: LicenseTokenStoreAdapter,
{
    fn new(adapter: A) -> Self {
        Self { adapter }
    }

    fn store(&self, account: &str, token: &str) -> Result<()> {
        if token.trim().is_empty() {
            return Err(AppInfraError::LicenseTokenStore(
                "license token must not be empty".to_string(),
            ));
        }
        self.adapter.store_token(account, token)
    }

    fn load(&self, account: &str) -> Result<Option<String>> {
        self.adapter.load_token(account)
    }

    fn delete(&self, account: &str) -> Result<()> {
        self.adapter.delete_token(account)
    }

    fn has(&self, account: &str) -> Result<bool> {
        Ok(self.adapter.load_token(account)?.is_some())
    }
}

// The plaintext file-backed store exists only for tests; release builds must
// route every token through the OS keychain (the "tokens live ONLY in the
// keychain" invariant), so it is compiled out entirely outside `cfg(test)`.
#[cfg(test)]
#[derive(Debug, Clone)]
struct FileLicenseTokenStoreAdapter {
    token_dir: PathBuf,
}

#[cfg(test)]
impl FileLicenseTokenStoreAdapter {
    fn new(token_dir: impl Into<PathBuf>) -> Self {
        Self {
            token_dir: token_dir.into(),
        }
    }

    fn token_path(&self, account: &str) -> PathBuf {
        self.token_dir.join(format!("{account}.token"))
    }
}

#[cfg(test)]
impl LicenseTokenStoreAdapter for FileLicenseTokenStoreAdapter {
    fn load_token(&self, account: &str) -> Result<Option<String>> {
        std::fs::create_dir_all(&self.token_dir)?;
        let path = self.token_path(account);
        if !path.exists() {
            return Ok(None);
        }

        let token = std::fs::read_to_string(path)?;
        let token = token.trim();
        if token.is_empty() {
            return Ok(None);
        }
        Ok(Some(token.to_string()))
    }

    fn store_token(&self, account: &str, token: &str) -> Result<()> {
        std::fs::create_dir_all(&self.token_dir)?;
        std::fs::write(self.token_path(account), token)?;
        Ok(())
    }

    fn delete_token(&self, account: &str) -> Result<()> {
        let path = self.token_path(account);
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct PlatformKeychainLicenseTokenStoreAdapter;

impl LicenseTokenStoreAdapter for PlatformKeychainLicenseTokenStoreAdapter {
    fn load_token(&self, account: &str) -> Result<Option<String>> {
        load_platform_token(account)
    }

    fn store_token(&self, account: &str, token: &str) -> Result<()> {
        store_platform_token(account, token)
    }

    fn delete_token(&self, account: &str) -> Result<()> {
        delete_platform_token(account)
    }
}

/// Store the raw signed license key text in the OS keychain.
pub fn store_license_key(key: &str) -> Result<()> {
    #[cfg(test)]
    if let Ok(token_dir) = std::env::var("MNEMA_LICENSE_TOKEN_DIR") {
        return LicenseTokenStore::new(FileLicenseTokenStoreAdapter::new(token_dir))
            .store(LICENSE_KEY_ACCOUNT, key);
    }

    LicenseTokenStore::new(PlatformKeychainLicenseTokenStoreAdapter).store(LICENSE_KEY_ACCOUNT, key)
}

/// Load the stored license key, or `None` when the app is not activated.
pub fn load_license_key() -> Result<Option<String>> {
    #[cfg(test)]
    if let Ok(token_dir) = std::env::var("MNEMA_LICENSE_TOKEN_DIR") {
        return LicenseTokenStore::new(FileLicenseTokenStoreAdapter::new(token_dir))
            .load(LICENSE_KEY_ACCOUNT);
    }

    LicenseTokenStore::new(PlatformKeychainLicenseTokenStoreAdapter).load(LICENSE_KEY_ACCOUNT)
}

/// Delete the stored license key. A missing key is treated as success.
pub fn delete_license_key() -> Result<()> {
    #[cfg(test)]
    if let Ok(token_dir) = std::env::var("MNEMA_LICENSE_TOKEN_DIR") {
        return LicenseTokenStore::new(FileLicenseTokenStoreAdapter::new(token_dir))
            .delete(LICENSE_KEY_ACCOUNT);
    }

    LicenseTokenStore::new(PlatformKeychainLicenseTokenStoreAdapter).delete(LICENSE_KEY_ACCOUNT)
}

/// Whether a license key is currently stored.
pub fn has_license_key() -> Result<bool> {
    #[cfg(test)]
    if let Ok(token_dir) = std::env::var("MNEMA_LICENSE_TOKEN_DIR") {
        return LicenseTokenStore::new(FileLicenseTokenStoreAdapter::new(token_dir))
            .has(LICENSE_KEY_ACCOUNT);
    }

    LicenseTokenStore::new(PlatformKeychainLicenseTokenStoreAdapter).has(LICENSE_KEY_ACCOUNT)
}

/// Store the signed trial record text in the OS keychain.
pub fn store_trial_record(record: &str) -> Result<()> {
    #[cfg(test)]
    if let Ok(token_dir) = std::env::var("MNEMA_LICENSE_TOKEN_DIR") {
        return LicenseTokenStore::new(FileLicenseTokenStoreAdapter::new(token_dir))
            .store(TRIAL_RECORD_ACCOUNT, record);
    }

    LicenseTokenStore::new(PlatformKeychainLicenseTokenStoreAdapter)
        .store(TRIAL_RECORD_ACCOUNT, record)
}

/// Delete the stored trial record. A missing record is treated as success.
/// Dev-only test knob (`MNEMA_TRIAL_RESET`); production never un-starts a trial.
pub fn delete_trial_record() -> Result<()> {
    #[cfg(test)]
    if let Ok(token_dir) = std::env::var("MNEMA_LICENSE_TOKEN_DIR") {
        return LicenseTokenStore::new(FileLicenseTokenStoreAdapter::new(token_dir))
            .delete(TRIAL_RECORD_ACCOUNT);
    }

    LicenseTokenStore::new(PlatformKeychainLicenseTokenStoreAdapter).delete(TRIAL_RECORD_ACCOUNT)
}

/// Load the stored trial record, or `None` when the trial has never started.
pub fn load_trial_record() -> Result<Option<String>> {
    #[cfg(test)]
    if let Ok(token_dir) = std::env::var("MNEMA_LICENSE_TOKEN_DIR") {
        return LicenseTokenStore::new(FileLicenseTokenStoreAdapter::new(token_dir))
            .load(TRIAL_RECORD_ACCOUNT);
    }

    LicenseTokenStore::new(PlatformKeychainLicenseTokenStoreAdapter).load(TRIAL_RECORD_ACCOUNT)
}

/// Store the signed activation receipt (wire string) in the OS keychain.
pub fn store_activation_receipt(receipt: &str) -> Result<()> {
    #[cfg(test)]
    if let Ok(token_dir) = std::env::var("MNEMA_LICENSE_TOKEN_DIR") {
        return LicenseTokenStore::new(FileLicenseTokenStoreAdapter::new(token_dir))
            .store(ACTIVATION_RECEIPT_ACCOUNT, receipt);
    }

    LicenseTokenStore::new(PlatformKeychainLicenseTokenStoreAdapter)
        .store(ACTIVATION_RECEIPT_ACCOUNT, receipt)
}

/// Load the stored activation receipt, or `None` when never activated.
pub fn load_activation_receipt() -> Result<Option<String>> {
    #[cfg(test)]
    if let Ok(token_dir) = std::env::var("MNEMA_LICENSE_TOKEN_DIR") {
        return LicenseTokenStore::new(FileLicenseTokenStoreAdapter::new(token_dir))
            .load(ACTIVATION_RECEIPT_ACCOUNT);
    }

    LicenseTokenStore::new(PlatformKeychainLicenseTokenStoreAdapter)
        .load(ACTIVATION_RECEIPT_ACCOUNT)
}

/// Delete the stored activation receipt. A missing receipt is treated as success.
pub fn delete_activation_receipt() -> Result<()> {
    #[cfg(test)]
    if let Ok(token_dir) = std::env::var("MNEMA_LICENSE_TOKEN_DIR") {
        return LicenseTokenStore::new(FileLicenseTokenStoreAdapter::new(token_dir))
            .delete(ACTIVATION_RECEIPT_ACCOUNT);
    }

    LicenseTokenStore::new(PlatformKeychainLicenseTokenStoreAdapter)
        .delete(ACTIVATION_RECEIPT_ACCOUNT)
}

/// Store the provisional activation-state record (JSON) in the OS keychain.
pub fn store_activation_state(state_json: &str) -> Result<()> {
    #[cfg(test)]
    if let Ok(token_dir) = std::env::var("MNEMA_LICENSE_TOKEN_DIR") {
        return LicenseTokenStore::new(FileLicenseTokenStoreAdapter::new(token_dir))
            .store(ACTIVATION_STATE_ACCOUNT, state_json);
    }

    LicenseTokenStore::new(PlatformKeychainLicenseTokenStoreAdapter)
        .store(ACTIVATION_STATE_ACCOUNT, state_json)
}

/// Load the stored activation-state record, or `None` when none is set.
pub fn load_activation_state() -> Result<Option<String>> {
    #[cfg(test)]
    if let Ok(token_dir) = std::env::var("MNEMA_LICENSE_TOKEN_DIR") {
        return LicenseTokenStore::new(FileLicenseTokenStoreAdapter::new(token_dir))
            .load(ACTIVATION_STATE_ACCOUNT);
    }

    LicenseTokenStore::new(PlatformKeychainLicenseTokenStoreAdapter).load(ACTIVATION_STATE_ACCOUNT)
}

/// Delete the stored activation-state record. A missing record is treated as success.
pub fn delete_activation_state() -> Result<()> {
    #[cfg(test)]
    if let Ok(token_dir) = std::env::var("MNEMA_LICENSE_TOKEN_DIR") {
        return LicenseTokenStore::new(FileLicenseTokenStoreAdapter::new(token_dir))
            .delete(ACTIVATION_STATE_ACCOUNT);
    }

    LicenseTokenStore::new(PlatformKeychainLicenseTokenStoreAdapter)
        .delete(ACTIVATION_STATE_ACCOUNT)
}

// errSecItemNotFound: the keychain has no entry for this service/account.
// Treated as "absent", not an error, on every read/delete path.
#[cfg(target_os = "macos")]
const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;

// Use the Keychain Services API directly rather than shelling out to
// `/usr/bin/security` (see `ai_provider_key_store` for the rationale — the CLI's
// `-w` prompt hangs on a tty, and the API adds this app to the item ACL).
#[cfg(target_os = "macos")]
fn load_platform_token(account: &str) -> Result<Option<String>> {
    match security_framework::passwords::get_generic_password(KEYCHAIN_SERVICE, account) {
        Ok(bytes) => {
            let token = String::from_utf8_lossy(&bytes).trim().to_string();
            Ok((!token.is_empty()).then_some(token))
        }
        Err(error) if error.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(None),
        Err(error) => Err(AppInfraError::LicenseTokenStore(error.to_string())),
    }
}

#[cfg(target_os = "macos")]
fn store_platform_token(account: &str, token: &str) -> Result<()> {
    security_framework::passwords::set_generic_password(KEYCHAIN_SERVICE, account, token.as_bytes())
        .map_err(|error| AppInfraError::LicenseTokenStore(error.to_string()))
}

#[cfg(target_os = "macos")]
fn delete_platform_token(account: &str) -> Result<()> {
    match security_framework::passwords::delete_generic_password(KEYCHAIN_SERVICE, account) {
        Ok(()) => Ok(()),
        // Deleting an absent token is a no-op.
        Err(error) if error.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(()),
        Err(error) => Err(AppInfraError::LicenseTokenStore(error.to_string())),
    }
}

#[cfg(not(target_os = "macos"))]
fn load_platform_token(_account: &str) -> Result<Option<String>> {
    Err(AppInfraError::LicenseTokenStore(
        "license token store is unsupported on this platform".to_string(),
    ))
}

#[cfg(not(target_os = "macos"))]
fn store_platform_token(_account: &str, _token: &str) -> Result<()> {
    Err(AppInfraError::LicenseTokenStore(
        "license token store is unsupported on this platform".to_string(),
    ))
}

#[cfg(not(target_os = "macos"))]
fn delete_platform_token(_account: &str) -> Result<()> {
    Err(AppInfraError::LicenseTokenStore(
        "license token store is unsupported on this platform".to_string(),
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
            let path = std::env::temp_dir().join(format!("license-token-store-{label}-{unique}"));

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
        token_dir: impl Into<PathBuf>,
    ) -> LicenseTokenStore<FileLicenseTokenStoreAdapter> {
        LicenseTokenStore::new(FileLicenseTokenStoreAdapter::new(token_dir))
    }

    #[test]
    fn file_store_round_trips_store_and_load() {
        let token_dir = TestDir::new("round-trip");
        let store = file_store(token_dir.path());

        assert!(store
            .load(LICENSE_KEY_ACCOUNT)
            .expect("load should succeed")
            .is_none());

        store
            .store(LICENSE_KEY_ACCOUNT, "payload.signature")
            .expect("store should succeed");

        assert_eq!(
            store
                .load(LICENSE_KEY_ACCOUNT)
                .expect("load should succeed"),
            Some("payload.signature".to_string())
        );

        let token_path = token_dir.path().join("license_key.token");
        assert!(token_path.exists());
    }

    #[test]
    fn file_store_reports_presence() {
        let token_dir = TestDir::new("has");
        let store = file_store(token_dir.path());

        assert!(!store.has(LICENSE_KEY_ACCOUNT).expect("has should succeed"));

        store
            .store(LICENSE_KEY_ACCOUNT, "payload.signature")
            .expect("store should succeed");

        assert!(store.has(LICENSE_KEY_ACCOUNT).expect("has should succeed"));
    }

    #[test]
    fn file_store_treats_empty_token_as_absent() {
        let token_dir = TestDir::new("empty");
        let store = file_store(token_dir.path());
        std::fs::write(token_dir.path().join("license_key.token"), "   ")
            .expect("empty token should be written");

        assert!(store
            .load(LICENSE_KEY_ACCOUNT)
            .expect("load should succeed")
            .is_none());
        assert!(!store.has(LICENSE_KEY_ACCOUNT).expect("has should succeed"));
    }

    #[test]
    fn store_rejects_empty_or_whitespace_token() {
        let token_dir = TestDir::new("reject-empty");
        let store = file_store(token_dir.path());

        assert!(store.store(LICENSE_KEY_ACCOUNT, "").is_err());
        assert!(store.store(LICENSE_KEY_ACCOUNT, "   ").is_err());
        assert!(store
            .load(LICENSE_KEY_ACCOUNT)
            .expect("load should succeed")
            .is_none());
    }

    #[test]
    fn file_store_trims_token_on_read() {
        let token_dir = TestDir::new("trim");
        let store = file_store(token_dir.path());
        std::fs::write(
            token_dir.path().join("trial_record.token"),
            "  signed-trial-record\n",
        )
        .expect("padded token should be written");

        assert_eq!(
            store
                .load(TRIAL_RECORD_ACCOUNT)
                .expect("load should succeed"),
            Some("signed-trial-record".to_string())
        );
    }

    #[test]
    fn file_store_deletes_existing_token() {
        let token_dir = TestDir::new("delete");
        let store = file_store(token_dir.path());

        store
            .store(LICENSE_KEY_ACCOUNT, "payload.signature")
            .expect("store should succeed");
        assert!(store.has(LICENSE_KEY_ACCOUNT).expect("has should succeed"));

        store
            .delete(LICENSE_KEY_ACCOUNT)
            .expect("delete should succeed");

        assert!(!store.has(LICENSE_KEY_ACCOUNT).expect("has should succeed"));
        assert!(!token_dir.path().join("license_key.token").exists());
    }

    #[test]
    fn file_store_delete_missing_token_is_noop() {
        let token_dir = TestDir::new("delete-missing");
        let store = file_store(token_dir.path());

        store
            .delete(LICENSE_KEY_ACCOUNT)
            .expect("deleting an absent token should succeed");
    }

    #[test]
    fn public_api_uses_env_file_fallback() {
        let token_dir = TestDir::new("public-api");
        std::env::set_var("MNEMA_LICENSE_TOKEN_DIR", token_dir.path());

        assert!(!has_license_key().expect("has should succeed"));

        store_license_key("payload.signature").expect("store should succeed");
        assert!(has_license_key().expect("has should succeed"));
        assert_eq!(
            load_license_key().expect("load should succeed"),
            Some("payload.signature".to_string())
        );

        store_trial_record("signed-trial-record").expect("store should succeed");
        assert_eq!(
            load_trial_record().expect("load should succeed"),
            Some("signed-trial-record".to_string())
        );

        // Activation receipt + state are distinct accounts with their own round-trip.
        assert!(load_activation_receipt()
            .expect("load should succeed")
            .is_none());
        store_activation_receipt("receipt.payload.sig").expect("store should succeed");
        assert_eq!(
            load_activation_receipt().expect("load should succeed"),
            Some("receipt.payload.sig".to_string())
        );
        store_activation_state(r#"{"license_id":"order:x","provisional_started_at_ms":1}"#)
            .expect("store should succeed");
        assert_eq!(
            load_activation_state().expect("load should succeed"),
            Some(r#"{"license_id":"order:x","provisional_started_at_ms":1}"#.to_string())
        );
        delete_activation_receipt().expect("delete should succeed");
        assert!(load_activation_receipt()
            .expect("load should succeed")
            .is_none());
        delete_activation_state().expect("delete should succeed");
        assert!(load_activation_state()
            .expect("load should succeed")
            .is_none());

        delete_license_key().expect("delete should succeed");
        assert!(!has_license_key().expect("has should succeed"));
        // The trial record is a distinct account and survives license deletion.
        assert!(load_trial_record().expect("load should succeed").is_some());

        std::env::remove_var("MNEMA_LICENSE_TOKEN_DIR");
    }
}
