//! Shared test-support helpers for the `search` module's co-located unit tests.
//!
//! Each sibling `#[cfg(test)] mod tests` pulls these in via
//! `use crate::search::test_support::*;`.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use audio_transcription::{TranscriptionMetadata, TranscriptionSegment};

use super::retrieval::FrameHit;
use crate::{
    AppInfra, AudioSegmentSourceKind, Frame, NewAudioSegment, NewFrame, ProcessingJobDraft,
    ProcessingResultDraft,
};

pub(super) static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

pub(super) fn test_dir(name: &str) -> PathBuf {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    std::env::temp_dir().join(format!("mnema-search-{name}-{}-{id}", std::process::id()))
}

pub(super) async fn complete_job(
    infra: &AppInfra,
    job: crate::ProcessingJob,
    result: ProcessingResultDraft,
) {
    let running = infra
        .claim_queued_processing_job(job.id)
        .await
        .expect("job should claim")
        .expect("job should exist");
    infra
        .complete_processing_job(running.id, &result)
        .await
        .expect("job should complete");
}

pub(super) fn run_async_test(test: impl std::future::Future<Output = ()>) {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime should build")
        .block_on(test);
}

pub(super) fn frame_with_app(
    bundle_id: Option<&str>,
    name: Option<&str>,
) -> capture_metadata::FrameMetadataSnapshot {
    capture_metadata::FrameMetadataSnapshot {
        app_bundle_id: bundle_id.map(str::to_string),
        app_name: name.map(str::to_string),
        window_title: None,
        window_id: None,
        browser_url: None,
        display_id: Some(1),
        metadata_redaction_reason: None,
        metadata_redaction_source_id: None,
    }
}

pub(super) async fn seed_frame_with_text(
    infra: &AppInfra,
    path: &str,
    captured_at: &str,
    metadata: Option<capture_metadata::FrameMetadataSnapshot>,
    text: &str,
) -> crate::Frame {
    let mut new_frame = NewFrame::new("screen-session", path, captured_at);
    if let Some(metadata) = metadata {
        new_frame = new_frame.with_metadata_snapshot(metadata);
    }
    let frame = infra
        .insert_frame(&new_frame)
        .await
        .expect("frame should insert");
    let job = infra
        .enqueue_processing_job(&ProcessingJobDraft::for_frame_ocr(frame.id))
        .await
        .expect("ocr job should enqueue");
    complete_job(
        infra,
        job,
        ProcessingResultDraft::new().with_result_text(text),
    )
    .await;
    frame
}

/// The default English tier (`nomic-embed-text-v1.5`) embedding width — the
/// dimension of the slice-1 `search_document_vectors vec0(embedding float[768])`
/// table. Test vectors must match it or `store_vector` errors.
pub(super) const TEST_EMBED_DIM: usize = 768;

/// Build a deterministic unit f32 vector keyed to `seed`, so two distinct
/// seeds are far apart in L2 distance and a query close to one seed's vector
/// is nearest to that anchor's stored vector under the brute-force KNN.
pub(super) fn seeded_vector(seed: usize) -> Vec<f32> {
    let mut v = vec![0.0_f32; TEST_EMBED_DIM];
    // One-hot in a slot chosen by the seed: orthogonal vectors are maximally
    // separated, so KNN nearest-neighbor order is unambiguous.
    v[seed % TEST_EMBED_DIM] = 1.0;
    v
}

/// Seed a `direct` frame anchor with OCR `text` and return its
/// `search_documents.id` (the `vec0` rowid). The completed OCR projects the
/// anchor on write, exactly as production does.
pub(super) async fn seed_frame_anchor(infra: &AppInfra, captured_at: &str, text: &str) -> i64 {
    let frame = infra
        .insert_frame(&NewFrame::new(
            "screen-session",
            &format!("/tmp/hybrid-{captured_at}.jpg"),
            captured_at,
        ))
        .await
        .expect("frame should insert");
    let job = infra
        .enqueue_processing_job(&ProcessingJobDraft::for_frame_ocr(frame.id))
        .await
        .expect("ocr job should enqueue");
    complete_job(
        infra,
        job,
        ProcessingResultDraft::new().with_result_text(text),
    )
    .await;
    sqlx::query_scalar::<_, i64>(
        "SELECT id FROM search_documents WHERE frame_id = ?1 AND text_source_kind = 'direct' LIMIT 1",
    )
    .bind(frame.id)
    .fetch_one(infra.pool())
    .await
    .expect("direct anchor id should load")
}

/// Seed a `direct` audio (transcription) anchor with transcript `text` and
/// return its `search_documents.id` (the `vec0` rowid). A single-segment
/// transcription projects one `direct` `anchor_type = 'audio'` document on
/// write, exactly as production does — the audio counterpart of
/// [`seed_frame_anchor`].
pub(super) async fn seed_audio_anchor(
    infra: &AppInfra,
    started_at: &str,
    ended_at: &str,
    text: &str,
) -> i64 {
    let segment = infra
        .upsert_audio_segment(&NewAudioSegment::new(
            AudioSegmentSourceKind::Microphone,
            "mic-session",
            1,
            &format!("/tmp/hybrid-audio-{started_at}.m4a"),
            started_at,
            ended_at,
        ))
        .await
        .expect("segment should insert");
    let metadata = TranscriptionMetadata {
        provider: "test".to_string(),
        model_id: None,
        language: "en".to_string(),
        segments: vec![TranscriptionSegment {
            start_ms: 0,
            end_ms: 2_000,
            text: text.to_string(),
            confidence: None,
        }],
        words: Vec::new(),
        provenance: Default::default(),
    };
    let job = infra
        .enqueue_processing_job(&ProcessingJobDraft::for_audio_segment_transcription(
            segment.id,
        ))
        .await
        .expect("transcription job should enqueue");
    complete_job(
        infra,
        job,
        ProcessingResultDraft::new()
            .with_result_text(text)
            .with_structured_payload_json(
                serde_json::to_string(&metadata).expect("metadata should serialize"),
            ),
    )
    .await;
    sqlx::query_scalar::<_, i64>(
        "SELECT id FROM search_documents \
         WHERE audio_segment_id = ?1 AND text_source_kind = 'direct' AND anchor_type = 'audio' LIMIT 1",
    )
    .bind(segment.id)
    .fetch_one(infra.pool())
    .await
    .expect("direct audio anchor id should load")
}

pub(super) fn frame_hit_for_fusion(
    anchor_id: i64,
    snippet: &str,
    found_by_meaning: bool,
) -> FrameHit {
    FrameHit {
        anchor_id,
        group_key: format!("frame:{anchor_id}"),
        frame: Frame {
            id: anchor_id,
            session_id: "screen-session".to_string(),
            file_path: format!("/tmp/fuse-{anchor_id}.jpg"),
            captured_at: "2026-05-17T10:00:00Z".to_string(),
            width: None,
            height: None,
            equivalence: crate::FrameEquivalence {
                hint: None,
                proof: None,
                version: None,
                status: None,
                error: None,
            },
            metadata_snapshot: None,
            created_at: "2026-05-17T10:00:00Z".to_string(),
            updated_at: "2026-05-17T10:00:00Z".to_string(),
        },
        snippet: snippet.to_string(),
        rank: 0.0,
        app_bundle_id: None,
        app_name: None,
        window_title: None,
        text_source_kind: "direct".to_string(),
        secret_redaction_count: 0,
        found_by_meaning,
    }
}

pub(super) async fn seed_frame_anchor_with_app(
    infra: &AppInfra,
    captured_at: &str,
    text: &str,
    bundle_id: &str,
    app_name: &str,
) -> i64 {
    let frame = infra
        .insert_frame(
            &NewFrame::new(
                "screen-session",
                &format!("/tmp/hybrid-app-{captured_at}.jpg"),
                captured_at,
            )
            .with_metadata_snapshot(capture_metadata::FrameMetadataSnapshot {
                app_bundle_id: Some(bundle_id.to_string()),
                app_name: Some(app_name.to_string()),
                window_title: None,
                window_id: None,
                browser_url: None,
                display_id: None,
                metadata_redaction_reason: None,
                metadata_redaction_source_id: None,
            }),
        )
        .await
        .expect("frame should insert");
    let job = infra
        .enqueue_processing_job(&ProcessingJobDraft::for_frame_ocr(frame.id))
        .await
        .expect("ocr job should enqueue");
    complete_job(
        infra,
        job,
        ProcessingResultDraft::new().with_result_text(text),
    )
    .await;
    sqlx::query_scalar::<_, i64>(
        "SELECT id FROM search_documents WHERE frame_id = ?1 AND text_source_kind = 'direct' LIMIT 1",
    )
    .bind(frame.id)
    .fetch_one(infra.pool())
    .await
    .expect("direct anchor id should load")
}
