use serde::{Deserialize, Serialize};
use sqlx::{sqlite::SqliteRow, QueryBuilder, Row, Sqlite, SqlitePool};

use crate::Result;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AudioSegmentSourceKind {
    Microphone,
    SystemAudio,
}

impl AudioSegmentSourceKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Microphone => "microphone",
            Self::SystemAudio => "system_audio",
        }
    }

    fn from_str(value: &str) -> Self {
        match value {
            "system_audio" => Self::SystemAudio,
            _ => Self::Microphone,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NewAudioSegment {
    pub source_kind: AudioSegmentSourceKind,
    pub source_session_id: String,
    pub segment_index: i64,
    pub file_path: String,
    pub started_at: String,
    pub ended_at: String,
}

impl NewAudioSegment {
    pub fn new(
        source_kind: AudioSegmentSourceKind,
        source_session_id: impl Into<String>,
        segment_index: i64,
        file_path: impl Into<String>,
        started_at: impl Into<String>,
        ended_at: impl Into<String>,
    ) -> Self {
        Self {
            source_kind,
            source_session_id: source_session_id.into(),
            segment_index,
            file_path: file_path.into(),
            started_at: started_at.into(),
            ended_at: ended_at.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AudioSegment {
    pub id: i64,
    pub source_kind: AudioSegmentSourceKind,
    pub source_session_id: String,
    pub segment_index: i64,
    pub file_path: String,
    pub started_at: String,
    pub ended_at: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone)]
pub struct AudioSegmentStore {
    pool: SqlitePool,
}

impl AudioSegmentStore {
    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn upsert(&self, segment: &NewAudioSegment) -> Result<AudioSegment> {
        sqlx::query(
            "INSERT INTO audio_segments \
                (source_kind, source_session_id, segment_index, file_path, started_at, ended_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
             ON CONFLICT(source_kind, source_session_id, file_path) DO UPDATE SET \
                segment_index = excluded.segment_index, \
                started_at = excluded.started_at, \
                ended_at = excluded.ended_at, \
                updated_at = CURRENT_TIMESTAMP",
        )
        .bind(segment.source_kind.as_str())
        .bind(&segment.source_session_id)
        .bind(segment.segment_index)
        .bind(&segment.file_path)
        .bind(&segment.started_at)
        .bind(&segment.ended_at)
        .execute(&self.pool)
        .await?;

        let row = sqlx::query(
            "SELECT id, source_kind, source_session_id, segment_index, file_path, started_at, ended_at, created_at, updated_at \
             FROM audio_segments \
             WHERE source_kind = ?1 AND source_session_id = ?2 AND file_path = ?3",
        )
        .bind(segment.source_kind.as_str())
        .bind(&segment.source_session_id)
        .bind(&segment.file_path)
        .fetch_one(&self.pool)
        .await?;

        map_audio_segment(row)
    }

    pub async fn get(&self, audio_segment_id: i64) -> Result<Option<AudioSegment>> {
        let row = sqlx::query(
            "SELECT id, source_kind, source_session_id, segment_index, file_path, started_at, ended_at, created_at, updated_at \
             FROM audio_segments \
             WHERE id = ?1",
        )
        .bind(audio_segment_id)
        .fetch_optional(&self.pool)
        .await?;

        row.map(map_audio_segment).transpose()
    }

    pub async fn list_overlapping_range(
        &self,
        range_start: &str,
        range_end: &str,
        source_kind: Option<AudioSegmentSourceKind>,
        source_session_id: Option<&str>,
    ) -> Result<Vec<AudioSegment>> {
        let mut query = QueryBuilder::<Sqlite>::new(
            "SELECT id, source_kind, source_session_id, segment_index, file_path, started_at, ended_at, created_at, updated_at FROM audio_segments WHERE started_at <= ",
        );
        query.push_bind(range_end);
        query.push(" AND ended_at >= ");
        query.push_bind(range_start);

        if let Some(source_kind) = source_kind.as_ref() {
            query.push(" AND source_kind = ");
            query.push_bind(source_kind.as_str());
        }

        if let Some(source_session_id) = source_session_id {
            query.push(" AND source_session_id = ");
            query.push_bind(source_session_id);
        }

        query.push(" ORDER BY started_at ASC, ended_at ASC, id ASC");

        let rows = query.build().fetch_all(&self.pool).await?;
        rows.into_iter().map(map_audio_segment).collect()
    }
}

fn map_audio_segment(row: SqliteRow) -> Result<AudioSegment> {
    Ok(AudioSegment {
        id: row.get("id"),
        source_kind: AudioSegmentSourceKind::from_str(row.get("source_kind")),
        source_session_id: row.get("source_session_id"),
        segment_index: row.get("segment_index"),
        file_path: row.get("file_path"),
        started_at: row.get("started_at"),
        ended_at: row.get("ended_at"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}
