use std::{
    collections::HashSet,
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

use sqlx::migrate::Migrate;
use sqlx::{
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
    Row, SqlitePool,
};

use crate::error::{AppInfraError, Result};

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

const DATABASE_DIR_NAME: &str = "db";
const DATABASE_FILE_NAME: &str = "app.sqlite3";
const INDEX_IDENTITY_FILE_NAME: &str = "capture-index.json";
const KEYCHAIN_SERVICE: &str = "com.shaikzeeshan.mnema.capture-index";

#[derive(Clone)]
pub struct Database {
    pool: SqlitePool,
    base_dir: PathBuf,
    database_path: PathBuf,
    migrations_ran: bool,
}

impl Database {
    pub async fn initialize(base_dir: &Path) -> Result<Self> {
        let database_path = prepare_database_path(base_dir)?;
        let encryption = resolve_encryption_material(base_dir, &database_path)?;
        let pool = connect(&database_path, encryption).await?;
        let migrations_ran = has_pending_migrations(&pool).await?;

        MIGRATOR.run(&pool).await?;

        Ok(Self {
            pool,
            base_dir: base_dir.to_path_buf(),
            database_path,
            migrations_ran,
        })
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub fn database_path(&self) -> &Path {
        &self.database_path
    }

    pub fn base_dir(&self) -> &Path {
        &self.base_dir
    }

    pub fn migrations_ran(&self) -> bool {
        self.migrations_ran
    }
}

fn prepare_database_path(base_dir: &Path) -> Result<PathBuf> {
    fs::create_dir_all(base_dir)?;

    let database_dir = base_dir.join(DATABASE_DIR_NAME);
    fs::create_dir_all(&database_dir)?;

    Ok(database_dir.join(DATABASE_FILE_NAME))
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct CaptureIndexIdentity {
    index_id: String,
    encryption_schema_version: u32,
    app_id: String,
}

#[derive(Debug, Clone)]
struct CaptureIndexEncryptionMaterial {
    key: String,
}

async fn connect(
    database_path: &Path,
    encryption: Option<CaptureIndexEncryptionMaterial>,
) -> Result<SqlitePool> {
    let mut options = SqliteConnectOptions::new()
        .filename(database_path)
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(Duration::from_secs(5));

    let mut pool_options = SqlitePoolOptions::new().max_connections(4);
    if let Some(encryption) = encryption {
        let escaped_key = encryption.key.replace('\'', "''");
        options = options.pragma("key", format!("'{escaped_key}'"));
        pool_options = pool_options.after_connect(move |connection, _metadata| {
            Box::pin(async move {
                let row = sqlx::query("PRAGMA cipher_version")
                    .fetch_optional(&mut *connection)
                    .await?;
                let cipher_version = row
                    .and_then(|row| row.try_get::<String, _>(0).ok())
                    .unwrap_or_default();
                if cipher_version.trim().is_empty() {
                    return Err(sqlx::Error::Protocol(
                        "SQLCipher is not available; refusing to open encrypted capture index"
                            .to_string(),
                    ));
                }
                Ok(())
            })
        });
    }

    let pool = pool_options.connect_with(options).await?;

    Ok(pool)
}

fn resolve_encryption_material(
    base_dir: &Path,
    database_path: &Path,
) -> Result<Option<CaptureIndexEncryptionMaterial>> {
    let sidecar_path = base_dir
        .join(DATABASE_DIR_NAME)
        .join(INDEX_IDENTITY_FILE_NAME);
    let mut database_exists = database_path.exists();

    if cfg!(test) && std::env::var("MNEMA_TEST_ENCRYPTED_INDEX").ok().as_deref() != Some("1") {
        return Ok(None);
    }

    if database_exists && is_plaintext_sqlite_database(database_path)? {
        if sidecar_path.exists() && is_empty_plaintext_database_file(database_path)? {
            remove_database_files(database_path)?;
            database_exists = false;
        } else {
            return Ok(None);
        }
    }

    let sidecar_exists = sidecar_path.exists();
    let identity = if sidecar_exists {
        serde_json::from_str::<CaptureIndexIdentity>(&fs::read_to_string(&sidecar_path)?)?
    } else {
        let identity = CaptureIndexIdentity {
            index_id: generate_index_id()?,
            encryption_schema_version: 1,
            app_id: "com.shaikzeeshan.mnema".to_string(),
        };
        fs::write(&sidecar_path, serde_json::to_string_pretty(&identity)?)?;
        identity
    };

    let key = load_or_create_index_key(&identity.index_id, database_exists && sidecar_exists)?;
    Ok(Some(CaptureIndexEncryptionMaterial { key }))
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
    let mut file = fs::File::open("/dev/urandom")?;
    let mut bytes = vec![0_u8; byte_count];
    file.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn load_or_create_index_key(index_id: &str, require_existing: bool) -> Result<String> {
    if let Ok(key_dir) = std::env::var("MNEMA_CAPTURE_INDEX_KEY_DIR") {
        return load_or_create_file_key(Path::new(&key_dir), index_id, require_existing);
    }
    load_or_create_platform_key(index_id, require_existing)
}

fn load_or_create_file_key(
    key_dir: &Path,
    index_id: &str,
    require_existing: bool,
) -> Result<String> {
    fs::create_dir_all(key_dir)?;
    let path = key_dir.join(format!("{index_id}.key"));
    if path.exists() {
        let key = fs::read_to_string(path)?;
        if key.trim().is_empty() {
            return Err(AppInfraError::CaptureIndexEncryption(
                "stored capture index key is empty".to_string(),
            ));
        }
        return Ok(key);
    }
    if require_existing {
        return Err(AppInfraError::CaptureIndexEncryption(format!(
            "capture index key for {index_id} is missing"
        )));
    }
    let key = random_hex(32)?;
    fs::write(path, &key)?;
    Ok(key)
}

#[cfg(target_os = "macos")]
fn load_or_create_platform_key(index_id: &str, require_existing: bool) -> Result<String> {
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
    if lookup.status.success() {
        let key = String::from_utf8_lossy(&lookup.stdout).trim().to_string();
        if !key.is_empty() {
            return Ok(key);
        }
    }
    if require_existing {
        return Err(AppInfraError::CaptureIndexEncryption(format!(
            "capture index key for {index_id} is missing from Keychain"
        )));
    }

    let key = random_hex(32)?;
    let add = Command::new("security")
        .args([
            "add-generic-password",
            "-U",
            "-s",
            KEYCHAIN_SERVICE,
            "-a",
            index_id,
            "-w",
            &key,
        ])
        .output()?;
    if !add.status.success() {
        return Err(AppInfraError::CaptureIndexEncryption(
            String::from_utf8_lossy(&add.stderr).trim().to_string(),
        ));
    }
    Ok(key)
}

#[cfg(not(target_os = "macos"))]
fn load_or_create_platform_key(_index_id: &str, _require_existing: bool) -> Result<String> {
    Err(AppInfraError::CaptureIndexEncryption(
        "capture index key store is unsupported on this platform".to_string(),
    ))
}

async fn has_pending_migrations(pool: &SqlitePool) -> Result<bool> {
    let mut connection = pool.acquire().await?;
    let connection = connection.as_mut();

    connection.ensure_migrations_table().await?;

    let applied_versions = connection
        .list_applied_migrations()
        .await?
        .into_iter()
        .map(|migration| migration.version)
        .collect::<HashSet<_>>();

    Ok(MIGRATOR
        .iter()
        .filter(|migration| !migration.migration_type.is_down_migration())
        .any(|migration| !applied_versions.contains(&migration.version)))
}
