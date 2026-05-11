mod debug_log;

pub use debug_log::{
    configure_debug_log, debug_log_files_exist, delete_debug_log_files, write_debug_log,
    write_debug_log_fmt, write_debug_log_to_file,
};

#[macro_export]
macro_rules! debug_log {
    ($($arg:tt)*) => {
        $crate::write_debug_log_fmt(format_args!($($arg)*))
    };
}

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use chrono::Local;

pub fn current_date_prefix() -> String {
    Local::now().format("%Y/%m/%d").to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeState {
    Idle,
    Starting,
    Running,
    Rotating,
    Stopping,
    Failed,
}

impl Default for RuntimeState {
    fn default() -> Self {
        Self::Idle
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeSignal {
    StartRequested,
    RotateRequested,
    StopRequested,
    SourcesReady,
    SourcesStopped,
    SourceFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeTransitionError {
    pub from: RuntimeState,
    pub signal: RuntimeSignal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeController {
    state: RuntimeState,
}

impl Default for RuntimeController {
    fn default() -> Self {
        Self {
            state: RuntimeState::Idle,
        }
    }
}

impl RuntimeController {
    pub fn state(&self) -> RuntimeState {
        self.state
    }

    pub fn apply(&mut self, signal: RuntimeSignal) -> Result<RuntimeState, RuntimeTransitionError> {
        let next_state = match (self.state, signal) {
            (RuntimeState::Idle | RuntimeState::Failed, RuntimeSignal::StartRequested) => {
                RuntimeState::Starting
            }
            (RuntimeState::Starting | RuntimeState::Rotating, RuntimeSignal::SourcesReady) => {
                RuntimeState::Running
            }
            (RuntimeState::Running, RuntimeSignal::RotateRequested) => RuntimeState::Rotating,
            (
                RuntimeState::Starting
                | RuntimeState::Running
                | RuntimeState::Rotating
                | RuntimeState::Failed,
                RuntimeSignal::StopRequested,
            ) => RuntimeState::Stopping,
            (RuntimeState::Stopping, RuntimeSignal::SourcesStopped) => RuntimeState::Idle,
            (
                RuntimeState::Starting
                | RuntimeState::Running
                | RuntimeState::Rotating
                | RuntimeState::Stopping,
                RuntimeSignal::SourceFailed,
            ) => RuntimeState::Failed,
            _ => {
                return Err(RuntimeTransitionError {
                    from: self.state,
                    signal,
                });
            }
        };

        self.state = next_state;
        Ok(next_state)
    }
}

#[derive(Debug, Clone)]
pub struct SegmentPlanner {
    save_root_dir: String,
    session_id: String,
    /// Date folder component for the next allocated outputs: "YYYY/MM/DD"
    date_prefix: String,
}

impl SegmentPlanner {
    pub fn new(save_root_dir: impl Into<String>, session_id: impl Into<String>) -> Self {
        Self::with_date_prefix(save_root_dir, session_id, current_date_prefix())
    }

    /// Build a planner with an explicit date prefix (useful for testing).
    pub fn with_date_prefix(
        save_root_dir: impl Into<String>,
        session_id: impl Into<String>,
        date_prefix: impl Into<String>,
    ) -> Self {
        Self {
            save_root_dir: save_root_dir.into(),
            session_id: session_id.into(),
            date_prefix: date_prefix.into(),
        }
    }

    pub fn save_root_dir(&self) -> &str {
        &self.save_root_dir
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn date_prefix(&self) -> &str {
        &self.date_prefix
    }

    pub fn set_date_prefix(&mut self, date_prefix: impl Into<String>) {
        self.date_prefix = date_prefix.into();
    }

    /// Base directory for this session's date: `<save_root>/YYYY/MM/DD`
    fn date_dir(&self) -> PathBuf {
        Path::new(&self.save_root_dir).join(&self.date_prefix)
    }

    /// Per-segment workspace directory for screen artifacts (frames, etc.).
    /// Hidden (dot-prefixed) to avoid collision with the final .mov file.
    /// `<save_root>/YYYY/MM/DD/.<session_id>-segment-####`
    pub fn segment_workspace_dir(&self, segment_index: u64) -> PathBuf {
        self.date_dir()
            .join(format!(".{}-segment-{segment_index:04}", self.session_id))
    }

    /// Final visible screen output path.
    /// `<save_root>/YYYY/MM/DD/<session_id>-segment-####.mov`
    pub fn segment_screen_output(&self, segment_index: u64) -> PathBuf {
        self.date_dir().join(format!(
            "{}-segment-{segment_index:04}.mov",
            self.session_id
        ))
    }

    /// Legacy alias – returns the workspace dir so existing callers that
    /// create child directories (e.g. `frames/`) keep working.
    pub fn segment_dir(&self, segment_index: u64) -> PathBuf {
        self.segment_workspace_dir(segment_index)
    }

    /// Flat dated audio directory shared by all audio sources: `<save_root>/YYYY/MM/DD/audio`
    ///
    /// All microphone/system-audio files for every segment live directly in this directory;
    /// no per-session or per-segment sub-directories are created.
    pub fn audio_dir(&self) -> PathBuf {
        self.date_dir().join("audio")
    }

    /// `<save_root>/YYYY/MM/DD/audio/<session_id>-segment-####.m4a`
    pub fn microphone_file(&self, segment_index: u64) -> PathBuf {
        self.audio_dir().join(format!(
            "{}-segment-{segment_index:04}.m4a",
            self.session_id
        ))
    }

    /// Collision-safe reconnect path for a microphone restart within a segment.
    /// `<save_root>/YYYY/MM/DD/audio/<session_id>-segment-####-<ts>.m4a`
    ///
    /// If the base timestamp path already exists (e.g. two reconnects in the same
    /// millisecond), an incrementing suffix is appended to guarantee uniqueness.
    pub fn microphone_reconnect_file(
        &self,
        segment_index: u64,
        reconnect_started_at_unix_ms: u64,
    ) -> PathBuf {
        let audio_dir = self.audio_dir();
        let base = audio_dir.join(format!(
            "{}-segment-{segment_index:04}-{reconnect_started_at_unix_ms}.m4a",
            self.session_id
        ));
        if !base.exists() {
            return base;
        }
        let mut counter = 1u32;
        loop {
            let candidate = audio_dir.join(format!(
                "{}-segment-{segment_index:04}-{reconnect_started_at_unix_ms}-{counter}.m4a",
                self.session_id
            ));
            if !candidate.exists() {
                return candidate;
            }
            counter += 1;
        }
    }

    /// `<save_root>/YYYY/MM/DD/audio/<session_id>-segment-####.m4a`
    pub fn system_audio_file(&self, segment_index: u64) -> PathBuf {
        self.audio_dir().join(format!(
            "{}-segment-{segment_index:04}.m4a",
            self.session_id
        ))
    }

    /// Collision-safe resume path for a system-audio writer restart within a segment.
    /// `<save_root>/YYYY/MM/DD/audio/<session_id>-segment-####-<ts>.m4a`
    ///
    /// If the base timestamp path already exists (e.g. two resumes in the same
    /// millisecond), an incrementing suffix is appended to guarantee uniqueness.
    pub fn system_audio_resume_file(&self, segment_index: u64, resumed_at_unix_ms: u64) -> PathBuf {
        let audio_dir = self.audio_dir();
        let base = audio_dir.join(format!(
            "{}-segment-{segment_index:04}-{resumed_at_unix_ms}.m4a",
            self.session_id
        ));
        if !base.exists() {
            return base;
        }
        let mut counter = 1u32;
        loop {
            let candidate = audio_dir.join(format!(
                "{}-segment-{segment_index:04}-{resumed_at_unix_ms}-{counter}.m4a",
                self.session_id
            ));
            if !candidate.exists() {
                return candidate;
            }
            counter += 1;
        }
    }
}

/// Parse the embedded restart timestamp from planner-generated audio filenames.
///
/// Returns `None` for base segment files without a restart timestamp.
pub fn parse_audio_restart_started_at_unix_ms(file_path: impl AsRef<Path>) -> Option<u64> {
    let file_name = file_path.as_ref().file_name()?.to_str()?;
    let stem = file_name.strip_suffix(".m4a")?;

    let marker = "-segment-";
    let marker_start = stem.rfind(marker)?;
    let after_marker = &stem[marker_start + marker.len()..];
    if after_marker.len() < 4 {
        return None;
    }

    let (segment_index, remainder) = after_marker.split_at(4);
    if !segment_index.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }

    let timestamp_with_suffix = remainder.strip_prefix('-')?;
    let (timestamp, suffix) = timestamp_with_suffix
        .split_once('-')
        .map_or((timestamp_with_suffix, None), |(timestamp, suffix)| {
            (timestamp, Some(suffix))
        });

    if timestamp.is_empty() || !timestamp.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    if suffix.is_some_and(|suffix| {
        suffix.is_empty() || !suffix.bytes().all(|byte| byte.is_ascii_digit())
    }) {
        return None;
    }

    timestamp.parse().ok()
}

#[derive(Debug, Clone)]
pub struct CaptureClock {
    started_at: Instant,
}

impl CaptureClock {
    pub fn start_now() -> Self {
        Self {
            started_at: Instant::now(),
        }
    }

    pub fn elapsed(&self) -> Duration {
        self.started_at.elapsed()
    }
}

#[derive(Debug, Clone)]
pub struct SegmentSchedule {
    segment_duration: Duration,
}

impl SegmentSchedule {
    pub fn new(segment_duration: Duration) -> Self {
        Self { segment_duration }
    }

    pub fn segment_duration(&self) -> Duration {
        self.segment_duration
    }

    pub fn segment_duration_reached(&self, elapsed: Duration) -> bool {
        !self.segment_duration.is_zero() && elapsed >= self.segment_duration
    }

    pub fn current_segment_index(&self, elapsed: Duration) -> u64 {
        if self.segment_duration.is_zero() {
            return 1;
        }

        let elapsed_segments = elapsed.as_nanos() / self.segment_duration.as_nanos();
        let elapsed_segments = elapsed_segments.min(u128::from(u64::MAX - 1));
        elapsed_segments as u64 + 1
    }

    pub fn next_boundary_after(&self, elapsed: Duration) -> Duration {
        if self.segment_duration.is_zero() {
            return Duration::ZERO;
        }

        let segment_duration_nanos = self.segment_duration.as_nanos();
        let elapsed_segments = elapsed.as_nanos() / segment_duration_nanos;
        let next_segment = elapsed_segments.saturating_add(1);
        duration_from_total_nanos_saturating(segment_duration_nanos.saturating_mul(next_segment))
    }

    pub fn sleep_until_next_boundary(&self, clock: &CaptureClock) -> Duration {
        let elapsed = clock.elapsed();
        self.next_boundary_after(elapsed).saturating_sub(elapsed)
    }
}

fn duration_from_total_nanos_saturating(total_nanos: u128) -> Duration {
    let clamped_nanos = total_nanos.min(Duration::MAX.as_nanos());
    let secs = (clamped_nanos / 1_000_000_000) as u64;
    let nanos = (clamped_nanos % 1_000_000_000) as u32;
    Duration::new(secs, nanos)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planner_uses_date_based_layout() {
        let planner =
            SegmentPlanner::with_date_prefix("/tmp/records", "native-session-123", "2026/04/16");

        // Workspace dir (dot-prefixed, for frames etc.)
        assert_eq!(
            planner.segment_workspace_dir(7),
            PathBuf::from("/tmp/records/2026/04/16/.native-session-123-segment-0007")
        );

        // segment_dir is an alias for workspace
        assert_eq!(planner.segment_dir(7), planner.segment_workspace_dir(7));

        // Final visible screen output
        assert_eq!(
            planner.segment_screen_output(7),
            PathBuf::from("/tmp/records/2026/04/16/native-session-123-segment-0007.mov")
        );

        // Audio layout: all audio files are flat under dated audio/
        assert_eq!(
            planner.audio_dir(),
            PathBuf::from("/tmp/records/2026/04/16/audio")
        );
        assert_eq!(
            planner.microphone_file(7),
            PathBuf::from("/tmp/records/2026/04/16/audio/native-session-123-segment-0007.m4a")
        );
        // microphone_reconnect_file: base path returned when no file exists on disk
        // (path-based collision probe; no files created in this test)
        assert_eq!(
            planner.microphone_reconnect_file(7, 12345),
            PathBuf::from(
                "/tmp/records/2026/04/16/audio/native-session-123-segment-0007-12345.m4a"
            )
        );
        assert_eq!(
            planner.system_audio_file(7),
            PathBuf::from("/tmp/records/2026/04/16/audio/native-session-123-segment-0007.m4a")
        );
    }

    #[test]
    fn planner_workspace_supports_frames_child() {
        let planner = SegmentPlanner::with_date_prefix("/tmp/records", "sess-1", "2026/01/01");
        let frames = planner.segment_workspace_dir(1).join("frames");
        assert_eq!(
            frames,
            PathBuf::from("/tmp/records/2026/01/01/.sess-1-segment-0001/frames")
        );
    }

    #[test]
    fn planner_new_captures_today() {
        let planner = SegmentPlanner::new("/tmp/records", "sess-1");
        let today = current_date_prefix();
        assert_eq!(planner.date_prefix(), today);
    }

    #[test]
    fn microphone_reconnect_file_avoids_collision() {
        let dir = std::env::temp_dir().join("capture-runtime-test-mic-reconnect-collision");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let ts: u64 = 1700000000000;
        let planner =
            SegmentPlanner::with_date_prefix(dir.to_str().unwrap(), "sess-mic", "2026/01/01");
        let audio_dir = planner.audio_dir();
        std::fs::create_dir_all(&audio_dir).unwrap();

        // First call returns base path (file does not exist yet).
        let first = planner.microphone_reconnect_file(1, ts);
        assert_eq!(
            first,
            audio_dir.join("sess-mic-segment-0001-1700000000000.m4a")
        );

        // Create that file so the next call must dodge it.
        std::fs::write(&first, b"").unwrap();
        let second = planner.microphone_reconnect_file(1, ts);
        assert_eq!(
            second,
            audio_dir.join("sess-mic-segment-0001-1700000000000-1.m4a")
        );

        // Create that too; third call increments again.
        std::fs::write(&second, b"").unwrap();
        let third = planner.microphone_reconnect_file(1, ts);
        assert_eq!(
            third,
            audio_dir.join("sess-mic-segment-0001-1700000000000-2.m4a")
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn system_audio_resume_file_avoids_collision() {
        let dir = std::env::temp_dir().join("capture-runtime-test-resume-collision");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let ts: u64 = 1700000000000;
        let planner =
            SegmentPlanner::with_date_prefix(dir.to_str().unwrap(), "sess-col", "2026/01/01");
        let audio_dir = planner.audio_dir();
        std::fs::create_dir_all(&audio_dir).unwrap();

        // First call returns base path.
        let first = planner.system_audio_resume_file(1, ts);
        assert_eq!(
            first,
            audio_dir.join("sess-col-segment-0001-1700000000000.m4a")
        );

        // Create that file so the next call must dodge it.
        std::fs::write(&first, b"").unwrap();
        let second = planner.system_audio_resume_file(1, ts);
        assert_eq!(
            second,
            audio_dir.join("sess-col-segment-0001-1700000000000-1.m4a")
        );

        // Create that too; third call increments again.
        std::fs::write(&second, b"").unwrap();
        let third = planner.system_audio_resume_file(1, ts);
        assert_eq!(
            third,
            audio_dir.join("sess-col-segment-0001-1700000000000-2.m4a")
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn audio_restart_timestamp_parser_handles_planner_filenames() {
        assert_eq!(
            parse_audio_restart_started_at_unix_ms(
                "/tmp/audio/mic_session-segment-0001-1712345678901.m4a"
            ),
            Some(1_712_345_678_901)
        );
        assert_eq!(
            parse_audio_restart_started_at_unix_ms(
                "/tmp/audio/mic_session-segment-0001-1712345678901-1.m4a"
            ),
            Some(1_712_345_678_901)
        );
        assert_eq!(
            parse_audio_restart_started_at_unix_ms(
                "/tmp/audio/sysaudio_session-segment-0001-1712345678901.m4a"
            ),
            Some(1_712_345_678_901)
        );
        assert_eq!(
            parse_audio_restart_started_at_unix_ms(
                "/tmp/audio/sysaudio_session-segment-0001-1712345678901-1.m4a"
            ),
            Some(1_712_345_678_901)
        );
    }

    #[test]
    fn audio_restart_timestamp_parser_ignores_base_files() {
        assert_eq!(
            parse_audio_restart_started_at_unix_ms("/tmp/audio/mic_session-segment-0001.m4a"),
            None
        );
        assert_eq!(
            parse_audio_restart_started_at_unix_ms("/tmp/audio/sysaudio_session-segment-0001.m4a"),
            None
        );
    }

    #[test]
    fn audio_restart_timestamp_parser_handles_hyphenated_session_ids() {
        assert_eq!(
            parse_audio_restart_started_at_unix_ms(
                "/tmp/audio/team-alpha-session-segment-0007-1712345678901.m4a"
            ),
            Some(1_712_345_678_901)
        );
        assert_eq!(
            parse_audio_restart_started_at_unix_ms(
                "/tmp/audio/team-alpha-session-segment-0007-1712345678901-2.m4a"
            ),
            Some(1_712_345_678_901)
        );
    }

    #[test]
    fn audio_restart_timestamp_parser_rejects_invalid_names() {
        assert_eq!(
            parse_audio_restart_started_at_unix_ms(
                "/tmp/audio/microphone-session-segment-0001-1712345678901.mov"
            ),
            None
        );
        assert_eq!(
            parse_audio_restart_started_at_unix_ms(
                "/tmp/audio/microphone-session-segment-0001-not-a-timestamp.m4a"
            ),
            None
        );
        assert_eq!(
            parse_audio_restart_started_at_unix_ms(
                "/tmp/audio/system-audio-session-segment-0001-1712345678901-copy.m4a"
            ),
            None
        );
        assert_eq!(
            parse_audio_restart_started_at_unix_ms(
                "/tmp/audio/microphone-session-segment-abc1-1712345678901.m4a"
            ),
            None
        );
    }

    #[test]
    fn controller_tracks_explicit_runtime_transitions() {
        let mut controller = RuntimeController::default();

        assert_eq!(controller.state(), RuntimeState::Idle);
        assert_eq!(
            controller.apply(RuntimeSignal::StartRequested),
            Ok(RuntimeState::Starting)
        );
        assert_eq!(
            controller.apply(RuntimeSignal::SourcesReady),
            Ok(RuntimeState::Running)
        );
        assert_eq!(
            controller.apply(RuntimeSignal::RotateRequested),
            Ok(RuntimeState::Rotating)
        );
        assert_eq!(
            controller.apply(RuntimeSignal::SourcesReady),
            Ok(RuntimeState::Running)
        );
        assert_eq!(
            controller.apply(RuntimeSignal::StopRequested),
            Ok(RuntimeState::Stopping)
        );
        assert_eq!(
            controller.apply(RuntimeSignal::SourcesStopped),
            Ok(RuntimeState::Idle)
        );
    }

    #[test]
    fn controller_rejects_invalid_transition() {
        let mut controller = RuntimeController::default();
        let err = controller
            .apply(RuntimeSignal::RotateRequested)
            .expect_err("idle cannot rotate");
        assert_eq!(err.from, RuntimeState::Idle);
        assert_eq!(err.signal, RuntimeSignal::RotateRequested);
    }

    #[test]
    fn schedule_maps_elapsed_time_to_segment_index() {
        let schedule = SegmentSchedule::new(Duration::from_secs(60));

        assert_eq!(schedule.current_segment_index(Duration::from_secs(0)), 1);
        assert_eq!(schedule.current_segment_index(Duration::from_secs(59)), 1);
        assert_eq!(schedule.current_segment_index(Duration::from_secs(60)), 2);
        assert_eq!(schedule.current_segment_index(Duration::from_secs(120)), 3);
    }

    #[test]
    fn schedule_computes_next_boundary() {
        let schedule = SegmentSchedule::new(Duration::from_secs(10));

        assert_eq!(
            schedule.next_boundary_after(Duration::from_secs(0)),
            Duration::from_secs(10)
        );
        assert_eq!(
            schedule.next_boundary_after(Duration::from_secs(9)),
            Duration::from_secs(10)
        );
        assert_eq!(
            schedule.next_boundary_after(Duration::from_secs(10)),
            Duration::from_secs(20)
        );
    }

    #[test]
    fn schedule_detects_when_segment_duration_is_reached() {
        let schedule = SegmentSchedule::new(Duration::from_secs(10));

        assert!(!schedule.segment_duration_reached(Duration::from_secs(9)));
        assert!(schedule.segment_duration_reached(Duration::from_secs(10)));
        assert!(schedule.segment_duration_reached(Duration::from_secs(12)));
    }

    #[test]
    fn schedule_zero_duration_never_reaches_rotation_boundary() {
        let schedule = SegmentSchedule::new(Duration::ZERO);

        assert!(!schedule.segment_duration_reached(Duration::ZERO));
        assert!(!schedule.segment_duration_reached(Duration::from_secs(1)));
    }

    #[test]
    fn schedule_uses_integer_math_for_fractional_boundaries() {
        let schedule = SegmentSchedule::new(Duration::from_millis(33));

        assert_eq!(schedule.current_segment_index(Duration::from_millis(98)), 3);
        assert_eq!(schedule.current_segment_index(Duration::from_millis(99)), 4);
        assert_eq!(
            schedule.next_boundary_after(Duration::from_millis(98)),
            Duration::from_millis(99)
        );
        assert_eq!(
            schedule.next_boundary_after(Duration::from_millis(99)),
            Duration::from_millis(132)
        );
    }
}
