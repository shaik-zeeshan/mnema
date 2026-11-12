//! Where the drawer's data comes from, and the transcript load
//! (`loadSelectedAudioSegmentTranscript` and the speaker pass after it) as one
//! sync function over the backend. The view runs it on the background executor.

use std::sync::Arc;

use app_core::app_infra as ai;
use app_core::{AppCore, AppInfraState};
use serde::de::DeserializeOwned;
use serde::Serialize;

use super::model::{
    self, Cluster, Job, ModelLabel, ProcResult, Profile, Provenance, Run, Segment, Src, Turn,
};
use super::view_model::TranscriptStatus;

pub type Res<T> = Result<T, String>;

/// A speaker write or a reprocess request (the fixture records these).
#[derive(Clone, Debug, PartialEq)]
pub enum Write {
    /// `create_person_profile` then `link_speaker_cluster_to_person`.
    NameAndLink {
        cluster: i64,
        name: String,
    },
    Link {
        cluster: i64,
        person: i64,
    },
    Confirm {
        cluster: i64,
    },
    Reject {
        cluster: i64,
    },
    Unlink {
        cluster: i64,
    },
    Merge {
        source: i64,
        target: i64,
    },
    /// `move_speaker_turn_to_cluster` per turn, in order.
    MoveTurns {
        turns: Vec<i64>,
        target: i64,
    },
    /// `reprocess_audio_segment_transcription` (or speech activity for system audio).
    Rerun {
        segment: i64,
        src: Src,
    },
    RetrySpeakers {
        segment: i64,
    },
}

/// The drawer's commands, synchronous (called off the main thread).
pub trait Backend: Send + Sync + 'static {
    fn segment(&self, _id: i64) -> Res<Option<Segment>> {
        Ok(None)
    }
    fn jobs(&self, _segment: i64) -> Res<Vec<Job>> {
        Ok(Vec::new())
    }
    fn result(&self, _job: i64) -> Res<Option<ProcResult>> {
        Ok(None)
    }
    fn turns(&self, _segment: i64) -> Res<Vec<Turn>> {
        Ok(Vec::new())
    }
    fn clusters(&self, _session: &str) -> Res<Vec<Cluster>> {
        Ok(Vec::new())
    }
    fn profiles(&self) -> Res<Vec<Profile>> {
        Ok(Vec::new())
    }
    /// `get_audio_segment_waveform_peaks`: empty on any failure (the plain bar).
    fn peaks(&self, _segment: i64, _buckets: usize) -> Vec<f32> {
        Vec::new()
    }
    fn write(&self, _w: &Write) -> Res<()> {
        Err("no app core".into())
    }
}

/// No `AppCore` (a window without one).
pub struct NoBackend;
impl Backend for NoBackend {}

/// Everything the drawer shows about one segment's transcript and speakers.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Transcript {
    pub status: TranscriptStatus,
    pub text: Option<String>,
    pub runs: Vec<Run>,
    pub words: Vec<Run>,
    pub model: Option<ModelLabel>,
    pub error: Option<String>,
    pub turns: Vec<Turn>,
    pub clusters: Vec<Cluster>,
    pub profiles: Vec<Profile>,
    pub speakers_running: bool,
    pub speakers_failed: bool,
    pub speakers_error: Option<String>,
    pub speakers_notice: Option<String>,
    pub provenance: Option<Provenance>,
    /// A queued/running job: the view loads again after `POLL`.
    pub pending: Option<Job>,
}

fn latest<'a>(jobs: &'a [Job], processor: &str) -> Option<&'a Job> {
    jobs.iter()
        .filter(|j| j.processor == processor)
        .max_by_key(|j| j.id)
}

fn stopped(status: TranscriptStatus, error: Option<String>) -> Transcript {
    Transcript {
        status,
        error,
        ..Default::default()
    }
}

/// The whole load, as the Svelte page runs it.
pub fn load(b: &dyn Backend, seg: &Segment) -> Transcript {
    let jobs = match b.jobs(seg.id) {
        Ok(j) => j,
        Err(e) => return stopped(TranscriptStatus::Error, Some(e)),
    };
    if seg.source == Src::Sys {
        let Some(job) = latest(&jobs, model::SPEECH_ACTIVITY) else {
            return stopped(TranscriptStatus::Missing, None);
        };
        if job.pending() {
            return Transcript {
                pending: Some(job.clone()),
                ..stopped(TranscriptStatus::Running, None)
            };
        }
        if job.status == "failed" {
            let e = job
                .last_error
                .clone()
                .unwrap_or("Speech detection failed".into());
            return stopped(TranscriptStatus::Error, Some(e));
        }
        match b.result(job.id) {
            Ok(r) if model::speech_detected(r.as_ref()) == Some(false) => {
                return stopped(TranscriptStatus::Empty, None)
            }
            Ok(_) => {}
            Err(e) => return stopped(TranscriptStatus::Error, Some(e)),
        }
    }
    let transcription: Vec<&Job> = jobs
        .iter()
        .filter(|j| j.processor == model::TRANSCRIPTION)
        .collect();
    if transcription.is_empty() {
        let mut t = stopped(TranscriptStatus::Missing, None);
        speakers(b, seg, &jobs, &mut t);
        return t;
    }
    let job = transcription
        .iter()
        .filter(|j| j.status == "completed")
        .max_by_key(|j| j.id)
        .or_else(|| transcription.iter().max_by_key(|j| j.id))
        .copied()
        .expect("non-empty");
    let model = model::model_label(job.payload_json.as_deref(), None);
    if job.pending() {
        return Transcript {
            model,
            pending: Some(job.clone()),
            ..stopped(TranscriptStatus::Running, None)
        };
    }
    if job.status == "failed" {
        let e = job
            .last_error
            .clone()
            .unwrap_or("Transcription job failed".into());
        return Transcript {
            model,
            ..stopped(TranscriptStatus::Error, Some(e))
        };
    }
    let result = match b.result(job.id) {
        Ok(Some(r)) => r,
        Ok(None) => {
            let e = "Transcription result not available".to_string();
            return stopped(TranscriptStatus::Error, Some(e));
        }
        Err(e) => return stopped(TranscriptStatus::Error, Some(e)),
    };
    let (runs, words) = model::transcript(&result);
    let text = match result.result_text.as_deref().map(str::trim) {
        Some(t) if !t.is_empty() => t.to_string(),
        _ => runs
            .iter()
            .map(|r| r.text.as_str())
            .collect::<Vec<_>>()
            .join(" "),
    };
    let mut t = Transcript {
        status: if text.trim().is_empty() {
            TranscriptStatus::Empty
        } else {
            TranscriptStatus::Success
        },
        text: Some(text),
        runs,
        words,
        model: model::model_label(
            job.payload_json.as_deref(),
            result.structured_payload_json.as_deref(),
        ),
        ..Default::default()
    };
    speakers(b, seg, &jobs, &mut t);
    t
}

/// `waitForSelectedAudioSpeakerAnalysisIfNeeded` + `loadSelectedAudioSpeakerTurns`.
fn speakers(b: &dyn Backend, seg: &Segment, jobs: &[Job], t: &mut Transcript) {
    let job = latest(jobs, model::SPEAKER_ANALYSIS);
    if let Some(job) = job {
        if job.pending() {
            t.speakers_running = true;
            t.pending = Some(job.clone());
            return;
        }
        if job.status == "failed" {
            t.speakers_failed = true;
            t.speakers_error = Some(
                job.last_error
                    .clone()
                    .unwrap_or("Speaker analysis failed".into()),
            );
            return;
        }
    }
    if let Err(e) = turns_into(b, seg.id, t) {
        t.turns.clear();
        t.clusters.clear();
        t.speakers_error = Some(e);
        return;
    }
    if let (true, Some(job)) = (t.turns.is_empty(), job) {
        let result = b.result(job.id).ok().flatten();
        t.provenance = model::provenance(result.as_ref());
        if t.status == TranscriptStatus::Success {
            t.speakers_notice = Some(
                match t.provenance.as_ref().and_then(|p| p.skip_reason.as_deref()) {
                    Some("too_short") => "Speaker analysis skipped: audio segment is too short.",
                    Some("silent") => "Speaker analysis skipped: no speech-level audio detected.",
                    _ => "No speaker turns found.",
                }
                .into(),
            );
        }
    }
}

/// Turns, profiles and the session's clusters (also the refresh after a write).
pub fn turns_into(b: &dyn Backend, segment: i64, t: &mut Transcript) -> Res<()> {
    t.turns = b.turns(segment)?;
    t.profiles = b.profiles()?;
    t.clusters = match t.turns.first() {
        Some(turn) => b.clusters(&turn.session_id)?,
        None => Vec::new(),
    };
    Ok(())
}

/// The real app.
pub struct CoreBackend(pub Arc<AppCore>);

impl CoreBackend {
    fn infra(&self) -> Res<Arc<AppInfraState>> {
        self.0
            .try_state::<AppInfraState>()
            .ok_or_else(|| "app infrastructure isn't ready".into())
    }
}

/// A DTO through its camelCase JSON into the view model (one shape for both doors).
fn view<D: Serialize, T: DeserializeOwned>(d: D) -> Res<T> {
    serde_json::to_value(d)
        .and_then(serde_json::from_value)
        .map_err(|e| e.to_string())
}

impl Backend for CoreBackend {
    fn segment(&self, id: i64) -> Res<Option<Segment>> {
        let i = self.infra()?;
        let req = ai::GetAudioSegmentRequest {
            audio_segment_id: id,
        };
        let dto = self
            .0
            .run(async move { ai::get_audio_segment(req, &i).await })?;
        Ok(dto
            .and_then(|d| serde_json::to_value(d).ok())
            .and_then(|v| model::segment(&v)))
    }
    fn jobs(&self, segment: i64) -> Res<Vec<Job>> {
        let i = self.infra()?;
        let req = ai::ListProcessingJobsRequest {
            subject_type: model::SUBJECT_TYPE.into(),
            subject_id: segment,
        };
        view(
            self.0
                .run(async move { ai::list_processing_jobs(req, &i).await })?,
        )
    }
    fn result(&self, job: i64) -> Res<Option<ProcResult>> {
        let i = self.infra()?;
        let req = ai::GetProcessingResultRequest { job_id: job };
        view(
            self.0
                .run(async move { ai::get_processing_result(req, &i).await })?,
        )
    }
    fn turns(&self, segment: i64) -> Res<Vec<Turn>> {
        let i = self.infra()?;
        let req = ai::ListSpeakerTurnsRequest {
            audio_segment_id: segment,
        };
        view(
            self.0
                .run(async move { ai::list_speaker_turns(req, &i).await })?,
        )
    }
    fn clusters(&self, session: &str) -> Res<Vec<Cluster>> {
        let i = self.infra()?;
        let req = ai::ListSpeakerClustersRequest {
            session_id: session.into(),
        };
        view(
            self.0
                .run(async move { ai::list_speaker_clusters(req, &i).await })?,
        )
    }
    fn profiles(&self) -> Res<Vec<Profile>> {
        let i = self.infra()?;
        view(
            self.0
                .run(async move { ai::list_person_profiles(&i).await })?,
        )
    }
    fn peaks(&self, segment: i64, buckets: usize) -> Vec<f32> {
        let Ok(i) = self.infra() else {
            return Vec::new();
        };
        let req = ai::GetAudioSegmentWaveformPeaksRequest {
            audio_segment_id: segment,
            bucket_count: buckets as u32,
        };
        self.0
            .run(async move { ai::get_audio_segment_waveform_peaks(req, &i).await })
            .unwrap_or_default()
    }
    fn write(&self, w: &Write) -> Res<()> {
        let (i, core) = (self.infra()?, &self.0);
        let link = |cluster_id, person_id| {
            let req = ai::LinkSpeakerClusterRequest {
                cluster_id,
                person_id,
                add_embedding: true,
            };
            let i = i.clone();
            core.run(async move { ai::link_speaker_cluster_to_person(req, &i).await })
                .map(drop)
        };
        let cluster = |cluster_id| ai::SpeakerClusterRequest { cluster_id };
        match w {
            Write::NameAndLink { cluster, name } => {
                let req = ai::CreatePersonProfileRequest {
                    display_name: name.trim().into(),
                    notes: None,
                };
                let i = i.clone();
                let person = core.run(async move { ai::create_person_profile(req, &i).await })?;
                link(*cluster, person.id)
            }
            Write::Link { cluster, person } => link(*cluster, *person),
            Write::Confirm { cluster } => {
                let req = ai::ConfirmSpeakerSuggestionRequest {
                    cluster_id: *cluster,
                    add_embedding: true,
                };
                core.run(async move { ai::confirm_speaker_recognition_suggestion(req, &i).await })
                    .map(drop)
            }
            Write::Reject { cluster: c } => {
                let req = cluster(*c);
                core.run(async move { ai::reject_speaker_recognition_suggestion(req, &i).await })
                    .map(drop)
            }
            Write::Unlink { cluster: c } => {
                let req = cluster(*c);
                core.run(async move { ai::unlink_speaker_cluster_from_person(req, &i).await })
                    .map(drop)
            }
            Write::Merge { source, target } => {
                let req = ai::MergeSpeakerClustersRequest {
                    source_cluster_id: *source,
                    target_cluster_id: *target,
                };
                core.run(async move { ai::merge_speaker_clusters(req, &i).await })
                    .map(drop)
            }
            Write::MoveTurns { turns, target } => turns.iter().try_for_each(|turn| {
                let req = ai::MoveSpeakerTurnRequest {
                    turn_id: *turn,
                    target_cluster_id: *target,
                };
                let i = i.clone();
                core.run(async move { ai::move_speaker_turn_to_cluster(req, &i).await })
                    .map(drop)
            }),
            Write::Rerun { segment, src } => {
                let req = ai::ReprocessAudioSegmentTranscriptionRequest {
                    audio_segment_id: *segment,
                };
                let c = core.clone();
                match src {
                    Src::Sys => core
                        .run(async move {
                            ai::reprocess_system_audio_speech_activity(req, &i, &c).await
                        })
                        .map(drop),
                    Src::Mic => {
                        core.run(async move {
                            ai::reprocess_audio_segment_transcription(req, &i, &c).await
                        })
                        .map(drop)
                    }
                }
            }
            Write::RetrySpeakers { segment } => {
                let req = ai::ReprocessAudioSegmentTranscriptionRequest {
                    audio_segment_id: *segment,
                };
                let c = core.clone();
                core.run(
                    async move { ai::reprocess_audio_segment_speaker_analysis(req, &i, &c).await },
                )
                .map(drop)
            }
        }
    }
}
