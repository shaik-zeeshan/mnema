#[cfg(test)]
use std::path::PathBuf;

use crate::error::{AppInfraError, Result};

const KEYCHAIN_SERVICE: &str = "day.mnema.licensing";
// Pre-rename service (installs before the day.mnema bundle-id change): each
// account is read once, migrated to the new service, then deleted — preserving
// the anti-reset stamps (first-seen, trial-issuance) across the rename.
#[cfg(target_os = "macos")]
const LEGACY_KEYCHAIN_SERVICE: &str = "com.shaikzeeshan.mnema.licensing";

// licensegate-era accounts (2026-07-16 migration). Old accounts (`license_key`,
// `trial_record`, `activation_receipt`, `activation_state`) held old-format
// artifacts and are deliberately NEVER read and NEVER deleted — an upgraded
// machine is indistinguishable from a fresh install to the new code. Keychain
// survives uninstall, defeating a casual reinstall-reset (ADR 0045/0053).
const LICENSE_KEY_ACCOUNT: &str = "licensegate_key";
const ACTIVATION_RECEIPT_ACCOUNT: &str = "licensegate_receipt";
const FIRST_SEEN_ACCOUNT: &str = "licensegate_first_seen";
const TRIAL_ISSUANCE_ACCOUNT: &str = "licensegate_trial_issuance";

trait LicenseTokenStoreAdapter {
    fn load_token(&self, account: &str) -> Result<Option<String>>;
    fn store_token(&self, account: &str, token: &str) -> Result<()>;
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
}

fn store(account: &str, token: &str) -> Result<()> {
    #[cfg(test)]
    if let Ok(token_dir) = std::env::var("MNEMA_LICENSE_TOKEN_DIR") {
        return LicenseTokenStore::new(FileLicenseTokenStoreAdapter::new(token_dir))
            .store(account, token);
    }

    LicenseTokenStore::new(PlatformKeychainLicenseTokenStoreAdapter).store(account, token)
}

fn load(account: &str) -> Result<Option<String>> {
    #[cfg(test)]
    if let Ok(token_dir) = std::env::var("MNEMA_LICENSE_TOKEN_DIR") {
        return LicenseTokenStore::new(FileLicenseTokenStoreAdapter::new(token_dir)).load(account);
    }

    LicenseTokenStore::new(PlatformKeychainLicenseTokenStoreAdapter).load(account)
}

/// Store the raw signed license key wire string in the OS keychain.
pub fn store_license_key(key: &str) -> Result<()> {
    store(LICENSE_KEY_ACCOUNT, key)
}

/// Load the stored license key, or `None` when no key is installed.
pub fn load_license_key() -> Result<Option<String>> {
    load(LICENSE_KEY_ACCOUNT)
}

/// Store the signed activation receipt (wire string) in the OS keychain.
pub fn store_activation_receipt(receipt: &str) -> Result<()> {
    store(ACTIVATION_RECEIPT_ACCOUNT, receipt)
}

/// Load the stored activation receipt, or `None` when never activated.
pub fn load_activation_receipt() -> Result<Option<String>> {
    load(ACTIVATION_RECEIPT_ACCOUNT)
}

/// Store the first-seen record (JSON `{license_id, first_seen_at_ms}`) beside
/// the key. Write-once-per-license-id policy is the caller's job (the adapter
/// only replaces it when the stored record is for a different license id).
pub fn store_first_seen(record_json: &str) -> Result<()> {
    store(FIRST_SEEN_ACCOUNT, record_json)
}

/// Load the stored first-seen record, or `None` when no key was ever stored.
pub fn load_first_seen() -> Result<Option<String>> {
    load(FIRST_SEEN_ACCOUNT)
}

/// Store the trial-issuance stamp (JSON `{first_attempt_at_ms, used}`).
/// Write-once policy for the timestamp is the caller's job.
pub fn store_trial_issuance(record_json: &str) -> Result<()> {
    store(TRIAL_ISSUANCE_ACCOUNT, record_json)
}

/// Load the trial-issuance stamp, or `None` when issuance was never attempted.
pub fn load_trial_issuance() -> Result<Option<String>> {
    load(TRIAL_ISSUANCE_ACCOUNT)
}

/// Remove the trial-issuance stamp. Dev-only reset knob (`MNEMA_TRIAL_RESET`);
/// an absent entry is success.
// ponytail: standalone delete instead of a trait method — one caller, one account.
pub fn clear_trial_issuance() -> Result<()> {
    #[cfg(test)]
    if let Ok(token_dir) = std::env::var("MNEMA_LICENSE_TOKEN_DIR") {
        let path = PathBuf::from(token_dir).join(format!("{TRIAL_ISSUANCE_ACCOUNT}.token"));
        return match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        };
    }
    delete_platform_token(TRIAL_ISSUANCE_ACCOUNT)
}

// errSecItemNotFound: the keychain has no entry for this service/account.
// Treated as "absent", not an error, on every read path.
#[cfg(target_os = "macos")]
const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;

// Use the Keychain Services API directly rather than shelling out to
// `/usr/bin/security` (see `ai_provider_key_store` for the rationale — the CLI's
// `-w` prompt hangs on a tty, and the API adds this app to the item ACL).
#[cfg(target_os = "macos")]
fn read_service_token(service: &str, account: &str) -> Result<Option<String>> {
    match security_framework::passwords::get_generic_password(service, account) {
        Ok(bytes) => {
            let token = String::from_utf8_lossy(&bytes).trim().to_string();
            Ok((!token.is_empty()).then_some(token))
        }
        Err(error) if error.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(None),
        Err(error) => Err(AppInfraError::LicenseTokenStore(error.to_string())),
    }
}

#[cfg(target_os = "macos")]
fn load_platform_token(account: &str) -> Result<Option<String>> {
    if let Some(token) = read_service_token(KEYCHAIN_SERVICE, account)? {
        return Ok(Some(token));
    }
    let Some(token) = read_service_token(LEGACY_KEYCHAIN_SERVICE, account)? else {
        return Ok(None);
    };
    store_platform_token(account, &token)?;
    let _ = security_framework::passwords::delete_generic_password(LEGACY_KEYCHAIN_SERVICE, account);
    Ok(Some(token))
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
        Err(error) if error.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(()),
        Err(error) => Err(AppInfraError::LicenseTokenStore(error.to_string())),
    }
}

// Nothing is ever stored on non-macOS (store/load error), so there is nothing
// to delete — success keeps the dev reset knob harmless there.
#[cfg(not(target_os = "macos"))]
fn delete_platform_token(_account: &str) -> Result<()> {
    Ok(())
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
    fn accounts_are_the_new_licensegate_names_never_the_old_ones() {
        // The migration contract: new-format artifacts live under NEW account
        // names; the old accounts are never read (an upgraded machine looks
        // fresh). A rename back to an old account would silently resurrect
        // old-format blobs into the new parser.
        for (account, old) in [
            (LICENSE_KEY_ACCOUNT, "license_key"),
            (ACTIVATION_RECEIPT_ACCOUNT, "activation_receipt"),
            (FIRST_SEEN_ACCOUNT, "activation_state"),
            (TRIAL_ISSUANCE_ACCOUNT, "trial_record"),
        ] {
            assert!(account.starts_with("licensegate_"), "{account}");
            assert_ne!(account, old);
        }
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

        let token_path = token_dir.path().join("licensegate_key.token");
        assert!(token_path.exists());
    }

    #[test]
    fn file_store_treats_empty_token_as_absent() {
        let token_dir = TestDir::new("empty");
        let store = file_store(token_dir.path());
        std::fs::write(token_dir.path().join("licensegate_key.token"), "   ")
            .expect("empty token should be written");

        assert!(store
            .load(LICENSE_KEY_ACCOUNT)
            .expect("load should succeed")
            .is_none());
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
            token_dir.path().join("licensegate_receipt.token"),
            "  receipt.sig\n",
        )
        .expect("padded token should be written");

        assert_eq!(
            store
                .load(ACTIVATION_RECEIPT_ACCOUNT)
                .expect("load should succeed"),
            Some("receipt.sig".to_string())
        );
    }

    #[test]
    fn public_api_uses_env_file_fallback_with_distinct_accounts() {
        let token_dir = TestDir::new("public-api");
        std::env::set_var("MNEMA_LICENSE_TOKEN_DIR", token_dir.path());

        assert!(load_license_key().expect("load should succeed").is_none());
        store_license_key("payload.signature").expect("store should succeed");
        assert_eq!(
            load_license_key().expect("load should succeed"),
            Some("payload.signature".to_string())
        );

        assert!(load_activation_receipt()
            .expect("load should succeed")
            .is_none());
        store_activation_receipt("receipt.payload.sig").expect("store should succeed");
        assert_eq!(
            load_activation_receipt().expect("load should succeed"),
            Some("receipt.payload.sig".to_string())
        );

        assert!(load_first_seen().expect("load should succeed").is_none());
        store_first_seen(r#"{"license_id":"01J","first_seen_at_ms":1}"#)
            .expect("store should succeed");
        assert_eq!(
            load_first_seen().expect("load should succeed"),
            Some(r#"{"license_id":"01J","first_seen_at_ms":1}"#.to_string())
        );

        // Trial-issuance stamp: round-trips, and clear removes it (clearing an
        // already-absent stamp stays success — the dev reset knob is idempotent).
        clear_trial_issuance().expect("clearing an absent stamp should succeed");
        assert!(load_trial_issuance().expect("load should succeed").is_none());
        store_trial_issuance(r#"{"first_attempt_at_ms":42,"used":false}"#)
            .expect("store should succeed");
        assert_eq!(
            load_trial_issuance().expect("load should succeed"),
            Some(r#"{"first_attempt_at_ms":42,"used":false}"#.to_string())
        );
        clear_trial_issuance().expect("clear should succeed");
        assert!(load_trial_issuance().expect("load should succeed").is_none());

        std::env::remove_var("MNEMA_LICENSE_TOKEN_DIR");
    }
}
