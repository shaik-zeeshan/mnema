use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use sqlx::migrate::Migrate;
use sqlx::{
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
    Row, SqlitePool,
};

use crate::capture_index_key_store::{
    resolve_capture_index_database_key_for_current_process, CaptureIndexDatabaseKey,
    CAPTURE_INDEX_DATABASE_DIR_NAME,
};
use crate::error::Result;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

const DATABASE_FILE_NAME: &str = "app.sqlite3";

/// Register sqlite-vec's `vec0` virtual table as a SQLite auto-extension so
/// every connection in the encrypted pool can store and KNN-scan Semantic
/// Search Vectors. sqlite-vec is statically linked into the same SQLCipher
/// amalgamation (`SQLITE_CORE`, one `libsqlite3-sys`), so this only flips the
/// auto-extension switch — it does not pull in a second SQLite. Registration is
/// process-global and idempotent via `Once`; it must happen before the first
/// connection opens or that connection will not see `vec0`.
fn register_vec0_auto_extension() {
    static REGISTER: std::sync::Once = std::sync::Once::new();
    REGISTER.call_once(|| {
        // SAFETY: `sqlite3_vec_init` is the sqlite-vec entrypoint; SQLite calls
        // auto-extensions through the `xEntryPoint` ABI below. Transmuting the
        // FFI symbol pointer to that signature is the documented registration
        // shape (mirrors sqlite-vec's own `sqlite3_auto_extension` usage).
        type AutoExtensionEntry = unsafe extern "C" fn(
            db: *mut libsqlite3_sys::sqlite3,
            pz_err_msg: *mut *mut std::os::raw::c_char,
            api: *const libsqlite3_sys::sqlite3_api_routines,
        ) -> std::os::raw::c_int;
        unsafe {
            let entry: AutoExtensionEntry =
                std::mem::transmute(sqlite_vec::sqlite3_vec_init as *const ());
            libsqlite3_sys::sqlite3_auto_extension(Some(entry));
        }
    });
}

/// The transaction-begin statement used for every write transaction. `BEGIN
/// IMMEDIATE` takes SQLite's write (RESERVED) lock up front rather than starting
/// deferred and upgrading read→write later. Upgrading is what produced the
/// instant `SQLITE_BUSY` deadlock between two writers; with IMMEDIATE the second
/// writer simply waits on `busy_timeout` instead, so the **Writer Pool** can hold
/// several connections without reintroducing the deadlock.
const BEGIN_IMMEDIATE: &str = "BEGIN IMMEDIATE";

/// A handle to the Encrypted Capture Index that carries both the **Writer Pool**
/// (`write`) and the **Reader Pool** (`read`). Cheap to clone (the inner
/// `SqlitePool`s are `Arc`-backed). Stores hold this instead of a bare
/// `SqlitePool` and pick `write()`/`read()` per method. Write transactions must
/// begin via [`CaptureDb::begin_write`] (`BEGIN IMMEDIATE`); plain single-statement
/// writes can use `write()` directly. For a **Brokered Reader** both pools are
/// `query_only` handles, so a write routed to `write()` correctly fails — that is
/// the read-only guarantee.
#[derive(Clone)]
pub struct CaptureDb {
    write: SqlitePool,
    read: SqlitePool,
}

impl CaptureDb {
    /// Owner-internal write path: the Writer Pool. Use for single auto-commit
    /// write statements; for multi-statement / read-modify-write transactions use
    /// [`Self::begin_write`].
    pub fn write(&self) -> &SqlitePool {
        &self.write
    }
    /// Read path: the Reader Pool (concurrent with writers under WAL).
    pub fn read(&self) -> &SqlitePool {
        &self.read
    }
    /// Begin a write transaction with `BEGIN IMMEDIATE` (see [`BEGIN_IMMEDIATE`]).
    /// Every explicit writer transaction must start here so the writer-writer
    /// upgrade deadlock cannot occur.
    pub async fn begin_write(
        &self,
    ) -> std::result::Result<sqlx::Transaction<'static, sqlx::Sqlite>, sqlx::Error> {
        self.write.begin_with(BEGIN_IMMEDIATE).await
    }
    /// Test/back-compat helper: use one pool for both roles. ONLY for tests.
    pub fn single(pool: SqlitePool) -> Self {
        Self {
            write: pool.clone(),
            read: pool,
        }
    }
}

impl From<SqlitePool> for CaptureDb {
    fn from(pool: SqlitePool) -> Self {
        Self::single(pool)
    }
}

#[derive(Clone)]
pub struct Database {
    write_pool: SqlitePool,
    read_pool: SqlitePool,
    base_dir: PathBuf,
    database_path: PathBuf,
    migrations_ran: bool,
}

impl Database {
    /// Owner path: sole writer and sole migrator. Builds the Writer Pool and the
    /// Reader Pool, then runs the migrator on the writer only. Write transactions
    /// use `BEGIN IMMEDIATE` (see [`CaptureDb::begin_write`]).
    pub async fn initialize(base_dir: &Path) -> Result<Self> {
        let database_path = prepare_database_path(base_dir)?;
        let encryption =
            resolve_capture_index_database_key_for_current_process(base_dir, &database_path)?;

        let write_pool =
            connect_pool(&database_path, encryption.clone(), PoolConfig::owner_writer()).await?;
        let read_pool =
            connect_pool(&database_path, encryption, PoolConfig::owner_reader()).await?;

        let migrations_ran = has_pending_migrations(&write_pool).await?;

        MIGRATOR.run(&write_pool).await?;

        Ok(Self {
            write_pool,
            read_pool,
            base_dir: base_dir.to_path_buf(),
            database_path,
            migrations_ran,
        })
    }

    /// Brokered Reader path: opens a genuinely read-only handle for out-of-app
    /// readers (the `mnema` CLI, Ask AI). Never runs the migrator and never sets
    /// `journal_mode`; both pools are `query_only`. A WAL recovery read runs
    /// before flipping `query_only=ON` so a `-wal` left by an uncleanly-exited
    /// Owner is folded in.
    pub async fn initialize_brokered_reader(base_dir: &Path) -> Result<Self> {
        let database_path = prepare_database_path(base_dir)?;
        let encryption =
            resolve_capture_index_database_key_for_current_process(base_dir, &database_path)?;

        // Observability: a non-empty `-wal` sidecar at open time implies the
        // Owner exited uncleanly (it would otherwise have checkpointed and
        // truncated the WAL on a clean close).
        if wal_sidecar_is_non_empty(&database_path) {
            capture_runtime::debug_log!(
                "[app-infra] brokered reader opening with non-empty -wal (owner exited uncleanly?)"
            );
        }

        let write_pool =
            connect_pool(&database_path, encryption.clone(), PoolConfig::brokered_writer()).await?;
        let read_pool =
            connect_pool(&database_path, encryption, PoolConfig::brokered_reader()).await?;

        Ok(Self {
            write_pool,
            read_pool,
            base_dir: base_dir.to_path_buf(),
            database_path,
            migrations_ran: false,
        })
    }

    /// A cheap-to-clone `CaptureDb` carrying both pools; stores hold this.
    pub fn handle(&self) -> CaptureDb {
        CaptureDb {
            write: self.write_pool.clone(),
            read: self.read_pool.clone(),
        }
    }

    /// Back-compat / test-only accessor returning the writer pool. Do not call
    /// from new code: use `read()` / `write()` / `begin_write()` (or `read_pool()`
    /// / `write_pool()`). Retained only so existing tests can share a single pool.
    pub fn pool(&self) -> &SqlitePool {
        self.write_pool()
    }

    /// Begin a write transaction with `BEGIN IMMEDIATE` on the Writer Pool. Use
    /// for every explicit writer transaction so the upgrade deadlock cannot occur.
    pub async fn begin_write(
        &self,
    ) -> std::result::Result<sqlx::Transaction<'static, sqlx::Sqlite>, sqlx::Error> {
        self.write_pool.begin_with(BEGIN_IMMEDIATE).await
    }

    pub fn write_pool(&self) -> &SqlitePool {
        &self.write_pool
    }

    pub fn read_pool(&self) -> &SqlitePool {
        &self.read_pool
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

/// Pragma/pool configuration for one role of the Encrypted Capture Index.
/// Folds Owner Writer/Reader and Brokered Writer/Reader into one builder so the
/// pragmas live in exactly one place.
struct PoolConfig {
    max_connections: u32,
    /// Pool `acquire` timeout. `None` keeps sqlx's default (30s). Overridable so
    /// the saturation regression (a held connection starving acquirers) is
    /// observable in a test in <30s rather than wall-clock-blocking the suite.
    acquire_timeout: Option<Duration>,
    busy_timeout: Duration,
    /// Whether to set `journal_mode=WAL`. Brokered readers must NOT touch
    /// `journal_mode` (read-only handle), so this is false for them.
    set_wal: bool,
    /// `synchronous` pragma; only set on the Owner Writer. Left default
    /// everywhere else so durability semantics are unchanged on read/brokered
    /// paths.
    synchronous: Option<SqliteSynchronous>,
    /// `journal_size_limit` in bytes; only bounds the Owner Writer's `-wal`.
    journal_size_limit: Option<u64>,
    /// Apply `PRAGMA query_only=ON` in `after_connect` (after the cipher check).
    query_only: bool,
    create_if_missing: bool,
    /// Run a harmless read in `after_connect` (after the cipher check, before
    /// `query_only`) to force WAL recovery on a brokered open.
    force_wal_recovery: bool,
}

impl PoolConfig {
    fn owner_writer() -> Self {
        Self {
            // SQLite still serializes writers at the lock level, but several
            // connections let independent write transactions queue on the write
            // lock (fairly, via `busy_timeout`) instead of starving on a single
            // pooled connection's acquire — which a `max_connections(1)` writer
            // turned into 30s `pool timed out` errors under capture load. The
            // upgrade deadlock is prevented by `BEGIN IMMEDIATE`, not by a single
            // connection.
            max_connections: 4,
            acquire_timeout: None,
            busy_timeout: Duration::from_secs(10),
            set_wal: true,
            synchronous: Some(SqliteSynchronous::Normal),
            // 64 MiB cap on the WAL sidecar.
            journal_size_limit: Some(67_108_864),
            query_only: false,
            create_if_missing: true,
            force_wal_recovery: false,
        }
    }

    fn owner_reader() -> Self {
        Self {
            max_connections: 4,
            acquire_timeout: None,
            busy_timeout: Duration::from_secs(5),
            set_wal: true,
            synchronous: None,
            journal_size_limit: None,
            query_only: true,
            create_if_missing: true,
            force_wal_recovery: false,
        }
    }

    fn brokered_writer() -> Self {
        Self {
            max_connections: 1,
            acquire_timeout: None,
            busy_timeout: Duration::from_secs(10),
            set_wal: false,
            synchronous: None,
            journal_size_limit: None,
            query_only: true,
            create_if_missing: false,
            force_wal_recovery: true,
        }
    }

    fn brokered_reader() -> Self {
        Self {
            max_connections: 4,
            acquire_timeout: None,
            busy_timeout: Duration::from_secs(5),
            set_wal: false,
            synchronous: None,
            journal_size_limit: None,
            query_only: true,
            create_if_missing: false,
            force_wal_recovery: true,
        }
    }
}

/// The `-wal` sidecar next to `database_path` exists and is non-empty.
fn wal_sidecar_is_non_empty(database_path: &Path) -> bool {
    let mut wal_path = database_path.as_os_str().to_os_string();
    wal_path.push("-wal");
    let wal_path = PathBuf::from(wal_path);
    fs::metadata(&wal_path)
        .map(|metadata| metadata.len() > 0)
        .unwrap_or(false)
}

fn prepare_database_path(base_dir: &Path) -> Result<PathBuf> {
    fs::create_dir_all(base_dir)?;

    let database_dir = base_dir.join(CAPTURE_INDEX_DATABASE_DIR_NAME);
    fs::create_dir_all(&database_dir)?;

    Ok(database_dir.join(DATABASE_FILE_NAME))
}

/// Build one pool of the Encrypted Capture Index for the given `role`'s pragmas.
/// The SQLCipher key + `cipher_version` verification, the WAL-recovery read, and
/// the `query_only` flip all run in `after_connect` in that order.
async fn connect_pool(
    database_path: &Path,
    encryption: Option<CaptureIndexDatabaseKey>,
    config: PoolConfig,
) -> Result<SqlitePool> {
    register_vec0_auto_extension();

    let mut options = SqliteConnectOptions::new()
        .filename(database_path)
        .create_if_missing(config.create_if_missing)
        .foreign_keys(true)
        .busy_timeout(config.busy_timeout);

    if config.set_wal {
        options = options.journal_mode(SqliteJournalMode::Wal);
    }
    if let Some(synchronous) = config.synchronous {
        options = options.synchronous(synchronous);
    }
    if let Some(limit) = config.journal_size_limit {
        options = options.pragma("journal_size_limit", limit.to_string());
    }

    let has_encryption = encryption.is_some();
    if let Some(encryption) = encryption {
        options = options.pragma("key", encryption.sqlcipher_pragma_value());
    }

    let force_wal_recovery = config.force_wal_recovery;
    let query_only = config.query_only;

    let mut pool_options = SqlitePoolOptions::new().max_connections(config.max_connections);
    if let Some(acquire_timeout) = config.acquire_timeout {
        pool_options = pool_options.acquire_timeout(acquire_timeout);
    }
    let pool_options =
        pool_options
            .after_connect(move |connection, _metadata| {
                Box::pin(async move {
                    if has_encryption {
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
                    }
                    // Force WAL recovery BEFORE flipping query_only: a brokered
                    // reader must fold in a `-wal` left by an uncleanly-exited
                    // Owner, and that recovery would be blocked under query_only.
                    if force_wal_recovery {
                        sqlx::query("SELECT count(*) FROM sqlite_master")
                            .fetch_optional(&mut *connection)
                            .await?;
                    }
                    if query_only {
                        sqlx::query("PRAGMA query_only=ON")
                            .execute(&mut *connection)
                            .await?;
                    }
                    Ok(())
                })
            });

    let pool = pool_options.connect_with(options).await?;

    Ok(pool)
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

#[cfg(test)]
mod tests {
    use super::*;

    use std::time::{SystemTime, UNIX_EPOCH};

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build")
            .block_on(future)
    }

    /// A fresh, unique temp directory for an on-disk encrypted test database,
    /// matching the crate's `std::env::temp_dir()` test idiom.
    fn unique_test_dir(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("app-infra-db-{label}-{unique}"));
        fs::create_dir_all(&path).expect("test directory should be created");
        path
    }

    /// Opens an SQLCipher-encrypted pool against a fresh on-disk database,
    /// mirroring `connect`'s key/PRAGMA wiring, and registers the `vec0`
    /// auto-extension so Semantic Search Vectors can be stored and KNN-scanned
    /// inside the Encrypted Capture Index.
    async fn open_encrypted_pool(database_path: &Path, key: &str) -> SqlitePool {
        register_vec0_auto_extension();

        let escaped_key = key.replace('\'', "''");
        let pragma_value = format!("'{escaped_key}'");

        let options = SqliteConnectOptions::new()
            .filename(database_path)
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(Duration::from_secs(5))
            .pragma("key", pragma_value);

        SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .expect("encrypted db should open")
    }

    /// A brokered reader is genuinely read-only: a write routed to the writer
    /// pool fails because `query_only=ON`, while reads through the reader pool
    /// succeed. Open an Owner first to create + migrate the DB, close it cleanly,
    /// then open the brokered reader against it.
    #[test]
    fn brokered_reader_rejects_writes() {
        block_on(async {
            let dir = unique_test_dir("brokered-readonly");
            let owner = Database::initialize(&dir).await.expect("owner init");
            owner.write_pool().close().await;
            owner.read_pool().close().await;
            drop(owner);

            let brokered = Database::initialize_brokered_reader(&dir)
                .await
                .expect("brokered init");

            // A write through the writer pool fails under query_only=ON.
            let write_result = sqlx::query(
                "INSERT INTO frames (session_id, file_path, captured_at) \
                 VALUES ('sess-x', '/frames/x.jpg', '2026-06-17T00:00:00Z')",
            )
            .execute(brokered.write_pool())
            .await;
            assert!(
                write_result.is_err(),
                "brokered writer pool must reject writes under query_only"
            );

            // Reads through the reader pool still work.
            let frame_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM frames")
                .fetch_one(brokered.read_pool())
                .await
                .expect("brokered read should succeed");
            assert_eq!(frame_count, 0, "no frames were inserted");
        });
    }

    /// Opening a brokered reader does NOT run the migrator: the
    /// `_sqlx_migrations` table is untouched (no rows added) and `migrations_ran`
    /// is false, while a `SELECT` against the existing schema still works.
    #[test]
    fn brokered_reader_does_not_run_migrator() {
        block_on(async {
            let dir = unique_test_dir("brokered-no-migrate");
            let owner = Database::initialize(&dir).await.expect("owner init");
            let before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations")
                .fetch_one(owner.write_pool())
                .await
                .expect("count migrations before");
            assert!(before > 0, "owner should have applied migrations");
            owner.write_pool().close().await;
            owner.read_pool().close().await;
            drop(owner);

            let brokered = Database::initialize_brokered_reader(&dir)
                .await
                .expect("brokered init");
            assert!(
                !brokered.migrations_ran(),
                "brokered reader must report migrations_ran = false"
            );

            let after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations")
                .fetch_one(brokered.read_pool())
                .await
                .expect("count migrations after");
            assert_eq!(
                after, before,
                "brokered reader must not add migration rows"
            );
        });
    }

    /// A brokered reader opens and reads a cleanly-closed database, seeing rows
    /// the Owner wrote before it exited.
    #[test]
    fn brokered_reader_reads_cleanly_closed_db() {
        block_on(async {
            let dir = unique_test_dir("brokered-clean-read");
            let owner = Database::initialize(&dir).await.expect("owner init");
            sqlx::query(
                "INSERT INTO frames (session_id, file_path, captured_at) \
                 VALUES ('sess-1', '/frames/1.jpg', '2026-06-17T00:00:00Z')",
            )
            .execute(owner.write_pool())
            .await
            .expect("owner write");
            owner.write_pool().close().await;
            owner.read_pool().close().await;
            drop(owner);

            let brokered = Database::initialize_brokered_reader(&dir)
                .await
                .expect("brokered init");
            let frame_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM frames")
                .fetch_one(brokered.read_pool())
                .await
                .expect("brokered read should succeed");
            assert_eq!(frame_count, 1, "brokered reader should see the owner's row");
        });
    }

    /// A f32 embedding serialized as the little-endian byte BLOB that `vec0`
    /// expects for a `float[N]` column — the one canonical serializer, shared
    /// with the worker store and query path.
    fn embedding_blob(values: &[f32]) -> Vec<u8> {
        crate::semantic_search::vector_to_le_bytes(values)
    }

    /// The full embedded migration chain applies cleanly against a fresh
    /// (unencrypted, in-memory) SQLite database. This exercises every
    /// `00NN_*.sql` — including 0024's `ALTER TABLE ... ADD COLUMN`, the
    /// `user_context_confidence_history` table, and 0039's `vec0` Semantic
    /// Search Vector table — so a malformed migration fails loudly here rather
    /// than at app startup. The `vec0` table requires the auto-extension to be
    /// registered first.
    #[test]
    fn embedded_migrations_apply_to_fresh_database() {
        block_on(async {
            register_vec0_auto_extension();
            let pool = SqlitePoolOptions::new()
                .max_connections(1)
                .connect("sqlite::memory:")
                .await
                .expect("in-memory db should open");
            MIGRATOR.run(&pool).await.expect("migrations should apply");

            // 0024 added the confidence-history table.
            let history_exists: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM sqlite_master \
                 WHERE type = 'table' AND name = 'user_context_confidence_history'",
            )
            .fetch_one(&pool)
            .await
            .expect("query sqlite_master");
            assert_eq!(history_exists, 1, "confidence-history table should exist");

            // 0024 added the last_decayed_at_ms column to user_context_conclusions.
            let column_exists: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM pragma_table_info('user_context_conclusions') \
                 WHERE name = 'last_decayed_at_ms'",
            )
            .fetch_one(&pool)
            .await
            .expect("query pragma_table_info");
            assert_eq!(column_exists, 1, "last_decayed_at_ms column should exist");

            // 0039 added the Semantic Search Vector substrate.
            let vec_table_exists: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM sqlite_master \
                 WHERE type = 'table' AND name = 'search_document_vectors'",
            )
            .fetch_one(&pool)
            .await
            .expect("query sqlite_master");
            assert_eq!(
                vec_table_exists, 1,
                "search_document_vectors vec0 table should exist"
            );
        });
    }

    /// The migration chain (including 0039) is idempotent against an existing
    /// database: running every migration a second time on the same encrypted
    /// pool is a no-op and the `vec0` table survives.
    #[test]
    fn embedded_migrations_apply_to_existing_database() {
        block_on(async {
            let dir = unique_test_dir("existing");
            let database_path = dir.join("existing.sqlite3");
            let pool = open_encrypted_pool(&database_path, "round-trip-key").await;

            MIGRATOR.run(&pool).await.expect("first run should apply");
            MIGRATOR
                .run(&pool)
                .await
                .expect("second run on existing db should be a no-op");

            let vec_table_exists: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM sqlite_master \
                 WHERE type = 'table' AND name = 'search_document_vectors'",
            )
            .fetch_one(&pool)
            .await
            .expect("query sqlite_master");
            assert_eq!(vec_table_exists, 1, "vec0 table should still exist");
        });
    }

    /// `vec0` KNN and SQLCipher encryption-at-rest coexist in the same pool: the
    /// pool reports an SQLCipher `cipher_version`, a Semantic Search Vector
    /// round-trips through the encrypted store, and a KNN scan returns it.
    #[test]
    fn vec0_knn_round_trips_inside_encrypted_db() {
        block_on(async {
            let dir = unique_test_dir("knn");
            let database_path = dir.join("knn.sqlite3");
            let pool = open_encrypted_pool(&database_path, "knn-key").await;
            MIGRATOR.run(&pool).await.expect("migrations should apply");

            // The pool is genuinely SQLCipher-encrypted.
            let cipher_version: String = sqlx::query_scalar("PRAGMA cipher_version")
                .fetch_one(&pool)
                .await
                .expect("cipher_version");
            assert!(
                !cipher_version.trim().is_empty(),
                "encrypted pool should report an SQLCipher cipher_version, got {cipher_version:?}"
            );

            // vec0 is loadable in the encrypted pool.
            let vec_version: String = sqlx::query_scalar("SELECT vec_version()")
                .fetch_one(&pool)
                .await
                .expect("vec_version");
            assert!(
                vec_version.starts_with('v'),
                "vec0 should be loadable, got {vec_version:?}"
            );

            // Store two Semantic Search Vectors keyed to anchor rowids.
            let near = embedding_blob(&vec![1.0_f32; 768]);
            let far = {
                let mut values = vec![0.0_f32; 768];
                values[0] = -1.0;
                embedding_blob(&values)
            };
            sqlx::query("INSERT INTO search_document_vectors (rowid, embedding) VALUES (?, ?)")
                .bind(1_i64)
                .bind(&near)
                .execute(&pool)
                .await
                .expect("insert near vector");
            sqlx::query("INSERT INTO search_document_vectors (rowid, embedding) VALUES (?, ?)")
                .bind(2_i64)
                .bind(&far)
                .execute(&pool)
                .await
                .expect("insert far vector");

            // KNN inside the encrypted DB returns the nearest rowid first.
            let query_vec = embedding_blob(&vec![1.0_f32; 768]);
            let nearest: i64 = sqlx::query_scalar(
                "SELECT rowid FROM search_document_vectors \
                 WHERE embedding MATCH ? ORDER BY distance LIMIT 1",
            )
            .bind(&query_vec)
            .fetch_one(&pool)
            .await
            .expect("knn query");
            assert_eq!(nearest, 1, "nearest neighbor should be the matching anchor");
        });
    }

    /// The Semantic Search Vector lifecycle is nearly free: deleting a `frames`
    /// row CASCADEs to its `search_documents` anchor, which fires the
    /// `search_document_vectors_after_delete` trigger and drops the vector — the
    /// load-bearing path (retention / Delete Recent), not a direct
    /// `DELETE FROM search_documents`.
    #[test]
    fn cascade_frame_delete_drops_semantic_vector() {
        block_on(async {
            let dir = unique_test_dir("cascade");
            let database_path = dir.join("cascade.sqlite3");
            let pool = open_encrypted_pool(&database_path, "cascade-key").await;
            MIGRATOR.run(&pool).await.expect("migrations should apply");

            // A captured frame and its direct Search Result Anchor.
            let frame_id: i64 = sqlx::query_scalar(
                "INSERT INTO frames (session_id, file_path, captured_at) \
                 VALUES ('sess-1', '/frames/1.jpg', '2026-06-17T00:00:00Z') RETURNING id",
            )
            .fetch_one(&pool)
            .await
            .expect("insert frame");

            let anchor_id: i64 = sqlx::query_scalar(
                "INSERT INTO search_documents \
                 (anchor_type, frame_id, absolute_start_at, absolute_end_at, \
                  session_id, group_key, text_source_kind, body_text) \
                 VALUES ('frame', ?, '2026-06-17T00:00:00Z', '2026-06-17T00:00:01Z', \
                  'sess-1', 'grp-1', 'direct', 'hello world') RETURNING id",
            )
            .bind(frame_id)
            .fetch_one(&pool)
            .await
            .expect("insert anchor");

            // Project the anchor into the FTS5 external-content index, exactly as
            // production does on insert. The shipping `search_documents_fts_after_delete`
            // trigger fires alongside ours during the same cascade, so the FTS content
            // must exist or its `'delete'` command would itself corrupt the index — this
            // makes the test exercise the real FTS + vec0 coexistence under one cascade.
            sqlx::query(
                "INSERT INTO search_documents_fts (rowid, body_text, context_text) \
                 VALUES (?, 'hello world', '')",
            )
            .bind(anchor_id)
            .execute(&pool)
            .await
            .expect("project anchor into fts");

            // Its Semantic Search Vector.
            sqlx::query("INSERT INTO search_document_vectors (rowid, embedding) VALUES (?, ?)")
                .bind(anchor_id)
                .bind(embedding_blob(&vec![0.5_f32; 768]))
                .execute(&pool)
                .await
                .expect("insert vector");

            let before: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM search_document_vectors WHERE rowid = ?")
                    .bind(anchor_id)
                    .fetch_one(&pool)
                    .await
                    .expect("count before");
            assert_eq!(before, 1, "vector should exist before delete");

            // Delete the frame; CASCADE removes the anchor, the AFTER DELETE
            // trigger removes the vector.
            sqlx::query("DELETE FROM frames WHERE id = ?")
                .bind(frame_id)
                .execute(&pool)
                .await
                .expect("delete frame");

            let anchor_gone: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM search_documents WHERE id = ?")
                    .bind(anchor_id)
                    .fetch_one(&pool)
                    .await
                    .expect("count anchor after");
            assert_eq!(anchor_gone, 0, "anchor should be CASCADE-deleted");

            let after: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM search_document_vectors WHERE rowid = ?")
                    .bind(anchor_id)
                    .fetch_one(&pool)
                    .await
                    .expect("count after");
            assert_eq!(
                after, 0,
                "trigger should drop the vector on CASCADE-driven frame delete"
            );
        });
    }

    /// Headline concurrency regression: an Owner (single Writer Connection +
    /// Reader Pool) plus two Brokered Readers all hammer the SAME on-disk
    /// encrypted database concurrently — many writes through `write_pool()`,
    /// many reads through `read_pool()`, and reads through each Brokered Reader —
    /// and NO operation returns a `SQLITE_BUSY` / "database is locked" error.
    /// This is the structural guarantee of the single-writer + read-only-broker
    /// design (ADR 0041); it also clears the `write_pool` dead-code warning.
    ///
    /// Runs on a multi-thread runtime so the tasks are genuinely concurrent.
    #[test]
    fn concurrent_owner_writes_reads_and_brokered_reads_never_lock() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .enable_all()
            .build()
            .expect("multi-thread runtime should build");

        runtime.block_on(async {
            const ITERS: usize = 200;

            let dir = unique_test_dir("concurrency");
            let owner = Database::initialize(&dir).await.expect("owner init");

            // Two Brokered Readers opened against the LIVE owner (the CLI / Ask AI
            // shape: read-write OS handle made read-only via query_only).
            let brokered_a = Database::initialize_brokered_reader(&dir)
                .await
                .expect("brokered reader a init");
            let brokered_b = Database::initialize_brokered_reader(&dir)
                .await
                .expect("brokered reader b init");

            let mut handles = Vec::new();

            // Writers — single auto-commit inserts through the Writer Pool.
            for writer in 0..3 {
                let write_pool = owner.write_pool().clone();
                handles.push(tokio::spawn(async move {
                    let mut errors = Vec::new();
                    for i in 0..ITERS {
                        let result = sqlx::query(
                            "INSERT INTO frames (session_id, file_path, captured_at) \
                             VALUES (?, ?, '2026-06-17T00:00:00Z')",
                        )
                        .bind(format!("sess-{writer}"))
                        .bind(format!("/frames/{writer}-{i}.jpg"))
                        .execute(&write_pool)
                        .await;
                        if let Err(error) = result {
                            errors.push(error.to_string());
                        }
                    }
                    errors
                }));
            }

            // Owner reads — through the Reader Pool, concurrent with the writer
            // under WAL.
            for _ in 0..3 {
                let read_pool = owner.read_pool().clone();
                handles.push(tokio::spawn(async move {
                    let mut errors = Vec::new();
                    for _ in 0..ITERS {
                        let result = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM frames")
                            .fetch_one(&read_pool)
                            .await;
                        if let Err(error) = result {
                            errors.push(error.to_string());
                        }
                    }
                    errors
                }));
            }

            // Brokered reads — each Brokered Reader's own read-only pool.
            for brokered in [&brokered_a, &brokered_b] {
                let read_pool = brokered.read_pool().clone();
                handles.push(tokio::spawn(async move {
                    let mut errors = Vec::new();
                    for _ in 0..ITERS {
                        let result = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM frames")
                            .fetch_one(&read_pool)
                            .await;
                        if let Err(error) = result {
                            errors.push(error.to_string());
                        }
                    }
                    errors
                }));
            }

            let mut all_errors = Vec::new();
            for handle in handles {
                all_errors.extend(handle.await.expect("task should join"));
            }

            let lock_errors: Vec<&String> = all_errors
                .iter()
                .filter(|message| {
                    let lower = message.to_lowercase();
                    lower.contains("database is locked") || lower.contains("sqlite_busy")
                })
                .collect();
            assert!(
                lock_errors.is_empty(),
                "no operation should hit a lock error, got: {lock_errors:?}"
            );
            // Stronger: nothing should error at all under the new design.
            assert!(
                all_errors.is_empty(),
                "no concurrent operation should error, got: {all_errors:?}"
            );
        });
    }

    /// Concurrent read-modify-write TRANSACTIONS do not deadlock or time out.
    ///
    /// This is the regression guard for the writer-pool design (ADR 0041): each
    /// task opens a write transaction that first reads then writes. With a
    /// multi-connection writer pool and *deferred* (`BEGIN`) transactions this is
    /// exactly the read→write upgrade that returns an instant `SQLITE_BUSY`
    /// deadlock; with a single-connection writer pool it instead starves on the
    /// pool `acquire` and surfaces as `pool timed out while waiting for an open
    /// connection`. `begin_write` (`BEGIN IMMEDIATE`) on a multi-connection writer
    /// pool resolves both — the second writer waits on `busy_timeout` and then
    /// proceeds, so every transaction must commit cleanly.
    #[test]
    fn concurrent_read_modify_write_transactions_never_lock_or_timeout() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .enable_all()
            .build()
            .expect("multi-thread runtime should build");

        runtime.block_on(async {
            const TASKS: usize = 6;
            const ITERS: usize = 60;

            let dir = unique_test_dir("rmw-transactions");
            let owner = Database::initialize(&dir).await.expect("owner init");
            let handle = owner.handle();

            let mut tasks = Vec::new();
            for task in 0..TASKS {
                let db = handle.clone();
                tasks.push(tokio::spawn(async move {
                    let mut errors = Vec::new();
                    for i in 0..ITERS {
                        let attempt = async {
                            let mut tx = db.begin_write().await?;
                            // Read inside the transaction...
                            let _count: i64 =
                                sqlx::query_scalar("SELECT COUNT(*) FROM frames")
                                    .fetch_one(&mut *tx)
                                    .await?;
                            // ...then write inside the same transaction (the
                            // read→write that deadlocks under deferred begins).
                            sqlx::query(
                                "INSERT INTO frames (session_id, file_path, captured_at) \
                                 VALUES (?, ?, '2026-06-17T00:00:00Z')",
                            )
                            .bind(format!("rmw-{task}"))
                            .bind(format!("/frames/rmw-{task}-{i}.jpg"))
                            .execute(&mut *tx)
                            .await?;
                            tx.commit().await?;
                            Ok::<(), sqlx::Error>(())
                        };
                        if let Err(error) = attempt.await {
                            errors.push(error.to_string());
                        }
                    }
                    errors
                }));
            }

            let mut all_errors = Vec::new();
            for task in tasks {
                all_errors.extend(task.await.expect("task should join"));
            }
            assert!(
                all_errors.is_empty(),
                "concurrent read-modify-write transactions should never lock or \
                 time out, got: {all_errors:?}"
            );

            // Every transaction committed: TASKS * ITERS rows landed.
            let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM frames")
                .fetch_one(owner.read_pool())
                .await
                .expect("count frames");
            assert_eq!(total, (TASKS * ITERS) as i64, "all inserts should commit");
        });
    }

    /// Drive the acquire-saturation scenario against a writer pool of the given
    /// `max_connections`: a 1s `acquire_timeout`, one connection held ~2s, and
    /// `2 * max_connections` concurrent single-statement writers. Returns each
    /// writer's error (if any) plus the committed row count.
    ///
    /// The holder takes a pooled connection via a *read* transaction — it does
    /// NOT take the write lock, so this isolates the **pool-acquire** dimension
    /// (the lever ADR 0041's amendment moved from 1→4 connections) from the
    /// `BEGIN`-vs-`BEGIN IMMEDIATE` write-lock deadlock that
    /// `concurrent_read_modify_write_transactions_never_lock_or_timeout` covers.
    async fn run_acquire_saturation(max_connections: u32) -> (Vec<String>, i64) {
        let dir = unique_test_dir(&format!("acquire-sat-{max_connections}"));
        let database_path = prepare_database_path(&dir).expect("db path");
        let pool = connect_pool(
            &database_path,
            None,
            PoolConfig {
                max_connections,
                acquire_timeout: Some(Duration::from_secs(1)),
                ..PoolConfig::owner_writer()
            },
        )
        .await
        .expect("writer pool should open");
        sqlx::query("CREATE TABLE IF NOT EXISTS sat (id INTEGER PRIMARY KEY AUTOINCREMENT, v TEXT)")
            .execute(&pool)
            .await
            .expect("create table");

        // Hold one pooled connection for ~2s (longer than the 1s acquire
        // timeout). A read tx holds the connection without the write lock.
        let (acquired_tx, acquired_rx) = tokio::sync::oneshot::channel();
        let holder_pool = pool.clone();
        let holder = tokio::spawn(async move {
            let mut tx = holder_pool.begin().await.expect("holder begin");
            let _n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sat")
                .fetch_one(&mut *tx)
                .await
                .expect("holder read");
            acquired_tx.send(()).expect("signal holder acquired");
            tokio::time::sleep(Duration::from_secs(2)).await;
            tx.commit().await.expect("holder commit");
        });
        acquired_rx.await.expect("holder should acquire its connection");

        // Now fire 2 * max_connections single-statement writers at the pool.
        let writer_count = (max_connections * 2) as usize;
        let mut writers = Vec::new();
        for i in 0..writer_count {
            let writer_pool = pool.clone();
            writers.push(tokio::spawn(async move {
                sqlx::query("INSERT INTO sat (v) VALUES (?)")
                    .bind(format!("w-{i}"))
                    .execute(&writer_pool)
                    .await
                    .map(|_| ())
                    .map_err(|error| error.to_string())
            }));
        }
        let mut errors = Vec::new();
        for writer in writers {
            if let Err(message) = writer.await.expect("writer task should join") {
                errors.push(message);
            }
        }
        holder.await.expect("holder task should join");

        let committed: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sat")
            .fetch_one(&pool)
            .await
            .expect("count committed rows");
        (errors, committed)
    }

    /// Writer-Pool saturation guard (ADR 0041 amendment). One connection held
    /// for ~2s must NOT starve concurrent writers on a multi-connection pool:
    /// every writer still acquires a connection and commits within the 1s
    /// acquire timeout. The single-connection witness documents the regression
    /// the amendment fixed — there, the held connection starves every acquirer
    /// and they surface `pool timed out`. (The named
    /// `concurrent_read_modify_write_transactions_never_lock_or_timeout` guard
    /// passes even at `max_connections=1`, so it does NOT catch this; this one
    /// does.)
    #[test]
    fn held_writer_does_not_starve_concurrent_writers_on_acquire() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .enable_all()
            .build()
            .expect("multi-thread runtime should build");

        runtime.block_on(async {
            // Fix: owner_writer's 4 connections absorb the held one; all 8
            // writers acquire and commit, none time out on acquire.
            let (errors, committed) = run_acquire_saturation(4).await;
            let timed_out: Vec<&String> = errors
                .iter()
                .filter(|message| message.to_lowercase().contains("pool timed out"))
                .collect();
            assert!(
                timed_out.is_empty(),
                "a multi-connection writer pool must not starve acquirers under a \
                 held connection, got: {timed_out:?}"
            );
            assert!(
                errors.is_empty(),
                "every concurrent writer should commit cleanly, got: {errors:?}"
            );
            assert_eq!(committed, 8, "all 8 single-statement writers committed");

            // Regression witness: a single-connection writer pool starves — the
            // held connection blocks every writer's acquire past the 1s timeout.
            let (errors_single, _committed_single) = run_acquire_saturation(1).await;
            let timed_out_single: Vec<&String> = errors_single
                .iter()
                .filter(|message| message.to_lowercase().contains("pool timed out"))
                .collect();
            assert!(
                !timed_out_single.is_empty(),
                "a single-connection writer pool must surface 'pool timed out' \
                 when one connection is held; got errors: {errors_single:?}"
            );
        });
    }

    /// WAL concurrency: a read held open on the Reader Pool does NOT block a
    /// write on the Writer Connection. Begin a read transaction and run a SELECT
    /// to hold a read lock, then — while it is still open — issue a write on the
    /// Writer Connection and assert it succeeds (no lock error), then finish the
    /// read. Deterministic: no sleeps, no iteration counts.
    #[test]
    fn reader_does_not_block_writer() {
        block_on(async {
            let dir = unique_test_dir("reader-no-block-writer");
            let owner = Database::initialize(&dir).await.expect("owner init");

            // Hold a read open inside a transaction on the Reader Pool.
            let mut read_tx = owner.read_pool().begin().await.expect("begin read tx");
            let _held: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM frames")
                .fetch_one(&mut *read_tx)
                .await
                .expect("read inside held tx");

            // While the read is in flight, a write on the Writer Connection must
            // complete without a lock error.
            sqlx::query(
                "INSERT INTO frames (session_id, file_path, captured_at) \
                 VALUES ('sess-concurrent', '/frames/c.jpg', '2026-06-17T00:00:00Z')",
            )
            .execute(owner.write_pool())
            .await
            .expect("writer must not be blocked by the held reader");

            // Finish the read.
            read_tx.rollback().await.expect("rollback read tx");

            let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM frames")
                .fetch_one(owner.read_pool())
                .await
                .expect("post-write read");
            assert_eq!(count, 1, "the concurrent write should be visible");
        });
    }

    /// Opening a Brokered Reader does NOT change `journal_mode`: the Owner put
    /// the database into WAL, and the Brokered Reader (which never sets
    /// `journal_mode`) still observes WAL rather than resetting it to the
    /// `delete` default. Complements `brokered_reader_does_not_run_migrator`.
    #[test]
    fn brokered_reader_preserves_journal_mode() {
        block_on(async {
            let dir = unique_test_dir("brokered-journal-mode");
            let owner = Database::initialize(&dir).await.expect("owner init");
            let owner_mode: String = sqlx::query_scalar("PRAGMA journal_mode")
                .fetch_one(owner.write_pool())
                .await
                .expect("owner journal_mode");
            assert_eq!(
                owner_mode.to_lowercase(),
                "wal",
                "owner should put the database into WAL"
            );
            owner.write_pool().close().await;
            owner.read_pool().close().await;
            drop(owner);

            let brokered = Database::initialize_brokered_reader(&dir)
                .await
                .expect("brokered init");
            let brokered_mode: String = sqlx::query_scalar("PRAGMA journal_mode")
                .fetch_one(brokered.read_pool())
                .await
                .expect("brokered journal_mode");
            assert_eq!(
                brokered_mode.to_lowercase(),
                "wal",
                "brokered reader must not change journal_mode away from WAL"
            );
        });
    }

    /// A Brokered Reader reads correctly when the `-wal` sidecar is non-empty at
    /// open time, exercising the forced-WAL-recovery-before-`query_only` path.
    ///
    /// NOTE on the crash case: a TRUE no-owner dirty `-wal` (Owner crashed,
    /// nothing open) is not deterministically reproducible in-process, because
    /// sqlx checkpoints the WAL on last-connection close — so we keep the Owner
    /// open here (its uncheckpointed commit lives in the `-wal`) to guarantee a
    /// non-empty sidecar. The crashed-Owner scenario is otherwise covered by the
    /// brokered path's `force_wal_recovery` read, which runs before `query_only`
    /// is flipped on precisely so a `-wal` left behind can be folded in; the
    /// clean-close case is `brokered_reader_reads_cleanly_closed_db`.
    #[test]
    fn brokered_reader_reads_against_non_empty_wal() {
        block_on(async {
            let dir = unique_test_dir("brokered-nonempty-wal");
            let owner = Database::initialize(&dir).await.expect("owner init");
            sqlx::query(
                "INSERT INTO frames (session_id, file_path, captured_at) \
                 VALUES ('sess-wal', '/frames/wal.jpg', '2026-06-17T00:00:00Z')",
            )
            .execute(owner.write_pool())
            .await
            .expect("owner write");

            // The commit lives in the `-wal` (single small write, no checkpoint).
            assert!(
                wal_sidecar_is_non_empty(owner.database_path()),
                "the uncheckpointed commit should leave a non-empty -wal"
            );

            let brokered = Database::initialize_brokered_reader(&dir)
                .await
                .expect("brokered init");
            let frame_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM frames")
                .fetch_one(brokered.read_pool())
                .await
                .expect("brokered read should succeed against a non-empty -wal");
            assert_eq!(
                frame_count, 1,
                "brokered reader should fold in the -wal commit"
            );
        });
    }

    /// Sentinel env var that re-purposes this very test binary as the crashing
    /// Owner child of `brokered_reader_recovers_crashed_owner_dirty_wal`: when it
    /// is set, the re-exec'd test acts as the Owner, commits a row, and aborts.
    #[cfg(unix)]
    const WAL_CRASH_HELPER_ENV: &str = "MNEMA_WAL_CRASH_HELPER";

    /// The real crashed-Owner recovery path: an Owner process commits a row, then
    /// dies via `abort()` WITHOUT closing its pool, leaving a dirty `-wal` with NO
    /// live owner — the exact post-crash state that
    /// `initialize_brokered_reader`'s `force_wal_recovery` read (run *before*
    /// flipping `query_only=ON`) exists to fold back in. This is the cross-process
    /// scenario `brokered_reader_reads_against_non_empty_wal` can only approximate
    /// in-process (it must keep the Owner alive to hold a non-empty `-wal`).
    ///
    /// The test re-execs *itself* (`--exact <this test>`) as the crashing Owner:
    /// the child branch (sentinel env set) commits + aborts; the parent branch
    /// (env unset) spawns it, waits for the SIGABRT, then opens the brokered
    /// reader and asserts the committed row survives. Unix-only because the
    /// not-flaky guarantee keys off asserting termination-by-signal.
    #[cfg(unix)]
    #[test]
    fn brokered_reader_recovers_crashed_owner_dirty_wal() {
        // Child role: re-exec'd by the parent below with the sentinel set. Acts as
        // the Owner that crashes mid-life so the `-wal` is left dirty, no owner.
        if let Ok(dir) = std::env::var(WAL_CRASH_HELPER_ENV) {
            block_on(async {
                let owner = Database::initialize(Path::new(&dir))
                    .await
                    .expect("crash-helper owner init");
                sqlx::query(
                    "INSERT INTO frames (session_id, file_path, captured_at) \
                     VALUES ('sess-crash', '/frames/crash.jpg', '2026-06-17T00:00:00Z')",
                )
                .execute(owner.write_pool())
                .await
                .expect("crash-helper owner write");
                // Crash with the pool still OPEN: the committed row lives in the
                // uncheckpointed `-wal`, and no clean close (which would checkpoint
                // + truncate it) ever runs. `abort()` raises SIGABRT immediately,
                // so no destructor folds the WAL back into the main DB.
                std::process::abort();
            });
            unreachable!("crash-helper child must abort before block_on returns");
        }

        // Parent role.
        use std::os::unix::process::ExitStatusExt;

        let dir = unique_test_dir("brokered-crash-wal");
        let exe = std::env::current_exe().expect("current test exe");
        let status = std::process::Command::new(exe)
            .args([
                "db::tests::brokered_reader_recovers_crashed_owner_dirty_wal",
                "--exact",
                "--test-threads=1",
            ])
            .env(WAL_CRASH_HELPER_ENV, &dir)
            .status()
            .expect("spawn crashing-owner child");

        // The child must have died by signal (SIGABRT = 6), never exited cleanly:
        // a clean exit would mean it closed/checkpointed its pool, which would
        // have truncated the `-wal` and defeated the dirty-WAL scenario.
        assert!(
            status.code().is_none(),
            "crashing-owner child should be killed by a signal, not exit with a code: {status:?}"
        );
        assert_eq!(
            status.signal(),
            Some(6),
            "crashing-owner child should terminate via SIGABRT"
        );

        // The crash left a dirty `-wal` with no live owner.
        let database_path = prepare_database_path(&dir).expect("database path");
        assert!(
            wal_sidecar_is_non_empty(&database_path),
            "the crashed owner should leave a non-empty -wal"
        );

        // The brokered reader's force-WAL-recovery-before-`query_only` path must
        // fold in the crashed Owner's committed row.
        block_on(async {
            let brokered = Database::initialize_brokered_reader(&dir)
                .await
                .expect("brokered init against crashed-owner dirty -wal");
            let frame_count: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM frames WHERE session_id = 'sess-crash'")
                    .fetch_one(brokered.read_pool())
                    .await
                    .expect("brokered read should succeed against a crashed-owner -wal");
            assert_eq!(
                frame_count, 1,
                "brokered reader must recover the crashed owner's committed row"
            );
        });
    }
}
