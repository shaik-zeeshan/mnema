use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

use rand::RngCore;

#[cfg(target_os = "macos")]
use std::process::Command;

use crate::error::{AppInfraError, Result};

#[cfg(any(target_os = "macos", test))]
mod resolution;
#[cfg(target_os = "macos")]
mod shared_group;

pub(crate) const CAPTURE_INDEX_DATABASE_DIR_NAME: &str = "db";

const INDEX_IDENTITY_FILE_NAME: &str = "capture-index.json";
const KEYCHAIN_SERVICE: &str = "com.shaikzeeshan.mnema.capture-index";
// Written into new identity files only; existing files keep the pre-rename
// com.shaikzeeshan.mnema value and nothing compares against it.
const APP_ID: &str = "day.mnema";
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
    passphrase: String,
}

impl CaptureIndexDatabaseKey {
    fn new(passphrase: String) -> Self {
        Self { passphrase }
    }

    pub(crate) fn sqlcipher_pragma_value(&self) -> String {
        let escaped_key = self.passphrase.replace('\'', "''");
        format!("'{escaped_key}'")
    }
}

trait CaptureIndexKeyStoreAdapter {
    fn load_key(&self, index_id: &str) -> Result<Option<String>>;
    fn store_key(&self, index_id: &str, key: &str) -> Result<()>;
    fn missing_key_error(&self, index_id: &str) -> AppInfraError;
    /// Removes the stored key. Only the owner-migration path deletes anything
    /// (the old silent item, after the database opened on the new path), so the
    /// default is a no-op.
    fn delete_key(&self, _index_id: &str) -> Result<()> {
        Ok(())
    }
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

    fn load_or_create_key(&self, index_id: &str, require_existing: bool) -> Result<String> {
        if let Some(key) = self.adapter.load_key(index_id)? {
            return Ok(key);
        }
        if require_existing {
            return Err(self.adapter.missing_key_error(index_id));
        }

        let key = random_hex(32)?;
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
            "capture index key for {index_id} is missing from Keychain"
        ))
    }

    fn delete_key(&self, index_id: &str) -> Result<()> {
        delete_platform_key(index_id)
    }
}

/// Which role of the Encrypted Capture Index is asking for the key (ADR 0041):
/// only the Owner (the app) migrates the key into the shared access group; the
/// Brokered Reader (the CLI) resolves new-then-old and never writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CaptureIndexKeyRole {
    Owner,
    Reader,
}

/// A resolved key plus an optional post-open step: the owner migration only
/// deletes the old silent keychain item once the database has actually opened
/// with the migrated key (ADR 0057, migrate-and-delete gated on proof).
pub(crate) struct ResolvedCaptureIndexKey {
    key: Option<CaptureIndexDatabaseKey>,
    on_open_success: Option<Box<dyn FnOnce() + Send>>,
}

impl ResolvedCaptureIndexKey {
    fn plain(key: Option<CaptureIndexDatabaseKey>) -> Self {
        Self {
            key,
            on_open_success: None,
        }
    }

    pub(crate) fn key(&self) -> Option<CaptureIndexDatabaseKey> {
        self.key.clone()
    }

    /// Call once the database has successfully opened with the resolved key.
    pub(crate) fn database_opened(&mut self) {
        if let Some(cleanup) = self.on_open_success.take() {
            cleanup();
        }
    }
}

pub(crate) fn resolve_capture_index_database_key_for_current_process(
    base_dir: &Path,
    database_path: &Path,
    role: CaptureIndexKeyRole,
) -> Result<ResolvedCaptureIndexKey> {
    if test_process_allows_plaintext_index() {
        return Ok(ResolvedCaptureIndexKey::plain(None));
    }

    if let Ok(key_dir) = std::env::var("MNEMA_CAPTURE_INDEX_KEY_DIR") {
        let key = CaptureIndexKeyStore::new(FileCaptureIndexKeyStoreAdapter::new(key_dir))
            .resolve_database_key(base_dir, database_path)?;
        return Ok(ResolvedCaptureIndexKey::plain(key));
    }

    resolve_platform_key_for_role(base_dir, database_path, role)
}

#[cfg(target_os = "macos")]
fn resolve_platform_key_for_role(
    base_dir: &Path,
    database_path: &Path,
    role: CaptureIndexKeyRole,
) -> Result<ResolvedCaptureIndexKey> {
    match role {
        CaptureIndexKeyRole::Owner => {
            let store = CaptureIndexKeyStore::new(resolution::OwnerMigratingAdapter::new(
                shared_group::SharedGroupKeychainAdapter,
                PlatformKeychainCaptureIndexKeyStoreAdapter,
            ));
            let key = store.resolve_database_key(base_dir, database_path)?;
            let pending = store.adapter.has_pending_old_delete();
            Ok(ResolvedCaptureIndexKey {
                key,
                on_open_success: pending.then(|| {
                    Box::new(move || store.adapter.delete_old_item_after_open())
                        as Box<dyn FnOnce() + Send>
                }),
            })
        }
        CaptureIndexKeyRole::Reader => {
            let key = CaptureIndexKeyStore::new(resolution::ReaderFallbackAdapter::new(
                shared_group::SharedGroupKeychainAdapter,
                PlatformKeychainCaptureIndexKeyStoreAdapter,
            ))
            .resolve_database_key(base_dir, database_path)?;
            Ok(ResolvedCaptureIndexKey::plain(key))
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn resolve_platform_key_for_role(
    base_dir: &Path,
    database_path: &Path,
    _role: CaptureIndexKeyRole,
) -> Result<ResolvedCaptureIndexKey> {
    let key = CaptureIndexKeyStore::new(PlatformKeychainCaptureIndexKeyStoreAdapter)
        .resolve_database_key(base_dir, database_path)?;
    Ok(ResolvedCaptureIndexKey::plain(key))
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

// The interpolated values are wrapped in plain double quotes for security's stdin
// tokenizer, which is only safe while they contain no quote, backslash, or newline.
// Production inputs are hex or reverse-DNS constants, so rejection never fires.
#[cfg(any(target_os = "macos", test))]
fn security_add_command(service: &str, account: &str, key: &str) -> Result<String> {
    for value in [service, account, key] {
        if value.contains(['"', '\\', '\n', '\r']) {
            return Err(AppInfraError::CaptureIndexEncryption(
                "keychain command value contains characters unsafe for double-quoting".to_string(),
            ));
        }
    }
    Ok(format!(
        "add-generic-password -U -s \"{service}\" -a \"{account}\" -w \"{key}\"\n"
    ))
}

#[cfg(target_os = "macos")]
fn store_platform_key(index_id: &str, key: &str) -> Result<()> {
    use std::io::Write as _;
    use std::process::Stdio;

    // The command goes through `security -i` (stdin) instead of argv so the key is
    // never visible to other processes via `ps`.
    let command = security_add_command(KEYCHAIN_SERVICE, index_id, key)?;
    let mut add = Command::new("/usr/bin/security")
        .arg("-i")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    add.stdin
        .take()
        .expect("child stdin is piped")
        .write_all(command.as_bytes())?;
    let output = add.wait_with_output()?;
    if !output.status.success() {
        return Err(AppInfraError::CaptureIndexEncryption(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn delete_platform_key(index_id: &str) -> Result<()> {
    let output = Command::new("/usr/bin/security")
        .args(["delete-generic-password", "-s", KEYCHAIN_SERVICE, "-a", index_id])
        .output()?;
    if !output.status.success() {
        return Err(AppInfraError::CaptureIndexEncryption(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn delete_platform_key(_index_id: &str) -> Result<()> {
    Err(AppInfraError::CaptureIndexEncryption(
        "capture index key store is unsupported on this platform".to_string(),
    ))
}

#[cfg(not(target_os = "macos"))]
fn load_platform_key(_index_id: &str) -> Result<Option<String>> {
    Err(AppInfraError::CaptureIndexEncryption(
        "capture index key store is unsupported on this platform".to_string(),
    ))
}

#[cfg(not(target_os = "macos"))]
fn store_platform_key(_index_id: &str, _key: &str) -> Result<()> {
    Err(AppInfraError::CaptureIndexEncryption(
        "capture index key store is unsupported on this platform".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    pub(super) struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        pub(super) fn new(label: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time should be after unix epoch")
                .as_nanos();
            let path =
                std::env::temp_dir().join(format!("capture-index-key-store-{label}-{unique}"));

            fs::create_dir_all(&path).expect("test directory should be created");

            Self { path }
        }

        pub(super) fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    pub(super) fn database_path(base_dir: &Path) -> PathBuf {
        base_dir
            .join(CAPTURE_INDEX_DATABASE_DIR_NAME)
            .join("app.sqlite3")
    }

    pub(super) fn write_identity(base_dir: &Path, index_id: &str) {
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
    fn security_add_command_builds_exact_command_for_hex_values() {
        let command = security_add_command(
            KEYCHAIN_SERVICE,
            "mnema-index-0123456789abcdef",
            "deadbeefdeadbeefdeadbeefdeadbeef",
        )
        .expect("hex/reverse-DNS values should build a command");

        assert_eq!(
            command,
            "add-generic-password -U -s \"com.shaikzeeshan.mnema.capture-index\" -a \"mnema-index-0123456789abcdef\" -w \"deadbeefdeadbeefdeadbeefdeadbeef\"\n"
        );
    }

    #[test]
    fn security_add_command_rejects_values_unsafe_for_double_quoting() {
        for unsafe_value in ["with\"quote", "back\\slash", "new\nline", "carriage\rreturn"] {
            let error = security_add_command(KEYCHAIN_SERVICE, "mnema-index-x", unsafe_value)
                .expect_err("unsafe key should be rejected");
            assert!(error
                .to_string()
                .contains("unsafe for double-quoting"));

            security_add_command(KEYCHAIN_SERVICE, unsafe_value, "deadbeef")
                .expect_err("unsafe account should be rejected");
            security_add_command(unsafe_value, "mnema-index-x", "deadbeef")
                .expect_err("unsafe service should be rejected");
        }
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
