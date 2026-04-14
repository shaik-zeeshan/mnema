use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use sqlx::migrate::Migrate;
use sqlx::{
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
    SqlitePool,
};

use crate::error::Result;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

const DATABASE_DIR_NAME: &str = "db";
const DATABASE_FILE_NAME: &str = "app.sqlite3";

#[derive(Clone)]
pub struct Database {
    pool: SqlitePool,
    database_path: PathBuf,
    migrations_ran: bool,
}

impl Database {
    pub async fn initialize(base_dir: &Path) -> Result<Self> {
        let database_path = prepare_database_path(base_dir)?;
        let pool = connect(&database_path).await?;
        let migrations_ran = has_pending_migrations(&pool).await?;

        MIGRATOR.run(&pool).await?;

        Ok(Self {
            pool,
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

async fn connect(database_path: &Path) -> Result<SqlitePool> {
    let options = SqliteConnectOptions::new()
        .filename(database_path)
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(Duration::from_secs(5));

    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(options)
        .await?;

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
