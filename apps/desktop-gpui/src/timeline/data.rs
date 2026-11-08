//! Where the Timeline's data comes from. Views never call the backend: the view
//! runs these on the background executor and hands the answers to `paging` /
//! `scrub_engine`. `CoreSource` asks `AppCore`; `fixture::FixtureSource` serves
//! the parity JSON (goldens, tests, the scrub bench).

use std::path::PathBuf;
use app_infra::MatchBox;
use capture_types::Activity;

use super::model::{AudioSegment, FocusedWindow, Frame, Landing, OcrBox, Preview, ScrubInterval};

pub type Res<T> = Result<T, String>;

/// The timeline commands, synchronous (the view calls them off the main thread).
pub trait Source: Send + Sync + 'static {
    /// `list_frames { limit, beforeId }`, newest first.
    fn list_frames(&self, before_id: Option<i64>, limit: usize) -> Res<Vec<Frame>>;
    /// `get_timeline_window_around_frame`.
    fn window_around(&self, frame_id: i64, newer: usize, older: usize) -> Res<FocusedWindow>;
    /// `get_latest_frame_in_range` (the jumper's "latest at or before").
    fn latest_in_range(&self, start_ms: i64, end_ms: i64) -> Res<Option<Frame>>;
    /// `list_frame_summaries` capture instants in a range (the jumper's density).
    fn frame_times(&self, start_ms: i64, end_ms: i64) -> Res<Vec<i64>>;
    /// `get_frame_preview { videoScope: active_frame }`.
    fn frame_preview(&self, frame_id: i64) -> Res<Option<Preview>>;
    /// `get_scrub_preview_availability`.
    fn scrub_availability(
        &self,
        start_ms: i64,
        end_ms: i64,
        enqueue: bool,
    ) -> Res<Vec<ScrubInterval>>;
    /// `cancel_active_frame_preview_video_requests`.
    fn cancel_video(&self) {}
    /// `list_audio_segments` over the loaded window (padded).
    fn audio_segments(&self, start_ms: i64, end_ms: i64) -> Res<Vec<AudioSegment>>;
    /// `list_user_context_activities` (band colours only).
    fn episodes(&self, start_ms: i64, end_ms: i64) -> Res<Vec<Activity>>;
    /// `resolve_app_icons`: bundle id → icon PNG on disk.
    fn app_icons(&self, bundle_ids: &[String]) -> Vec<(String, PathBuf)>;
    /// `get_frame_match_boxes`.
    fn match_boxes(&self, frame_id: i64, query: &str, by_meaning: bool) -> Res<Vec<MatchBox>>;
    /// The active frame's OCR (`loadOcrForFrame`): `None` = never OCR'd.
    fn ocr(&self, frame_id: i64) -> Res<Option<Vec<OcrBox>>>;
    /// `drain_pending_broker_open_capture_results` (a Quick Recall hit).
    fn take_landing(&self) -> Option<Landing>;
}

/// The real app: AppCore.
/// shortcut: holds no `AppCore` yet; B6 adds the handle with the first wired command.
#[derive(Default)]
pub struct CoreSource;

// shortcut: every timeline command (`list_frames`, `get_timeline_window_around_frame`,
// `get_latest_frame_in_range`, `list_frame_summaries`, `get_frame_preview`,
// `get_scrub_preview_availability`, `cancel_active_frame_preview_video_requests`,
// `list_audio_segments`, `resolve_app_icons`, `get_frame_match_boxes`, the OCR
// result reads, `drain_pending_broker_open_capture_results`) lives in src-tauri
// `app_infra.rs` / `frame_preview.rs` / `quick_recall.rs` / `native_capture.rs`
// until S2 B6 moves them (and `list_user_context_activities` until S3). Each one
// answers empty here; wire them to `self.core` (via `core.block_on`, we are off
// the main thread) once B6 lands. The real app shows "Nothing recorded yet".
impl Source for CoreSource {
    fn list_frames(&self, _: Option<i64>, _: usize) -> Res<Vec<Frame>> {
        Ok(Vec::new())
    }
    fn window_around(&self, _: i64, _: usize, _: usize) -> Res<FocusedWindow> {
        Err("timeline jumps arrive with S2 B6".into())
    }
    fn latest_in_range(&self, _: i64, _: i64) -> Res<Option<Frame>> {
        Ok(None)
    }
    fn frame_times(&self, _: i64, _: i64) -> Res<Vec<i64>> {
        Ok(Vec::new())
    }
    fn frame_preview(&self, _: i64) -> Res<Option<Preview>> {
        Ok(None)
    }
    fn scrub_availability(&self, _: i64, _: i64, _: bool) -> Res<Vec<ScrubInterval>> {
        Ok(Vec::new())
    }
    fn audio_segments(&self, _: i64, _: i64) -> Res<Vec<AudioSegment>> {
        Ok(Vec::new())
    }
    fn episodes(&self, _: i64, _: i64) -> Res<Vec<Activity>> {
        Ok(Vec::new())
    }
    fn app_icons(&self, _: &[String]) -> Vec<(String, PathBuf)> {
        Vec::new()
    }
    fn match_boxes(&self, _: i64, _: &str, _: bool) -> Res<Vec<MatchBox>> {
        Ok(Vec::new())
    }
    fn ocr(&self, _: i64) -> Res<Option<Vec<OcrBox>>> {
        Ok(None)
    }
    fn take_landing(&self) -> Option<Landing> {
        None
    }
}
