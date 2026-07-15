use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

use rand::RngCore;
use zeroize::Zeroizing;

#[cfg(target_os = "macos")]
use std::process::Command;

#[cfg(target_os = "windows")]
use windows_sys::Win32::{
    Foundation::{GetLastError, ERROR_NOT_FOUND},
    Security::Credentials::{
        CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC,
    },
};

use crate::error::{AppInfraError, Result};

pub(crate) const CAPTURE_INDEX_DATABASE_DIR_NAME: &str = "db";

const INDEX_IDENTITY_FILE_NAME: &str = "capture-index.json";
const KEYCHAIN_SERVICE: &str = "com.shaikzeeshan.mnema.capture-index";
const APP_ID: &str = "com.shaikzeeshan.mnema";
const ENCRYPTION_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct CaptureIndexIdentity {
    index_id: String,
    encryption_schema_version: u32,
    app_id: String,
}

#[derive(Debug, Clone)]
pub(crate) struct CaptureIndexDatabaseKey {
    // Held for the lifetime of the process; `Zeroizing` scrubs the passphrase
    // bytes when the key is dropped so the SQLCipher secret does not linger in
    // freed heap memory.
    passphrase: Zeroizing<String>,
}

impl CaptureIndexDatabaseKey {
    fn new(passphrase: impl Into<Zeroizing<String>>) -> Self {
        Self {
            passphrase: passphrase.into(),
        }
    }

    pub(crate) fn sqlcipher_pragma_value(&self) -> String {
        // The intermediate escaped copy is scrubbed on drop; the returned String
        // is consumed immediately by sqlx's pragma handling.
        let escaped_key = Zeroizing::new(self.passphrase.replace('\'', "''"));
        format!("'{}'", escaped_key.as_str())
    }
}

trait CaptureIndexKeyStoreAdapter {
    fn load_key(&self, index_id: &str) -> Result<Option<String>>;
    fn store_key(&self, index_id: &str, key: &str) -> Result<()>;
    fn missing_key_error(&self, index_id: &str) -> AppInfraError;
}

struct CaptureIndexKeyStore<A> {
    adapter: A,
}

impl<A> CaptureIndexKeyStore<A>
where
    A: CaptureIndexKeyStoreAdapter,
{
    fn new(adapter: A) -> Self {
        Self { adapter }
    }

    fn resolve_database_key(
        &self,
        base_dir: &Path,
        database_path: &Path,
    ) -> Result<Option<CaptureIndexDatabaseKey>> {
        let identity_path = capture_index_identity_path(base_dir);
        let mut database_exists = database_path.exists();

        if database_exists && is_plaintext_sqlite_database(database_path)? {
            if identity_path.exists() && is_empty_plaintext_database_file(database_path)? {
                remove_database_files(database_path)?;
                database_exists = false;
            } else if identity_path.exists() {
                return Err(AppInfraError::CaptureIndexEncryption(
                    "plaintext capture index database exists for an encrypted identity".to_string(),
                ));
            } else {
                return Ok(None);
            }
        }

        let identity_exists = identity_path.exists();
        if database_exists && !identity_exists {
            return Err(AppInfraError::CaptureIndexEncryption(
                "capture index database exists but capture-index.json is missing".to_string(),
            ));
        }
        let identity = load_or_create_identity(&identity_path)?;
        let require_existing_key = database_exists;
        let passphrase = self.load_or_create_key(&identity.index_id, require_existing_key)?;
        Ok(Some(CaptureIndexDatabaseKey::new(passphrase)))
    }

    fn load_or_create_key(&self, index_id: &str, require_existing: bool) -> Result<Zeroizing<String>> {
        if let Some(key) = self.adapter.load_key(index_id)? {
            return Ok(Zeroizing::new(key));
        }
        if require_existing {
            return Err(self.adapter.missing_key_error(index_id));
        }

        let key = Zeroizing::new(random_hex(32)?);
        self.adapter.store_key(index_id, &key)?;
        Ok(key)
    }
}

#[derive(Debug, Clone)]
struct FileCaptureIndexKeyStoreAdapter {
    key_dir: PathBuf,
}

impl FileCaptureIndexKeyStoreAdapter {
    fn new(key_dir: impl Into<PathBuf>) -> Self {
        Self {
            key_dir: key_dir.into(),
        }
    }

    fn key_path(&self, index_id: &str) -> PathBuf {
        self.key_dir.join(format!("{index_id}.key"))
    }
}

impl CaptureIndexKeyStoreAdapter for FileCaptureIndexKeyStoreAdapter {
    fn load_key(&self, index_id: &str) -> Result<Option<String>> {
        fs::create_dir_all(&self.key_dir)?;
        let path = self.key_path(index_id);
        if !path.exists() {
            return Ok(None);
        }

        let key = fs::read_to_string(path)?;
        if key.trim().is_empty() {
            return Err(AppInfraError::CaptureIndexEncryption(
                "stored capture index key is empty".to_string(),
            ));
        }
        Ok(Some(key))
    }

    fn store_key(&self, index_id: &str, key: &str) -> Result<()> {
        fs::create_dir_all(&self.key_dir)?;
        fs::write(self.key_path(index_id), key)?;
        Ok(())
    }

    fn missing_key_error(&self, index_id: &str) -> AppInfraError {
        AppInfraError::CaptureIndexEncryption(format!(
            "capture index key for {index_id} is missing"
        ))
    }
}

#[derive(Debug, Clone, Copy)]
struct PlatformKeychainCaptureIndexKeyStoreAdapter;

impl CaptureIndexKeyStoreAdapter for PlatformKeychainCaptureIndexKeyStoreAdapter {
    fn load_key(&self, index_id: &str) -> Result<Option<String>> {
        load_platform_key(index_id)
    }

    fn store_key(&self, index_id: &str, key: &str) -> Result<()> {
        store_platform_key(index_id, key)
    }

    fn missing_key_error(&self, index_id: &str) -> AppInfraError {
        AppInfraError::CaptureIndexEncryption(format!(
            "capture index key for {index_id} is missing from the platform key store"
        ))
    }
}

pub(crate) fn resolve_capture_index_database_key_for_current_process(
    base_dir: &Path,
    database_path: &Path,
) -> Result<Option<CaptureIndexDatabaseKey>> {
    if test_process_allows_plaintext_index() {
        return Ok(None);
    }

    if let Ok(key_dir) = std::env::var("MNEMA_CAPTURE_INDEX_KEY_DIR") {
        return CaptureIndexKeyStore::new(FileCaptureIndexKeyStoreAdapter::new(key_dir))
            .resolve_database_key(base_dir, database_path);
    }

    CaptureIndexKeyStore::new(PlatformKeychainCaptureIndexKeyStoreAdapter)
        .resolve_database_key(base_dir, database_path)
}

fn test_process_allows_plaintext_index() -> bool {
    cfg!(test) && std::env::var("MNEMA_TEST_ENCRYPTED_INDEX").ok().as_deref() != Some("1")
}

fn load_or_create_identity(identity_path: &Path) -> Result<CaptureIndexIdentity> {
    if identity_path.exists() {
        return Ok(serde_json::from_str::<CaptureIndexIdentity>(
            &fs::read_to_string(identity_path)?,
        )?);
    }

    if let Some(parent) = identity_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let identity = CaptureIndexIdentity {
        index_id: generate_index_id()?,
        encryption_schema_version: ENCRYPTION_SCHEMA_VERSION,
        app_id: APP_ID.to_string(),
    };
    fs::write(identity_path, serde_json::to_string_pretty(&identity)?)?;
    Ok(identity)
}

fn capture_index_identity_path(base_dir: &Path) -> PathBuf {
    base_dir
        .join(CAPTURE_INDEX_DATABASE_DIR_NAME)
        .join(INDEX_IDENTITY_FILE_NAME)
}

fn is_plaintext_sqlite_database(database_path: &Path) -> Result<bool> {
    const SQLITE_HEADER: &[u8; 16] = b"SQLite format 3\0";
    let mut file = fs::File::open(database_path)?;
    let mut header = [0_u8; 16];
    let bytes_read = file.read(&mut header)?;
    Ok(bytes_read == SQLITE_HEADER.len() && &header == SQLITE_HEADER)
}

fn is_empty_plaintext_database_file(database_path: &Path) -> Result<bool> {
    Ok(fs::metadata(database_path)?.len() <= 4096)
}

fn remove_database_files(database_path: &Path) -> Result<()> {
    for path in [
        database_path.to_path_buf(),
        PathBuf::from(format!("{}-wal", database_path.display())),
        PathBuf::from(format!("{}-shm", database_path.display())),
    ] {
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn generate_index_id() -> Result<String> {
    Ok(format!("mnema-index-{}", random_hex(16)?))
}

fn random_hex(byte_count: usize) -> Result<String> {
    let mut bytes = vec![0_u8; byte_count];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[cfg(target_os = "macos")]
fn load_platform_key(index_id: &str) -> Result<Option<String>> {
    let lookup = Command::new("security")
        .args([
            "find-generic-password",
            "-s",
            KEYCHAIN_SERVICE,
            "-a",
            index_id,
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
fn store_platform_key(index_id: &str, key: &str) -> Result<()> {
    let add = Command::new("security")
        .args([
            "add-generic-password",
            "-U",
            "-s",
            KEYCHAIN_SERVICE,
            "-a",
            index_id,
            "-w",
            key,
        ])
        .output()?;
    if !add.status.success() {
        return Err(AppInfraError::CaptureIndexEncryption(
            String::from_utf8_lossy(&add.stderr).trim().to_string(),
        ));
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn load_platform_key(index_id: &str) -> Result<Option<String>> {
    let target = windows_credential_target(index_id);
    let mut credential = std::ptr::null_mut::<CREDENTIALW>();

    let ok = unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut credential) };
    if ok == 0 {
        let code = unsafe { GetLastError() };
        if code == ERROR_NOT_FOUND {
            return Ok(None);
        }
        return Err(AppInfraError::CaptureIndexEncryption(format!(
            "Windows Credential Manager failed to read capture index key for {index_id}: error {code}"
        )));
    }

    if credential.is_null() {
        return Ok(None);
    }
    let _guard = WindowsCredentialGuard(credential);
    let credential = unsafe { &*credential };
    if credential.CredentialBlobSize == 0 {
        return Ok(None);
    }
    if credential.CredentialBlob.is_null() {
        return Err(AppInfraError::CaptureIndexEncryption(format!(
            "Windows Credential Manager returned an empty capture index key blob for {index_id}"
        )));
    }

    let bytes = unsafe {
        std::slice::from_raw_parts(
            credential.CredentialBlob,
            credential.CredentialBlobSize as usize,
        )
    };
    let key = std::str::from_utf8(bytes)
        .map_err(|error| {
            AppInfraError::CaptureIndexEncryption(format!(
                "Windows Credential Manager returned a non-UTF-8 capture index key for {index_id}: {error}"
            ))
        })?
        .trim()
        .to_string();
    if key.is_empty() {
        return Ok(None);
    }

    Ok(Some(key))
}

#[cfg(target_os = "windows")]
fn store_platform_key(index_id: &str, key: &str) -> Result<()> {
    let mut target = windows_credential_target(index_id);
    let mut user_name = windows_credential_user_name();
    // Scrub the plaintext passphrase bytes once the credential write completes.
    let mut key_bytes = Zeroizing::new(key.as_bytes().to_vec());

    let mut credential = CREDENTIALW::default();
    credential.Type = CRED_TYPE_GENERIC;
    credential.TargetName = target.as_mut_ptr();
    credential.CredentialBlobSize = key_bytes.len().try_into().map_err(|_| {
        AppInfraError::CaptureIndexEncryption("capture index key is too large".to_string())
    })?;
    credential.CredentialBlob = key_bytes.as_mut_ptr();
    credential.Persist = CRED_PERSIST_LOCAL_MACHINE;
    credential.UserName = user_name.as_mut_ptr();

    let ok = unsafe { CredWriteW(&credential, 0) };
    if ok == 0 {
        let code = unsafe { GetLastError() };
        return Err(AppInfraError::CaptureIndexEncryption(format!(
            "Windows Credential Manager failed to store capture index key for {index_id}: error {code}"
        )));
    }

    Ok(())
}

#[cfg(target_os = "windows")]
struct WindowsCredentialGuard(*mut CREDENTIALW);

#[cfg(target_os = "windows")]
impl Drop for WindowsCredentialGuard {
    fn drop(&mut self) {
        unsafe { CredFree(self.0.cast()) };
    }
}

#[cfg(target_os = "windows")]
fn windows_credential_target(index_id: &str) -> Vec<u16> {
    format!("{KEYCHAIN_SERVICE}:{index_id}")
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect()
}

#[cfg(target_os = "windows")]
fn windows_credential_user_name() -> Vec<u16> {
    APP_ID.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn load_platform_key(_index_id: &str) -> Result<Option<String>> {
    Err(AppInfraError::CaptureIndexEncryption(
        "capture index key store is unsupported on this platform".to_string(),
    ))
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn store_platform_key(_index_id: &str, _key: &str) -> Result<()> {
    Err(AppInfraError::CaptureIndexEncryption(
        "capture index key store is unsupported on this platform".to_string(),
    ))
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
            let path =
                std::env::temp_dir().join(format!("capture-index-key-store-{label}-{unique}"));

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

    fn database_path(base_dir: &Path) -> PathBuf {
        base_dir
            .join(CAPTURE_INDEX_DATABASE_DIR_NAME)
            .join("app.sqlite3")
    }

    fn write_identity(base_dir: &Path, index_id: &str) {
        let identity_path = capture_index_identity_path(base_dir);
        fs::create_dir_all(identity_path.parent().expect("identity should have parent"))
            .expect("identity parent should exist");
        let identity = CaptureIndexIdentity {
            index_id: index_id.to_string(),
            encryption_schema_version: ENCRYPTION_SCHEMA_VERSION,
            app_id: APP_ID.to_string(),
        };
        fs::write(
            identity_path,
            serde_json::to_string_pretty(&identity).expect("identity should serialize"),
        )
        .expect("identity should be written");
    }

    fn file_key_store(
        key_dir: impl Into<PathBuf>,
    ) -> CaptureIndexKeyStore<FileCaptureIndexKeyStoreAdapter> {
        CaptureIndexKeyStore::new(FileCaptureIndexKeyStoreAdapter::new(key_dir))
    }

    #[test]
    fn file_key_store_creates_identity_and_key_outside_save_directory() {
        let save_dir = TestDir::new("save");
        let key_dir = TestDir::new("keys");
        let database_path = database_path(save_dir.path());

        let key = file_key_store(key_dir.path())
            .resolve_database_key(save_dir.path(), &database_path)
            .expect("key should resolve")
            .expect("encrypted index should have a key");

        let identity: CaptureIndexIdentity = serde_json::from_str(
            &fs::read_to_string(capture_index_identity_path(save_dir.path()))
                .expect("identity should be written"),
        )
        .expect("identity should parse");
        let key_path = key_dir.path().join(format!("{}.key", identity.index_id));

        assert!(identity.index_id.starts_with("mnema-index-"));
        assert_eq!(
            identity.encryption_schema_version,
            ENCRYPTION_SCHEMA_VERSION
        );
        assert_eq!(identity.app_id, APP_ID);
        assert!(key_path.exists());
        assert!(!key_path.starts_with(save_dir.path()));
        assert!(key.sqlcipher_pragma_value().starts_with('\''));
    }

    #[test]
    fn file_key_store_requires_existing_key_for_existing_index() {
        let save_dir = TestDir::new("missing-key-save");
        let key_dir = TestDir::new("missing-key-store");
        let database_path = database_path(save_dir.path());
        write_identity(save_dir.path(), "mnema-index-existing");
        fs::write(&database_path, b"not-sqlite-ciphertext").expect("database should exist");

        let error = file_key_store(key_dir.path())
            .resolve_database_key(save_dir.path(), &database_path)
            .expect_err("existing encrypted index should require its original key");

        assert!(error
            .to_string()
            .contains("capture index key for mnema-index-existing is missing"));
    }

    #[test]
    fn file_key_store_rejects_existing_database_without_identity() {
        let save_dir = TestDir::new("missing-identity-save");
        let key_dir = TestDir::new("missing-identity-store");
        let database_path = database_path(save_dir.path());
        fs::create_dir_all(database_path.parent().expect("database should have parent"))
            .expect("database parent should exist");
        fs::write(&database_path, b"not-sqlite-ciphertext").expect("database should exist");

        let error = file_key_store(key_dir.path())
            .resolve_database_key(save_dir.path(), &database_path)
            .expect_err("existing encrypted index should require its identity");

        assert!(error
            .to_string()
            .contains("capture index database exists but capture-index.json is missing"));
        assert!(!capture_index_identity_path(save_dir.path()).exists());
    }

    #[test]
    fn file_key_store_rejects_empty_stored_key() {
        let save_dir = TestDir::new("empty-key-save");
        let key_dir = TestDir::new("empty-key-store");
        let database_path = database_path(save_dir.path());
        write_identity(save_dir.path(), "mnema-index-empty-key");
        fs::write(&database_path, b"not-sqlite-ciphertext").expect("database should exist");
        fs::write(key_dir.path().join("mnema-index-empty-key.key"), "")
            .expect("empty key should be written");

        let error = file_key_store(key_dir.path())
            .resolve_database_key(save_dir.path(), &database_path)
            .expect_err("empty key should fail");

        assert!(error
            .to_string()
            .contains("stored capture index key is empty"));
    }

    #[test]
    fn plaintext_database_without_identity_stays_plaintext() {
        let save_dir = TestDir::new("legacy-plaintext");
        let key_dir = TestDir::new("legacy-keys");
        let database_path = database_path(save_dir.path());
        fs::create_dir_all(database_path.parent().expect("database should have parent"))
            .expect("database parent should exist");
        fs::write(&database_path, b"SQLite format 3\0").expect("plaintext database should exist");

        let key = file_key_store(key_dir.path())
            .resolve_database_key(save_dir.path(), &database_path)
            .expect("plaintext database should be accepted");

        assert!(key.is_none());
        assert!(!capture_index_identity_path(save_dir.path()).exists());
    }

    #[test]
    fn plaintext_database_with_identity_is_rejected_when_non_empty() {
        let save_dir = TestDir::new("plaintext-identity");
        let key_dir = TestDir::new("plaintext-identity-keys");
        let database_path = database_path(save_dir.path());
        write_identity(save_dir.path(), "mnema-index-plaintext-identity");
        let mut database_bytes = b"SQLite format 3\0".to_vec();
        database_bytes.resize(8192, 0);
        fs::write(&database_path, database_bytes).expect("plaintext database should exist");

        let error = file_key_store(key_dir.path())
            .resolve_database_key(save_dir.path(), &database_path)
            .expect_err("plaintext database with encrypted identity should fail closed");

        assert!(error
            .to_string()
            .contains("plaintext capture index database exists for an encrypted identity"));
    }

    #[test]
    fn empty_plaintext_database_with_identity_is_repaired_before_key_resolution() {
        let save_dir = TestDir::new("empty-plaintext");
        let key_dir = TestDir::new("empty-plaintext-keys");
        let database_path = database_path(save_dir.path());
        write_identity(save_dir.path(), "mnema-index-empty-plaintext");
        fs::write(&database_path, b"SQLite format 3\0").expect("plaintext database should exist");
        fs::write(
            PathBuf::from(format!("{}-wal", database_path.display())),
            b"stale wal",
        )
        .expect("wal should exist");
        fs::write(
            PathBuf::from(format!("{}-shm", database_path.display())),
            b"stale shm",
        )
        .expect("shm should exist");

        let key = file_key_store(key_dir.path())
            .resolve_database_key(save_dir.path(), &database_path)
            .expect("key should resolve")
            .expect("repaired encrypted index should have a key");

        assert!(!database_path.exists());
        assert!(!PathBuf::from(format!("{}-wal", database_path.display())).exists());
        assert!(!PathBuf::from(format!("{}-shm", database_path.display())).exists());
        assert!(key.sqlcipher_pragma_value().starts_with('\''));
    }
}
