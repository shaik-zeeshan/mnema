//! Lock-screen frames are time **away**, not activity: the foreground app while
//! the Mac is locked is `loginwindow`
//! ([`capture_metadata::FrameMetadataSnapshot::is_lock_screen`] owns the list).
//! The capture-window reader and the usage charts skip them per frame; this
//! module finds them by metadata snapshot for the SQL paths (Journal frame
//! summaries, the heal below).

use sqlx::{Row, Sqlite};

use super::store::{cascade_delete_activities_in, UserContextCascadeSummary, UserContextStore};
use crate::Result;

/// Ids of the metadata snapshots whose foreground app is a lock-screen process,
/// as a JSON array (`"[3,17]"`) to bind into `json_each(?)`. A cheap `LIKE`
/// prefilter on the app names (each bundle id contains its app name) narrows
/// the scan; the parsed snapshot is the authority, so a window title that
/// merely mentions "loginwindow" does not match.
pub(crate) async fn lock_screen_snapshot_ids_json<'c, E>(executor: E) -> Result<String>
where
    E: sqlx::Executor<'c, Database = Sqlite>,
{
    let names = capture_metadata::LOCK_SCREEN_APP_NAMES;
    let filter = vec!["snapshot_json LIKE ?"; names.len()].join(" OR ");
    let sql = format!("SELECT id, snapshot_json FROM frame_metadata_snapshots WHERE {filter}");
    let mut query = sqlx::query(&sql);
    for name in names {
        query = query.bind(format!("%{name}%"));
    }
    let ids: Vec<i64> = query
        .fetch_all(executor)
        .await?
        .into_iter()
        .filter(|row| {
            serde_json::from_str::<capture_metadata::FrameMetadataSnapshot>(
                row.get("snapshot_json"),
            )
            .is_ok_and(|snapshot| snapshot.is_lock_screen())
        })
        .map(|row| row.get("id"))
        .collect();
    Ok(serde_json::to_string(&ids).expect("ids serialize"))
}

impl UserContextStore {
    /// Heal for Activities written before derivation skipped the lock screen:
    /// delete every Activity whose evidence is ENTIRELY lock-screen frames,
    /// through the same cascade Delete Recent Capture uses (overlapping Digests,
    /// evidence links, Conclusions that fall below the formation bar, orphaned
    /// Subject Vectors). Mixed Activities stay, and so does any Activity with
    /// audio evidence or evidence frames retention already removed (they can't
    /// be shown to be lock-screen only). Idempotent: a clean store is a no-op.
    pub async fn delete_lock_screen_only_activities(&self) -> Result<UserContextCascadeSummary> {
        let mut tx = self.db.begin_write().await?;
        let lock_ids = lock_screen_snapshot_ids_json(&mut *tx).await?;
        let activity_ids: Vec<i64> = sqlx::query_scalar(
            "SELECT a.id FROM user_context_activities a \
             WHERE EXISTS (SELECT 1 FROM user_context_activity_evidence e WHERE e.activity_id = a.id) \
               AND NOT EXISTS (\
                 SELECT 1 FROM user_context_activity_evidence e \
                 LEFT JOIN frames f ON e.subject_type = 'frame' AND f.id = e.subject_id \
                 WHERE e.activity_id = a.id \
                   AND (f.metadata_snapshot_id IS NULL \
                        OR f.metadata_snapshot_id NOT IN (SELECT value FROM json_each(?1))))",
        )
        .bind(&lock_ids)
        .fetch_all(&mut *tx)
        .await?;
        if activity_ids.is_empty() {
            return Ok(UserContextCascadeSummary::default());
        }
        let summary = cascade_delete_activities_in(&mut tx, &activity_ids).await?;
        tx.commit().await?;
        Ok(summary)
    }
}

#[cfg(test)]
mod tests {
    use capture_types::EvidenceStance;

    use crate::{
        AppInfra, NewActivity, NewActivityEvidence, NewConclusion, NewConclusionEvidence, NewFrame,
    };

    async fn insert_frames(infra: &AppInfra) -> Vec<i64> {
        let mut ids = Vec::new();
        for (name, bundle, at) in [
            (
                "loginwindow",
                "com.apple.loginwindow",
                "2026-01-01T00:00:00Z",
            ),
            // Bundle id alone still identifies the lock screen.
            ("", "com.apple.loginwindow", "2026-01-01T00:00:01Z"),
            ("Safari", "com.apple.Safari", "2026-01-01T00:00:02Z"),
            // Also prompts on an unlocked Mac, so it is NOT the lock screen.
            (
                "SecurityAgent",
                "com.apple.SecurityAgent",
                "2026-01-01T00:00:03Z",
            ),
        ] {
            let frame = NewFrame {
                metadata_snapshot: Some(capture_metadata::FrameMetadataSnapshot {
                    app_name: Some(name.to_string()).filter(|n| !n.is_empty()),
                    app_bundle_id: Some(bundle.to_string()),
                    ..Default::default()
                }),
                ..NewFrame::new("s", format!("{at}.png"), at)
            };
            ids.push(infra.insert_frame(&frame).await.expect("frame").id);
        }
        ids
    }

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build")
            .block_on(future)
    }

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "{tag}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    /// The Journal's frame read drops lock-screen frames (so their span reads
    /// as Away); every other caller still sees them.
    #[test]
    fn frame_summaries_exclude_lock_screen_only_when_asked() {
        block_on(async {
            let dir = temp_dir("lock-screen-summaries");
            let infra = AppInfra::initialize(&dir).await.expect("infra");
            let ids = insert_frames(&infra).await;
            let (start, end) = ("2026-01-01T00:00:00Z", "2026-01-01T00:01:00Z");

            let journal = infra
                .list_frame_summaries_in_range(start, end, true)
                .await
                .expect("journal");
            assert_eq!(
                journal.iter().map(|f| f.id).collect::<Vec<_>>(),
                vec![ids[3], ids[2]],
                "Safari and SecurityAgent stay; both loginwindow frames go"
            );
            let all = infra
                .list_frame_summaries_in_range(start, end, false)
                .await
                .expect("all");
            assert_eq!(all.len(), 4);
            let _ = std::fs::remove_dir_all(&dir);
        });
    }

    /// Pure-lock Activities go (with the Conclusion they alone supported); a
    /// mixed one, an audio-backed one and a SecurityAgent one stay; a second run is a no-op.
    #[test]
    fn heal_deletes_lock_screen_only_activities_and_is_idempotent() {
        block_on(async {
            let dir = temp_dir("lock-screen-heal");
            let infra = AppInfra::initialize(&dir).await.expect("infra");
            let store = infra.user_context();
            let frame_ids = insert_frames(&infra).await;
            let [lock_a, lock_b, safari, security_agent] = frame_ids[..] else {
                unreachable!()
            };

            let ev = |subject_type: &str, subject_id: i64| NewActivityEvidence {
                subject_type: subject_type.to_string(),
                subject_id,
                captured_at_ms: None,
                is_headline: false,
            };
            let activity = |evidence: Vec<NewActivityEvidence>| NewActivity {
                title: "Unlock Mac".to_string(),
                summary: String::new(),
                category: None,
                focus: None,
                started_at_ms: 0,
                ended_at_ms: 1_000,
                derivation_run_id: None,
                evidence,
            };
            let insert = |evidence| store.insert_activity_with_evidence(activity(evidence));
            let pure_a = insert(vec![ev("frame", lock_a), ev("frame", lock_b)])
                .await
                .expect("pure a");
            let pure_b = insert(vec![ev("frame", lock_a)]).await.expect("pure b");
            let mixed = insert(vec![ev("frame", lock_a), ev("frame", safari)])
                .await
                .expect("mixed");
            let with_audio = insert(vec![ev("frame", lock_a), ev("audio_segment", 9)])
                .await
                .expect("audio");
            let with_agent = insert(vec![ev("frame", lock_a), ev("frame", security_agent)])
                .await
                .expect("security agent");

            // "Mac unlock routine" rests only on the two pure-lock Activities.
            let routine = store
                .upsert_conclusion(NewConclusion {
                    subject: "Mac unlock routine".to_string(),
                    statement: "Unlocks the Mac often".to_string(),
                    confidence: 0.9,
                    formed_at_ms: 1,
                    last_supported_at_ms: 1,
                })
                .await
                .expect("conclusion");
            store
                .replace_conclusion_evidence(
                    routine,
                    [pure_a, pure_b]
                        .map(|activity_id| NewConclusionEvidence {
                            activity_id,
                            stance: EvidenceStance::Support,
                        })
                        .to_vec(),
                )
                .await
                .expect("evidence");

            let summary = store
                .delete_lock_screen_only_activities()
                .await
                .expect("heal");
            assert_eq!(summary.deleted_activities, 2);
            assert_eq!(summary.deleted_conclusions, 1);

            let mut left: Vec<i64> = store
                .list_activities_in_range(-1, i64::MAX)
                .await
                .expect("list")
                .into_iter()
                .map(|a| a.id)
                .collect();
            left.sort_unstable();
            assert_eq!(left, vec![mixed, with_audio, with_agent]);
            assert!(store
                .list_conclusions(true)
                .await
                .expect("conclusions")
                .iter()
                .all(|c| c.id != routine));
            // No evidence link outlives its Activity.
            let dangling: i64 = sqlx::query_scalar(
            "SELECT (SELECT COUNT(*) FROM user_context_activity_evidence WHERE activity_id IN (?1, ?2)) \
                  + (SELECT COUNT(*) FROM user_context_conclusion_evidence WHERE activity_id IN (?1, ?2))",
        )
        .bind(pure_a)
        .bind(pure_b)
        .fetch_one(store.db.read())
        .await
        .expect("count");
            assert_eq!(dangling, 0);

            let again = store
                .delete_lock_screen_only_activities()
                .await
                .expect("heal again");
            assert_eq!(again.deleted_activities, 0);
            let _ = std::fs::remove_dir_all(&dir);
        });
    }
}
