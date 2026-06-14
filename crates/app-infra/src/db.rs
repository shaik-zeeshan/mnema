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

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build")
            .block_on(future)
    }

    /// The full embedded migration chain applies cleanly against a fresh
    /// (unencrypted, in-memory) SQLite database. This exercises every
    /// `00NN_*.sql` — including 0024's `ALTER TABLE ... ADD COLUMN` and the new
    /// `user_context_confidence_history` table — so a malformed migration fails
    /// loudly here rather than at app startup.
    #[test]
    fn embedded_migrations_apply_to_fresh_database() {
        block_on(async {
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
        });
    }
}
