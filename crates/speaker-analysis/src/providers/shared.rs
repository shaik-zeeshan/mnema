//! Provider-agnostic speaker-analysis helpers shared by the on-device
//! diarization provider (speakrs).
//!
//! These items hold recognition policy, embedding encoding, turn
//! post-processing, audio sanity checks, provenance shaping, and the macOS
//! audio decode entry — none of which are specific to a particular runtime. They
//! are gated on `feature = "speakrs"`.

use serde_json::json;

use crate::{
    RecognitionConfidence, SpeakerAnalysisOutput, SpeakerAnalysisRequest, SpeakerRecognitionSuggestion,
    SpeakerTurn,
};

// ---------------------------------------------------------------------------
// Sample rate + audio sanity
// ---------------------------------------------------------------------------

/// Fixed analysis sample rate. Both providers decode/resample to mono 16 kHz.
pub(crate) const SAMPLE_RATE_HZ: u32 = 16_000;
/// Below this duration a job produces a successful EMPTY result (`skipReason =
/// too_short`), not a failure (see CONTEXT.md).
pub(crate) const MIN_DIARIZATION_AUDIO_MS: u64 = 1_000;
/// Below this absolute peak the audio is treated as silent (`skipReason =
/// silent`) and produces a successful EMPTY result.
pub(crate) const MIN_DIARIZATION_PEAK: f32 = 1.0e-5;

// ---------------------------------------------------------------------------
// Recognition policy
// ---------------------------------------------------------------------------

/// Minimum cosine similarity for a recognition suggestion to surface at all.
pub(crate) const MIN_RECOGNITION_SUGGESTION_SCORE: f32 = 0.60;
/// Cosine similarity at/above which a suggestion is High confidence.
pub(crate) const HIGH_RECOGNITION_SUGGESTION_SCORE: f32 = 0.72;
/// If the top two distinct people are within this margin the match is ambiguous
/// and is suppressed.
pub(crate) const PERSON_AMBIGUITY_MARGIN: f32 = 0.05;
/// A person is skipped if a prior rejection embedding for them is at least this
/// similar to the cluster embedding.
pub(crate) const REJECTED_PERSON_SIMILARITY_THRESHOLD: f32 = 0.80;

/// Cautious recognition: returns the single best enrolled-person suggestion for
/// a cluster embedding, or `None` when below threshold, rejected, or ambiguous.
/// Recognition only compares within the active preset's Voiceprint Space (the
/// `model_id` filter), per CONTEXT.md.
pub(crate) fn best_enrollment_match(
    request: &SpeakerAnalysisRequest,
    embedding: &[f32],
    model_id: &str,
) -> Option<SpeakerRecognitionSuggestion> {
    let mut matches = request
        .enrolled_people
        .iter()
        .filter(|person| person.embedding_model_id == model_id)
        .filter_map(|person| {
            let enrolled = f32_embedding_from_le_bytes(&person.embedding)?;
            let score = cosine_similarity(&enrolled, embedding);
            if score < MIN_RECOGNITION_SUGGESTION_SCORE
                || has_similar_rejection(request, person.person_id, embedding, model_id)
            {
                return None;
            }
            let confidence = if score >= HIGH_RECOGNITION_SUGGESTION_SCORE {
                RecognitionConfidence::High
            } else {
                RecognitionConfidence::Medium
            };
            Some(SpeakerRecognitionSuggestion {
                person_id: person.person_id,
                display_name: person.display_name.clone(),
                confidence,
                score,
            })
        })
        .collect::<Vec<_>>();
    matches.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.person_id.cmp(&right.person_id))
    });

    let best = matches.first()?;
    if matches
        .iter()
        .find(|candidate| candidate.person_id != best.person_id)
        .is_some_and(|second| best.score - second.score < PERSON_AMBIGUITY_MARGIN)
    {
        return None;
    }
    Some(best.clone())
}

/// Whether the request carries a prior rejection of `person_id` whose embedding
/// is similar enough to this cluster embedding to suppress the suggestion.
pub(crate) fn has_similar_rejection(
    request: &SpeakerAnalysisRequest,
    person_id: i64,
    embedding: &[f32],
    model_id: &str,
) -> bool {
    request
        .rejected_people
        .iter()
        .filter(|rejection| {
            rejection.person_id == person_id && rejection.embedding_model_id == model_id
        })
        .filter_map(|rejection| f32_embedding_from_le_bytes(&rejection.embedding))
        .any(|rejected| {
            cosine_similarity(&rejected, embedding) >= REJECTED_PERSON_SIMILARITY_THRESHOLD
        })
}

// ---------------------------------------------------------------------------
// Embedding encoding + similarity
// ---------------------------------------------------------------------------

/// Encode an f32 speaker embedding as little-endian bytes for storage.
pub(crate) fn f32_embedding_to_le_bytes(embedding: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(embedding.len() * 4);
    for value in embedding {
        out.extend_from_slice(&value.to_le_bytes());
    }
    out
}

/// Decode a little-endian byte embedding back to f32, or `None` if malformed.
pub(crate) fn f32_embedding_from_le_bytes(bytes: &[u8]) -> Option<Vec<f32>> {
    if bytes.len() % 4 != 0 {
        return None;
    }
    Some(
        bytes
            .chunks_exact(4)
            .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .collect(),
    )
}

/// Cosine similarity of two equal-length embeddings; 0.0 for length mismatch or
/// empty input.
pub(crate) fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot = a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>();
    let a_norm = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let b_norm = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    dot / (a_norm * b_norm).max(f32::EPSILON)
}

// ---------------------------------------------------------------------------
// Provider-local cluster ids + time/index helpers
// ---------------------------------------------------------------------------

// The speakrs path goes through the pure `speakrs_mapping` module (which carries
// its own always-compiled `provider_cluster_id`/`seconds_to_ms` so the
// highest-value mapping test compiles with no features), so no provider-local
// sample-range helpers live here anymore.

// ---------------------------------------------------------------------------
// Turn post-processing
// ---------------------------------------------------------------------------

/// Adjacent same-cluster turns within this gap are merged into one.
pub(crate) const MERGE_ADJACENT_TURN_GAP_MS: u64 = 250;

/// Merge adjacent turns of the same cluster whose gap is within
/// [`MERGE_ADJACENT_TURN_GAP_MS`]. Sorts by (start, end) first.
pub(crate) fn merge_adjacent_turns(mut turns: Vec<SpeakerTurn>) -> Vec<SpeakerTurn> {
    turns.sort_by_key(|turn| (turn.start_ms, turn.end_ms));
    let mut merged = Vec::<SpeakerTurn>::new();
    for turn in turns {
        if let Some(last) = merged.last_mut() {
            if last.provider_cluster_id == turn.provider_cluster_id
                && turn.start_ms <= last.end_ms.saturating_add(MERGE_ADJACENT_TURN_GAP_MS)
            {
                last.end_ms = last.end_ms.max(turn.end_ms);
                continue;
            }
        }
        merged.push(turn);
    }
    merged
}

/// Mark each turn that time-overlaps a turn from a *different* cluster.
pub(crate) fn mark_overlapping_turns(mut turns: Vec<SpeakerTurn>) -> Vec<SpeakerTurn> {
    for index in 0..turns.len() {
        let overlaps = turns.iter().enumerate().any(|(other_index, other)| {
            other_index != index
                && other.provider_cluster_id != turns[index].provider_cluster_id
                && other.end_ms > turns[index].start_ms
                && other.start_ms < turns[index].end_ms
        });
        if overlaps {
            turns[index].overlaps = true;
        }
    }
    turns
}

// ---------------------------------------------------------------------------
// Audio sanity
// ---------------------------------------------------------------------------

/// Reject decoded audio that contains non-finite samples (a runtime failure,
/// not a skip).
pub(crate) fn validate_decoded_samples(samples: &[f32]) -> crate::SpeakerAnalysisResult<()> {
    if samples.iter().any(|sample| !sample.is_finite()) {
        return Err(crate::SpeakerAnalysisError::Runtime {
            stage: "validate_decoded_samples".to_string(),
            message: "decoded speaker-analysis audio contained non-finite samples".to_string(),
        });
    }
    Ok(())
}

/// Largest absolute sample value (a cheap silence detector input).
pub(crate) fn audio_peak(samples: &[f32]) -> f32 {
    samples
        .iter()
        .map(|sample| sample.abs())
        .fold(0.0_f32, f32::max)
}

/// Returns a skip reason (too_short / silent) when the audio should produce a
/// successful EMPTY result rather than running diarization.
pub(crate) fn speaker_skip_reason(audio_peak: f32, duration_ms: u64) -> Option<&'static str> {
    if duration_ms < MIN_DIARIZATION_AUDIO_MS {
        return Some("too_short");
    }

    if audio_peak < MIN_DIARIZATION_PEAK {
        return Some("silent");
    }

    None
}

// ---------------------------------------------------------------------------
// Provenance shaping
// ---------------------------------------------------------------------------

/// Finalize the per-job count + duration provenance keys both providers carry.
///
/// `analysis_started_at` is the start of the analysis pass (decode + diarize);
/// `analysisDurationMs` records its wall-clock so the two providers report a
/// uniform diagnostic. `clustersPerSegment` mirrors the final global cluster
/// count for this Audio Segment. Called on both the successful and the
/// skip/empty paths so the keys are always present.
pub(crate) fn finalize_provenance_counts(
    output: &mut SpeakerAnalysisOutput,
    analysis_duration_ms: u64,
) {
    output
        .metadata
        .provenance
        .insert("turnCount".to_string(), json!(output.turns.len()));
    let cluster_count = output.clusters.len();
    output
        .metadata
        .provenance
        .insert("clusterCount".to_string(), json!(cluster_count));
    // Slice 3: the final global cluster count for this segment, named with a
    // clear stable key so downstream stays uniform across providers.
    output
        .metadata
        .provenance
        .insert("clustersPerSegment".to_string(), json!(cluster_count));
    // Slice 3: wall-clock of the analysis pass (decode + diarize) in ms.
    output
        .metadata
        .provenance
        .insert("analysisDurationMs".to_string(), json!(analysis_duration_ms));
}

/// Append a warning reason to the `warningReasons` provenance array.
pub(crate) fn add_warning_reason(output: &mut SpeakerAnalysisOutput, reason: &str) {
    let mut reasons = output
        .metadata
        .provenance
        .get("warningReasons")
        .and_then(serde_json::Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    reasons.push(reason.to_string());
    output
        .metadata
        .provenance
        .insert("warningReasons".to_string(), json!(reasons));
}

// ---------------------------------------------------------------------------
// Audio decode entry (per-platform; feeds the platform-agnostic pipeline)
// ---------------------------------------------------------------------------

/// Decode an audio file to mono 16 kHz f32 via AVFoundation (macOS). Both
/// providers feed the diarizer mono 16 kHz, so the decode is shared.
#[cfg(target_os = "macos")]
pub(crate) fn decode_audio_to_mono_16khz(
    path: &std::path::Path,
) -> crate::SpeakerAnalysisResult<Vec<f32>> {
    let decoded =
        crate::macos_audio_decode::decode_audio_to_mono_with_avassetreader_fallback(path)?;
    Ok(crate::macos_audio_decode::resample_linear(
        &decoded.samples,
        decoded.sample_rate_hz,
        SAMPLE_RATE_HZ,
    ))
}

/// Windows decode goes through the shared `media-decode` Media Foundation seam
/// (ADR 0024) to mono 16 kHz f32, feeding the same platform-agnostic pipeline.
/// The CPU Execution Backend consumes the identical mono-16-kHz contract.
#[cfg(target_os = "windows")]
pub(crate) fn decode_audio_to_mono_16khz(
    path: &std::path::Path,
) -> crate::SpeakerAnalysisResult<Vec<f32>> {
    crate::windows_audio_decode::decode_audio_to_mono_16khz(path)
}

/// Other platforms: no decode backend in v1 (speakrs is macOS/Windows only).
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub(crate) fn decode_audio_to_mono_16khz(
    _path: &std::path::Path,
) -> crate::SpeakerAnalysisResult<Vec<f32>> {
    Err(crate::SpeakerAnalysisError::ProviderUnavailable(
        "speaker-analysis audio decoding is only implemented with AVFoundation (macOS) and \
         Media Foundation (Windows) in v1"
            .to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        PersonEnrollment, PersonRecognitionRejection, SpeakerAnalysisRequest,
        SPEAKRS_DEFAULT_MODEL_ID, SPEAKRS_PROVIDER_ID,
    };

    fn request_with_enrollment(score: f32) -> SpeakerAnalysisRequest {
        let mut request = SpeakerAnalysisRequest::new(
            "/tmp/audio.m4a",
            SPEAKRS_PROVIDER_ID,
            Some(SPEAKRS_DEFAULT_MODEL_ID.to_string()),
            "session-a",
            7,
        );
        request.enrolled_people.push(PersonEnrollment {
            person_id: 1,
            display_name: "Jack".to_string(),
            embedding: f32_embedding_to_le_bytes(&unit_embedding_for_score(score)),
            embedding_model_id: SPEAKRS_DEFAULT_MODEL_ID.to_string(),
        });
        request
    }

    fn unit_embedding_for_score(score: f32) -> [f32; 2] {
        [score, (1.0 - score.powi(2)).max(0.0).sqrt()]
    }

    #[test]
    fn embedding_bytes_round_trip() {
        let embedding = vec![0.1, -0.2, 0.3];
        let bytes = f32_embedding_to_le_bytes(&embedding);
        assert_eq!(f32_embedding_from_le_bytes(&bytes), Some(embedding));
    }

    #[test]
    fn cosine_similarity_handles_mismatched_and_empty() {
        assert_eq!(cosine_similarity(&[], &[]), 0.0);
        assert_eq!(cosine_similarity(&[1.0, 0.0], &[1.0]), 0.0);
        assert!((cosine_similarity(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn recognition_skips_weak_best_match() {
        let request = request_with_enrollment(0.59);
        let suggestion = best_enrollment_match(&request, &[1.0, 0.0], SPEAKRS_DEFAULT_MODEL_ID);
        assert!(suggestion.is_none());
    }

    #[test]
    fn recognition_maps_high_confidence_from_strict_threshold() {
        let request = request_with_enrollment(0.72);
        let suggestion = best_enrollment_match(&request, &[1.0, 0.0], SPEAKRS_DEFAULT_MODEL_ID)
            .expect("suggestion");
        assert_eq!(suggestion.confidence, RecognitionConfidence::High);
        assert!(suggestion.score >= 0.72);
    }

    #[test]
    fn recognition_maps_medium_confidence_from_minimum_threshold() {
        let request = request_with_enrollment(0.60);
        let suggestion = best_enrollment_match(&request, &[1.0, 0.0], SPEAKRS_DEFAULT_MODEL_ID)
            .expect("suggestion");
        assert_eq!(suggestion.confidence, RecognitionConfidence::Medium);
        assert!(suggestion.score >= 0.60);
        assert!(suggestion.score < 0.72);
    }

    #[test]
    fn recognition_skips_person_with_similar_rejection() {
        let mut request = request_with_enrollment(1.0);
        request.rejected_people.push(PersonRecognitionRejection {
            person_id: 1,
            embedding: f32_embedding_to_le_bytes(&[1.0, 0.0]),
            embedding_model_id: SPEAKRS_DEFAULT_MODEL_ID.to_string(),
        });
        let suggestion = best_enrollment_match(&request, &[1.0, 0.0], SPEAKRS_DEFAULT_MODEL_ID);
        assert!(suggestion.is_none());
    }

    #[test]
    fn recognition_skips_ambiguous_top_two_people() {
        let mut request = request_with_enrollment(0.72);
        request.enrolled_people.push(PersonEnrollment {
            person_id: 2,
            display_name: "Jill".to_string(),
            embedding: f32_embedding_to_le_bytes(&unit_embedding_for_score(0.68)),
            embedding_model_id: SPEAKRS_DEFAULT_MODEL_ID.to_string(),
        });
        let suggestion = best_enrollment_match(&request, &[1.0, 0.0], SPEAKRS_DEFAULT_MODEL_ID);
        assert!(suggestion.is_none());
    }

    #[test]
    fn recognition_keeps_close_same_person_enrollments_unambiguous() {
        let mut request = request_with_enrollment(0.72);
        request.enrolled_people.push(PersonEnrollment {
            person_id: 1,
            display_name: "Jack".to_string(),
            embedding: f32_embedding_to_le_bytes(&unit_embedding_for_score(0.71)),
            embedding_model_id: SPEAKRS_DEFAULT_MODEL_ID.to_string(),
        });
        let suggestion = best_enrollment_match(&request, &[1.0, 0.0], SPEAKRS_DEFAULT_MODEL_ID)
            .expect("same-person enrollments should not be ambiguous");
        assert_eq!(suggestion.person_id, 1);
        assert!(suggestion.score >= 0.72);
    }

    #[test]
    fn merges_adjacent_turns_for_same_cluster() {
        let turns = vec![
            SpeakerTurn {
                provider_cluster_id: "speaker_00".to_string(),
                start_ms: 0,
                end_ms: 1_000,
                transcript_text: None,
                overlaps: false,
            },
            SpeakerTurn {
                provider_cluster_id: "speaker_00".to_string(),
                start_ms: 1_050,
                end_ms: 2_000,
                transcript_text: None,
                overlaps: false,
            },
            SpeakerTurn {
                provider_cluster_id: "speaker_01".to_string(),
                start_ms: 2_500,
                end_ms: 3_000,
                transcript_text: None,
                overlaps: false,
            },
        ];

        let merged = merge_adjacent_turns(turns);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].start_ms, 0);
        assert_eq!(merged[0].end_ms, 2_000);
    }

    #[test]
    fn skips_for_short_or_silent_audio() {
        assert_eq!(speaker_skip_reason(0.1, 500), Some("too_short"));
        assert_eq!(speaker_skip_reason(0.0, 2_000), Some("silent"));
        assert_eq!(speaker_skip_reason(0.1, 2_000), None);
    }

    #[test]
    fn non_finite_samples_return_typed_runtime_error() {
        let error =
            validate_decoded_samples(&[0.0, f32::NAN]).expect_err("non-finite samples should fail");
        assert!(matches!(
            error,
            crate::SpeakerAnalysisError::Runtime { ref stage, .. }
                if stage == "validate_decoded_samples"
        ));
    }
}
