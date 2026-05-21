use serde::{Deserialize, Serialize};
use sqlx::{Row, Sqlite, SqlitePool, Transaction};

use crate::Result;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CaptureSafetyGapReason {
    CredentialEntry,
}

impl CaptureSafetyGapReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::CredentialEntry => "credential_entry",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CaptureSafetyGapSourceFamily {
    NativeSecureEntry,
    BrowserSecureEntry,
    Mixed,
}

impl CaptureSafetyGapSourceFamily {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NativeSecureEntry => "native_secure_entry",
            Self::BrowserSecureEntry => "browser_secure_entry",
            Self::Mixed => "mixed",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CaptureSafetyGapTerminalStatus {
    Cleared,
    SourceLostFailClosed,
    RecordingStopped,
    UserPauseTookOver,
}

impl CaptureSafetyGapTerminalStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Cleared => "cleared",
            Self::SourceLostFailClosed => "source_lost_fail_closed",
            Self::RecordingStopped => "recording_stopped",
            Self::UserPauseTookOver => "user_pause_took_over",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CaptureSafetyGap {
    pub id: i64,
    pub capture_session_id: String,
    pub reason: CaptureSafetyGapReason,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub source_family: CaptureSafetyGapSourceFamily,
    pub terminal_status: CaptureSafetyGapTerminalStatus,
    pub created_at: String,
}

#[derive(Clone)]
pub struct CaptureSafetyStore {
    pool: SqlitePool,
}

impl CaptureSafetyStore {
    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn start_gap(
        &self,
        capture_session_id: &str,
        reason: CaptureSafetyGapReason,
        source_family: CaptureSafetyGapSourceFamily,
        started_at: &str,
    ) -> Result<CaptureSafetyGap> {
        let id = sqlx::query(
            "INSERT INTO capture_safety_gaps (capture_session_id, reason, source_family, started_at)
             VALUES (?1, ?2, ?3, ?4)",
        )
        .bind(capture_session_id)
        .bind(reason.as_str())
        .bind(source_family.as_str())
        .bind(started_at)
        .execute(&self.pool)
        .await?
        .last_insert_rowid();

        Ok(self
            .get_gap(id)
            .await?
            .expect("inserted capture safety gap should be readable"))
    }

    pub async fn end_gap(
        &self,
        id: i64,
        ended_at: &str,
        terminal_status: CaptureSafetyGapTerminalStatus,
    ) -> Result<Option<CaptureSafetyGap>> {
        sqlx::query(
            "UPDATE capture_safety_gaps
             SET ended_at = ?2, terminal_status = ?3
             WHERE id = ?1 AND ended_at IS NULL",
        )
        .bind(id)
        .bind(ended_at)
        .bind(terminal_status.as_str())
        .execute(&self.pool)
        .await?;

        self.get_gap(id).await
    }

    pub async fn list_gaps_between(
        &self,
        started_at: &str,
        ended_at: &str,
        limit: i64,
    ) -> Result<Vec<CaptureSafetyGap>> {
        let rows = sqlx::query(
            "SELECT id, capture_session_id, reason, started_at, ended_at, source_family, terminal_status, created_at
             FROM capture_safety_gaps
             WHERE started_at <= ?2 AND COALESCE(ended_at, started_at) >= ?1
             ORDER BY started_at ASC, id ASC
             LIMIT ?3",
        )
        .bind(started_at)
        .bind(ended_at)
        .bind(limit.clamp(0, 1_000))
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(map_capture_safety_gap).collect()
    }

    pub async fn get_gap(&self, id: i64) -> Result<Option<CaptureSafetyGap>> {
        let row = sqlx::query(
            "SELECT id, capture_session_id, reason, started_at, ended_at, source_family, terminal_status, created_at
             FROM capture_safety_gaps
             WHERE id = ?1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        row.map(map_capture_safety_gap).transpose()
    }

    pub async fn record_browser_integration_coverage_event(
        &self,
        occurred_at: &str,
        browser_family: &str,
        state: &str,
        reason: &str,
    ) -> Result<i64> {
        let id = sqlx::query(
            "INSERT INTO browser_integration_coverage_events
                (occurred_at, browser_family, state, reason)
             VALUES (?1, ?2, ?3, ?4)",
        )
        .bind(occurred_at)
        .bind(browser_family)
        .bind(state)
        .bind(reason)
        .execute(&self.pool)
        .await?
        .last_insert_rowid();
        Ok(id)
    }
}

pub(crate) async fn delete_capture_safety_gaps_overlapping_in_transaction(
    tx: &mut Transaction<'_, Sqlite>,
    started_at: &str,
    ended_at: &str,
) -> Result<i64> {
    let table_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (
            SELECT 1 FROM sqlite_master
            WHERE type = 'table' AND name = 'capture_safety_gaps'
        )",
    )
    .fetch_one(&mut **tx)
    .await?;
    if !table_exists {
        return Ok(0);
    }

    let deleted = sqlx::query(
        "DELETE FROM capture_safety_gaps
         WHERE started_at <= ?2 AND COALESCE(ended_at, started_at) >= ?1",
    )
    .bind(started_at)
    .bind(ended_at)
    .execute(&mut **tx)
    .await?
    .rows_affected() as i64;
    Ok(deleted)
}

fn map_capture_safety_gap(row: sqlx::sqlite::SqliteRow) -> Result<CaptureSafetyGap> {
    let reason = match row.get::<String, _>("reason").as_str() {
        "credential_entry" => CaptureSafetyGapReason::CredentialEntry,
        value => {
            return Err(crate::AppInfraError::InvalidCaptureSafetyGapReason(
                value.to_string(),
            ));
        }
    };
    let source_family = match row.get::<String, _>("source_family").as_str() {
        "native_secure_entry" => CaptureSafetyGapSourceFamily::NativeSecureEntry,
        "browser_secure_entry" => CaptureSafetyGapSourceFamily::BrowserSecureEntry,
        "mixed" => CaptureSafetyGapSourceFamily::Mixed,
        value => {
            return Err(crate::AppInfraError::InvalidCaptureSafetyGapReason(
                value.to_string(),
            ));
        }
    };
    let terminal_status = match row.get::<String, _>("terminal_status").as_str() {
        "cleared" => CaptureSafetyGapTerminalStatus::Cleared,
        "source_lost_fail_closed" => CaptureSafetyGapTerminalStatus::SourceLostFailClosed,
        "recording_stopped" => CaptureSafetyGapTerminalStatus::RecordingStopped,
        "user_pause_took_over" => CaptureSafetyGapTerminalStatus::UserPauseTookOver,
        value => {
            return Err(crate::AppInfraError::InvalidCaptureSafetyGapReason(
                value.to_string(),
            ));
        }
    };
    Ok(CaptureSafetyGap {
        id: row.get("id"),
        capture_session_id: row.get("capture_session_id"),
        reason,
        started_at: row.get("started_at"),
        ended_at: row.get("ended_at"),
        source_family,
        terminal_status,
        created_at: row.get("created_at"),
    })
}
