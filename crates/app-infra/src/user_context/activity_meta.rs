//! Per-Activity apps and speaker counts for the Journal cards, derived on read
//! from the Activity's evidence (nothing persisted, no migration).

use std::collections::BTreeMap;

use capture_types::ActivityMeta;
use sqlx::Row;

use super::store::UserContextStore;
use crate::Result;

/// How many app names one card shows.
const TOP_APPS: usize = 3;

impl UserContextStore {
    /// Apps and speaker counts for every Activity overlapping `[start_ms,
    /// end_ms)` (the same overlap rule as `list_activities_in_range`), in two
    /// batched queries:
    ///
    /// - **apps**: frame evidence → `search_documents.app_name`, distinct apps
    ///   ranked by evidence-frame count (ties by name), top [`TOP_APPS`];
    /// - **speakers**: `audio_segment` evidence → `speaker_turns`, counting a
    ///   named person once across sessions and an unnamed cluster per cluster.
    ///   Merges/moves rewrite `speaker_turns.cluster_id`, so it is effective.
    ///
    /// Evidence keeps no FK to captures (ADR 0029), so an Activity whose frames
    /// or audio aged out simply reports fewer apps / speakers. Activities with
    /// neither are omitted.
    pub async fn list_activity_meta_in_range(
        &self,
        start_ms: i64,
        end_ms: i64,
    ) -> Result<Vec<ActivityMeta>> {
        let mut meta: BTreeMap<i64, ActivityMeta> = BTreeMap::new();

        let app_rows = sqlx::query(
            "SELECT e.activity_id AS activity_id, TRIM(d.app_name) AS app_name, \
                    COUNT(DISTINCT e.subject_id) AS frames \
             FROM user_context_activities a \
             JOIN user_context_activity_evidence e \
               ON e.activity_id = a.id AND e.subject_type = 'frame' \
             JOIN search_documents d ON d.anchor_type = 'frame' AND d.frame_id = e.subject_id \
             WHERE a.started_at_ms < ?2 AND a.ended_at_ms >= ?1 \
               AND TRIM(COALESCE(d.app_name, '')) <> '' \
             GROUP BY e.activity_id, TRIM(d.app_name) \
             ORDER BY e.activity_id, frames DESC, app_name ASC",
        )
        .bind(start_ms)
        .bind(end_ms)
        .fetch_all(self.db.read())
        .await?;
        for row in app_rows {
            let apps = &mut slot(&mut meta, row.get("activity_id")).apps;
            if apps.len() < TOP_APPS {
                apps.push(row.get("app_name"));
            }
        }

        let speaker_rows = sqlx::query(
            "SELECT e.activity_id AS activity_id, \
                    COUNT(DISTINCT COALESCE('p' || c.person_id, 'c' || t.cluster_id)) AS speakers \
             FROM user_context_activities a \
             JOIN user_context_activity_evidence e \
               ON e.activity_id = a.id AND e.subject_type = 'audio_segment' \
             JOIN speaker_turns t ON t.audio_segment_id = e.subject_id \
             LEFT JOIN recording_speaker_clusters c ON c.id = t.cluster_id \
             WHERE a.started_at_ms < ?2 AND a.ended_at_ms >= ?1 \
             GROUP BY e.activity_id",
        )
        .bind(start_ms)
        .bind(end_ms)
        .fetch_all(self.db.read())
        .await?;
        for row in speaker_rows {
            slot(&mut meta, row.get("activity_id")).speakers = row.get("speakers");
        }

        Ok(meta.into_values().collect())
    }
}

fn slot(meta: &mut BTreeMap<i64, ActivityMeta>, activity_id: i64) -> &mut ActivityMeta {
    meta.entry(activity_id).or_insert_with(|| ActivityMeta {
        activity_id,
        apps: Vec::new(),
        speakers: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::CaptureDb;
    use sqlx::sqlite::SqlitePoolOptions;

    /// Only the columns the two queries read.
    const SCHEMA: &str = "
        CREATE TABLE user_context_activities (id INTEGER PRIMARY KEY,
            started_at_ms INTEGER NOT NULL, ended_at_ms INTEGER NOT NULL);
        CREATE TABLE user_context_activity_evidence (activity_id INTEGER NOT NULL,
            subject_type TEXT NOT NULL, subject_id INTEGER NOT NULL);
        CREATE TABLE search_documents (id INTEGER PRIMARY KEY, anchor_type TEXT NOT NULL,
            frame_id INTEGER, app_name TEXT);
        CREATE TABLE recording_speaker_clusters (id INTEGER PRIMARY KEY, person_id INTEGER);
        CREATE TABLE speaker_turns (id INTEGER PRIMARY KEY, audio_segment_id INTEGER NOT NULL,
            cluster_id INTEGER NOT NULL);

        -- Activity 1 (in range): frames 1-3 Zed, 4-5 Safari (frame 5 indexed
        -- twice), 6 Slack, 7 Mail, 8 blank app; audio 10 + 11.
        INSERT INTO user_context_activities VALUES (1, 100, 200), (2, 150, 300), (3, 900, 950);
        INSERT INTO user_context_activity_evidence VALUES
            (1,'frame',1),(1,'frame',2),(1,'frame',3),(1,'frame',4),(1,'frame',5),
            (1,'frame',6),(1,'frame',7),(1,'frame',8),(1,'frame',99),
            (1,'audio_segment',10),(1,'audio_segment',11),
            (2,'audio_segment',12),
            (3,'frame',1);
        INSERT INTO search_documents (anchor_type, frame_id, app_name) VALUES
            ('frame',1,'Zed'),('frame',2,'Zed'),('frame',3,' Zed '),('frame',4,'Safari'),
            ('frame',5,'Safari'),('frame',5,'Safari'),('frame',6,'Slack'),('frame',7,'Mail'),
            ('frame',8,'  ');
        -- Clusters 1 + 2 are the same named person in two sessions; 3 is unnamed.
        INSERT INTO recording_speaker_clusters VALUES (1, 7), (2, 7), (3, NULL);
        INSERT INTO speaker_turns (audio_segment_id, cluster_id) VALUES
            (10,1),(10,3),(10,3),(11,2),(12,3);
    ";

    #[test]
    fn apps_rank_by_frames_and_speakers_count_people_once() {
        let meta = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
            .block_on(async {
                let pool = SqlitePoolOptions::new()
                    .max_connections(1)
                    .connect("sqlite::memory:")
                    .await
                    .expect("db");
                sqlx::raw_sql(SCHEMA).execute(&pool).await.expect("fixture");
                UserContextStore::new(CaptureDb::single(pool))
                    .list_activity_meta_in_range(0, 500)
                    .await
                    .expect("meta")
            });

        let apps = |apps: &[&str]| apps.iter().map(|a| a.to_string()).collect::<Vec<_>>();
        assert_eq!(
            meta,
            vec![
                // Top 3 by frames (Zed 3, Safari 2, then Mail before Slack by name);
                // blank app names and unindexed frames drop out.
                ActivityMeta {
                    activity_id: 1,
                    apps: apps(&["Zed", "Safari", "Mail"]),
                    speakers: 2
                },
                // Audio-only activity; activity 3 is out of range.
                ActivityMeta {
                    activity_id: 2,
                    apps: Vec::new(),
                    speakers: 1
                },
            ]
        );
        assert_eq!(
            serde_json::to_value(&meta[1]).unwrap(),
            serde_json::json!({ "activityId": 2, "apps": [], "speakers": 1 })
        );
    }
}
