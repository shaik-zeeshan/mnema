//! Pure mapping from a speakrs `DiarizationResult` (decomposed into plain types)
//! to the provider-neutral [`SpeakerTurn`] / per-cluster centroid contract.
//!
//! This module is compiled by default (no feature gate) and names NO speakrs or
//! `ndarray` types in its signatures, so the highest-value mapping test runs
//! under `cargo test -p speaker-analysis` with no features and no native speakrs
//! build. The feature-gated `providers::speakrs` glue decomposes the real
//! `DiarizationResult` arrays into the flat slices this function takes.
//!
//! ## Label/cluster alignment (verified against speakrs source)
//!
//! speakrs `segments` come from `discrete_diarization.to_segments(...)`, whose
//! speaker label index is the *column index* of the reconstructed activation
//! matrix. `Reconstructor::frame_activations` builds that matrix so that column
//! `c` corresponds to global cluster id `c` from `hard_clusters` (it maps each
//! chunk-speaker slot into `mapping[label]` and writes `activations[[frame,
//! cluster_idx]]`). Therefore a `"SPEAKER_NN"` segment label index == the global
//! cluster id stored in `hard_clusters`: **the two share the SPEAKER_NN label
//! space, so turns need no remap.**

use crate::SpeakerTurn;

/// One global speaker cluster's mean-pooled, L2-normalized 256-d centroid.
#[derive(Debug, Clone, PartialEq)]
pub struct SpeakerClusterCentroid {
    /// Global cluster id (the `hard_clusters` value == the `SPEAKER_NN` index).
    pub global_id: usize,
    /// L2-normalized 256-d WeSpeaker embedding (mean of the per-chunk rows).
    pub embedding: Vec<f32>,
}

/// The mapped turns + per-global-cluster centroids for one Audio Segment.
#[derive(Debug, Clone, PartialEq)]
pub struct SpeakrsMapping {
    /// Speaker turns, one per speakrs segment, with provider-local cluster ids.
    pub turns: Vec<SpeakerTurn>,
    /// Per-global-cluster centroids, sorted by `global_id`.
    pub clusters: Vec<SpeakerClusterCentroid>,
}

/// Provider-local cluster id string ("speaker_NN").
///
/// An always-compiled copy of `providers::shared::provider_cluster_id` (which is
/// feature-gated): this pure module must compile and emit identical ids without
/// pulling in a provider feature. The two MUST stay in sync.
pub(crate) fn provider_cluster_id(speaker: i32) -> String {
    format!("speaker_{speaker:02}")
}

/// Round seconds (clamped non-negative) to whole milliseconds (matches
/// `providers::shared::seconds_to_ms`).
fn seconds_to_ms(seconds: f64) -> u64 {
    (seconds.max(0.0) * 1000.0).round() as u64
}

/// Map a decomposed speakrs `DiarizationResult` into turns + centroids.
///
/// - `segments`: `(start_sec, end_sec, speaker_label)` e.g. `"SPEAKER_03"`.
/// - `embeddings`: flattened `(chunks * speakers * dim)` row-major.
/// - `hard_clusters`: flattened `(chunks * speakers)` row-major; ANY negative
///   value (speakrs uses `-2` for unassigned/inactive slots, with a stale `-1`
///   in its doc comment) marks a slot to skip.
///
/// Turns: each segment becomes a [`SpeakerTurn`] whose `provider_cluster_id`
/// derives from the numeric global id parsed from its `"SPEAKER_NN"` label
/// (no remap — labels and `hard_clusters` ids share the same space).
///
/// Clusters: for each `(chunk, speaker)` slot with a non-negative cluster id and
/// a finite embedding row, accumulate the row into that global id's running sum;
/// then mean-pool, L2-normalize, and emit one centroid per global id, sorted by
/// `global_id`.
pub fn map_speakrs_result(
    segments: &[(f64, f64, String)],
    chunks: usize,
    speakers: usize,
    dim: usize,
    embeddings: &[f32],
    hard_clusters: &[i32],
) -> SpeakrsMapping {
    let turns = segments
        .iter()
        .map(|(start_sec, end_sec, label)| {
            let global_id = parse_speaker_label(label);
            SpeakerTurn {
                provider_cluster_id: provider_cluster_id(global_id),
                start_ms: seconds_to_ms(*start_sec),
                end_ms: seconds_to_ms(*end_sec),
                transcript_text: None,
                overlaps: false,
            }
        })
        .collect();

    let clusters = accumulate_centroids(chunks, speakers, dim, embeddings, hard_clusters);

    SpeakrsMapping { turns, clusters }
}

/// Parse the numeric global id from a `"SPEAKER_NN"` label. Unparseable labels
/// map to 0 (defensive; speakrs always emits the zero-padded numeric form).
fn parse_speaker_label(label: &str) -> i32 {
    label
        .rsplit('_')
        .next()
        .and_then(|digits| digits.parse::<i32>().ok())
        .unwrap_or(0)
}

/// Running mean accumulator for one global cluster.
struct CentroidAccumulator {
    sum: Vec<f32>,
    count: usize,
}

fn accumulate_centroids(
    chunks: usize,
    speakers: usize,
    dim: usize,
    embeddings: &[f32],
    hard_clusters: &[i32],
) -> Vec<SpeakerClusterCentroid> {
    use std::collections::BTreeMap;

    // Guard against shape mismatches so a malformed decomposition can never
    // index out of bounds — emit no centroids rather than panic.
    if dim == 0
        || embeddings.len() != chunks.saturating_mul(speakers).saturating_mul(dim)
        || hard_clusters.len() != chunks.saturating_mul(speakers)
    {
        return Vec::new();
    }

    let mut accumulators: BTreeMap<usize, CentroidAccumulator> = BTreeMap::new();

    for chunk in 0..chunks {
        for speaker in 0..speakers {
            let slot = chunk * speakers + speaker;
            let global = hard_clusters[slot];
            // Skip unassigned/inactive slots (any negative sentinel).
            if global < 0 {
                continue;
            }
            let row_start = slot * dim;
            let row = &embeddings[row_start..row_start + dim];
            // Skip rows with any non-finite value; they would poison the mean.
            if row.iter().any(|value| !value.is_finite()) {
                continue;
            }
            let entry = accumulators
                .entry(global as usize)
                .or_insert_with(|| CentroidAccumulator {
                    sum: vec![0.0_f32; dim],
                    count: 0,
                });
            for (acc, value) in entry.sum.iter_mut().zip(row) {
                *acc += *value;
            }
            entry.count += 1;
        }
    }

    // BTreeMap iterates in ascending key order, so the result is already sorted
    // by global_id.
    accumulators
        .into_iter()
        .filter_map(|(global_id, acc)| {
            if acc.count == 0 {
                return None;
            }
            let mut embedding: Vec<f32> = acc
                .sum
                .iter()
                .map(|value| value / acc.count as f32)
                .collect();
            l2_normalize(&mut embedding);
            Some(SpeakerClusterCentroid {
                global_id,
                embedding,
            })
        })
        .collect()
}

/// L2-normalize in place; leaves an all-zero (degenerate) vector untouched.
fn l2_normalize(embedding: &mut [f32]) {
    let norm = embedding.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm > f32::EPSILON {
        for value in embedding.iter_mut() {
            *value /= norm;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn l2_norm(embedding: &[f32]) -> f32 {
        embedding.iter().map(|v| v * v).sum::<f32>().sqrt()
    }

    /// 2 chunks x 2 speakers x dim 2. Speaker 0 is global cluster 0 in both
    /// chunks; speaker 1 is global cluster 1 in both chunks. Hand-computable
    /// mean-pool + L2 norm.
    #[test]
    fn maps_two_chunk_two_speaker_fixture() {
        let segments = vec![
            (0.0_f64, 1.0_f64, "SPEAKER_00".to_string()),
            (1.0_f64, 2.0_f64, "SPEAKER_01".to_string()),
        ];
        let chunks = 2;
        let speakers = 2;
        let dim = 2;
        // Layout per slot (chunk-major, then speaker): rows for
        // (c0,s0),(c0,s1),(c1,s0),(c1,s1).
        let embeddings = vec![
            2.0, 0.0, // c0 s0 -> cluster 0
            0.0, 4.0, // c0 s1 -> cluster 1
            4.0, 0.0, // c1 s0 -> cluster 0
            0.0, 8.0, // c1 s1 -> cluster 1
        ];
        let hard_clusters = vec![0, 1, 0, 1];

        let mapping =
            map_speakrs_result(&segments, chunks, speakers, dim, &embeddings, &hard_clusters);

        // Turns: count + cluster ids line up with SPEAKER_NN labels.
        assert_eq!(mapping.turns.len(), 2);
        assert_eq!(mapping.turns[0].provider_cluster_id, "speaker_00");
        assert_eq!(mapping.turns[0].start_ms, 0);
        assert_eq!(mapping.turns[0].end_ms, 1_000);
        assert_eq!(mapping.turns[1].provider_cluster_id, "speaker_01");

        // Clusters: two, sorted by global_id 0 then 1.
        assert_eq!(mapping.clusters.len(), 2);
        assert_eq!(mapping.clusters[0].global_id, 0);
        assert_eq!(mapping.clusters[1].global_id, 1);

        // Cluster 0 mean = ((2,0)+(4,0))/2 = (3,0) -> L2-normalized -> (1,0).
        let c0 = &mapping.clusters[0].embedding;
        assert!((c0[0] - 1.0).abs() < 1e-6, "c0 = {c0:?}");
        assert!(c0[1].abs() < 1e-6, "c0 = {c0:?}");
        assert!((l2_norm(c0) - 1.0).abs() < 1e-6);

        // Cluster 1 mean = ((0,4)+(0,8))/2 = (0,6) -> L2-normalized -> (0,1).
        let c1 = &mapping.clusters[1].embedding;
        assert!(c1[0].abs() < 1e-6, "c1 = {c1:?}");
        assert!((c1[1] - 1.0).abs() < 1e-6, "c1 = {c1:?}");
        assert!((l2_norm(c1) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn skips_negative_sentinel_slots() {
        // One slot is -2 (speakrs sentinel), one is -1 (stale doc value): both
        // must be excluded from their cluster's mean. Cluster 0 should then
        // average only the single remaining valid row.
        let segments: Vec<(f64, f64, String)> = vec![];
        let chunks = 2;
        let speakers = 2;
        let dim = 2;
        let embeddings = vec![
            10.0, 0.0, // c0 s0 -> cluster 0 (valid)
            99.0, 99.0, // c0 s1 -> -2 sentinel (skipped)
            99.0, 99.0, // c1 s0 -> -1 stale sentinel (skipped)
            0.0, 5.0, // c1 s1 -> cluster 1 (valid)
        ];
        let hard_clusters = vec![0, -2, -1, 1];

        let mapping =
            map_speakrs_result(&segments, chunks, speakers, dim, &embeddings, &hard_clusters);

        assert_eq!(mapping.clusters.len(), 2);
        // Cluster 0: only the (10,0) row contributed -> normalized (1,0).
        assert_eq!(mapping.clusters[0].global_id, 0);
        assert!((mapping.clusters[0].embedding[0] - 1.0).abs() < 1e-6);
        assert!(mapping.clusters[0].embedding[1].abs() < 1e-6);
        // Cluster 1: only the (0,5) row contributed -> normalized (0,1).
        assert_eq!(mapping.clusters[1].global_id, 1);
        assert!((mapping.clusters[1].embedding[1] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn skips_non_finite_embedding_rows() {
        // Cluster 0 has one finite row and one NaN row; the NaN row must be
        // dropped so the centroid stays finite and equals the finite row.
        let segments: Vec<(f64, f64, String)> = vec![];
        let chunks = 2;
        let speakers = 1;
        let dim = 2;
        let embeddings = vec![
            3.0, 4.0, // c0 s0 -> cluster 0 (finite)
            f32::NAN, 1.0, // c1 s0 -> cluster 0 (NaN, skipped)
        ];
        let hard_clusters = vec![0, 0];

        let mapping =
            map_speakrs_result(&segments, chunks, speakers, dim, &embeddings, &hard_clusters);

        assert_eq!(mapping.clusters.len(), 1);
        let centroid = &mapping.clusters[0].embedding;
        assert!(centroid.iter().all(|v| v.is_finite()));
        // (3,4) normalized -> (0.6, 0.8).
        assert!((centroid[0] - 0.6).abs() < 1e-6, "centroid = {centroid:?}");
        assert!((centroid[1] - 0.8).abs() < 1e-6, "centroid = {centroid:?}");
        assert!((l2_norm(centroid) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn label_index_aligns_with_cluster_global_id() {
        // A "SPEAKER_01" segment must yield provider_cluster_id(1) and line up
        // with the centroid whose global_id is 1.
        let segments = vec![(0.0_f64, 0.5_f64, "SPEAKER_01".to_string())];
        let chunks = 1;
        let speakers = 2;
        let dim = 2;
        let embeddings = vec![
            1.0, 0.0, // s0 -> cluster 0
            0.0, 1.0, // s1 -> cluster 1
        ];
        let hard_clusters = vec![0, 1];

        let mapping =
            map_speakrs_result(&segments, chunks, speakers, dim, &embeddings, &hard_clusters);

        assert_eq!(mapping.turns.len(), 1);
        assert_eq!(mapping.turns[0].provider_cluster_id, provider_cluster_id(1));
        assert!(mapping.clusters.iter().any(|c| c.global_id == 1));
    }

    #[test]
    fn empty_input_yields_empty_mapping() {
        let mapping = map_speakrs_result(&[], 0, 0, 0, &[], &[]);
        assert!(mapping.turns.is_empty());
        assert!(mapping.clusters.is_empty());
    }

    #[test]
    fn provider_cluster_id_matches_shared_format() {
        assert_eq!(provider_cluster_id(0), "speaker_00");
        assert_eq!(provider_cluster_id(3), "speaker_03");
        assert_eq!(provider_cluster_id(12), "speaker_12");
    }
}
