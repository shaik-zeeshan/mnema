//! System audio as an independent capture family (ADR 0052).
//!
//! The sibling of `microphone.rs`: a thin adapter between the segment machinery
//! and `capture-system-audio`'s tap session. Nothing here knows about the screen,
//! and that is the point — the tap has no display dependency, so system audio
//! records through display sleep, lock and disconnect exactly like the microphone,
//! and a session may request system audio with no screen at all.
//!
//! The one shape worth explaining is the bus. `SystemAudioCaptureSession` owns
//! callbacks that ask for the next output file and hand back the finished one,
//! but the session itself lives inside `NativeCaptureRuntime` — so a callback can
//! never borrow the runtime it is stored in. The bus is the seam: the caller
//! parks the planner and the pause flag in it before driving the session, and
//! reads back what the session did.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use capture_runtime::SegmentPlanner;
use capture_system_audio::{
    SystemAudioCaptureSession, SystemAudioEvidence, SystemAudioSegmentFinalization,
    SystemAudioSegmentHooks, LOG_PREFIX, SILENT_SESSION_AFTER_MS,
};
use capture_types::{CaptureErrorResponse, CapturePermissionState};
use tauri::Manager;

use super::runtime::now_unix_ms;

/// The shared state the tap session's hooks read and write.
#[derive(Default)]
struct SegmentBus {
    planner: Option<SegmentPlanner>,
    segment_index: u64,
    /// Inactivity pause. The tap and its watchdog stay alive; only the writing
    /// stops (ADR 0052) — a wedged tap could otherwise never deliver the sound
    /// that would resume it.
    paused: bool,
    /// The path the caller wants the next segment to use. A rebuild takes no
    /// path from here: it synthesizes a collision-safe one, because it can fall
    /// anywhere inside a segment the caller already named.
    pending_output_file: Option<PathBuf>,
    /// What the session most recently opened, or `None` while paused.
    live_output_file: Option<PathBuf>,
}

impl SegmentBus {
    fn take_next_output_file(&mut self) -> Option<PathBuf> {
        if self.paused {
            self.live_output_file = None;
            return None;
        }

        let next = match self.pending_output_file.take() {
            Some(next) => next,
            // A rebuild lands mid-segment, so it needs a fresh path under the
            // segment the caller last named; `system_audio_resume_file` is the
            // same collision-safe naming an inactivity resume already uses.
            None => self
                .planner
                .as_ref()?
                .system_audio_resume_file(self.segment_index, now_unix_ms()),
        };

        if let Some(audio_dir) = next.parent() {
            if let Err(error) = std::fs::create_dir_all(audio_dir) {
                super::debug_log::log(format!(
                    "{LOG_PREFIX} failed to create system audio output directory: {error}"
                ));
                self.live_output_file = None;
                return None;
            }
        }

        self.live_output_file = Some(next.clone());
        Some(next)
    }
}

/// A live system-audio capture, parked in the runtime and driven from the
/// capture tick.
pub(crate) struct SystemAudioFamily {
    session: SystemAudioCaptureSession,
    bus: Arc<Mutex<SegmentBus>>,
}

impl std::fmt::Debug for SystemAudioFamily {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SystemAudioFamily")
            .field("rebuilds", &self.session.rebuild_count())
            .field("output_file", &self.live_output_file())
            .finish()
    }
}

fn lock_bus(bus: &Mutex<SegmentBus>) -> std::sync::MutexGuard<'_, SegmentBus> {
    bus.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl SystemAudioFamily {
    /// Starts the tap and opens the first segment. `excluded_bundle_ids` is the
    /// privacy list, which the tap excludes exactly as ScreenCaptureKit's content
    /// filter used to.
    pub(super) fn start(
        planner: SegmentPlanner,
        segment_index: u64,
        first_output_file: PathBuf,
        excluded_bundle_ids: Vec<String>,
    ) -> Result<Self, CaptureErrorResponse> {
        let bus = Arc::new(Mutex::new(SegmentBus {
            planner: Some(planner),
            segment_index,
            paused: false,
            pending_output_file: Some(first_output_file),
            live_output_file: None,
        }));

        let next_bus = Arc::clone(&bus);
        let session = SystemAudioCaptureSession::start(
            excluded_bundle_ids,
            SystemAudioSegmentHooks {
                next_output_file: Box::new(move || lock_bus(&next_bus).take_next_output_file()),
                segment_finalized: Box::new(log_finalization),
            },
        )?;

        Ok(Self { session, bus })
    }

    /// The file being written right now, or `None` while paused for inactivity.
    /// The runtime mirrors this into `system_audio_recording_file`.
    pub(super) fn live_output_file(&self) -> Option<String> {
        lock_bus(&self.bus)
            .live_output_file
            .as_ref()
            .map(|path| path.to_string_lossy().to_string())
    }

    /// Closes the in-flight segment and opens `next`, or none when `next` is
    /// `None` — which is what an inactivity pause is. Returns the file now being
    /// written, which is `None` when nothing is.
    pub(super) fn advance_segment(
        &mut self,
        planner: &SegmentPlanner,
        segment_index: u64,
        next: Option<PathBuf>,
    ) -> Option<String> {
        {
            let mut bus = lock_bus(&self.bus);
            bus.planner = Some(planner.clone());
            bus.segment_index = segment_index;
            bus.paused = next.is_none();
            bus.pending_output_file = next;
            // Cleared up front so the answer below comes from what the session
            // actually opened. A session left with no tap by a failed rebuild
            // advances nothing, and reporting the previous file as still-live
            // would commit it a second time under the next segment's clock.
            bus.live_output_file = None;
        }
        self.session.advance_segment();
        self.live_output_file()
    }

    /// Drives the tap's rebuild engine from the existing tick. Must be called
    /// whether or not system audio is paused: the zero-watchdog is the only thing
    /// that notices a wedged tap, and a paused tap is exactly where a wedge hides.
    ///
    /// Returns the file being written if a rebuild rotated onto a new one — an
    /// `Err` from the session is a failed rebuild, never a dead session, so it is
    /// logged and polling continues on the watchdog's own backoff.
    pub(super) fn poll(&mut self) -> Option<String> {
        let before = self.live_output_file();
        if let Err(error) = self.session.poll() {
            super::debug_log::log(format!(
                "{LOG_PREFIX} tap rebuild failed; retrying on the watchdog's backoff: [{}] {}",
                error.code, error.message
            ));
        }

        let after = self.live_output_file();
        (after != before).then_some(after).flatten()
    }

    /// The privacy-edit hook: hands the new privacy list to the tap's exclude
    /// watcher. Cheap and lock-safe — it only stores the set and marks the
    /// watcher dirty; the next `poll` does the Core Audio read, reconcile, and
    /// any rebuild off the caller's lock.
    pub(super) fn set_excluded_bundle_ids(&self, excluded_bundle_ids: Vec<String>) {
        self.session.set_excluded_bundle_ids(excluded_bundle_ids);
    }

    pub(super) fn stop(mut self) {
        self.session.stop();
    }
}

// ── Permission (ADR 0052) ──────────────────────────────────────────────────
//
// Nothing here asks macOS anything, because nothing can: process taps have no
// authorization query. `capture_system_audio::system_audio_permission_state`
// owns the rule; this owns the two boring parts around it — where the evidence
// is kept and when it is judged.
//
// The evidence is cached in an atomic rather than read from `app_settings` on
// demand because `get_capture_permissions` is a sync command polled on every
// window focus, and because the tick that updates it must not block on SQLite.
// The DB is the durable copy: hydrated once at startup, written only when the
// answer actually moves (at most twice in an install's life).

const EVIDENCE_NONE: u8 = 0;
const EVIDENCE_SILENT: u8 = 1;
const EVIDENCE_SOUND: u8 = 2;
/// Nothing has been read off disk yet — a different fact from having read "no tap
/// has ever been judged", and the one the fold in [`note_permission_evidence`]
/// depends on. Reads as `None` everywhere else.
const EVIDENCE_UNHYDRATED: u8 = u8::MAX;

static EVIDENCE: AtomicU8 = AtomicU8::new(EVIDENCE_UNHYDRATED);

fn evidence_from_cache(cache: u8) -> SystemAudioEvidence {
    match cache {
        EVIDENCE_SILENT => SystemAudioEvidence::SilentSession,
        EVIDENCE_SOUND => SystemAudioEvidence::SoundHeard,
        _ => SystemAudioEvidence::None,
    }
}

fn cached_evidence() -> SystemAudioEvidence {
    evidence_from_cache(EVIDENCE.load(Ordering::Relaxed))
}

/// The evidence to persist given the cached value and what the tap delivered, or
/// `None` when nothing should be written.
///
/// `observe` is monotonic against the *stored* evidence, and until the DB read
/// lands the cache is not evidence — it is ignorance that happens to read as
/// `None`. Folding a quiet session onto it would write `silent_session` over a
/// persisted `sound_heard` and accuse a working install of being blocked. The
/// tick runs every second, so declining to judge until hydration costs nothing.
fn next_evidence(cache: u8, heard_sound: bool) -> Option<SystemAudioEvidence> {
    if cache == EVIDENCE_UNHYDRATED {
        return None;
    }

    let current = evidence_from_cache(cache);
    let next = current.observe(heard_sound);
    (next != current).then_some(next)
}

fn store_evidence(evidence: SystemAudioEvidence) {
    EVIDENCE.store(
        match evidence {
            SystemAudioEvidence::None => EVIDENCE_NONE,
            SystemAudioEvidence::SilentSession => EVIDENCE_SILENT,
            SystemAudioEvidence::SoundHeard => EVIDENCE_SOUND,
        },
        Ordering::Relaxed,
    );
}

/// The tri-state the permission surfaces render. Until [`hydrate_evidence`] has
/// run this reads as "not yet requested", which is the honest answer while the
/// evidence is still on disk — and the frontend re-polls on focus.
pub(crate) fn permission_state() -> CapturePermissionState {
    capture_system_audio::system_audio_permission_state(
        capture_screen::supports_system_audio_capture(),
        cached_evidence(),
    )
}

/// Loads the persisted evidence into the cache. Runs on the deferred-startup
/// thread — a permission read before it lands simply says "not yet requested".
///
/// A failed read leaves the cache un-hydrated, which reads the same but stops
/// this session judging: the stored answer may be stronger than anything this
/// session could observe, and it is the one thing that must not be overwritten
/// blind.
pub(crate) async fn hydrate_evidence(infra: &::app_infra::AppInfra) {
    match infra.system_audio_evidence().evidence().await {
        Ok(stored) => store_evidence(
            stored
                .as_deref()
                .map(SystemAudioEvidence::from_str)
                .unwrap_or_default(),
        ),
        Err(error) => super::debug_log::log(format!(
            "{LOG_PREFIX} failed to read stored permission evidence; leaving it unjudged for this session: {error}"
        )),
    }
}

/// Judges the live tap against what it has delivered, persisting only when the
/// answer moves.
///
/// `session_age_ms` is how long the tap has been running: silence only counts as
/// evidence once a tap has had [`SILENT_SESSION_AFTER_MS`] to deliver something,
/// so a recording started and stopped in seconds never accuses anyone.
pub(super) fn note_permission_evidence(app_handle: &tauri::AppHandle, session_age_ms: u64) {
    let heard_sound = capture_system_audio::system_audio_sound_observed();
    if !heard_sound && session_age_ms < SILENT_SESSION_AFTER_MS {
        return;
    }

    let Some(next) = next_evidence(EVIDENCE.load(Ordering::Relaxed), heard_sound) else {
        return;
    };
    store_evidence(next);

    super::debug_log::log(format!(
        "{LOG_PREFIX} permission evidence moved to {} after {session_age_ms}ms of tap",
        next.as_str()
    ));

    let Some(infra) = app_handle.try_state::<crate::app_infra::AppInfraState>() else {
        return;
    };
    let infra = std::sync::Arc::clone(&infra);
    // Off the capture tick: the cache above is what the next permission read
    // uses, and this is only the durable copy catching up.
    tauri::async_runtime::spawn(async move {
        if let Err(error) = infra.system_audio_evidence().set_evidence(next.as_str()).await {
            super::debug_log::log(format!(
                "{LOG_PREFIX} failed to persist permission evidence: {error}"
            ));
        }
    });
}

/// The finalized file is already tracked in `current_segment_output_files`, which
/// the segment machinery validates and commits at the next boundary, so there is
/// nothing to route here. A segment that never got a sample is expected — a
/// paused-through rotation produces one every time — and the finalize path drops
/// zero-duration files on its own.
fn log_finalization(finalization: SystemAudioSegmentFinalization) {
    if let Err(error) = finalization.result {
        super::debug_log::log(format!(
            "{LOG_PREFIX} segment {} closed without usable audio: [{}] {}",
            finalization.output_file.display(),
            error.code,
            error.message
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Regression: the fold ran on the process-local cache, which `hydrate_evidence`
    // leaves at its initial value when the startup DB read fails. An install with
    // `sound_heard` persisted plus one failed read plus one quiet Mac then folded
    // `None.observe(false)` = `SilentSession` and overwrote the row — telling a
    // user whose system audio works fine that it may be blocked. `set_evidence` has
    // no compare-and-set precisely because it trusts callers to be monotonic.
    #[test]
    fn an_unhydrated_cache_never_judges_the_evidence_it_has_not_read() {
        assert_eq!(next_evidence(EVIDENCE_UNHYDRATED, false), None);
        assert_eq!(next_evidence(EVIDENCE_UNHYDRATED, true), None);
    }

    #[test]
    fn a_hydrated_cache_folds_monotonically_and_only_writes_on_a_move() {
        assert_eq!(
            next_evidence(EVIDENCE_NONE, false),
            Some(SystemAudioEvidence::SilentSession)
        );
        assert_eq!(
            next_evidence(EVIDENCE_NONE, true),
            Some(SystemAudioEvidence::SoundHeard)
        );
        assert_eq!(
            next_evidence(EVIDENCE_SILENT, true),
            Some(SystemAudioEvidence::SoundHeard)
        );

        // Already at the answer: nothing to write.
        assert_eq!(next_evidence(EVIDENCE_SILENT, false), None);
        assert_eq!(next_evidence(EVIDENCE_SOUND, true), None);
        // Terminal: a proven grant is never un-proved by a quiet session.
        assert_eq!(next_evidence(EVIDENCE_SOUND, false), None);
    }

    /// The un-hydrated sentinel must stay invisible to the permission surfaces:
    /// "we have not read it yet" and "no tap has been judged" render the same.
    #[test]
    fn an_unhydrated_cache_reads_as_no_evidence() {
        assert_eq!(
            evidence_from_cache(EVIDENCE_UNHYDRATED),
            SystemAudioEvidence::None
        );
    }

    // ── SegmentBus::take_next_output_file ──────────────────────────────────
    //
    // The one method the tap's next-output-file hook runs, so its three arms
    // decide what an inactivity pause, a caller-named segment, and a mid-segment
    // rebuild each write (or don't).

    fn test_bus_root(label: &str) -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("system-audio-bus-{label}-{unique}"))
    }

    #[test]
    fn take_next_output_file_returns_none_and_clears_live_when_paused() {
        let root = test_bus_root("paused");
        let mut bus = SegmentBus {
            planner: Some(SegmentPlanner::new(
                root.to_string_lossy().to_string(),
                "sess-paused",
            )),
            segment_index: 1,
            paused: true,
            pending_output_file: Some(root.join("pending.m4a")),
            live_output_file: Some(root.join("previous.m4a")),
        };

        assert_eq!(
            bus.take_next_output_file(),
            None,
            "a paused bus must hand the session no file to open"
        );
        assert_eq!(
            bus.live_output_file, None,
            "pausing must clear the live file — nothing is being written"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn take_next_output_file_uses_the_pending_file_and_tracks_it() {
        let root = test_bus_root("pending");
        let pending = root.join("audio").join("caller-named.m4a");
        let mut bus = SegmentBus {
            planner: None,
            segment_index: 1,
            paused: false,
            pending_output_file: Some(pending.clone()),
            live_output_file: None,
        };

        assert_eq!(
            bus.take_next_output_file(),
            Some(pending.clone()),
            "the caller-named pending file must win"
        );
        assert_eq!(
            bus.live_output_file,
            Some(pending),
            "the opened file must be tracked as live"
        );
        assert_eq!(
            bus.pending_output_file, None,
            "the pending slot is one-shot"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn take_next_output_file_synthesizes_a_resume_file_from_the_planner_when_no_pending() {
        // The rebuild arm: no caller-named path, so the bus asks the planner for
        // a collision-safe resume path under the segment it last knew about.
        let root = test_bus_root("rebuild");
        let planner = SegmentPlanner::new(root.to_string_lossy().to_string(), "sess-rebuild");
        let audio_dir = planner.audio_dir();
        let mut bus = SegmentBus {
            planner: Some(planner),
            segment_index: 3,
            paused: false,
            pending_output_file: None,
            live_output_file: None,
        };

        let next = bus
            .take_next_output_file()
            .expect("a rebuild with a planner must synthesize a path");
        assert_eq!(
            next.parent(),
            Some(audio_dir.as_path()),
            "the synthesized file must live in the planner's audio dir"
        );
        assert!(
            next.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("sess-rebuild-segment-0003-")),
            "the synthesized file must be a resume file for the current segment (got {next:?})"
        );
        assert_eq!(
            bus.live_output_file,
            Some(next),
            "the synthesized file must be tracked as live"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
