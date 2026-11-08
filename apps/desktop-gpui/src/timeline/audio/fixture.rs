//! A [`Backend`] over the P0 parity map (`apps/desktop/parity/fixtures/timeline/
//! <state>.json`, command → JSON): goldens and tests see what the Svelte stub
//! serves. Every write is recorded and answered `Ok` (or the set error), and the
//! map's turns are not changed by it.

use std::sync::Mutex;

use serde::de::DeserializeOwned;
use serde_json::Value;

use super::data::{Backend, Res, Write};
use super::model::{self, Cluster, Job, ProcResult, Profile, Segment, Turn};

#[derive(Default)]
pub struct FixtureAudio {
    map: Value,
    pub writes: Mutex<Vec<Write>>,
    /// Loads (`list_processing_jobs` calls), for the poll test.
    pub loads: Mutex<u32>,
    /// Every write fails with this.
    pub write_error: Mutex<Option<String>>,
}

impl FixtureAudio {
    pub fn from_map(map: Value) -> Self {
        Self {
            map,
            ..Default::default()
        }
    }

    /// `apps/desktop/parity/fixtures/timeline/<state>.json`.
    pub fn load(state: &str) -> anyhow::Result<Self> {
        Ok(Self::from_map(crate::shell::fixture::load(
            "timeline", state,
        )?))
    }

    /// The map's `get_audio_segment` (the segment a golden opens).
    pub fn opened(&self) -> Option<Segment> {
        model::segment(self.map.get("get_audio_segment")?)
    }

    pub fn writes(&self) -> Vec<Write> {
        self.writes.lock().unwrap().clone()
    }

    fn get<T: DeserializeOwned + Default>(&self, cmd: &str) -> T {
        // A malformed fixture fails loudly, as the timeline's does.
        self.map
            .get(cmd)
            .filter(|v| !v.is_null())
            .map_or_else(T::default, |v| {
                serde_json::from_value(v.clone()).unwrap_or_else(|e| panic!("fixture {cmd}: {e}"))
            })
    }
}

impl Backend for FixtureAudio {
    fn segment(&self, id: i64) -> Res<Option<Segment>> {
        Ok(self.opened().filter(|s| s.id == id))
    }
    fn jobs(&self, _: i64) -> Res<Vec<Job>> {
        *self.loads.lock().unwrap() += 1;
        Ok(self.get("list_processing_jobs"))
    }
    fn result(&self, _: i64) -> Res<Option<ProcResult>> {
        Ok(self.get("get_processing_result"))
    }
    fn turns(&self, _: i64) -> Res<Vec<Turn>> {
        Ok(self.get("list_speaker_turns"))
    }
    fn clusters(&self, _: &str) -> Res<Vec<Cluster>> {
        Ok(self.get("list_speaker_clusters"))
    }
    fn profiles(&self) -> Res<Vec<Profile>> {
        Ok(self.get("list_person_profiles"))
    }
    fn peaks(&self, _: i64, _: usize) -> Vec<f32> {
        self.get("get_audio_segment_waveform_peaks")
    }
    fn write(&self, w: &Write) -> Res<()> {
        self.writes.lock().unwrap().push(w.clone());
        match self.write_error.lock().unwrap().clone() {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}

/// A player for goldens and tests: no sound, position only moves when told to
/// (`seek`, or `advance` from a test).
#[derive(Clone, Default)]
pub struct FakePlayer(pub std::sync::Arc<Mutex<FakeState>>);

#[derive(Debug, Default)]
pub struct FakeState {
    pub position: f64,
    pub duration: f64,
    pub playing: bool,
}

impl FakePlayer {
    pub fn new(duration: f64) -> Self {
        let p = Self::default();
        p.0.lock().unwrap().duration = duration;
        p
    }
    /// The drawer's player factory, handing out clones of this one.
    pub fn factory(&self) -> super::OpenPlayer {
        let me = self.clone();
        std::sync::Arc::new(move |_| {
            Ok(Box::new(me.clone()) as Box<dyn super::transport::Player + Send>)
        })
    }
    /// Playback moved on by `seconds` (stops at the end, like AVAudioPlayer).
    pub fn advance(&self, seconds: f64) {
        let mut s = self.0.lock().unwrap();
        if s.playing {
            s.position = (s.position + seconds).min(s.duration);
            if s.position >= s.duration {
                s.playing = false;
                s.position = 0.;
            }
        }
    }
    pub fn state(&self) -> (f64, bool) {
        let s = self.0.lock().unwrap();
        (s.position, s.playing)
    }
}

impl super::transport::Player for FakePlayer {
    fn play(&mut self) -> bool {
        self.0.lock().unwrap().playing = true;
        true
    }
    fn pause(&mut self) {
        self.0.lock().unwrap().playing = false;
    }
    fn seek(&mut self, seconds: f64) {
        self.0.lock().unwrap().position = seconds;
    }
    fn position(&self) -> f64 {
        self.0.lock().unwrap().position
    }
    fn duration(&self) -> f64 {
        self.0.lock().unwrap().duration
    }
    fn playing(&self) -> bool {
        self.0.lock().unwrap().playing
    }
}
