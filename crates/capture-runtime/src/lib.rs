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
}

impl SegmentPlanner {
    pub fn new(save_root_dir: impl Into<String>, session_id: impl Into<String>) -> Self {
        Self {
            save_root_dir: save_root_dir.into(),
            session_id: session_id.into(),
        }
    }

    pub fn save_root_dir(&self) -> &str {
        &self.save_root_dir
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn segment_dir(&self, segment_index: u64) -> PathBuf {
        Path::new(&self.save_root_dir)
            .join(format!("{}-segment-{segment_index:04}", self.session_id))
    }

    pub fn microphone_file(&self, segment_index: u64) -> PathBuf {
        self.segment_dir(segment_index).join("microphone.m4a")
    }
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
    fn planner_uses_stable_segment_directory_shape_without_parent_session_dir() {
        let planner = SegmentPlanner::new("/tmp/records", "native-session-123");

        assert_eq!(
            planner.segment_dir(7),
            PathBuf::from("/tmp/records/native-session-123-segment-0007")
        );
        assert_eq!(
            planner.microphone_file(7),
            PathBuf::from("/tmp/records/native-session-123-segment-0007/microphone.m4a")
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
