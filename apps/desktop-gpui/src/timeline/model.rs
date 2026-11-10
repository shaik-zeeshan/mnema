//! What the Timeline shows, as plain view-model structs, plus the pure rules
//! that turn frames into app bands, audio bars and labels. Views only render
//! these; `data.rs` (AppCore) and `fixture.rs` (parity JSON) fill them.
//!
//! These are view models, not app-core's wire DTOs (`FrameDto`, …): times are
//! unix ms and paths are shared `Arc<Path>`s, which the rail and the scrub
//! engine read every frame. `data::CoreSource` converts at the door.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use capture_types::{Activity, ActivityCategory};
use gpui_kit::Hsla;
use time::{OffsetDateTime, UtcOffset};

use crate::theme::Palette;

/// One `.timeline-rail__slot` is 8 px wide (`TIMELINE_SLOT_WIDTH`).
pub const SLOT_W: f32 = 8.;

#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub id: i64,
    pub session_id: String,
    pub file_path: Arc<Path>,
    pub captured_at_ms: i64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub app_bundle_id: Option<String>,
    pub app_name: Option<String>,
    pub window_title: Option<String>,
    /// Guarded host+path only (the raw URL never leaves the backend).
    pub url: Option<String>,
    pub equivalence_hint: Option<String>,
}

impl Frame {
    fn bundle(&self) -> Option<&str> {
        self.app_bundle_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
    }

    fn name(&self) -> Option<&str> {
        self.app_name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
    }

    /// `timelineAppIdentity`: bundle id, else the lower-cased name.
    pub fn app_identity(&self) -> Option<String> {
        match (self.bundle(), self.name()) {
            (Some(b), _) => Some(format!("bundle:{b}")),
            (None, Some(n)) => Some(format!("name:{}", n.to_lowercase())),
            _ => None,
        }
    }

    /// The readout's app label: name, else bundle id.
    pub fn app_label(&self) -> Option<&str> {
        self.name().or(self.bundle())
    }

    pub fn app_fallback(&self) -> String {
        icon_fallback(self.name(), self.bundle())
    }

    /// The guarded host (up to the first `/`); empty when the frame has no URL.
    pub fn host(&self) -> Option<&str> {
        self.url
            .as_deref()
            .and_then(|u| u.split('/').next())
            .filter(|h| !h.is_empty())
    }
}

/// `timelineAppIconFallback`: the first letter, upper-cased.
pub fn icon_fallback(name: Option<&str>, bundle: Option<&str>) -> String {
    let s = name.or(bundle).unwrap_or("?");
    s.chars()
        .next()
        .map(|c| c.to_uppercase().collect())
        .unwrap_or_else(|| "?".into())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioSource {
    Microphone,
    SystemAudio,
}

impl AudioSource {
    pub fn label(self) -> &'static str {
        match self {
            AudioSource::Microphone => "Microphone",
            AudioSource::SystemAudio => "System audio",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AudioSegment {
    pub id: i64,
    pub source: AudioSource,
    pub segment_index: i64,
    pub start_ms: i64,
    pub end_ms: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceKind {
    OriginalFrame,
    SegmentFrameFallback,
    VideoFallback,
    ScrubPreview,
}

impl SourceKind {
    pub fn name(self) -> &'static str {
        match self {
            SourceKind::OriginalFrame => "original_frame",
            SourceKind::SegmentFrameFallback => "segment_frame_fallback",
            SourceKind::VideoFallback => "video_fallback",
            SourceKind::ScrubPreview => "scrub_preview",
        }
    }
}

/// An exact (`get_frame_preview`) or scrub-tier image on disk.
#[derive(Clone, Debug, PartialEq)]
pub struct Preview {
    pub file_path: Arc<Path>,
    pub mime_type: String,
    pub source_kind: SourceKind,
    pub redactions: u32,
}

/// One `ScrubPreviewAvailabilityIntervalDto`: a cached low-res still that covers
/// `[start_ms, end_ms)` of capture time.
#[derive(Clone, Debug, PartialEq)]
pub struct ScrubInterval {
    pub segment_key: String,
    pub start_offset_ms: u64,
    pub start_ms: i64,
    pub end_ms: i64,
    pub preview: Option<Preview>,
    pub queued: bool,
}

/// `get_timeline_window_around_frame`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FocusedWindow {
    pub frames: Vec<Frame>,
    pub target_index: usize,
    pub has_newer: bool,
    pub has_older: bool,
}

/// A Quick Recall hit opened the Timeline: the chip, the rail mark, the boxes.
#[derive(Clone, Debug, PartialEq)]
pub struct Landing {
    pub query: String,
    pub frame_id: i64,
    pub screen: bool,
    pub by_meaning: bool,
}

/// One recognised OCR line, normalised to the frame with a top-left origin.
#[derive(Clone, Debug, PartialEq)]
pub struct OcrBox {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub text: String,
    pub confidence: f32,
}

/// One app run on the rail (`TimelineAppRun`): frames `start..=end`, newest first.
#[derive(Clone, Debug, PartialEq)]
pub struct AppRun {
    pub key: String,
    pub bundle_id: Option<String>,
    pub label: String,
    pub fallback: String,
    pub start: usize,
    pub end: usize,
    /// The id of the run's oldest frame when an older run follows (major tick).
    pub boundary_frame_id: Option<i64>,
}

impl AppRun {
    pub fn frame_count(&self) -> usize {
        self.end - self.start + 1
    }

    /// `timelineAppGroupTitle`.
    pub fn title(&self) -> String {
        let n = self.frame_count();
        format!(
            "{} · {n} frame{}",
            self.label,
            if n == 1 { "" } else { "s" }
        )
    }
}

/// `timelineAppRuns`: split on app identity, and within an app on a time gap
/// wider than `max_gap_ms` (one skipped capture interval must not split a run).
pub fn app_runs(frames: &[Frame], max_gap_ms: i64) -> Vec<AppRun> {
    let mut runs = Vec::new();
    let mut cur: Option<(String, Option<String>, Option<String>, usize, i64)> = None;
    let flush = |runs: &mut Vec<AppRun>,
                 cur: &Option<(String, Option<String>, Option<String>, usize, i64)>,
                 end_excl: usize| {
        if let Some((identity, bundle, name, start, _)) = cur {
            let end = end_excl - 1;
            runs.push(AppRun {
                key: format!("{identity}:oldest:{}", frames[end].id),
                bundle_id: bundle.clone(),
                label: name
                    .clone()
                    .or(bundle.clone())
                    .unwrap_or_else(|| "Unknown app".into()),
                fallback: icon_fallback(name.as_deref(), bundle.as_deref()),
                start: *start,
                end,
                boundary_frame_id: (end_excl < frames.len()).then(|| frames[end].id),
            });
        }
    };
    for (i, f) in frames.iter().enumerate() {
        let Some(identity) = f.app_identity() else {
            flush(&mut runs, &cur, i);
            cur = None;
            continue;
        };
        let at = f.captured_at_ms;
        if let Some((id, bundle, name, _, last)) = cur.as_mut() {
            if *id == identity {
                if (*last - at).abs() <= max_gap_ms {
                    if bundle.is_none() {
                        *bundle = f.bundle().map(String::from);
                    }
                    if name.is_none() {
                        *name = f.name().map(String::from);
                    }
                    *last = at;
                    continue;
                }
            }
        }
        flush(&mut runs, &cur, i);
        cur = Some((
            identity,
            f.bundle().map(String::from),
            f.name().map(String::from),
            i,
            at,
        ));
    }
    flush(&mut runs, &cur, frames.len());
    runs
}

/// `timelineGapMs`: three capture intervals, never under 10 s.
pub fn app_group_max_gap_ms(screen_fps: Option<f64>) -> i64 {
    let interval_ms = screen_fps.filter(|f| *f > 0.).map_or(2000., |f| 1000. / f);
    (interval_ms * 3.).max(10_000.) as i64
}

/// `categoryForSpan`: the episode overlapping `[start, end]` the most (ties go to
/// the earlier start); `None` when nothing overlaps.
pub fn category_for_span(start: i64, end: i64, episodes: &[Activity]) -> Option<ActivityCategory> {
    let mut best: Option<&Activity> = None;
    let mut best_overlap = -1i64;
    for e in episodes {
        let overlap = e.ended_at_ms.min(end) - e.started_at_ms.max(start);
        if overlap < 0 {
            continue;
        }
        let earlier = best.is_some_and(|b| e.started_at_ms < b.started_at_ms);
        if overlap > best_overlap || (overlap == best_overlap && earlier) {
            best = Some(e);
            best_overlap = overlap;
        }
    }
    best.and_then(|e| e.category)
}

/// The band / readout colour: the Insights category token, else uncategorized grey.
pub fn category_color(c: Option<ActivityCategory>, p: &Palette) -> Hsla {
    match c {
        Some(ActivityCategory::Creating) => p.cat_creating,
        Some(ActivityCategory::Communication) => p.cat_communication,
        Some(ActivityCategory::Meetings) => p.cat_meetings,
        Some(ActivityCategory::Research) => p.cat_research,
        Some(ActivityCategory::Learning) => p.cat_learning,
        Some(ActivityCategory::Organizing) => p.cat_organizing,
        Some(ActivityCategory::Personal) => p.cat_personal,
        Some(ActivityCategory::Entertainment) => p.cat_entertainment,
        None => p.chart_grey_3,
    }
}

/// `positionedAudioSegments`: slots `[newest..=oldest]` a segment covers, or
/// `None` when it misses the loaded window.
pub fn audio_slots(frames: &[Frame], seg: &AudioSegment) -> Option<(usize, usize)> {
    let (newest, oldest) = (frames.first()?, frames.last()?);
    if seg.end_ms < oldest.captured_at_ms || seg.start_ms > newest.captured_at_ms {
        return None;
    }
    let i_newest = frames
        .iter()
        .position(|f| f.captured_at_ms <= seg.end_ms)
        .unwrap_or(0);
    let i_oldest = frames
        .iter()
        .rposition(|f| f.captured_at_ms >= seg.start_ms)
        .unwrap_or(frames.len() - 1)
        .max(i_newest);
    Some((i_newest, i_oldest))
}

/// `activeFrameAudioMoment`: the segment covering `at_ms` (mic wins) and the offset.
pub fn audio_moment(segments: &[AudioSegment], at_ms: i64) -> Option<(&AudioSegment, i64)> {
    let mut over = segments
        .iter()
        .filter(|s| at_ms >= s.start_ms && at_ms <= s.end_ms);
    let first = over.clone().next()?;
    let chosen = over
        .find(|s| s.source == AudioSource::Microphone)
        .unwrap_or(first);
    Some((chosen, (at_ms - chosen.start_ms).max(0)))
}

/// Local wall time for a capture instant (the clock's offset: frozen in goldens).
pub fn local(ms: i64, offset: UtcOffset) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp_nanos(ms as i128 * 1_000_000)
        .unwrap_or(OffsetDateTime::UNIX_EPOCH)
        .to_offset(offset)
}

/// The Mac's 24-hour preference (NSDateFormatter "j"), set once at startup.
/// Off by default, so tests and goldens keep the parity fixture's 12-hour cycle.
static HOUR_24: AtomicBool = AtomicBool::new(false);

pub fn set_24_hour(on: bool) {
    HOUR_24.store(on, Ordering::Relaxed);
}

/// `formatClock(d, { seconds })`: `2:31:56 PM`, or `14:31:56` with the 24-hour cycle.
pub fn clock_label(t: OffsetDateTime, seconds: bool) -> String {
    if HOUR_24.load(Ordering::Relaxed) {
        return if seconds {
            format!("{:02}:{:02}:{:02}", t.hour(), t.minute(), t.second())
        } else {
            format!("{:02}:{:02}", t.hour(), t.minute())
        };
    }
    let h = t.hour() % 12;
    let h = if h == 0 { 12 } else { h };
    let ampm = if t.hour() < 12 { "AM" } else { "PM" };
    if seconds {
        format!("{h}:{:02}:{:02} {ampm}", t.minute(), t.second())
    } else {
        format!("{h}:{:02} {ampm}", t.minute())
    }
}

/// `toLocaleDateString({ weekday: short, month: short, day })`: `Wed, Nov 4`.
pub fn day_label(t: OffsetDateTime) -> String {
    let wd = &t.weekday().to_string()[..3];
    let mo = &t.month().to_string()[..3];
    format!("{wd}, {mo} {}", t.day())
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn frame(id: i64, at: i64, app: &str) -> Frame {
        Frame {
            id,
            session_id: "s".into(),
            file_path: Arc::from(Path::new("/tmp/x.jpg")),
            captured_at_ms: at,
            width: Some(1600),
            height: Some(1000),
            app_bundle_id: Some(format!("com.{app}")),
            app_name: Some(app.into()),
            window_title: None,
            url: Some("github.com/mnema".into()),
            equivalence_hint: None,
        }
    }

    #[test]
    fn runs_split_on_app_and_on_a_long_gap() {
        // Newest first: two Slack, a 30 s gap, one Slack, then Arc.
        let frames = vec![
            frame(9, 100_000, "Slack"),
            frame(8, 98_000, "Slack"),
            frame(7, 60_000, "Slack"),
            frame(6, 58_000, "Arc"),
        ];
        let runs = app_runs(&frames, app_group_max_gap_ms(Some(0.5)));
        assert_eq!(
            runs.iter().map(|r| (r.start, r.end)).collect::<Vec<_>>(),
            [(0, 1), (2, 2), (3, 3)]
        );
        assert_eq!(runs[0].boundary_frame_id, Some(8));
        assert_eq!(
            runs[2].boundary_frame_id, None,
            "the oldest run has no boundary"
        );
        assert_eq!(runs[0].title(), "Slack · 2 frames");
        assert_eq!(runs[0].fallback, "S");
        assert_eq!(app_group_max_gap_ms(Some(1. / 60.)), 180_000);
    }

    #[test]
    fn audio_bars_cover_the_frames_inside_them() {
        let frames: Vec<_> = (0..5)
            .map(|i| frame(10 - i, 10_000 - i * 1000, "A"))
            .collect();
        let seg = |s, e| AudioSegment {
            id: 1,
            source: AudioSource::Microphone,
            segment_index: 0,
            start_ms: s,
            end_ms: e,
        };
        assert_eq!(audio_slots(&frames, &seg(7_500, 9_100)), Some((1, 2)));
        assert_eq!(audio_slots(&frames, &seg(11_000, 12_000)), None);
        let both = [
            AudioSegment {
                source: AudioSource::SystemAudio,
                ..seg(0, 20_000)
            },
            seg(8_000, 9_000),
        ];
        let (s, off) = audio_moment(&both, 8_500).unwrap();
        assert_eq!((s.source, off), (AudioSource::Microphone, 500));
    }

    #[test]
    fn labels_match_the_svelte_formatters() {
        let t = local(1_793_782_916_000, UtcOffset::from_hms(5, 30, 0).unwrap());
        assert_eq!(clock_label(t, true), "2:31:56 PM");
        assert_eq!(day_label(t), "Wed, Nov 4");
        assert_eq!(frame(1, 0, "A").host(), Some("github.com"));
    }
}
