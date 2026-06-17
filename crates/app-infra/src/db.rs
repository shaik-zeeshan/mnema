use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use sqlx::migrate::Migrate;
use sqlx::{
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
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
        let encryption =
            resolve_capture_index_database_key_for_current_process(base_dir, &database_path)?;
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

    let database_dir = base_dir.join(CAPTURE_INDEX_DATABASE_DIR_NAME);
    fs::create_dir_all(&database_dir)?;

    Ok(database_dir.join(DATABASE_FILE_NAME))
}

async fn connect(
    database_path: &Path,
    encryption: Option<CaptureIndexDatabaseKey>,
) -> Result<SqlitePool> {
    register_vec0_auto_extension();

    let mut options = SqliteConnectOptions::new()
        .filename(database_path)
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(Duration::from_secs(5));

    let mut pool_options = SqlitePoolOptions::new().max_connections(4);
    if let Some(encryption) = encryption {
        options = options.pragma("key", encryption.sqlcipher_pragma_value());
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
}
