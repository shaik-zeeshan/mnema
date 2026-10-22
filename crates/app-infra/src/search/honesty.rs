//! Why a search may be partial: an empty index, audio still waiting to be
//! transcribed, frames still waiting to be read. Cheap counts beside the search,
//! so Quick Recall can say so instead of showing a bare "no results".

use serde::Serialize;
use sqlx::{QueryBuilder, Row, Sqlite};

use super::types::NormalizedSearchRefinements;
use super::SearchStore;
use crate::{
    Result, AUDIO_SEGMENT_SUBJECT_TYPE, AUDIO_TRANSCRIPTION_PROCESSOR, FRAME_SUBJECT_TYPE,
    OCR_PROCESSOR,
};

/// Frames captured but not yet read (OCR queued or running) — not searchable yet.
#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FrameReadingBacklog {
    pub queued_frames: i64,
    /// Capture time of the oldest unread frame, unix ms: anything after it may
    /// be missing from results.
    pub oldest_queued_ms: Option<i64>,
}

impl SearchStore {
    pub(super) async fn index_is_empty(&self) -> Result<bool> {
        let empty: bool = sqlx::query_scalar("SELECT NOT EXISTS (SELECT 1 FROM search_documents)")
            .fetch_one(self.db.read())
            .await?;
        Ok(empty)
    }

    /// Audio segments in the refinement's date range (and sources) whose
    /// transcription is still queued or running.
    pub(super) async fn untranscribed_segments_in_range(
        &self,
        refinements: &NormalizedSearchRefinements,
    ) -> Result<u32> {
        let mut query = QueryBuilder::<Sqlite>::new(
            "SELECT COUNT(DISTINCT audio_segments.id) FROM audio_segments \
             JOIN processing_jobs ON processing_jobs.subject_id = audio_segments.id \
             WHERE processing_jobs.subject_type = ",
        );
        query.push_bind(AUDIO_SEGMENT_SUBJECT_TYPE);
        query.push(" AND processing_jobs.processor = ");
        query.push_bind(AUDIO_TRANSCRIPTION_PROCESSOR);
        query.push(" AND processing_jobs.status IN ('queued', 'running')");
        if let Some(range) = &refinements.date_range {
            query.push(" AND julianday(audio_segments.ended_at) >= julianday(");
            query.push_bind(range.start_at.clone());
            query.push(") AND julianday(audio_segments.started_at) <= julianday(");
            query.push_bind(range.end_at.clone());
            query.push(")");
        }
        if !refinements.audio_sources.is_empty() {
            query.push(" AND audio_segments.source_kind IN (");
            let mut sources = query.separated(", ");
            for source in &refinements.audio_sources {
                sources.push_bind(source.as_str());
            }
            query.push(")");
        }
        let count: i64 = query.build().fetch_one(self.db.read()).await?.get(0);
        Ok(u32::try_from(count).unwrap_or(u32::MAX))
    }

    /// The OCR queue, as frames: how many are waiting and since when.
    /// ponytail: frames still inside an unfinalized batch have no OCR job yet and
    /// are not counted; add the batch backlog if the line reads too optimistic.
    pub async fn frame_reading_backlog(&self) -> Result<FrameReadingBacklog> {
        let row = sqlx::query(
            "SELECT COUNT(*) AS queued, MIN(julianday(frames.captured_at)) AS oldest \
             FROM processing_jobs JOIN frames ON frames.id = processing_jobs.subject_id \
             WHERE processing_jobs.subject_type = ?1 AND processing_jobs.processor = ?2 \
               AND processing_jobs.status IN ('queued', 'running')",
        )
        .bind(FRAME_SUBJECT_TYPE)
        .bind(OCR_PROCESSOR)
        .fetch_one(self.db.read())
        .await?;
        let oldest: Option<f64> = row.get("oldest");
        Ok(FrameReadingBacklog {
            queued_frames: row.get("queued"),
            // Julian day → unix ms (2440587.5 = the unix epoch's Julian day).
            oldest_queued_ms: oldest.map(|day| ((day - 2_440_587.5) * 86_400_000.0).round() as i64),
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::search::test_support::*;
    use crate::search::SearchCaptureRequest;
    use crate::{AppInfra, AudioSegmentSourceKind, NewAudioSegment, NewFrame, ProcessingJobDraft};

    fn request(query: &str) -> SearchCaptureRequest {
        SearchCaptureRequest {
            query: query.to_string(),
            frame_limit: Some(10),
            frame_offset: None,
            audio_limit: Some(10),
            audio_offset: None,
            snapshot_document_id: None,
            refinements: None,
            query_embedding: None,
        }
    }

    #[test]
    fn partial_flags_say_empty_index_waiting_audio_and_unread_frames() {
        run_async_test(async {
            let dir = test_dir("search-honesty");
            let infra = AppInfra::initialize(&dir).await.expect("infra");

            let empty = infra.search_capture(request("budget")).await.unwrap();
            assert!(empty.index_empty);
            assert_eq!(empty.untranscribed_segments_in_range, 0);

            seed_frame_with_text(
                &infra,
                "/tmp/honesty.jpg",
                "2026-05-17T10:00:00Z",
                None,
                "budget",
            )
            .await;
            // One recording waiting for its transcript, one frame waiting for OCR.
            let segment = infra
                .upsert_audio_segment(&NewAudioSegment::new(
                    AudioSegmentSourceKind::Microphone,
                    "mic-session",
                    1,
                    "/tmp/honesty.m4a",
                    "2026-05-17T10:00:00Z",
                    "2026-05-17T10:05:00Z",
                ))
                .await
                .expect("audio segment");
            infra
                .enqueue_processing_job(&ProcessingJobDraft::for_audio_segment_transcription(
                    segment.id,
                ))
                .await
                .expect("transcription job");
            let unread = infra
                .insert_frame(&NewFrame::new(
                    "s",
                    "/tmp/unread.jpg",
                    "2026-05-17T11:00:00Z",
                ))
                .await
                .expect("frame");
            infra
                .enqueue_processing_job(&ProcessingJobDraft::for_frame_ocr(unread.id))
                .await
                .expect("ocr job");

            let response = infra.search_capture(request("budget")).await.unwrap();
            assert!(!response.index_empty);
            assert!(!response.semantic_degraded);
            assert_eq!(response.untranscribed_segments_in_range, 1);
            let json = serde_json::to_value(&response).unwrap();
            assert_eq!(json["untranscribedSegmentsInRange"], 1);
            assert_eq!(json["indexEmpty"], false);
            let screen_only = infra
                .search_capture(request("budget source:screen"))
                .await
                .unwrap();
            assert_eq!(
                screen_only.untranscribed_segments_in_range, 0,
                "audio skipped"
            );
            let other_day = infra
                .search_capture(request("budget date:2026-05-18"))
                .await
                .unwrap();
            assert_eq!(other_day.untranscribed_segments_in_range, 0, "out of range");

            let backlog = infra.search().frame_reading_backlog().await.unwrap();
            assert_eq!(backlog.queued_frames, 1);
            assert_eq!(backlog.oldest_queued_ms, Some(1_779_015_600_000));
        });
    }

    #[test]
    fn a_failing_meaning_tier_is_reported_as_semantic_degraded() {
        run_async_test(async {
            let dir = test_dir("search-semantic-degraded");
            let infra = AppInfra::initialize(&dir).await.expect("infra");
            seed_frame_anchor(&infra, "2026-05-17T10:00:00Z", "quarterly budget").await;
            // A table that declares a width but is not vec0: the dimension gate
            // passes and the KNN `MATCH` then fails — a real DB error.
            sqlx::query("DROP TABLE search_document_vectors")
                .execute(infra.pool())
                .await
                .unwrap();
            sqlx::query(
                "CREATE TABLE search_document_vectors (\"embedding[768]\" BLOB, embedding BLOB)",
            )
            .execute(infra.pool())
            .await
            .unwrap();
            let mut degraded = request("budget");
            degraded.query_embedding = Some(seeded_vector(1));

            let response = infra.search_capture(degraded).await.expect("keyword-only");
            assert!(response.semantic_degraded);
            assert_eq!(response.frames.len(), 1, "keyword hits still return");
        });
    }
}
