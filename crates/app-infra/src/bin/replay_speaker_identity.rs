//! Replay cross-segment speaker identity resolution over dumped embeddings.
//!
//! Stage 2 of the segment-identity harness. Stage 1
//! (`scripts/diarization_bench/segment_identity_bench.py`) slices a VoxConverse
//! clip into fixed-length segments, runs the **real** speakrs provider on each,
//! and dumps every cluster centroid with the ground-truth speaker it belongs to.
//! This binary replays those centroids through the **real** shipped resolver
//! ([`resolve_stable_speaker_cluster_from_candidates`]) and reports what the user
//! would actually have seen.
//!
//! Splitting it this way is the point: embedding audio is the expensive step and
//! it does not change when you tune a threshold. Dump once per chunk size, then
//! sweep thresholds, ambiguity rules and centroid strategies for free — against
//! the shipped decision function, not a re-implementation of it.
//!
//! ## Why not DER
//!
//! `run_der.py` scores *within* one audio file: did we separate the voices. The
//! bug this harness exists for is *across* files: did we recognise the voice
//! again. A run can score DER 0% and still mint one "Unknown Speaker" per
//! segment. The metrics below measure the second thing.
//!
//! ## Modes
//!
//! - `single-session` — every segment shares one candidate pool, like one long
//!   recording. Measures the "merge with Unknown Speaker 2, 3, 5…" chain.
//! - `multi-session` — the pool resets every `--sessions-every` segments, and the
//!   dominant speaker is *assigned a name* at the end of the first session. Later
//!   sessions start with zero candidates, so the enrolled voiceprint is their only
//!   route back to that identity. Measures whether naming somebody sticks.
//!
//! ## Usage
//! ```text
//! cargo build -p app-infra --release --bin replay_speaker_identity
//! target/release/replay_speaker_identity --dump dumps/test-0003-60s.json
//! target/release/replay_speaker_identity --dump d.json --mode multi-session \
//!     --centroid reaverage --person-aware-ambiguity --recognition-steers
//! ```

use std::{collections::HashMap, fs, path::PathBuf, process::ExitCode};

use app_infra::processing::speaker_resolution::{
    resolve_stable_speaker_cluster_from_candidates, SpeakerResolutionTuning,
    StableSpeakerClusterCandidate,
};
use serde::{Deserialize, Serialize};

/// Minimum cosine for an enrolled voiceprint to produce a recognition
/// suggestion. Mirrors `MIN_RECOGNITION_SUGGESTION_SCORE` in
/// `speaker-analysis/src/providers/shared.rs` (crate-private there).
const MIN_RECOGNITION_SUGGESTION_SCORE: f32 = 0.60;
/// Top two *people* within this margin suppress the match as ambiguous.
/// Mirrors `PERSON_AMBIGUITY_MARGIN` in the same module.
const PERSON_AMBIGUITY_MARGIN: f32 = 0.05;

// ---------------------------------------------------------------------------
// Dump wire format (written by segment_identity_bench.py)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Dump {
    clip: String,
    chunk_seconds: f64,
    true_speakers: usize,
    segments: Vec<DumpSegment>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DumpSegment {
    index: usize,
    clusters: Vec<DumpCluster>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DumpCluster {
    embedding: Vec<f32>,
    #[serde(default)]
    speech_ms: u64,
    /// Ground-truth speaker this cluster mostly represents, by max time overlap.
    /// `None` when the cluster overlapped no reference speech at all.
    #[serde(default)]
    true_speaker: Option<String>,
}

// ---------------------------------------------------------------------------
// Replay state
// ---------------------------------------------------------------------------

/// A row of `recording_speaker_clusters`, as far as resolution cares.
#[derive(Debug, Clone)]
struct PooledCluster {
    id: i64,
    centroid: Vec<f32>,
    /// Total speech folded into this centroid — the weight for re-averaging.
    weight_ms: u64,
    /// Set once the user assigns a person.
    person_id: Option<i64>,
    /// Ground truth of whatever was folded in, for scoring correctness.
    true_speakers: Vec<String>,
}

impl PooledCluster {
    /// The ground-truth speaker this pooled cluster predominantly represents.
    fn dominant_true_speaker(&self) -> Option<&str> {
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for speaker in &self.true_speakers {
            *counts.entry(speaker.as_str()).or_default() += 1;
        }
        counts
            .into_iter()
            .max_by_key(|(_, count)| *count)
            .map(|(speaker, _)| speaker)
    }
}

/// How an auto-merge updates the surviving cluster's centroid.
#[derive(Debug, Clone, Copy, PartialEq)]
enum CentroidStrategy {
    /// Shipped behavior: the `ON CONFLICT … embedding = excluded.embedding`
    /// upsert in `store.rs` overwrites the survivor with the *newest* segment's
    /// centroid. The anchor does not improve — it drifts.
    Replace,
    /// Speech-duration-weighted mean of everything folded in so far.
    Reaverage,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct Metrics {
    clusters_minted: usize,
    auto_merges: usize,
    /// Auto-merges that fused two *different* ground-truth speakers. The
    /// dangerous failure — a config that wins on click count but scores
    /// non-zero here is a regression, not an improvement.
    wrong_auto_merges: usize,
    /// Merge suggestions raised — literally the clicks the user complains about.
    suggestions: usize,
    /// Suggestions pointing at a cluster that is a *different* real speaker.
    wrong_suggestions: usize,
    // --- stickiness: what happens to the NAMED person in later sessions ------
    //
    // A later session starts with an empty candidate pool, so the enrolled
    // voiceprint is the only thing that can carry the name across. These count
    // the named person's clusters in sessions after the one they were named in.
    //
    /// Recognition matched the right person — the user sees "Maybe <name>" and
    /// needs at most one confirm. This is the flow working as designed.
    stuck_recognized: usize,
    /// Recognition matched a *different* enrolled person. Actively wrong: the
    /// app offers to label this voice as somebody else.
    stuck_recognized_wrong_person: usize,
    /// Recognition returned nothing. The voice shows up as a plain "Unknown
    /// Speaker" and naming it earlier bought the user nothing at all.
    stuck_unrecognized: usize,
    /// Auto-merged into a cluster already carrying the person — zero clicks,
    /// the ideal outcome.
    stuck_auto_linked: usize,
    /// A merge suggestion that points at a cluster belonging to a *different*
    /// real speaker while recognition had already identified this one. The
    /// veto-not-steer defect: resolution refuses the auto-merge, then suggests
    /// the highest-scoring cluster regardless of who it belongs to.
    stuck_suggested_wrong_speaker: usize,
    /// How many distinct people the harness actually managed to name. Reported
    /// because a run that silently enrolled one person cannot observe the
    /// wrong-person failures at all, and would look deceptively clean.
    enrolled_people: usize,
}

fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {
    if left.len() != right.len() || left.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut left_norm = 0.0f32;
    let mut right_norm = 0.0f32;
    for (l, r) in left.iter().zip(right.iter()) {
        dot += l * r;
        left_norm += l * l;
        right_norm += r * r;
    }
    if left_norm <= 0.0 || right_norm <= 0.0 {
        return 0.0;
    }
    dot / (left_norm.sqrt() * right_norm.sqrt())
}

/// Weighted mean of two centroids, then L2-normalized so later cosines stay
/// comparable with freshly emitted embeddings.
fn weighted_mean(left: &[f32], left_weight: u64, right: &[f32], right_weight: u64) -> Vec<f32> {
    let total = (left_weight + right_weight).max(1) as f32;
    let left_share = left_weight as f32 / total;
    let right_share = right_weight as f32 / total;
    let mut merged: Vec<f32> = left
        .iter()
        .zip(right.iter())
        .map(|(l, r)| l * left_share + r * right_share)
        .collect();
    let norm = merged.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm > 0.0 {
        for value in &mut merged {
            *value /= norm;
        }
    }
    merged
}

/// The enrolled voiceprint of a named person.
struct Enrollment {
    person_id: i64,
    embedding: Vec<f32>,
}

/// Cautious recognition, mirroring `best_enrollment_match`: best match at or
/// above threshold, suppressed when the top two people are within the margin.
fn best_enrollment_match(enrollments: &[Enrollment], embedding: &[f32]) -> Option<i64> {
    let mut scored: Vec<(i64, f32)> = enrollments
        .iter()
        .map(|enrollment| {
            (
                enrollment.person_id,
                cosine_similarity(&enrollment.embedding, embedding),
            )
        })
        .filter(|(_, score)| *score >= MIN_RECOGNITION_SUGGESTION_SCORE)
        .collect();
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let (best_person, best_score) = *scored.first()?;
    if scored
        .get(1)
        .is_some_and(|(_, second)| best_score - second < PERSON_AMBIGUITY_MARGIN)
    {
        return None;
    }
    Some(best_person)
}

struct Config {
    dump: PathBuf,
    multi_session: bool,
    sessions_every: usize,
    centroid: CentroidStrategy,
    tuning: SpeakerResolutionTuning,
    /// How many people the harness names at the end of the first session.
    /// Needs >= 2 for recognition to be able to pick the *wrong* person.
    enroll: usize,
    /// Dump to build enrolled voiceprints from, mirroring the user naming every
    /// speaker they see after one pass. When set, the run becomes a pure
    /// recognition check of `--dump` against those voiceprints.
    enroll_from: Option<PathBuf>,
    json_out: Option<PathBuf>,
}

// ---------------------------------------------------------------------------
// Recognition check: enroll from one pass, recognize on another
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct RecognitionReport {
    enrolled_people: usize,
    /// Clusters carrying a ground-truth speaker that was enrolled.
    evaluated_clusters: usize,
    /// Recognition named the right person: the user sees "Maybe <name>".
    recognized: usize,
    /// Recognition named somebody else entirely.
    wrong_person: usize,
    /// Recognition said nothing: a plain "Unknown Speaker".
    unrecognized: usize,
    /// Of the unrecognized, how many were blocked by the score threshold vs by
    /// the person-ambiguity suppression. Separating these matters: the first is
    /// a tuning problem, the second is a logic problem.
    below_threshold: usize,
    suppressed_as_ambiguous: usize,
    /// Raw cosine of each cluster against the BEST voiceprint of the person it
    /// really is — regardless of whether recognition fired. This is the number
    /// that says whether 0.60 is the wrong threshold or the embeddings are.
    true_person_scores: Vec<f32>,
}

/// Build one voiceprint per distinct ground-truth speaker, taking that
/// speaker's longest-speech cluster — the cluster a user would actually click.
fn enroll_from_dump(dump: &Dump) -> (Vec<Enrollment>, HashMap<String, i64>) {
    let mut best_per_speaker: HashMap<&str, (&DumpCluster, u64)> = HashMap::new();
    for segment in &dump.segments {
        for cluster in &segment.clusters {
            let Some(speaker) = cluster.true_speaker.as_deref() else {
                continue;
            };
            let entry = best_per_speaker
                .entry(speaker)
                .or_insert((cluster, cluster.speech_ms));
            if cluster.speech_ms > entry.1 {
                *entry = (cluster, cluster.speech_ms);
            }
        }
    }
    let mut speakers: Vec<&str> = best_per_speaker.keys().copied().collect();
    speakers.sort_unstable();

    let mut enrollments = Vec::new();
    let mut assigned = HashMap::new();
    for (index, speaker) in speakers.iter().enumerate() {
        let person_id = index as i64 + 1;
        enrollments.push(Enrollment {
            person_id,
            embedding: best_per_speaker[speaker].0.embedding.clone(),
        });
        assigned.insert((*speaker).to_string(), person_id);
    }
    (enrollments, assigned)
}

fn recognition_check(
    enroll_dump: &Dump,
    eval_dump: &Dump,
) -> RecognitionReport {
    let (enrollments, assigned) = enroll_from_dump(enroll_dump);
    let mut report = RecognitionReport {
        enrolled_people: enrollments.len(),
        ..Default::default()
    };

    for segment in &eval_dump.segments {
        for cluster in &segment.clusters {
            let Some(expected) = cluster
                .true_speaker
                .as_deref()
                .and_then(|speaker| assigned.get(speaker))
                .copied()
            else {
                continue;
            };
            report.evaluated_clusters += 1;

            // Raw similarity to the person this really is, whatever the policy
            // decides — the diagnostic the pass/fail counts alone would hide.
            if let Some(enrollment) = enrollments
                .iter()
                .find(|enrollment| enrollment.person_id == expected)
            {
                report
                    .true_person_scores
                    .push(cosine_similarity(&enrollment.embedding, &cluster.embedding));
            }

            match best_enrollment_match(&enrollments, &cluster.embedding) {
                Some(person_id) if person_id == expected => report.recognized += 1,
                Some(_) => report.wrong_person += 1,
                None => {
                    report.unrecognized += 1;
                    // Why did it say nothing? Above threshold means the
                    // ambiguity rule suppressed it; below means the score did.
                    let best = enrollments
                        .iter()
                        .map(|e| cosine_similarity(&e.embedding, &cluster.embedding))
                        .fold(f32::NEG_INFINITY, f32::max);
                    if best >= MIN_RECOGNITION_SUGGESTION_SCORE {
                        report.suppressed_as_ambiguous += 1;
                    } else {
                        report.below_threshold += 1;
                    }
                }
            }
        }
    }
    report
}

fn replay(dump: &Dump, config: &Config) -> Metrics {
    let mut metrics = Metrics::default();
    let mut pool: Vec<PooledCluster> = Vec::new();
    let mut enrollments: Vec<Enrollment> = Vec::new();
    let mut next_id: i64 = 1;
    // The people the harness "assigns" at the end of the first session, keyed by
    // the ground-truth speaker they are, so later sessions can be scored for
    // stickiness. Enrolling more than one is what lets the wrong-person paths
    // fire at all: with a single enrollment, recognition can only be right or
    // silent, never confidently wrong.
    let mut assigned: HashMap<String, i64> = HashMap::new();

    for segment in &dump.segments {
        let session_index = if config.multi_session {
            segment.index / config.sessions_every.max(1)
        } else {
            0
        };

        // A new session starts with an empty candidate pool — cross-session
        // identity runs on voiceprints alone (`store.rs` filters by session_id).
        if config.multi_session && session_index > 0 && segment.index % config.sessions_every == 0 {
            if assigned.is_empty() {
                // Name the loudest speakers of the session we just finished,
                // exactly as a user assigning people in the Speakers panel
                // would: biggest talker first, one person per distinct voice.
                let mut by_weight: Vec<usize> = (0..pool.len()).collect();
                by_weight.sort_by_key(|index| std::cmp::Reverse(pool[*index].weight_ms));
                for index in by_weight {
                    if assigned.len() >= config.enroll {
                        break;
                    }
                    let Some(true_speaker) = pool[index].dominant_true_speaker().map(str::to_owned)
                    else {
                        continue;
                    };
                    if assigned.contains_key(&true_speaker) {
                        continue;
                    }
                    let person_id = assigned.len() as i64 + 1;
                    pool[index].person_id = Some(person_id);
                    enrollments.push(Enrollment {
                        person_id,
                        embedding: pool[index].centroid.clone(),
                    });
                    assigned.insert(true_speaker, person_id);
                }
            }
            pool.clear();
        }

        for cluster in &segment.clusters {
            let recognition_person_id = best_enrollment_match(&enrollments, &cluster.embedding);

            let mut candidates: Vec<StableSpeakerClusterCandidate> = pool
                .iter()
                .map(|pooled| StableSpeakerClusterCandidate {
                    id: pooled.id,
                    score: cosine_similarity(&pooled.centroid, &cluster.embedding),
                    person_id: pooled.person_id,
                })
                .filter(|candidate| candidate.score.is_finite())
                .collect();

            let resolution = resolve_stable_speaker_cluster_from_candidates(
                &mut candidates,
                recognition_person_id,
                &config.tuning,
            );

            // Which named person is this cluster really, if any, in a session
            // after the one they were named in?
            let expected_person_id = cluster
                .true_speaker
                .as_deref()
                .and_then(|speaker| assigned.get(speaker))
                .copied();
            let scoring_stickiness =
                config.multi_session && session_index > 0 && expected_person_id.is_some();

            if scoring_stickiness {
                // Did enrolling the voiceprint buy anything at all? This is the
                // question behind "I named someone and it didn't stick".
                match recognition_person_id {
                    id if id == expected_person_id => metrics.stuck_recognized += 1,
                    Some(_) => metrics.stuck_recognized_wrong_person += 1,
                    None => metrics.stuck_unrecognized += 1,
                }
            }

            if let Some(target_id) = resolution.auto_merge_target_cluster_id {
                metrics.auto_merges += 1;
                let target = pool
                    .iter_mut()
                    .find(|pooled| pooled.id == target_id)
                    .expect("auto-merge target is always a pooled cluster");

                let fused_wrong = match (target.dominant_true_speaker(), &cluster.true_speaker) {
                    (Some(existing), Some(incoming)) => existing != incoming.as_str(),
                    _ => false,
                };
                if fused_wrong {
                    metrics.wrong_auto_merges += 1;
                }
                if scoring_stickiness && target.person_id == expected_person_id {
                    metrics.stuck_auto_linked += 1;
                }

                target.centroid = match config.centroid {
                    CentroidStrategy::Replace => cluster.embedding.clone(),
                    CentroidStrategy::Reaverage => weighted_mean(
                        &target.centroid,
                        target.weight_ms,
                        &cluster.embedding,
                        cluster.speech_ms,
                    ),
                };
                target.weight_ms += cluster.speech_ms;
                if let Some(speaker) = &cluster.true_speaker {
                    target.true_speakers.push(speaker.clone());
                }
                continue;
            }

            // Not auto-merged: a new "Unknown Speaker N" row is minted, with or
            // without a suggestion attached.
            if let Some(target_id) = resolution.suggested_merge_target_cluster_id {
                metrics.suggestions += 1;
                let target = pool
                    .iter()
                    .find(|pooled| pooled.id == target_id)
                    .expect("suggestion target is always a pooled cluster");
                let points_elsewhere =
                    match (target.dominant_true_speaker(), &cluster.true_speaker) {
                        (Some(existing), Some(incoming)) => existing != incoming.as_str(),
                        _ => false,
                    };
                if points_elsewhere {
                    metrics.wrong_suggestions += 1;
                    if scoring_stickiness {
                        // Recognition already knew who this was, and resolution
                        // still pointed the user at a different speaker.
                        metrics.stuck_suggested_wrong_speaker += 1;
                    }
                }
            }

            metrics.clusters_minted += 1;
            pool.push(PooledCluster {
                id: next_id,
                centroid: cluster.embedding.clone(),
                weight_ms: cluster.speech_ms,
                person_id: None,
                true_speakers: cluster.true_speaker.iter().cloned().collect(),
            });
            next_id += 1;
        }
    }

    metrics.enrolled_people = assigned.len();
    metrics
}

const USAGE: &str = "replay_speaker_identity — replay cross-segment identity resolution over dumped embeddings

USAGE:
  replay_speaker_identity --dump <path> [options]

OPTIONS:
  --dump <path>               Cluster dump from segment_identity_bench.py (required).
  --mode <m>                  single-session (default) | multi-session.
  --sessions-every <n>        Segments per session in multi-session mode (default 10).
  --enroll <n>                People named after the first session (default 2). Two or
                              more is required for the wrong-person paths to fire.
  --enroll-from <path>        Build voiceprints from this dump (one per speaker, their
                              longest cluster) and report whether --dump's clusters are
                              recognized. Mirrors: name every speaker, then re-run.
  --centroid <s>              replace (default, shipped) | reaverage (F4).
  --auto-reuse <f>            Auto-reuse threshold (default 0.82).
  --suggest <f>               Suggest-merge threshold (default 0.68).
  --ambiguity-margin <f>      Ambiguity margin (default 0.06).
  --person-aware-ambiguity    Only block auto-reuse on genuine identity conflicts (F3).
  --recognition-steers        Let a recognition match pick the right cluster, not just veto.
  --json-out <path>           Write metrics as JSON.
  -h, --help                  Print this help.";

fn parse_config() -> Result<Config, String> {
    let mut dump: Option<PathBuf> = None;
    let mut multi_session = false;
    let mut sessions_every = 10usize;
    let mut centroid = CentroidStrategy::Replace;
    let mut tuning = SpeakerResolutionTuning::default();
    let mut enroll = 2usize;
    let mut enroll_from: Option<PathBuf> = None;
    let mut json_out: Option<PathBuf> = None;

    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let mut value = || {
            args.next()
                .ok_or_else(|| format!("missing value for {flag}"))
        };
        match flag.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            "--dump" => dump = Some(PathBuf::from(value()?)),
            "--mode" => {
                multi_session = match value()?.as_str() {
                    "single-session" => false,
                    "multi-session" => true,
                    other => return Err(format!("unknown --mode {other}")),
                }
            }
            "--sessions-every" => {
                sessions_every = value()?.parse().map_err(|_| "bad --sessions-every")?
            }
            "--enroll" => enroll = value()?.parse().map_err(|_| "bad --enroll")?,
            "--enroll-from" => enroll_from = Some(PathBuf::from(value()?)),
            "--centroid" => {
                centroid = match value()?.as_str() {
                    "replace" => CentroidStrategy::Replace,
                    "reaverage" => CentroidStrategy::Reaverage,
                    other => return Err(format!("unknown --centroid {other}")),
                }
            }
            "--auto-reuse" => {
                tuning.auto_reuse_threshold = value()?.parse().map_err(|_| "bad --auto-reuse")?
            }
            "--suggest" => {
                tuning.suggest_merge_threshold = value()?.parse().map_err(|_| "bad --suggest")?
            }
            "--ambiguity-margin" => {
                tuning.ambiguity_margin = value()?.parse().map_err(|_| "bad --ambiguity-margin")?
            }
            "--person-aware-ambiguity" => tuning.person_aware_ambiguity = true,
            "--recognition-steers" => tuning.recognition_steers = true,
            "--json-out" => json_out = Some(PathBuf::from(value()?)),
            other => return Err(format!("unknown argument: {other}")),
        }
    }

    Ok(Config {
        dump: dump.ok_or("--dump is required")?,
        multi_session,
        sessions_every,
        centroid,
        tuning,
        enroll,
        enroll_from,
        json_out,
    })
}

fn run(config: &Config) -> Result<(), String> {
    let raw = fs::read_to_string(&config.dump)
        .map_err(|e| format!("failed to read {}: {e}", config.dump.display()))?;
    let dump: Dump = serde_json::from_str(&raw)
        .map_err(|e| format!("failed to parse {}: {e}", config.dump.display()))?;

    // Recognition check: enroll from one pass, evaluate another.
    if let Some(enroll_path) = &config.enroll_from {
        let raw = fs::read_to_string(enroll_path)
            .map_err(|e| format!("failed to read {}: {e}", enroll_path.display()))?;
        let enroll_dump: Dump = serde_json::from_str(&raw)
            .map_err(|e| format!("failed to parse {}: {e}", enroll_path.display()))?;
        let report = recognition_check(&enroll_dump, &dump);

        let mut scores = report.true_person_scores.clone();
        scores.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let percentile = |p: f32| -> f32 {
            if scores.is_empty() {
                return f32::NAN;
            }
            scores[((scores.len() - 1) as f32 * p).round() as usize]
        };

        println!("enrolled from       {}", enroll_dump.clip);
        println!("evaluated           {}", dump.clip);
        println!("chunk seconds       {}", dump.chunk_seconds);
        println!("enrolled people     {}", report.enrolled_people);
        println!("---");
        println!("clusters evaluated  {}", report.evaluated_clusters);
        println!(
            "RECOGNIZED          {} ({:.0}%)",
            report.recognized,
            100.0 * report.recognized as f64 / report.evaluated_clusters.max(1) as f64
        );
        println!("wrong person        {}", report.wrong_person);
        println!("not recognized      {}", report.unrecognized);
        println!("  below 0.60        {}", report.below_threshold);
        println!("  ambiguity-blocked {}", report.suppressed_as_ambiguous);
        println!("--- cosine to the person it really is ---");
        println!(
            "min {:.3}  p25 {:.3}  median {:.3}  p75 {:.3}  max {:.3}",
            percentile(0.0),
            percentile(0.25),
            percentile(0.5),
            percentile(0.75),
            percentile(1.0),
        );

        if let Some(path) = &config.json_out {
            let payload = serde_json::json!({
                "enrolledFrom": enroll_dump.clip,
                "evaluated": dump.clip,
                "chunkSeconds": dump.chunk_seconds,
                "report": report,
            });
            fs::write(path, serde_json::to_string_pretty(&payload).map_err(|e| e.to_string())?)
                .map_err(|e| format!("failed to write {}: {e}", path.display()))?;
        }
        return Ok(());
    }

    let metrics = replay(&dump, config);
    let segments = dump.segments.len();

    println!("clip                {}", dump.clip);
    println!("chunk seconds       {}", dump.chunk_seconds);
    println!("segments            {segments}");
    println!("true speakers       {}", dump.true_speakers);
    println!(
        "mode                {}",
        if config.multi_session {
            format!("multi-session (every {} segments)", config.sessions_every)
        } else {
            "single-session".to_string()
        }
    );
    println!(
        "centroid            {}",
        match config.centroid {
            CentroidStrategy::Replace => "replace (shipped)",
            CentroidStrategy::Reaverage => "reaverage",
        }
    );
    println!("---");
    println!("clusters minted     {}", metrics.clusters_minted);
    println!(
        "  over-clustering   {:.1}x",
        metrics.clusters_minted as f64 / dump.true_speakers.max(1) as f64
    );
    println!("auto-merges         {}", metrics.auto_merges);
    println!("  WRONG             {}", metrics.wrong_auto_merges);
    println!("suggestions (clicks){}", format_args!(" {}", metrics.suggestions));
    println!("  pointing elsewhere{}", format_args!(" {}", metrics.wrong_suggestions));
    if config.multi_session {
        println!("enrolled people     {}", metrics.enrolled_people);
        let named_clusters = metrics.stuck_recognized
            + metrics.stuck_recognized_wrong_person
            + metrics.stuck_unrecognized;
        println!("--- did naming stick? ({named_clusters} later clusters of that person) ---");
        println!(
            "recognized          {} ({:.0}%)",
            metrics.stuck_recognized,
            100.0 * metrics.stuck_recognized as f64 / named_clusters.max(1) as f64
        );
        println!(
            "recognized as SOMEONE ELSE {}",
            metrics.stuck_recognized_wrong_person
        );
        println!("not recognized      {}", metrics.stuck_unrecognized);
        println!("auto-linked (0 clicks) {}", metrics.stuck_auto_linked);
        println!(
            "suggested WRONG speaker {}",
            metrics.stuck_suggested_wrong_speaker
        );
    }

    if let Some(path) = &config.json_out {
        let payload = serde_json::json!({
            "clip": dump.clip,
            "chunkSeconds": dump.chunk_seconds,
            "segments": segments,
            "trueSpeakers": dump.true_speakers,
            "mode": if config.multi_session { "multi-session" } else { "single-session" },
            "centroid": match config.centroid {
                CentroidStrategy::Replace => "replace",
                CentroidStrategy::Reaverage => "reaverage",
            },
            "tuning": config.tuning,
            "metrics": metrics,
        });
        fs::write(
            path,
            serde_json::to_string_pretty(&payload).map_err(|e| e.to_string())?,
        )
        .map_err(|e| format!("failed to write {}: {e}", path.display()))?;
    }

    Ok(())
}

fn main() -> ExitCode {
    let config = match parse_config() {
        Ok(config) => config,
        Err(message) => {
            eprintln!("error: {message}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match run(&config) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Three segments of one voice, each embedding near-identical.
    fn dump_of(embeddings: Vec<(Vec<f32>, &str)>) -> Dump {
        Dump {
            clip: "synthetic".to_string(),
            chunk_seconds: 60.0,
            true_speakers: 1,
            segments: embeddings
                .into_iter()
                .enumerate()
                .map(|(index, (embedding, speaker))| DumpSegment {
                    index,
                    clusters: vec![DumpCluster {
                        embedding,
                        speech_ms: 30_000,
                        true_speaker: Some(speaker.to_string()),
                    }],
                })
                .collect(),
        }
    }

    fn config(tuning: SpeakerResolutionTuning) -> Config {
        Config {
            dump: PathBuf::new(),
            multi_session: false,
            sessions_every: 10,
            centroid: CentroidStrategy::Replace,
            tuning,
            enroll: 2,
            enroll_from: None,
            json_out: None,
        }
    }

    #[test]
    fn identical_voice_auto_reuses_and_mints_once() {
        let dump = dump_of(vec![
            (vec![1.0, 0.0, 0.0], "spk0"),
            (vec![1.0, 0.0, 0.0], "spk0"),
            (vec![1.0, 0.0, 0.0], "spk0"),
        ]);

        let metrics = replay(&dump, &config(SpeakerResolutionTuning::default()));

        assert_eq!(metrics.clusters_minted, 1);
        assert_eq!(metrics.auto_merges, 2);
        assert_eq!(metrics.wrong_auto_merges, 0);
        assert_eq!(metrics.suggestions, 0);
    }

    /// Three vectors that are *mutually* 0.75 cosine apart: a shared component
    /// plus one orthogonal component each, sized so a²/(a²+b²) = 0.75. This is
    /// the reported chain — one voice whose fragments all land in the
    /// 0.68..0.82 grey zone against every other fragment, not just the newest.
    fn mutually_grey_zone() -> Vec<Vec<f32>> {
        let b = (1.0f32 / 3.0).sqrt();
        vec![
            vec![1.0, b, 0.0, 0.0],
            vec![1.0, 0.0, b, 0.0],
            vec![1.0, 0.0, 0.0, b],
        ]
    }

    #[test]
    fn grey_zone_voice_mints_a_speaker_and_a_click_per_segment() {
        let dump = dump_of(
            mutually_grey_zone()
                .into_iter()
                .map(|embedding| (embedding, "spk0"))
                .collect(),
        );

        let metrics = replay(&dump, &config(SpeakerResolutionTuning::default()));

        assert_eq!(metrics.clusters_minted, 3, "one speaker per segment");
        assert_eq!(metrics.auto_merges, 0);
        assert_eq!(metrics.suggestions, 2, "a click per extra fragment");
    }

    #[test]
    fn grey_zone_chain_needs_the_threshold_not_just_f3_and_f4() {
        // F3 (person-aware ambiguity) and F4 (re-averaging) only act once
        // candidates already reach the auto-reuse threshold or tie near it.
        // A voice sitting flat at 0.75 is untouched by either — only lowering
        // the threshold collapses it. Pins the ordering claim in the plan:
        // re-measure before tuning F6, but do not expect F3/F4 to do this alone.
        let dump = dump_of(
            mutually_grey_zone()
                .into_iter()
                .map(|embedding| (embedding, "spk0"))
                .collect(),
        );

        let mut with_f3_f4 = config(SpeakerResolutionTuning {
            person_aware_ambiguity: true,
            ..Default::default()
        });
        with_f3_f4.centroid = CentroidStrategy::Reaverage;
        assert_eq!(replay(&dump, &with_f3_f4).clusters_minted, 3);

        let mut with_f6 = config(SpeakerResolutionTuning {
            person_aware_ambiguity: true,
            auto_reuse_threshold: 0.70,
            ..Default::default()
        });
        with_f6.centroid = CentroidStrategy::Reaverage;
        let metrics = replay(&dump, &with_f6);
        assert_eq!(metrics.clusters_minted, 1, "one voice, one speaker");
        assert_eq!(metrics.wrong_auto_merges, 0);
    }

    #[test]
    fn different_speakers_are_never_auto_fused() {
        let dump = dump_of(vec![
            (vec![1.0, 0.0, 0.0], "spk0"),
            (vec![0.0, 1.0, 0.0], "spk1"),
        ]);

        let metrics = replay(&dump, &config(SpeakerResolutionTuning::default()));

        assert_eq!(metrics.clusters_minted, 2);
        assert_eq!(metrics.wrong_auto_merges, 0);
    }

    #[test]
    fn reaverage_pulls_the_anchor_toward_the_shared_voice() {
        // Two fragments either side of a centre. Replacing leaves the anchor at
        // the newest fragment; re-averaging moves it to the middle, which is
        // closer to both — the whole premise of F4.
        let first = vec![1.0f32, 0.0, 0.0];
        let second = vec![0.8f32, 0.6, 0.0];
        let replaced = second.clone();
        let averaged = weighted_mean(&first, 30_000, &second, 30_000);

        let probe = vec![0.9f32, 0.44, 0.0];
        assert!(
            cosine_similarity(&averaged, &probe) > cosine_similarity(&replaced, &probe),
            "re-averaged centroid should sit closer to a mid-voice probe",
        );
    }
}
