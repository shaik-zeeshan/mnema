use sqlx::{sqlite::SqliteRow, Row, SqlitePool};
use time::{format_description::well_known::Rfc3339, Duration, OffsetDateTime};

use super::projection::{normalized_source_session_id, timestamp_plus_ms};
use super::retrieval::{AudioHit, FrameHit};
use super::{AudioSearchResult, FrameSearchResult};
use crate::captured_frame_equivalence::CapturedFrameEquivalenceScope;
use crate::processing::{map_frame_for_search, Frame};
use crate::{AppInfraError, AudioSegment, AudioSegmentSourceKind, Result};

const AUDIO_GROUP_GAP_MS: u64 = 2_000;

pub(super) fn frame_search_group_key(frame: &Frame) -> String {
    frame
        .equivalence
        .ready_parts()
        .map(|(hint, proof, version)| {
            let scope = frame_search_group_scope_identity(frame);
            format!(
                "frame:eq:{}:{version}:{hint}:{}:{scope}",
                frame.session_id,
                proof_identity(proof)
            )
        })
        .unwrap_or_else(|| format!("frame:{}", frame.id))
}

fn frame_search_group_scope_identity(frame: &Frame) -> String {
    match CapturedFrameEquivalenceScope::from_frame(frame) {
        CapturedFrameEquivalenceScope::Session => "scope:session".to_string(),
        CapturedFrameEquivalenceScope::HiddenSegmentWorkspace { frames_dir_prefix } => {
            format!(
                "scope:hidden:{}",
                proof_identity(frames_dir_prefix.as_bytes())
            )
        }
    }
}

fn proof_identity(proof: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in proof {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

pub(super) fn group_frame_hits(hits: &[FrameHit]) -> Vec<FrameSearchResult> {
    let mut groups: Vec<(String, Vec<FrameHit>)> = Vec::new();
    for hit in hits {
        let group_index = groups.iter().position(|(_group_key, group_hits)| {
            group_hits
                .first()
                .is_some_and(|representative| frame_hits_are_equivalent(representative, &hit))
        });
        if let Some(index) = group_index {
            groups[index].1.push(hit.clone());
        } else {
            groups.push((hit.group_key.clone(), vec![hit.clone()]));
        }
    }

    let mut results = groups
        .into_iter()
        .filter_map(|(group_key, mut hits)| {
            hits.sort_by(|a, b| {
                a.rank
                    .total_cmp(&b.rank)
                    .then_with(|| b.frame.captured_at.cmp(&a.frame.captured_at))
            });
            let representative = hits
                .iter()
                .max_by(|a, b| a.frame.captured_at.cmp(&b.frame.captured_at))?;
            let group_start_at = hits
                .iter()
                .map(|hit| hit.frame.captured_at.as_str())
                .min()
                .unwrap_or(representative.frame.captured_at.as_str())
                .to_string();
            let group_end_at = hits
                .iter()
                .map(|hit| hit.frame.captured_at.as_str())
                .max()
                .unwrap_or(representative.frame.captured_at.as_str())
                .to_string();
            let best_rank = hits
                .iter()
                .map(|hit| hit.rank)
                .min_by(|a, b| a.total_cmp(b))
                .unwrap_or(f64::INFINITY);
            let secret_redaction_count = hits
                .iter()
                .map(|hit| hit.secret_redaction_count)
                .max()
                .unwrap_or(0);
            // The group is meaning-only when no grouped anchor matched **Text
            // Search**: then there is no FTS term to highlight, so the snippet is
            // the leading `body_text` excerpt the semantic fetch carried. As soon
            // as any anchor matched a query term we prefer that highlighted
            // snippet and the group is a normal **Text Search** result.
            let text_hit = hits.iter().find(|hit| !hit.found_by_meaning);
            let found_by_meaning = text_hit.is_none();
            let snippet = text_hit.unwrap_or(&hits[0]).snippet.clone();
            Some((
                best_rank,
                FrameSearchResult {
                    group_key,
                    representative_frame: representative.frame.clone(),
                    group_start_at,
                    group_end_at,
                    match_count: hits.len() as u32,
                    snippet,
                    app_bundle_id: representative.app_bundle_id.clone(),
                    app_name: representative.app_name.clone(),
                    window_title: representative.window_title.clone(),
                    // Read-time: the representative frame's snapshot already
                    // carries `browser_url` (parsed by `map_frame_for_search`
                    // from the existing `frame_metadata_snapshots` join), so any
                    // historical frame is covered without an index column or
                    // backfill. The broker guards this URL before exposure.
                    browser_url: representative
                        .frame
                        .metadata_snapshot
                        .as_ref()
                        .and_then(|snapshot| snapshot.browser_url.clone()),
                    thumbnail_frame_id: representative.frame.id,
                    text_source_kind: representative.text_source_kind.clone(),
                    secret_redaction_count,
                    has_secret_redactions: secret_redaction_count > 0,
                    found_by_meaning,
                },
            ))
        })
        .collect::<Vec<_>>();

    results.sort_by(|(a_rank, a), (b_rank, b)| {
        a_rank
            .total_cmp(b_rank)
            .then_with(|| b.group_end_at.cmp(&a.group_end_at))
    });
    results.into_iter().map(|(_rank, result)| result).collect()
}

fn frame_hits_are_equivalent(left: &FrameHit, right: &FrameHit) -> bool {
    if left.frame.session_id != right.frame.session_id {
        return false;
    }

    let Some((_left_hint, left_proof, left_version)) = left.frame.equivalence.ready_parts() else {
        return left.frame.id == right.frame.id;
    };
    let Some((_right_hint, right_proof, right_version)) = right.frame.equivalence.ready_parts()
    else {
        return false;
    };
    CapturedFrameEquivalenceScope::from_frame(&left.frame)
        == CapturedFrameEquivalenceScope::from_frame(&right.frame)
        && left_version == right_version
        && capture_screen::captured_frame_equivalence_proofs_match(
            left_version,
            left_proof,
            right_proof,
        )
}

pub(super) fn group_audio_hits(hits: &[AudioHit]) -> Result<Vec<AudioSearchResult>> {
    let mut hits = hits.to_vec();
    hits.sort_by(|a, b| {
        a.audio_segment
            .id
            .cmp(&b.audio_segment.id)
            .then_with(|| a.span_start_ms.cmp(&b.span_start_ms))
            .then_with(|| a.span_end_ms.cmp(&b.span_end_ms))
            .then_with(|| a.rank.total_cmp(&b.rank))
    });

    let mut groups: Vec<Vec<AudioHit>> = Vec::new();
    for hit in hits {
        if let Some(last_group) = groups.last_mut() {
            if let Some(last) = last_group.last() {
                if last.audio_segment.id == hit.audio_segment.id
                    && hit.span_start_ms <= last.span_end_ms.saturating_add(AUDIO_GROUP_GAP_MS)
                {
                    last_group.push(hit);
                    continue;
                }
            }
        }
        groups.push(vec![hit]);
    }

    let mut results = Vec::new();
    for mut group in groups {
        group.sort_by(|a, b| {
            a.rank
                .total_cmp(&b.rank)
                .then_with(|| a.span_start_ms.cmp(&b.span_start_ms))
        });
        let first = group.first().expect("group should not be empty");
        let span_start_ms = group.iter().map(|hit| hit.span_start_ms).min().unwrap_or(0);
        let span_end_ms = group
            .iter()
            .map(|hit| hit.span_end_ms)
            .max()
            .unwrap_or(span_start_ms);
        let absolute_start_at = timestamp_plus_ms(&first.audio_segment.started_at, span_start_ms)?;
        let absolute_end_at = timestamp_plus_ms(&first.audio_segment.started_at, span_end_ms)?;
        let secret_redaction_count = group
            .iter()
            .map(|hit| hit.secret_redaction_count)
            .max()
            .unwrap_or(0);
        // Meaning-only when no grouped span matched **Text Search** (see
        // `group_frame_hits`): then the snippet is the leading `body_text`
        // excerpt the semantic fetch carried, not a highlighted FTS snippet.
        let text_hit = group.iter().find(|hit| !hit.found_by_meaning);
        let found_by_meaning = text_hit.is_none();
        let snippet = text_hit.unwrap_or(first).snippet.clone();
        results.push((
            first.rank,
            AudioSearchResult {
                group_key: format!(
                    "audio:{}:{}-{}",
                    first.audio_segment.id, span_start_ms, span_end_ms
                ),
                audio_segment: first.audio_segment.clone(),
                source_kind: first.source_kind.clone(),
                span_start_ms,
                span_end_ms,
                absolute_start_at,
                absolute_end_at,
                match_count: group.len() as u32,
                snippet,
                aligned_frame: None,
                secret_redaction_count,
                has_secret_redactions: secret_redaction_count > 0,
                found_by_meaning,
            },
        ));
    }

    results.sort_by(|(a_rank, a), (b_rank, b)| {
        a_rank
            .total_cmp(b_rank)
            .then_with(|| b.absolute_start_at.cmp(&a.absolute_start_at))
            .then_with(|| a.group_key.cmp(&b.group_key))
    });
    Ok(results.into_iter().map(|(_rank, result)| result).collect())
}

pub(super) fn map_audio_hit(row: SqliteRow) -> Result<AudioHit> {
    let source_kind =
        AudioSegmentSourceKind::from_str(row.get::<String, _>("source_kind").as_str());
    let audio_segment = AudioSegment {
        id: row.get("id"),
        source_kind: source_kind.clone(),
        source_session_id: row.get("source_session_id"),
        segment_index: row.get("segment_index"),
        file_path: row.get("file_path"),
        started_at: row.get("started_at"),
        ended_at: row.get("ended_at"),
        capture_segment_id: row.get("capture_segment_id"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    };
    Ok(AudioHit {
        anchor_id: row.get("document_id"),
        audio_segment,
        source_kind,
        span_start_ms: row
            .get::<Option<i64>, _>("span_start_ms")
            .unwrap_or(0)
            .max(0) as u64,
        span_end_ms: row.get::<Option<i64>, _>("span_end_ms").unwrap_or(0).max(0) as u64,
        snippet: row.get("snippet"),
        rank: row.get("rank"),
        secret_redaction_count: u32::try_from(row.get::<i64, _>("secret_redaction_count"))
            .unwrap_or(u32::MAX),
        // An FTS `MATCH` hit is a **Text Search** match; the semantic fetch path
        // builds its own `AudioHit`s with `found_by_meaning: true`.
        found_by_meaning: false,
    })
}

pub(super) fn map_audio_segment_for_search(row: SqliteRow) -> Result<AudioSegment> {
    Ok(AudioSegment {
        id: row.get("id"),
        source_kind: AudioSegmentSourceKind::from_str(row.get::<String, _>("source_kind").as_str()),
        source_session_id: row.get("source_session_id"),
        segment_index: row.get("segment_index"),
        file_path: row.get("file_path"),
        started_at: row.get("started_at"),
        ended_at: row.get("ended_at"),
        capture_segment_id: row.get("capture_segment_id"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

const AUDIO_FRAME_ALIGNMENT_WINDOW_SECONDS: i64 = 10;

pub(super) async fn align_audio_results(
    pool: &SqlitePool,
    results: &mut [AudioSearchResult],
) -> Result<()> {
    for result in results {
        let mut candidate_session_ids = Vec::new();
        if let Some(screen_source_session_id) =
            screen_source_session_id_for_audio_alignment(pool, &result.audio_segment).await?
        {
            candidate_session_ids.push(screen_source_session_id);
        }
        if !candidate_session_ids
            .iter()
            .any(|session_id| session_id == &result.audio_segment.source_session_id)
        {
            candidate_session_ids.push(result.audio_segment.source_session_id.clone());
        }

        result.aligned_frame = None;
        for session_id in candidate_session_ids {
            if let Some(frame) =
                find_aligned_frame(pool, &session_id, &result.absolute_start_at).await?
            {
                result.aligned_frame = Some(frame);
                break;
            }
        }
    }
    Ok(())
}

async fn screen_source_session_id_for_audio_alignment(
    pool: &SqlitePool,
    segment: &AudioSegment,
) -> Result<Option<String>> {
    if let Some(capture_segment_id) = segment.capture_segment_id {
        let row = sqlx::query(
            "SELECT capture_sessions.screen_source_session_id \
             FROM capture_segments \
             JOIN capture_sessions ON capture_sessions.capture_session_id = capture_segments.capture_session_id \
             WHERE capture_segments.id = ?1 \
             ORDER BY capture_sessions.id DESC LIMIT 1",
        )
        .bind(capture_segment_id)
        .fetch_optional(pool)
        .await?;
        if let Some(session_id) =
            row.and_then(|row| normalized_source_session_id(row.get("screen_source_session_id")))
        {
            return Ok(Some(session_id));
        }
    }

    let source_column = match segment.source_kind {
        AudioSegmentSourceKind::Microphone => "microphone_source_session_id",
        AudioSegmentSourceKind::SystemAudio => "system_audio_source_session_id",
    };
    let query = format!(
        "SELECT screen_source_session_id \
         FROM capture_sessions \
         WHERE {source_column} = ?1 \
         ORDER BY id DESC LIMIT 1",
    );
    let row = sqlx::query(&query)
        .bind(&segment.source_session_id)
        .fetch_optional(pool)
        .await?;
    Ok(row.and_then(|row| normalized_source_session_id(row.get("screen_source_session_id"))))
}

async fn find_aligned_frame(
    pool: &SqlitePool,
    session_id: &str,
    absolute_start_at: &str,
) -> Result<Option<Frame>> {
    let target = OffsetDateTime::parse(absolute_start_at, &Rfc3339).map_err(|error| {
        AppInfraError::FrameBatchFinalize(format!(
            "invalid search timestamp '{absolute_start_at}': {error}"
        ))
    })?;
    let before_start = target
        .checked_sub(Duration::seconds(AUDIO_FRAME_ALIGNMENT_WINDOW_SECONDS))
        .ok_or_else(|| {
            AppInfraError::FrameBatchFinalize("search alignment timestamp overflow".to_string())
        })?
        .format(&Rfc3339)
        .map_err(|error| {
            AppInfraError::FrameBatchFinalize(format!("failed to format search timestamp: {error}"))
        })?;
    let after_end = target
        .checked_add(Duration::seconds(AUDIO_FRAME_ALIGNMENT_WINDOW_SECONDS))
        .ok_or_else(|| {
            AppInfraError::FrameBatchFinalize("search alignment timestamp overflow".to_string())
        })?
        .format(&Rfc3339)
        .map_err(|error| {
            AppInfraError::FrameBatchFinalize(format!("failed to format search timestamp: {error}"))
        })?;

    let before = sqlx::query(
        "SELECT frames.id, session_id, file_path, captured_at, width, height, \
                equivalence_hint, equivalence_proof, equivalence_version, equivalence_status, equivalence_error, \
                frame_metadata_snapshots.snapshot_json AS metadata_snapshot_json, \
                frames.created_at, frames.updated_at \
         FROM frames \
         LEFT JOIN frame_metadata_snapshots ON frame_metadata_snapshots.id = frames.metadata_snapshot_id \
         WHERE session_id = ?1 AND captured_at >= ?2 AND captured_at <= ?3 \
         ORDER BY captured_at DESC, frames.id DESC LIMIT 1",
    )
    .bind(session_id)
    .bind(before_start)
    .bind(absolute_start_at)
    .fetch_optional(pool)
    .await?;
    if let Some(row) = before {
        return map_frame_for_search(row).map(Some);
    }

    let after = sqlx::query(
        "SELECT frames.id, session_id, file_path, captured_at, width, height, \
                equivalence_hint, equivalence_proof, equivalence_version, equivalence_status, equivalence_error, \
                frame_metadata_snapshots.snapshot_json AS metadata_snapshot_json, \
                frames.created_at, frames.updated_at \
         FROM frames \
         LEFT JOIN frame_metadata_snapshots ON frame_metadata_snapshots.id = frames.metadata_snapshot_id \
         WHERE session_id = ?1 AND captured_at > ?2 AND captured_at <= ?3 \
         ORDER BY captured_at ASC, frames.id ASC LIMIT 1",
    )
    .bind(session_id)
    .bind(absolute_start_at)
    .bind(after_end)
    .fetch_optional(pool)
    .await?;

    after.map(map_frame_for_search).transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::test_support::*;
    use crate::search::SearchCaptureRequest;
    use crate::{
        AppInfra, NewAudioSegment, NewCaptureSession, NewFrame, ProcessingJobDraft,
        ProcessingResultDraft,
    };
    use audio_transcription::{TranscriptionMetadata, TranscriptionSegment};

    #[test]
    fn audio_search_alignment_uses_mapped_screen_source_session() {
        run_async_test(async {
            let dir = test_dir("audio-screen-alignment");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            infra
                .capture_retention()
                .create_capture_session(&NewCaptureSession {
                    capture_session_id: "capture-session".to_string(),
                    started_at: "2026-05-17T10:00:00Z".to_string(),
                    requested_screen: true,
                    requested_microphone: true,
                    requested_system_audio: false,
                    screen_source_session_id: Some("screen-session".to_string()),
                    microphone_source_session_id: Some("mic-session".to_string()),
                    system_audio_source_session_id: None,
                    segment_duration_seconds: 300,
                })
                .await
                .expect("capture session should insert");
            let frame = infra
                .insert_frame(&NewFrame::new(
                    "screen-session",
                    "/tmp/search-aligned-screen.jpg",
                    "2026-05-17T10:00:01Z",
                ))
                .await
                .expect("screen frame should insert");
            let segment = infra
                .upsert_audio_segment(&NewAudioSegment::new(
                    AudioSegmentSourceKind::Microphone,
                    "mic-session",
                    1,
                    "/tmp/search-aligned-audio.m4a",
                    "2026-05-17T10:00:00Z",
                    "2026-05-17T10:00:20Z",
                ))
                .await
                .expect("segment should insert");
            let metadata = TranscriptionMetadata {
                provider: "test".to_string(),
                model_id: None,
                language: "en".to_string(),
                segments: vec![TranscriptionSegment {
                    start_ms: 1_000,
                    end_ms: 2_000,
                    text: "aligned audio target".to_string(),
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
                &infra,
                job,
                ProcessingResultDraft::new()
                    .with_result_text("aligned audio target")
                    .with_structured_payload_json(
                        serde_json::to_string(&metadata).expect("metadata should serialize"),
                    ),
            )
            .await;

            let response = infra
                .search_capture(SearchCaptureRequest {
                    query: "aligned".to_string(),
                    frame_limit: Some(0),
                    frame_offset: None,
                    audio_limit: Some(5),
                    audio_offset: None,
                    snapshot_document_id: None,
                    refinements: None,
                    query_embedding: None,
                })
                .await
                .expect("search should succeed");

            assert_eq!(response.audio.len(), 1);
            assert_eq!(
                response.audio[0]
                    .aligned_frame
                    .as_ref()
                    .map(|frame| frame.id),
                Some(frame.id)
            );
        });
    }

    #[test]
    fn audio_hits_group_chronologically_before_rank_ordering() {
        let segment = AudioSegment {
            id: 7,
            source_kind: AudioSegmentSourceKind::Microphone,
            source_session_id: "mic-session".to_string(),
            segment_index: 1,
            file_path: "/tmp/audio.m4a".to_string(),
            started_at: "2026-05-17T10:00:00Z".to_string(),
            ended_at: "2026-05-17T10:00:20Z".to_string(),
            capture_segment_id: None,
            created_at: "2026-05-17T10:00:00Z".to_string(),
            updated_at: "2026-05-17T10:00:00Z".to_string(),
        };
        let hit = |span_start_ms, span_end_ms, rank| AudioHit {
            anchor_id: span_start_ms as i64,
            audio_segment: segment.clone(),
            source_kind: AudioSegmentSourceKind::Microphone,
            span_start_ms,
            span_end_ms,
            snippet: format!("hit {span_start_ms}"),
            rank,
            secret_redaction_count: 0,
            found_by_meaning: false,
        };

        let hits = vec![
            hit(4_000, 4_500, -10.0),
            hit(1_000, 1_500, -1.0),
            hit(2_200, 2_500, -5.0),
        ];
        let groups = group_audio_hits(&hits).expect("grouping should succeed");

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].span_start_ms, 1_000);
        assert_eq!(groups[0].span_end_ms, 4_500);
        assert_eq!(groups[0].match_count, 3);
    }

    #[test]
    fn audio_groups_preserve_best_relevance_before_recency() {
        let segment = AudioSegment {
            id: 7,
            source_kind: AudioSegmentSourceKind::Microphone,
            source_session_id: "mic-session".to_string(),
            segment_index: 1,
            file_path: "/tmp/audio.m4a".to_string(),
            started_at: "2026-05-17T10:00:00Z".to_string(),
            ended_at: "2026-05-17T10:00:20Z".to_string(),
            capture_segment_id: None,
            created_at: "2026-05-17T10:00:00Z".to_string(),
            updated_at: "2026-05-17T10:00:00Z".to_string(),
        };
        let hit = |span_start_ms, rank| AudioHit {
            anchor_id: span_start_ms as i64,
            audio_segment: segment.clone(),
            source_kind: AudioSegmentSourceKind::Microphone,
            span_start_ms,
            span_end_ms: span_start_ms + 500,
            snippet: format!("hit {span_start_ms}"),
            rank,
            secret_redaction_count: 0,
            found_by_meaning: false,
        };

        let hits = vec![hit(10_000, -1.0), hit(1_000, -10.0)];
        let groups = group_audio_hits(&hits).expect("grouping should succeed");

        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].span_start_ms, 1_000);
        assert_eq!(groups[1].span_start_ms, 10_000);
    }

    #[test]
    fn frame_groups_preserve_best_relevance_before_recency() {
        let frame = |id: i64, captured_at: &str| Frame {
            id,
            session_id: "screen-session".to_string(),
            file_path: format!("/tmp/relevance-{id}.jpg"),
            captured_at: captured_at.to_string(),
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
            created_at: captured_at.to_string(),
            updated_at: captured_at.to_string(),
        };
        let hit = |id, captured_at, rank| FrameHit {
            anchor_id: id,
            group_key: format!("frame:{id}"),
            frame: frame(id, captured_at),
            snippet: format!("hit {id}"),
            rank,
            app_bundle_id: None,
            app_name: None,
            window_title: None,
            text_source_kind: "direct".to_string(),
            secret_redaction_count: 0,
            found_by_meaning: false,
        };

        let hits = vec![
            hit(1, "2026-05-17T10:00:00Z", -10.0),
            hit(2, "2026-05-17T10:10:00Z", -1.0),
        ];
        let groups = group_frame_hits(&hits);

        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].representative_frame.id, 1);
        assert_eq!(groups[1].representative_frame.id, 2);
    }

    #[test]
    fn frame_group_carries_representative_browser_url_read_time() {
        // Read-time proof: `group_frame_hits` lifts `browser_url` from the
        // SAME representative frame's metadata snapshot whose id becomes the
        // result (and opaque) id — no index column, so any historical frame
        // with a snapshot browser_url is covered for free.
        let frame_with_url = |id: i64, captured_at: &str, browser_url: Option<&str>| Frame {
            id,
            session_id: "screen-session".to_string(),
            file_path: format!("/tmp/url-{id}.jpg"),
            captured_at: captured_at.to_string(),
            width: None,
            height: None,
            equivalence: crate::FrameEquivalence {
                hint: None,
                proof: None,
                version: None,
                status: None,
                error: None,
            },
            metadata_snapshot: browser_url.map(|url| capture_metadata::FrameMetadataSnapshot {
                app_bundle_id: Some("com.google.Chrome".to_string()),
                app_name: Some("Google Chrome".to_string()),
                window_title: Some("Tab".to_string()),
                window_id: None,
                browser_url: Some(url.to_string()),
                display_id: Some(1),
                metadata_redaction_reason: None,
                metadata_redaction_source_id: None,
            }),
            created_at: captured_at.to_string(),
            updated_at: captured_at.to_string(),
        };
        let hit = |id, captured_at, browser_url| FrameHit {
            anchor_id: id,
            group_key: format!("frame:{id}"),
            frame: frame_with_url(id, captured_at, browser_url),
            snippet: format!("hit {id}"),
            rank: -1.0,
            app_bundle_id: None,
            app_name: None,
            window_title: None,
            text_source_kind: "direct".to_string(),
            secret_redaction_count: 0,
            found_by_meaning: false,
        };

        // With no equivalence proof, each distinct frame is its own group; the
        // representative IS the single hit, so its snapshot browser_url surfaces
        // raw on the result (the broker boundary guards it, not search).
        let groups = group_frame_hits(&[
            hit(
                1,
                "2026-05-17T10:10:00Z",
                Some("https://github.com/owner/repo/commit/9fceb02d8f1c"),
            ),
            // A frame with no snapshot browser_url -> result browser_url is None.
            hit(2, "2026-05-17T10:00:00Z", None),
        ]);
        let by_id = |id: i64| {
            groups
                .iter()
                .find(|group| group.representative_frame.id == id)
                .expect("group should exist")
        };
        assert_eq!(
            by_id(1).browser_url.as_deref(),
            Some("https://github.com/owner/repo/commit/9fceb02d8f1c"),
            "browser_url comes from the representative frame's snapshot, raw"
        );
        assert_eq!(by_id(2).browser_url, None);
    }

    #[test]
    fn audio_search_aligns_to_near_earlier_frame() {
        run_async_test(async {
            let dir = test_dir("audio-alignment-near-earlier");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let frame = infra
                .insert_frame(&NewFrame::new(
                    "shared-session",
                    "/tmp/alignment-near-frame.jpg",
                    "2026-05-17T10:00:56Z",
                ))
                .await
                .expect("frame should insert");
            let segment = infra
                .upsert_audio_segment(&NewAudioSegment::new(
                    AudioSegmentSourceKind::Microphone,
                    "shared-session",
                    1,
                    "/tmp/search-audio-alignment-near.m4a",
                    "2026-05-17T10:01:00Z",
                    "2026-05-17T10:01:20Z",
                ))
                .await
                .expect("segment should insert");
            let metadata = TranscriptionMetadata {
                provider: "test".to_string(),
                model_id: None,
                language: "en".to_string(),
                segments: vec![TranscriptionSegment {
                    start_ms: 1_000,
                    end_ms: 2_500,
                    text: "alignment target phrase".to_string(),
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
                &infra,
                job,
                ProcessingResultDraft::new()
                    .with_result_text("alignment target phrase")
                    .with_structured_payload_json(
                        serde_json::to_string(&metadata).expect("metadata should serialize"),
                    ),
            )
            .await;

            let response = infra
                .search_capture(SearchCaptureRequest {
                    query: "alignment".to_string(),
                    frame_limit: Some(0),
                    frame_offset: None,
                    audio_limit: Some(5),
                    audio_offset: None,
                    snapshot_document_id: None,
                    refinements: None,
                    query_embedding: None,
                })
                .await
                .expect("search should succeed");

            assert_eq!(response.audio.len(), 1);
            assert_eq!(
                response.audio[0]
                    .aligned_frame
                    .as_ref()
                    .map(|frame| frame.id),
                Some(frame.id)
            );
        });
    }

    #[test]
    fn audio_search_does_not_align_stale_earlier_frame() {
        run_async_test(async {
            let dir = test_dir("audio-alignment-stale-earlier");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            infra
                .insert_frame(&NewFrame::new(
                    "shared-session",
                    "/tmp/alignment-frame.jpg",
                    "2026-05-17T10:00:00Z",
                ))
                .await
                .expect("frame should insert");
            let segment = infra
                .upsert_audio_segment(&NewAudioSegment::new(
                    AudioSegmentSourceKind::Microphone,
                    "shared-session",
                    1,
                    "/tmp/search-audio-alignment.m4a",
                    "2026-05-17T10:01:00Z",
                    "2026-05-17T10:01:20Z",
                ))
                .await
                .expect("segment should insert");
            let metadata = TranscriptionMetadata {
                provider: "test".to_string(),
                model_id: None,
                language: "en".to_string(),
                segments: vec![TranscriptionSegment {
                    start_ms: 1_000,
                    end_ms: 2_500,
                    text: "alignment target phrase".to_string(),
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
                &infra,
                job,
                ProcessingResultDraft::new()
                    .with_result_text("alignment target phrase")
                    .with_structured_payload_json(
                        serde_json::to_string(&metadata).expect("metadata should serialize"),
                    ),
            )
            .await;

            let response = infra
                .search_capture(SearchCaptureRequest {
                    query: "alignment".to_string(),
                    frame_limit: Some(0),
                    frame_offset: None,
                    audio_limit: Some(5),
                    audio_offset: None,
                    snapshot_document_id: None,
                    refinements: None,
                    query_embedding: None,
                })
                .await
                .expect("search should succeed");

            assert_eq!(response.audio.len(), 1);
            assert_eq!(
                response.audio[0]
                    .aligned_frame
                    .as_ref()
                    .map(|frame| frame.id),
                None
            );
        });
    }

    #[test]
    fn frame_search_does_not_group_same_hint_with_different_proofs() {
        run_async_test(async {
            let dir = test_dir("frame-proof-grouping");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");

            for (path, proof) in [
                ("/tmp/search-proof-a.jpg", vec![0; 1024]),
                ("/tmp/search-proof-b.jpg", vec![255; 1024]),
            ] {
                let frame = infra
                    .insert_frame(
                        &NewFrame::new("screen-session", path, "2026-05-17T10:00:00Z")
                            .with_equivalence(crate::FrameEquivalence {
                                hint: Some("same-hint".to_string()),
                                proof: Some(proof),
                                version: Some(1),
                                status: Some(crate::FrameEquivalenceStatus::Ready),
                                error: None,
                            }),
                    )
                    .await
                    .expect("frame should insert");
                let job = infra
                    .enqueue_processing_job(&ProcessingJobDraft::for_frame_ocr(frame.id))
                    .await
                    .expect("ocr job should enqueue");
                complete_job(
                    &infra,
                    job,
                    ProcessingResultDraft::new().with_result_text("proof target phrase"),
                )
                .await;
            }

            let response = infra
                .search_capture(SearchCaptureRequest {
                    query: "proof".to_string(),
                    frame_limit: Some(5),
                    frame_offset: None,
                    audio_limit: Some(0),
                    audio_offset: None,
                    snapshot_document_id: None,
                    refinements: None,
                    query_embedding: None,
                })
                .await
                .expect("search should succeed");

            assert_eq!(response.frames.len(), 2);
            let mut group_keys = response
                .frames
                .iter()
                .map(|frame| frame.group_key.as_str())
                .collect::<Vec<_>>();
            group_keys.sort_unstable();
            group_keys.dedup();
            assert_eq!(group_keys.len(), 2);
            assert_eq!(
                response
                    .frames
                    .iter()
                    .map(|frame| frame.match_count)
                    .collect::<Vec<_>>(),
                vec![1, 1]
            );
        });
    }

    #[test]
    fn frame_search_does_not_group_equivalent_proofs_across_hidden_workspaces() {
        run_async_test(async {
            let dir = test_dir("frame-hidden-workspace-grouping");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let equivalence = crate::FrameEquivalence {
                hint: Some("same-hidden-proof".to_string()),
                proof: Some(vec![31; 1024]),
                version: Some(1),
                status: Some(crate::FrameEquivalenceStatus::Ready),
                error: None,
            };

            for (index, segment) in ["0001", "0002"].into_iter().enumerate() {
                let frame_path = dir
                    .join(format!(
                        "recordings/2026/05/17/.screen-session-segment-{segment}/frames/frame-1.jpg"
                    ))
                    .to_string_lossy()
                    .to_string();
                let frame = infra
                    .insert_frame(
                        &NewFrame::new(
                            "screen-session",
                            &frame_path,
                            &format!("2026-05-17T10:00:0{index}Z"),
                        )
                        .with_equivalence(equivalence.clone()),
                    )
                    .await
                    .expect("frame should insert");
                let job = infra
                    .enqueue_processing_job(&ProcessingJobDraft::for_frame_ocr(frame.id))
                    .await
                    .expect("ocr job should enqueue");
                complete_job(
                    &infra,
                    job,
                    ProcessingResultDraft::new().with_result_text("hidden scope phrase"),
                )
                .await;
            }

            let response = infra
                .search_capture(SearchCaptureRequest {
                    query: "hidden".to_string(),
                    frame_limit: Some(5),
                    frame_offset: None,
                    audio_limit: Some(0),
                    audio_offset: None,
                    snapshot_document_id: None,
                    refinements: None,
                    query_embedding: None,
                })
                .await
                .expect("search should succeed");

            assert_eq!(response.frames.len(), 2);
            let mut group_keys = response
                .frames
                .iter()
                .map(|frame| frame.group_key.as_str())
                .collect::<Vec<_>>();
            group_keys.sort_unstable();
            group_keys.dedup();
            assert_eq!(group_keys.len(), 2);
            assert_eq!(
                response
                    .frames
                    .iter()
                    .map(|frame| frame.match_count)
                    .collect::<Vec<_>>(),
                vec![1, 1]
            );
        });
    }
}
