use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use rand::RngCore;

use crate::error::{AppInfraError, Result};

pub const SECRET_VAULT_FILE_NAME: &str = "secrets.vault";

const VAULT_FORMAT_VERSION: u8 = 1;
const NONCE_LEN: usize = 24;
const AEAD_TAG_LEN: usize = 16;
const MASTER_KEY_LEN: usize = 32;

#[cfg(target_os = "macos")]
const KEYCHAIN_SERVICE: &str = "com.shaikzeeshan.mnema.vault";
#[cfg(target_os = "macos")]
const KEYCHAIN_ACCOUNT: &str = "master-key";
// errSecItemNotFound: the keychain has no entry for this service/account.
#[cfg(target_os = "macos")]
pub(crate) const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;

fn vault_error(message: impl Into<String>) -> AppInfraError {
    AppInfraError::SecretVault(message.into())
}

/// Where the vault's 256-bit master key lives.
///
/// `load` distinguishes "no key stored" (`Ok(None)`) from "read failed"
/// (`Err`, e.g. a denied keychain prompt or a locked keychain). Unlock maps
/// the latter to `Denied` and never mints a replacement key over it.
pub trait MasterKeySource {
    fn load(&self) -> Result<Option<[u8; MASTER_KEY_LEN]>>;
    fn store(&self, key: &[u8; MASTER_KEY_LEN]) -> Result<()>;
}

/// Production source: one generic-password item created via the Keychain
/// Services API, so this app is on the item's ACL and reads silently forever.
#[derive(Debug, Clone, Copy)]
pub struct KeychainMasterKeySource;

#[cfg(target_os = "macos")]
impl MasterKeySource for KeychainMasterKeySource {
    fn load(&self) -> Result<Option<[u8; MASTER_KEY_LEN]>> {
        match security_framework::passwords::get_generic_password(
            KEYCHAIN_SERVICE,
            KEYCHAIN_ACCOUNT,
        ) {
            Ok(bytes) => {
                let key: [u8; MASTER_KEY_LEN] = bytes.as_slice().try_into().map_err(|_| {
                    vault_error(format!(
                        "vault master key has unexpected length {}",
                        bytes.len()
                    ))
                })?;
                Ok(Some(key))
            }
            Err(error) if error.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(None),
            Err(error) => Err(vault_error(format!(
                "keychain read of the vault master key failed: {error}"
            ))),
        }
    }

    fn store(&self, key: &[u8; MASTER_KEY_LEN]) -> Result<()> {
        security_framework::passwords::set_generic_password(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT, key)
            .map_err(|error| {
                vault_error(format!(
                    "keychain write of the vault master key failed: {error}"
                ))
            })
    }
}

#[cfg(not(target_os = "macos"))]
impl MasterKeySource for KeychainMasterKeySource {
    fn load(&self) -> Result<Option<[u8; MASTER_KEY_LEN]>> {
        Err(vault_error(
            "secret vault master key store is unsupported on this platform",
        ))
    }

    fn store(&self, _key: &[u8; MASTER_KEY_LEN]) -> Result<()> {
        Err(vault_error(
            "secret vault master key store is unsupported on this platform",
        ))
    }
}

/// Dev/test source: the master key as a hex string in a plain file. Selected
/// at runtime by `MNEMA_DEV_MASTER_KEY_FILE` in debug builds only.
#[derive(Debug, Clone)]
pub struct FileMasterKeySource {
    path: PathBuf,
}

impl FileMasterKeySource {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

impl MasterKeySource for FileMasterKeySource {
    fn load(&self) -> Result<Option<[u8; MASTER_KEY_LEN]>> {
        if !self.path.exists() {
            return Ok(None);
        }
        let hex = fs::read_to_string(&self.path)?;
        Ok(Some(parse_master_key_hex(hex.trim())?))
    }

    fn store(&self, key: &[u8; MASTER_KEY_LEN]) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        // The master key is plaintext at rest here: create the file owner-only
        // (0600) BEFORE any key bytes touch disk, so another local user can
        // never read it. On non-unix, fall back to a plain write.
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&self.path)?;
            file.write_all(encode_hex(key).as_bytes())?;
        }
        #[cfg(not(unix))]
        fs::write(&self.path, encode_hex(key))?;
        Ok(())
    }
}

fn parse_master_key_hex(hex: &str) -> Result<[u8; MASTER_KEY_LEN]> {
    if hex.len() != MASTER_KEY_LEN * 2 {
        return Err(vault_error(format!(
            "vault master key file must hold {} hex characters, found {}",
            MASTER_KEY_LEN * 2,
            hex.len()
        )));
    }
    let mut key = [0_u8; MASTER_KEY_LEN];
    for (index, byte) in key.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16)
            .map_err(|_| vault_error("vault master key file is not valid hex"))?;
    }
    Ok(key)
}

fn encode_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn mint_master_key() -> [u8; MASTER_KEY_LEN] {
    let mut key = [0_u8; MASTER_KEY_LEN];
    rand::rngs::OsRng.fill_bytes(&mut key);
    key
}

/// The three-state unlock outcome. `Missing` is the first-run path: no master
/// key item existed (and no vault file), so a fresh key was minted, stored,
/// and an empty vault created — it carries the ready-to-use vault. `Denied`
/// is any key-read failure OR the missing-key-with-existing-vault invariant
/// breach; neither ever mints a replacement key.
#[derive(Debug)]
pub enum SecretVaultUnlock {
    Unlocked(SecretVault),
    Missing(SecretVault),
    Denied(String),
}

/// AEAD-encrypted string→string secret map (account name → secret), stored as
/// `secrets.vault` in the app save directory. Every mutation rewrites the file
/// atomically (temp file + rename) with a fresh random nonce.
pub struct SecretVault {
    path: PathBuf,
    key: [u8; MASTER_KEY_LEN],
    secrets: BTreeMap<String, String>,
}

// Never derive Debug: it would print the master key and every secret.
impl std::fmt::Debug for SecretVault {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SecretVault")
            .field("path", &self.path)
            .field("accounts", &self.accounts())
            .finish_non_exhaustive()
    }
}

impl SecretVault {
    pub fn get(&self, account: &str) -> Option<&str> {
        self.secrets.get(account).map(String::as_str)
    }

    pub fn set(&mut self, account: &str, secret: &str) -> Result<()> {
        let previous = self.secrets.insert(account.to_string(), secret.to_string());
        if let Err(error) = self.persist() {
            // Roll back: the in-memory map is cached process-wide via the
            // vault handle, so it must never report a value the disk never
            // durably received (else the app believes the secret is saved
            // this session and it vanishes on relaunch).
            match previous {
                Some(old) => self.secrets.insert(account.to_string(), old),
                None => self.secrets.remove(account),
            };
            return Err(error);
        }
        Ok(())
    }

    /// Delete a secret. Deleting an absent account is a no-op.
    pub fn delete(&mut self, account: &str) -> Result<()> {
        let Some(previous) = self.secrets.remove(account) else {
            return Ok(());
        };
        if let Err(error) = self.persist() {
            // Roll back so a failed delete does not leave the cache reporting
            // the secret gone while it still lives on disk: a retried delete
            // would then no-op "success" and the secret resurrects on relaunch.
            self.secrets.insert(account.to_string(), previous);
            return Err(error);
        }
        Ok(())
    }

    pub fn accounts(&self) -> Vec<&str> {
        self.secrets.keys().map(String::as_str).collect()
    }

    fn create_empty(path: PathBuf, key: [u8; MASTER_KEY_LEN]) -> Result<Self> {
        let vault = Self {
            path,
            key,
            secrets: BTreeMap::new(),
        };
        vault.persist()?;
        Ok(vault)
    }

    fn open(path: PathBuf, key: [u8; MASTER_KEY_LEN]) -> Result<Self> {
        let bytes = fs::read(&path)?;
        if bytes.len() < 1 + NONCE_LEN + AEAD_TAG_LEN {
            return Err(vault_error(format!(
                "secret vault file {} is truncated or corrupt",
                path.display()
            )));
        }
        if bytes[0] != VAULT_FORMAT_VERSION {
            return Err(vault_error(format!(
                "secret vault file {} has unsupported format version {}",
                path.display(),
                bytes[0]
            )));
        }

        let cipher = XChaCha20Poly1305::new(Key::from_slice(&key));
        let nonce = XNonce::from_slice(&bytes[1..1 + NONCE_LEN]);
        let plaintext = cipher
            .decrypt(nonce, &bytes[1 + NONCE_LEN..])
            .map_err(|_| {
                vault_error(format!(
                    "failed to decrypt secret vault file {}: wrong master key or corrupted contents",
                    path.display()
                ))
            })?;
        let secrets: BTreeMap<String, String> = serde_json::from_slice(&plaintext)?;

        Ok(Self { path, key, secrets })
    }

    fn persist(&self) -> Result<()> {
        let plaintext = serde_json::to_vec(&self.secrets)?;
        let cipher = XChaCha20Poly1305::new(Key::from_slice(&self.key));
        let mut nonce = [0_u8; NONCE_LEN];
        rand::rngs::OsRng.fill_bytes(&mut nonce);
        let ciphertext = cipher
            .encrypt(XNonce::from_slice(&nonce), plaintext.as_slice())
            .map_err(|_| vault_error("failed to encrypt secret vault contents"))?;

        let mut bytes = Vec::with_capacity(1 + NONCE_LEN + ciphertext.len());
        bytes.push(VAULT_FORMAT_VERSION);
        bytes.extend_from_slice(&nonce);
        bytes.extend_from_slice(&ciphertext);

        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let temp_path = temp_vault_path(&self.path);
        let mut file = fs::File::create(&temp_path)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp_path, &self.path)?;
        Ok(())
    }
}

fn temp_vault_path(vault_path: &Path) -> PathBuf {
    let file_name = vault_path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| SECRET_VAULT_FILE_NAME.to_string());
    vault_path.with_file_name(format!("{file_name}.tmp"))
}

/// Unlock the vault in `save_dir` with the default master-key source: the OS
/// keychain, or (debug builds only) the file named by `MNEMA_DEV_MASTER_KEY_FILE`.
pub fn unlock_secret_vault(save_dir: &Path) -> Result<SecretVaultUnlock> {
    #[cfg(debug_assertions)]
    if let Ok(path) = std::env::var("MNEMA_DEV_MASTER_KEY_FILE") {
        return unlock_secret_vault_with_source(save_dir, &FileMasterKeySource::new(path));
    }

    unlock_secret_vault_with_source(save_dir, &KeychainMasterKeySource)
}

/// Unlock the vault in `save_dir` with an injected master-key source.
///
/// - Key present + vault file present → decrypt (`Unlocked`, or a hard error
///   on corruption/wrong key — never a silent reset).
/// - Key present + no vault file → start an empty vault (`Unlocked`; secrets
///   must be re-entered, nothing to recover).
/// - No key + no vault file → first run: mint + store a key, create an empty
///   vault (`Missing`).
/// - No key + vault file present → `Denied`; minting here would silently
///   orphan every stored secret.
/// - Key read failed → `Denied(reason)`.
pub fn unlock_secret_vault_with_source(
    save_dir: &Path,
    source: &dyn MasterKeySource,
) -> Result<SecretVaultUnlock> {
    let vault_path = save_dir.join(SECRET_VAULT_FILE_NAME);

    let key = match source.load() {
        Ok(Some(key)) => key,
        Ok(None) => {
            if vault_path.exists() {
                return Ok(SecretVaultUnlock::Denied(format!(
                    "vault file {} exists but its master key is missing; refusing to mint a replacement key",
                    vault_path.display()
                )));
            }
            let key = mint_master_key();
            source.store(&key)?;
            let vault = SecretVault::create_empty(vault_path, key)?;
            return Ok(SecretVaultUnlock::Missing(vault));
        }
        Err(error) => return Ok(SecretVaultUnlock::Denied(error.to_string())),
    };

    if !vault_path.exists() {
        return Ok(SecretVaultUnlock::Unlocked(SecretVault::create_empty(
            vault_path, key,
        )?));
    }

    Ok(SecretVaultUnlock::Unlocked(SecretVault::open(
        vault_path, key,
    )?))
}

#[cfg(test)]
mod tests {
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
            let path = std::env::temp_dir().join(format!("secret-vault-{label}-{unique}"));

            fs::create_dir_all(&path).expect("test directory should be created");

            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    struct DenyingMasterKeySource;

    impl MasterKeySource for DenyingMasterKeySource {
        fn load(&self) -> Result<Option<[u8; MASTER_KEY_LEN]>> {
            Err(vault_error("user denied keychain access"))
        }

        fn store(&self, _key: &[u8; MASTER_KEY_LEN]) -> Result<()> {
            Err(vault_error("user denied keychain access"))
        }
    }

    fn key_source(dir: &TestDir) -> FileMasterKeySource {
        FileMasterKeySource::new(dir.path().join("master.key"))
    }

    fn vault_path(dir: &TestDir) -> PathBuf {
        dir.path().join(SECRET_VAULT_FILE_NAME)
    }

    fn unlocked(outcome: SecretVaultUnlock) -> SecretVault {
        match outcome {
            SecretVaultUnlock::Unlocked(vault) | SecretVaultUnlock::Missing(vault) => vault,
            SecretVaultUnlock::Denied(reason) => {
                panic!("vault should unlock, got Denied: {reason}")
            }
        }
    }

    #[test]
    fn first_run_mints_key_and_creates_empty_vault() {
        let dir = TestDir::new("first-run");
        let source = key_source(&dir);

        let outcome = unlock_secret_vault_with_source(dir.path(), &source)
            .expect("first-run unlock should succeed");
        let vault = match outcome {
            SecretVaultUnlock::Missing(vault) => vault,
            other => panic!("first run should be Missing, got {other:?}"),
        };

        assert!(vault.accounts().is_empty());
        assert!(vault_path(&dir).exists());
        assert!(
            source.load().expect("key load should succeed").is_some(),
            "a master key should have been minted and stored"
        );
    }

    #[test]
    fn roundtrip_set_get_delete_across_reopen() {
        let dir = TestDir::new("roundtrip");
        let source = key_source(&dir);

        let mut vault = unlocked(
            unlock_secret_vault_with_source(dir.path(), &source).expect("unlock should succeed"),
        );
        vault
            .set("anthropic", "sk-ant-secret")
            .expect("set should succeed");
        vault
            .set("deepgram", "dg-secret")
            .expect("set should succeed");
        assert_eq!(vault.get("anthropic"), Some("sk-ant-secret"));
        assert_eq!(vault.accounts(), vec!["anthropic", "deepgram"]);

        let mut reopened = unlocked(
            unlock_secret_vault_with_source(dir.path(), &source).expect("reopen should succeed"),
        );
        assert_eq!(reopened.get("anthropic"), Some("sk-ant-secret"));
        assert_eq!(reopened.get("deepgram"), Some("dg-secret"));

        reopened.delete("anthropic").expect("delete should succeed");
        reopened
            .delete("never-existed")
            .expect("deleting an absent account should be a no-op");
        assert_eq!(reopened.get("anthropic"), None);

        let after_delete = unlocked(
            unlock_secret_vault_with_source(dir.path(), &source).expect("reopen should succeed"),
        );
        assert_eq!(after_delete.get("anthropic"), None);
        assert_eq!(after_delete.get("deepgram"), Some("dg-secret"));
    }

    #[test]
    fn corrupt_vault_file_is_an_error_not_a_reset() {
        let dir = TestDir::new("corrupt");
        let source = key_source(&dir);
        let mut vault = unlocked(
            unlock_secret_vault_with_source(dir.path(), &source).expect("unlock should succeed"),
        );
        vault
            .set("anthropic", "sk-ant-secret")
            .expect("set should succeed");

        // Keep the valid version byte so this exercises the AEAD integrity
        // check, not the version check (the wrong first byte is covered by
        // the version branch).
        let mut garbage = vec![VAULT_FORMAT_VERSION];
        garbage.extend_from_slice(b"not a vault at all, definitely garbage bytes");
        fs::write(vault_path(&dir), &garbage).expect("garbage should be written");

        let error = unlock_secret_vault_with_source(dir.path(), &source)
            .expect_err("corrupt vault should error");
        assert!(error.to_string().contains("wrong master key or corrupted"));
        assert_eq!(
            fs::read(vault_path(&dir)).expect("vault file should still exist"),
            garbage,
            "a corrupt vault must never be overwritten"
        );
    }

    #[test]
    fn truncated_vault_file_is_an_error_not_a_reset() {
        let dir = TestDir::new("truncated");
        let source = key_source(&dir);
        let mut vault = unlocked(
            unlock_secret_vault_with_source(dir.path(), &source).expect("unlock should succeed"),
        );
        vault
            .set("anthropic", "sk-ant-secret")
            .expect("set should succeed");

        let bytes = fs::read(vault_path(&dir)).expect("vault file should exist");
        fs::write(vault_path(&dir), &bytes[..10]).expect("truncated file should be written");

        let error = unlock_secret_vault_with_source(dir.path(), &source)
            .expect_err("truncated vault should error");
        assert!(error.to_string().contains("truncated or corrupt"));
        assert!(vault_path(&dir).exists());
    }

    #[test]
    fn wrong_master_key_is_a_decrypt_error_not_a_reset() {
        let dir = TestDir::new("wrong-key");
        let source = key_source(&dir);
        let mut vault = unlocked(
            unlock_secret_vault_with_source(dir.path(), &source).expect("unlock should succeed"),
        );
        vault
            .set("anthropic", "sk-ant-secret")
            .expect("set should succeed");
        let bytes_before = fs::read(vault_path(&dir)).expect("vault file should exist");

        source
            .store(&mint_master_key())
            .expect("replacement key should be stored");

        let error = unlock_secret_vault_with_source(dir.path(), &source)
            .expect_err("wrong master key should error");
        assert!(error.to_string().contains("wrong master key or corrupted"));
        assert_eq!(
            fs::read(vault_path(&dir)).expect("vault file should still exist"),
            bytes_before,
            "a vault that fails to decrypt must never be overwritten"
        );
    }

    #[test]
    fn missing_file_with_present_key_starts_an_empty_vault() {
        // Documented choice: key present + no vault file = fresh empty vault
        // (there is nothing to recover), surfaced as Unlocked, not Missing.
        let dir = TestDir::new("missing-file");
        let source = key_source(&dir);
        source
            .store(&mint_master_key())
            .expect("key should be stored");

        let outcome =
            unlock_secret_vault_with_source(dir.path(), &source).expect("unlock should succeed");
        let vault = match outcome {
            SecretVaultUnlock::Unlocked(vault) => vault,
            other => panic!("present key should be Unlocked, got {other:?}"),
        };

        assert!(vault.accounts().is_empty());
        assert!(vault_path(&dir).exists());
    }

    #[test]
    fn writes_are_atomic_and_leave_no_temp_file() {
        let dir = TestDir::new("atomic");
        let source = key_source(&dir);
        let mut vault = unlocked(
            unlock_secret_vault_with_source(dir.path(), &source).expect("unlock should succeed"),
        );

        vault
            .set("anthropic", "sk-ant-secret")
            .expect("set should succeed");

        assert!(!temp_vault_path(&vault_path(&dir)).exists());
        let reopened = unlocked(
            unlock_secret_vault_with_source(dir.path(), &source).expect("reopen should succeed"),
        );
        assert_eq!(reopened.get("anthropic"), Some("sk-ant-secret"));
    }

    #[test]
    fn existing_vault_with_missing_key_is_denied_and_never_mints() {
        let dir = TestDir::new("missing-key");
        let source = key_source(&dir);
        let mut vault = unlocked(
            unlock_secret_vault_with_source(dir.path(), &source).expect("unlock should succeed"),
        );
        vault
            .set("anthropic", "sk-ant-secret")
            .expect("set should succeed");
        let bytes_before = fs::read(vault_path(&dir)).expect("vault file should exist");

        fs::remove_file(dir.path().join("master.key")).expect("key file should be removed");

        let outcome = unlock_secret_vault_with_source(dir.path(), &source)
            .expect("unlock should not hard-fail");
        match outcome {
            SecretVaultUnlock::Denied(reason) => {
                assert!(reason.contains("refusing to mint a replacement key"))
            }
            other => panic!("missing key over an existing vault must be Denied, got {other:?}"),
        }
        assert!(
            source.load().expect("key load should succeed").is_none(),
            "no replacement key may be minted over an existing vault"
        );
        assert_eq!(
            fs::read(vault_path(&dir)).expect("vault file should still exist"),
            bytes_before
        );
    }

    #[test]
    fn denied_key_read_maps_to_denied() {
        let dir = TestDir::new("denied");

        let outcome = unlock_secret_vault_with_source(dir.path(), &DenyingMasterKeySource)
            .expect("unlock should not hard-fail");
        match outcome {
            SecretVaultUnlock::Denied(reason) => {
                assert!(reason.contains("user denied keychain access"))
            }
            other => panic!("a denied key read must be Denied, got {other:?}"),
        }
        assert!(
            !vault_path(&dir).exists(),
            "a denied unlock must not create a vault"
        );
    }

    #[cfg(unix)]
    #[test]
    fn failed_persist_rolls_back_the_in_memory_view() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TestDir::new("persist-fail-rollback");
        let source = key_source(&dir);
        let mut vault = unlocked(
            unlock_secret_vault_with_source(dir.path(), &source).expect("unlock should succeed"),
        );
        vault.set("keep", "v-keep").expect("first set persists");

        // Read-only save dir => the next atomic temp+rename persist fails
        // deterministically without needing a full disk.
        let original = fs::metadata(dir.path()).expect("metadata").permissions();
        let mut readonly = original.clone();
        readonly.set_mode(0o500);
        fs::set_permissions(dir.path(), readonly).expect("chmod read-only");

        let set_result = vault.set("added", "v-added");
        let delete_result = vault.delete("keep");

        // Restore write access before any assertion can unwind the test.
        fs::set_permissions(dir.path(), original).expect("chmod restore");

        assert!(set_result.is_err(), "a set whose persist fails must error");
        assert!(
            delete_result.is_err(),
            "a delete whose persist fails must error"
        );
        // The in-memory cache (shared process-wide via the handle) must match
        // disk: the failed insert must not linger, the failed delete must not
        // drop the still-durable secret.
        assert_eq!(
            vault.get("added"),
            None,
            "a key whose persist failed must not read back as saved"
        );
        assert_eq!(
            vault.get("keep"),
            Some("v-keep"),
            "a delete whose persist failed must not drop the still-durable secret"
        );
        // And a fresh reopen from disk agrees with the rolled-back view.
        let reopened = unlocked(
            unlock_secret_vault_with_source(dir.path(), &source).expect("reopen should succeed"),
        );
        assert_eq!(reopened.get("added"), None);
        assert_eq!(reopened.get("keep"), Some("v-keep"));
    }

    #[cfg(unix)]
    #[test]
    fn dev_master_key_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TestDir::new("key-perms");
        let source = key_source(&dir);
        source
            .store(&mint_master_key())
            .expect("store should succeed");

        let mode = fs::metadata(dir.path().join("master.key"))
            .expect("key file should exist")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(
            mode, 0o600,
            "the plaintext dev master key must be readable only by its owner, got {mode:o}"
        );
    }

    #[test]
    fn dev_env_knob_routes_to_the_file_key_source() {
        let dir = TestDir::new("env-knob");
        let key_path = dir.path().join("dev-master.key");
        std::env::set_var("MNEMA_DEV_MASTER_KEY_FILE", &key_path);

        let outcome = unlock_secret_vault(dir.path()).expect("unlock should succeed");
        std::env::remove_var("MNEMA_DEV_MASTER_KEY_FILE");

        let vault = unlocked(outcome);
        assert!(vault.accounts().is_empty());
        assert!(
            key_path.exists(),
            "the dev key file should hold the minted key"
        );
    }
}
