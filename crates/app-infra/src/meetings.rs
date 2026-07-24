//! The detected-meetings ledger (Warm Paper redesign, Slice 1; migration
//! `0052`).
//!
//! One row per detected meeting (ADR 0057 mic-hold), written by the desktop
//! meeting detector worker **whether or not a recap trigger fires** — the
//! Meetings surface reads this store, so a meeting with no trigger configured
//! still appears (transcript-only). Recap state is a two-phase story:
//!
//! - Decision time (the worker): `none` (nobody fired), `skipped` (dropped for
//!   cooldown / missing provider), or `pending` (a firing was spawned).
//! - Read time (here): `pending` resolves against the `trigger_firings` ledger
//!   — completed → recap ready with its conversation link; skipped/failed →
//!   skipped with the honest reason; no row yet → processing (Readiness Wait
//!   or the AI run is still in flight).
//!
//! The two-key ledger match exists because the run path stamps its own
//! `fired_at_ms` post-readiness-wait: completed/failed rows match on the
//! deterministic `conversation_id`, readiness-skip rows (conversation-less)
//! match on `trigger_id` + the claim-time `fired_at_ms`.

use sqlx::Row;

use crate::db::CaptureDb;
use crate::Result;

/// One stored meeting row, exactly as persisted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeetingRecord {
    pub id: String,
    pub bundle_id: String,
    pub app_display_name: String,
    pub meeting_url: Option<String>,
    pub start_ms: i64,
    pub end_ms: i64,
    pub trigger_id: Option<String>,
    pub fired_at_ms: Option<i64>,
    pub conversation_id: Option<String>,
    pub notes: Option<String>,
    /// JSON array of the recap checklist items the user has ticked (`None` when
    /// nothing is ticked). The recap markdown owns the item set.
    pub checklist_json: Option<String>,
}

/// The read-time recap state of a meeting (states per PLAN.md Slice 1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MeetingRecapState {
    /// A completed trigger run exists; `conversation_id` links it.
    RecapReady { conversation_id: String },
    /// A firing was spawned and its ledger row has not landed yet.
    Processing,
    /// The recap never ran (decision-time drop or a ledger skip/failure).
    Skipped { reason: Option<String> },
    /// No recap trigger fired for this meeting at all.
    TranscriptOnly,
}

/// A meeting row with its recap state resolved against the firing ledger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedMeeting {
    pub record: MeetingRecord,
    pub state: MeetingRecapState,
}

/// A new detection to persist ([`MeetingsStore::record_meeting`]).
#[derive(Debug, Clone)]
pub struct NewMeeting {
    /// Deterministic: `meeting-<start_ms>-<bundle_id>` (idempotent insert).
    pub id: String,
    pub bundle_id: String,
    pub app_display_name: String,
    pub meeting_url: Option<String>,
    pub start_ms: i64,
    pub end_ms: i64,
}

/// The stored recap_state × the newest matching ledger row → read-time state.
fn resolve(
    recap_state: &str,
    recap_reason: Option<String>,
    ledger: Option<(String, Option<String>, Option<String>)>,
) -> MeetingRecapState {
    match recap_state {
        "pending" => match ledger {
            None => MeetingRecapState::Processing,
            Some((outcome, reason, conversation_id)) => match (outcome.as_str(), conversation_id) {
                ("completed", Some(conversation_id)) => {
                    MeetingRecapState::RecapReady { conversation_id }
                }
                // skipped, failed, or a completed row missing its link:
                // the recap is not readable — honest degradation.
                (_, _) => MeetingRecapState::Skipped { reason },
            },
        },
        "skipped" => MeetingRecapState::Skipped {
            reason: recap_reason,
        },
        _ => MeetingRecapState::TranscriptOnly,
    }
}

/// Group resolved meetings (newest-first, as [`MeetingsStore::list_meetings`]
/// returns them) into local calendar days, keyed `YYYY-MM-DD` in the user's
/// local time. Non-adjacent same-day rows can't occur on sorted input, so this
/// is a single pass.
pub fn group_meetings_by_local_day(
    meetings: Vec<ResolvedMeeting>,
    offset_minutes: i32,
) -> Vec<(String, Vec<ResolvedMeeting>)> {
    let mut days: Vec<(String, Vec<ResolvedMeeting>)> = Vec::new();
    for meeting in meetings {
        let key = local_day_key(meeting.record.start_ms, offset_minutes);
        match days.last_mut() {
            Some((day, group)) if *day == key => group.push(meeting),
            _ => days.push((key, vec![meeting])),
        }
    }
    days
}

/// `YYYY-MM-DD` of the instant in the user's local time.
pub fn local_day_key(unix_ms: i64, offset_minutes: i32) -> String {
    let local_ms = unix_ms + i64::from(offset_minutes) * 60_000;
    let local = time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(local_ms) * 1_000_000)
        .unwrap_or(time::OffsetDateTime::UNIX_EPOCH);
    format!(
        "{:04}-{:02}-{:02}",
        local.year(),
        u8::from(local.month()),
        local.day()
    )
}

/// The list/get read: each meeting joined with its newest matching
/// `trigger_firings` row (see the module docs for the two-key match).
const RESOLVED_SELECT: &str = "SELECT m.id, m.bundle_id, m.app_display_name, m.meeting_url, \
        m.start_ms, m.end_ms, m.trigger_id, m.fired_at_ms, m.conversation_id, \
        m.recap_state, m.recap_reason, m.notes, m.checklist_json, \
        f.outcome AS f_outcome, f.reason AS f_reason, f.conversation_id AS f_conversation_id \
     FROM meetings m \
     LEFT JOIN trigger_firings f ON f.rowid = (\
        SELECT f2.rowid FROM trigger_firings f2 \
        WHERE (m.conversation_id IS NOT NULL AND f2.conversation_id = m.conversation_id) \
           OR (m.trigger_id IS NOT NULL AND f2.trigger_id = m.trigger_id \
               AND f2.fired_at_ms = m.fired_at_ms) \
        ORDER BY f2.fired_at_ms DESC, f2.rowid DESC LIMIT 1)";

fn map_resolved(row: &sqlx::sqlite::SqliteRow) -> ResolvedMeeting {
    let recap_state: String = row.get("recap_state");
    let recap_reason: Option<String> = row.get("recap_reason");
    let ledger = row
        .get::<Option<String>, _>("f_outcome")
        .map(|outcome| (outcome, row.get("f_reason"), row.get("f_conversation_id")));
    ResolvedMeeting {
        record: MeetingRecord {
            id: row.get("id"),
            bundle_id: row.get("bundle_id"),
            app_display_name: row.get("app_display_name"),
            meeting_url: row.get("meeting_url"),
            start_ms: row.get("start_ms"),
            end_ms: row.get("end_ms"),
            trigger_id: row.get("trigger_id"),
            fired_at_ms: row.get("fired_at_ms"),
            conversation_id: row.get("conversation_id"),
            notes: row.get("notes"),
            checklist_json: row.get("checklist_json"),
        },
        state: resolve(&recap_state, recap_reason, ledger),
    }
}

#[derive(Clone)]
pub struct MeetingsStore {
    db: CaptureDb,
}

impl MeetingsStore {
    pub fn new(db: CaptureDb) -> Self {
        Self { db }
    }

    /// Persist one detected meeting. Idempotent: the deterministic id makes a
    /// re-observation a no-op.
    pub async fn record_meeting(&self, meeting: &NewMeeting) -> Result<()> {
        sqlx::query(
            "INSERT OR IGNORE INTO meetings \
                (id, bundle_id, app_display_name, meeting_url, start_ms, end_ms) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .bind(&meeting.id)
        .bind(&meeting.bundle_id)
        .bind(&meeting.app_display_name)
        .bind(&meeting.meeting_url)
        .bind(meeting.start_ms)
        .bind(meeting.end_ms)
        .execute(self.db.write())
        .await?;
        Ok(())
    }

    /// A firing was spawned for this meeting: link the claim so the read side
    /// can resolve the eventual ledger row. Wins over a decision-time skip
    /// from another trigger (pending is the stronger claim).
    pub async fn link_recap_pending(
        &self,
        meeting_id: &str,
        trigger_id: &str,
        fired_at_ms: i64,
        conversation_id: &str,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE meetings SET recap_state = 'pending', trigger_id = ?2, \
                fired_at_ms = ?3, conversation_id = ?4, recap_reason = NULL \
             WHERE id = ?1",
        )
        .bind(meeting_id)
        .bind(trigger_id)
        .bind(fired_at_ms)
        .bind(conversation_id)
        .execute(self.db.write())
        .await?;
        Ok(())
    }

    /// The recap was dropped at decision time (cooldown / no provider). Never
    /// downgrades a pending link from another trigger.
    pub async fn mark_recap_skipped(
        &self,
        meeting_id: &str,
        trigger_id: &str,
        reason: &str,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE meetings SET recap_state = 'skipped', trigger_id = ?2, recap_reason = ?3 \
             WHERE id = ?1 AND recap_state = 'none'",
        )
        .bind(meeting_id)
        .bind(trigger_id)
        .bind(reason)
        .execute(self.db.write())
        .await?;
        Ok(())
    }

    /// The user's own notes text on the meeting row (`None` clears).
    pub async fn set_notes(&self, meeting_id: &str, notes: Option<&str>) -> Result<()> {
        sqlx::query("UPDATE meetings SET notes = ?2 WHERE id = ?1")
            .bind(meeting_id)
            .bind(notes)
            .execute(self.db.write())
            .await?;
        Ok(())
    }

    /// The ticked recap-checklist items, as the raw JSON array string the
    /// command serialized (`None` clears). Item identity lives in the recap
    /// markdown; this is only the done-set.
    pub async fn set_checklist_json(
        &self,
        meeting_id: &str,
        checklist_json: Option<&str>,
    ) -> Result<()> {
        sqlx::query("UPDATE meetings SET checklist_json = ?2 WHERE id = ?1")
            .bind(meeting_id)
            .bind(checklist_json)
            .execute(self.db.write())
            .await?;
        Ok(())
    }

    /// Newest-first meetings with resolved recap state, capped at `limit`.
    pub async fn list_meetings(&self, limit: u32) -> Result<Vec<ResolvedMeeting>> {
        let rows = sqlx::query(&format!(
            "{RESOLVED_SELECT} ORDER BY m.start_ms DESC, m.id DESC LIMIT ?1"
        ))
        .bind(limit)
        .fetch_all(self.db.read())
        .await?;
        Ok(rows.iter().map(map_resolved).collect())
    }

    /// One meeting with resolved recap state.
    pub async fn get_meeting(&self, meeting_id: &str) -> Result<Option<ResolvedMeeting>> {
        let row = sqlx::query(&format!("{RESOLVED_SELECT} WHERE m.id = ?1"))
            .bind(meeting_id)
            .fetch_optional(self.db.read())
            .await?;
        Ok(row.as_ref().map(map_resolved))
    }

    /// Distinct speaker display names heard in the window: a recognized
    /// person's saved name, else the diarization label ("Speaker 1").
    /// ponytail: window match is at audio-segment granularity (segments cap at
    /// 5 min, so bleed is bounded); filter individual turns if it ever matters.
    pub async fn list_speaker_names_for_range(
        &self,
        start_rfc3339: &str,
        end_rfc3339: &str,
    ) -> Result<Vec<String>> {
        let rows = sqlx::query(
            "SELECT DISTINCT COALESCE(person_profiles.display_name, \
                    COALESCE(rsc.transcript_local_label, rsc.stable_label)) AS name \
             FROM speaker_turns st \
             JOIN audio_segments a ON a.id = st.audio_segment_id \
             JOIN recording_speaker_clusters rsc ON rsc.id = st.cluster_id \
             LEFT JOIN person_profiles ON person_profiles.id = rsc.person_id \
             WHERE a.started_at <= ?2 AND a.ended_at >= ?1 \
             ORDER BY name ASC",
        )
        .bind(start_rfc3339)
        .bind(end_rfc3339)
        .fetch_all(self.db.read())
        .await?;
        Ok(rows
            .iter()
            .map(|row| row.get::<String, _>("name"))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    /// The crate's `tokio` dep has no `macros` feature (mirrors
    /// `trigger_firings`'s test pattern).
    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build")
            .block_on(future)
    }

    /// In-memory pool with the `meetings` (migration 0052) and
    /// `trigger_firings` (0051) tables.
    async fn test_pool() -> sqlx::SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite");
        sqlx::query(
            "CREATE TABLE meetings (
                id TEXT PRIMARY KEY,
                bundle_id TEXT NOT NULL,
                app_display_name TEXT NOT NULL,
                meeting_url TEXT,
                start_ms INTEGER NOT NULL,
                end_ms INTEGER NOT NULL,
                trigger_id TEXT,
                fired_at_ms INTEGER,
                conversation_id TEXT,
                recap_state TEXT NOT NULL DEFAULT 'none'
                    CHECK (recap_state IN ('none', 'pending', 'skipped')),
                recap_reason TEXT,
                notes TEXT,
                checklist_json TEXT
            )",
        )
        .execute(&pool)
        .await
        .expect("meetings table");
        sqlx::query(
            "CREATE TABLE trigger_firings (
                trigger_id TEXT NOT NULL,
                fired_at_ms INTEGER NOT NULL,
                outcome TEXT NOT NULL,
                reason TEXT,
                conversation_id TEXT
            )",
        )
        .execute(&pool)
        .await
        .expect("trigger_firings table");
        pool
    }

    fn store(pool: sqlx::SqlitePool) -> MeetingsStore {
        MeetingsStore::new(CaptureDb::single(pool))
    }

    fn zoom_meeting(id: &str, start_ms: i64, end_ms: i64) -> NewMeeting {
        NewMeeting {
            id: id.to_string(),
            bundle_id: "us.zoom.xos".to_string(),
            app_display_name: "Zoom".to_string(),
            meeting_url: None,
            start_ms,
            end_ms,
        }
    }

    const HOUR: i64 = 3_600_000;

    #[test]
    fn transcript_only_processing_and_decision_skipped_states_resolve() {
        block_on(async {
            let store = store(test_pool().await);
            // No trigger fired → transcript-only.
            store
                .record_meeting(&zoom_meeting("m-plain", 0, HOUR))
                .await
                .expect("record");
            // A firing spawned, ledger row not landed yet → processing.
            store
                .record_meeting(&zoom_meeting("m-pending", 2 * HOUR, 3 * HOUR))
                .await
                .expect("record");
            store
                .link_recap_pending("m-pending", "recap", 3 * HOUR, "trigger-recap-1")
                .await
                .expect("link");
            // Dropped at decision time (cooldown) → skipped with the reason.
            store
                .record_meeting(&zoom_meeting("m-cooled", 4 * HOUR, 5 * HOUR))
                .await
                .expect("record");
            store
                .mark_recap_skipped("m-cooled", "recap", "recap trigger was cooling down")
                .await
                .expect("skip");

            let meetings = store.list_meetings(50).await.expect("list");
            let state_of = |id: &str| {
                meetings
                    .iter()
                    .find(|meeting| meeting.record.id == id)
                    .expect("meeting listed")
                    .state
                    .clone()
            };
            assert_eq!(state_of("m-plain"), MeetingRecapState::TranscriptOnly);
            assert_eq!(state_of("m-pending"), MeetingRecapState::Processing);
            assert_eq!(
                state_of("m-cooled"),
                MeetingRecapState::Skipped {
                    reason: Some("recap trigger was cooling down".to_string())
                }
            );
        });
    }

    #[test]
    fn pending_resolves_against_the_firing_ledger() {
        block_on(async {
            let pool = test_pool().await;
            let store = store(pool.clone());
            let firings = crate::trigger_firings::TriggerFiringsStore::new(CaptureDb::single(pool));

            // Completed: the run path stamps its OWN fired_at (post readiness
            // wait), so the match is via the deterministic conversation id.
            store
                .record_meeting(&zoom_meeting("m-done", 0, HOUR))
                .await
                .expect("record");
            store
                .link_recap_pending("m-done", "recap", HOUR, "trigger-recap-3600000")
                .await
                .expect("link");
            firings
                .record_firing(
                    "recap",
                    HOUR + 600_000, // 10 min later than the claim
                    crate::trigger_firings::TriggerFiringOutcome::Completed,
                    None,
                    Some("trigger-recap-3600000"),
                )
                .await
                .expect("ledger");

            // Readiness skip: conversation-less ledger row, matched on
            // trigger_id + the claim-time fired_at.
            store
                .record_meeting(&zoom_meeting("m-skipped", 2 * HOUR, 3 * HOUR))
                .await
                .expect("record");
            store
                .link_recap_pending("m-skipped", "recap", 3 * HOUR, "trigger-recap-10800000")
                .await
                .expect("link");
            firings
                .record_firing(
                    "recap",
                    3 * HOUR,
                    crate::trigger_firings::TriggerFiringOutcome::Skipped,
                    Some("not recording during the meeting"),
                    None,
                )
                .await
                .expect("ledger");

            // Failed run: skipped with the honest reason.
            store
                .record_meeting(&zoom_meeting("m-failed", 4 * HOUR, 5 * HOUR))
                .await
                .expect("record");
            store
                .link_recap_pending("m-failed", "recap", 5 * HOUR, "trigger-recap-18000000")
                .await
                .expect("link");
            firings
                .record_firing(
                    "recap",
                    5 * HOUR + 600_000,
                    crate::trigger_firings::TriggerFiringOutcome::Failed,
                    Some("AI run did not complete after 3 attempts"),
                    Some("trigger-recap-18000000"),
                )
                .await
                .expect("ledger");

            let state_of = |id: &str| {
                let store = store.clone();
                let id = id.to_string();
                async move {
                    store
                        .get_meeting(&id)
                        .await
                        .expect("get")
                        .expect("exists")
                        .state
                }
            };
            assert_eq!(
                state_of("m-done").await,
                MeetingRecapState::RecapReady {
                    conversation_id: "trigger-recap-3600000".to_string()
                }
            );
            assert_eq!(
                state_of("m-skipped").await,
                MeetingRecapState::Skipped {
                    reason: Some("not recording during the meeting".to_string())
                }
            );
            assert_eq!(
                state_of("m-failed").await,
                MeetingRecapState::Skipped {
                    reason: Some("AI run did not complete after 3 attempts".to_string())
                }
            );
        });
    }

    #[test]
    fn record_is_idempotent_and_skip_never_downgrades_pending() {
        block_on(async {
            let store = store(test_pool().await);
            store
                .record_meeting(&zoom_meeting("m-1", 0, HOUR))
                .await
                .expect("record");
            store
                .link_recap_pending("m-1", "recap", HOUR, "conv-1")
                .await
                .expect("link");
            // A re-observation re-inserting the same id must not reset state.
            store
                .record_meeting(&zoom_meeting("m-1", 0, HOUR))
                .await
                .expect("re-record");
            // A second trigger's decision-time skip must not clobber pending.
            store
                .mark_recap_skipped("m-1", "other-recap", "cooling down")
                .await
                .expect("skip");
            let meeting = store.get_meeting("m-1").await.expect("get").expect("row");
            assert_eq!(meeting.state, MeetingRecapState::Processing);
            assert_eq!(meeting.record.trigger_id.as_deref(), Some("recap"));
        });
    }

    #[test]
    fn notes_set_read_and_clear() {
        block_on(async {
            let store = store(test_pool().await);
            store
                .record_meeting(&zoom_meeting("m-1", 0, HOUR))
                .await
                .expect("record");
            store
                .set_notes("m-1", Some("ask Dev about the queue lane"))
                .await
                .expect("set");
            let meeting = store.get_meeting("m-1").await.expect("get").expect("row");
            assert_eq!(
                meeting.record.notes.as_deref(),
                Some("ask Dev about the queue lane")
            );
            store.set_notes("m-1", None).await.expect("clear");
            let meeting = store.get_meeting("m-1").await.expect("get").expect("row");
            assert_eq!(meeting.record.notes, None);
        });
    }

    #[test]
    fn checklist_json_set_read_and_clear() {
        block_on(async {
            let store = store(test_pool().await);
            store
                .record_meeting(&zoom_meeting("m-1", 0, HOUR))
                .await
                .expect("record");
            let meeting = store.get_meeting("m-1").await.expect("get").expect("row");
            assert_eq!(meeting.record.checklist_json, None, "unset by default");
            store
                .set_checklist_json("m-1", Some(r#"["Send the report"]"#))
                .await
                .expect("set");
            let meeting = store.get_meeting("m-1").await.expect("get").expect("row");
            assert_eq!(
                meeting.record.checklist_json.as_deref(),
                Some(r#"["Send the report"]"#)
            );
            store.set_checklist_json("m-1", None).await.expect("clear");
            let meeting = store.get_meeting("m-1").await.expect("get").expect("row");
            assert_eq!(meeting.record.checklist_json, None);
        });
    }

    #[test]
    fn day_grouping_splits_on_local_midnight_and_respects_the_offset() {
        // 2026-07-22 23:30 and 2026-07-23 00:30 in UTC+5:30 (IST): straddle a
        // local midnight that is NOT a UTC midnight.
        const IST: i32 = 330;
        // 2026-07-22T18:00:00Z == 23:30 IST Jul 22.
        let late_jul22 = 1_784_743_200_000_i64;
        // 2026-07-22T19:00:00Z == 00:30 IST Jul 23.
        let early_jul23 = late_jul22 + HOUR;
        assert_eq!(local_day_key(late_jul22, IST), "2026-07-22");
        assert_eq!(local_day_key(early_jul23, IST), "2026-07-23");
        // In UTC both land on Jul 22.
        assert_eq!(local_day_key(early_jul23, 0), "2026-07-22");

        let resolved = |id: &str, start_ms: i64| ResolvedMeeting {
            record: MeetingRecord {
                id: id.to_string(),
                bundle_id: "us.zoom.xos".to_string(),
                app_display_name: "Zoom".to_string(),
                meeting_url: None,
                start_ms,
                end_ms: start_ms + HOUR,
                trigger_id: None,
                fired_at_ms: None,
                conversation_id: None,
                notes: None,
                checklist_json: None,
            },
            state: MeetingRecapState::TranscriptOnly,
        };
        // Newest-first, as list_meetings returns.
        let grouped = group_meetings_by_local_day(
            vec![
                resolved("m-new", early_jul23),
                resolved("m-old-2", late_jul22),
                resolved("m-old-1", late_jul22 - HOUR),
            ],
            IST,
        );
        assert_eq!(
            grouped
                .iter()
                .map(|(day, group)| (day.as_str(), group.len()))
                .collect::<Vec<_>>(),
            vec![("2026-07-23", 1), ("2026-07-22", 2)]
        );
        assert_eq!(grouped[1].1[0].record.id, "m-old-2");
    }

    #[test]
    fn speaker_names_prefer_the_saved_person_over_the_diarization_label() {
        block_on(async {
            let pool = test_pool().await;
            for ddl in [
                "CREATE TABLE audio_segments (id INTEGER PRIMARY KEY, started_at TEXT NOT NULL, ended_at TEXT NOT NULL)",
                "CREATE TABLE recording_speaker_clusters (id INTEGER PRIMARY KEY, stable_label TEXT NOT NULL, transcript_local_label TEXT, person_id INTEGER)",
                "CREATE TABLE person_profiles (id INTEGER PRIMARY KEY, display_name TEXT NOT NULL)",
                "CREATE TABLE speaker_turns (id INTEGER PRIMARY KEY, audio_segment_id INTEGER NOT NULL, cluster_id INTEGER NOT NULL, start_ms INTEGER NOT NULL, end_ms INTEGER NOT NULL)",
            ] {
                sqlx::query(ddl).execute(&pool).await.expect("ddl");
            }
            // A segment inside the meeting window, one outside it.
            sqlx::query(
                "INSERT INTO audio_segments (id, started_at, ended_at) VALUES \
                    (1, '2026-07-23T10:00:00Z', '2026-07-23T10:05:00Z'), \
                    (2, '2026-07-23T15:00:00Z', '2026-07-23T15:05:00Z')",
            )
            .execute(&pool)
            .await
            .expect("segments");
            sqlx::query("INSERT INTO person_profiles (id, display_name) VALUES (7, 'Sarah')")
                .execute(&pool)
                .await
                .expect("person");
            sqlx::query(
                "INSERT INTO recording_speaker_clusters (id, stable_label, transcript_local_label, person_id) VALUES \
                    (1, 'Speaker 1', NULL, 7), \
                    (2, 'Speaker 2', 'Speaker B', NULL), \
                    (3, 'Speaker 9', NULL, NULL)",
            )
            .execute(&pool)
            .await
            .expect("clusters");
            sqlx::query(
                "INSERT INTO speaker_turns (audio_segment_id, cluster_id, start_ms, end_ms) VALUES \
                    (1, 1, 0, 5000), (1, 2, 5000, 9000), (1, 1, 9000, 12000), \
                    (2, 3, 0, 5000)",
            )
            .execute(&pool)
            .await
            .expect("turns");

            let store = store(pool);
            let names = store
                .list_speaker_names_for_range("2026-07-23T09:58:00Z", "2026-07-23T10:20:00Z")
                .await
                .expect("names");
            // Person name wins, local label beats stable, out-of-window
            // cluster is absent, duplicates collapse.
            assert_eq!(names, vec!["Sarah".to_string(), "Speaker B".to_string()]);
        });
    }
}
