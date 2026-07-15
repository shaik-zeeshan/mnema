//! The rebuild engine: the one recovery mechanism (ADR 0052).
//!
//! Everything that can go wrong with a process tap is cured the same way — throw
//! the tap and its aggregate away and build another. Nothing here tries to patch
//! a live tap back to health: IOProc restarts and aggregate-only recreates are
//! both documented as insufficient for the macOS-26 all-zero wedge.
//!
//! Four triggers converge on one path:
//!
//! - the default output device changes (property listener),
//! - the device dies (`kAudioDevicePropertyDeviceIsAlive`),
//! - the exclude list moves — a privacy-list edit, or an excluded app starting
//!   or quitting an audio process ([`SystemAudioExcludeWatcher`]),
//! - the zero-watchdog gives up on the tap ([`ZeroWatchdog`]).
//!
//! All four only *request* a rebuild; the rebuild itself happens on the caller's
//! tick, in [`SystemAudioCaptureSession::poll`]. That is not tidiness: tearing a
//! tap down blocks until any in-flight IOProc finishes, so doing it from a
//! listener block or the IOProc's own queue would deadlock against the state
//! those callbacks hold.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use capture_types::CaptureErrorResponse;
use capture_writers::derive_audio_activity_level_from_sample_buf;
use cidre::{arc, cat, core_audio as ca, dispatch};

use crate::exclude::SystemAudioExcludeWatcher;
use crate::tap::SystemAudioTapSession;
use crate::watchdog::ZeroWatchdog;
use crate::writer::{
    SystemAudioOutputContext, SystemAudioSampleBuilder, SystemAudioSegmentFinalization,
};
use crate::LOG_PREFIX;

/// The seam Slice 5 wires to the real `SegmentPlanner`.
///
/// Both hooks run on the caller's thread — never on a Core Audio queue — so they
/// are free to touch the planner, the database, or the filesystem.
pub struct SystemAudioSegmentHooks {
    /// The file the next segment writes to. `None` keeps system audio paused:
    /// the tap (and so the watchdog) stays alive with nothing being written,
    /// which is what inactivity pause is.
    pub next_output_file: Box<dyn FnMut() -> Option<PathBuf> + Send>,

    /// The segment that just closed, for the planner to commit. `result` is
    /// `Err` for a segment that never received a usable sample.
    pub segment_finalized: Box<dyn FnMut(SystemAudioSegmentFinalization) + Send>,
}

/// Why a rebuild was requested. Stable strings — they land in `rust.log` behind
/// [`LOG_PREFIX`], which is what soak analysis greps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RebuildReason {
    DefaultOutputDeviceChanged,
    DeviceDied,
    ExcludeListMoved,
    ZeroWatchdog,
}

impl RebuildReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::DefaultOutputDeviceChanged => "default_output_device_changed",
            Self::DeviceDied => "device_died",
            Self::ExcludeListMoved => "exclude_list_moved",
            Self::ZeroWatchdog => "zero_watchdog",
        }
    }
}

/// The tap generation's live state, shared with the IOProc's queue.
struct Generation {
    builder: SystemAudioSampleBuilder,
    output: SystemAudioOutputContext,
}

/// Everything a Core Audio callback may touch. The listener blocks only set the
/// rebuild request; the IOProc block owns the rest.
struct Shared {
    generation: Mutex<Option<Generation>>,
    watchdog: Mutex<ZeroWatchdog>,
    pending_rebuild: Mutex<Option<RebuildReason>>,
}

impl Shared {
    fn request_rebuild(&self, reason: RebuildReason) {
        let mut pending = self.lock_pending_rebuild();
        // First reason wins: a device change that also moves the exclude list is
        // still one rebuild, and the reason that arrived first is the true one.
        if pending.is_none() {
            *pending = Some(reason);
        }
    }

    fn take_pending_rebuild(&self) -> Option<RebuildReason> {
        self.lock_pending_rebuild().take()
    }

    // A panic on the IOProc's queue is caught at the FFI boundary, but nothing
    // can un-poison what it was holding. Every lock below is therefore taken
    // through the poison: the state behind all three is regenerable, so a caught
    // panic costs a rebuild rather than panicking the capture tick — from
    // another thread, with an unrelated message, on the next `poll`.
    fn lock_generation(&self) -> std::sync::MutexGuard<'_, Option<Generation>> {
        self.generation
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn lock_watchdog(&self) -> std::sync::MutexGuard<'_, ZeroWatchdog> {
        self.watchdog
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn lock_pending_rebuild(&self) -> std::sync::MutexGuard<'_, Option<RebuildReason>> {
        self.pending_rebuild
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// One IOProc delivery. Runs on the tap's dispatch queue.
    ///
    /// Note what is *not* checked here: whether system audio is paused for
    /// inactivity. A paused segment has no output file, so nothing is written —
    /// but the tap still delivers, the watchdog still judges it, and the
    /// inactivity machinery still gets the level that will resume it.
    fn deliver(&self, now: Instant, time: &cat::AudioTimeStamp, buffers: &[cat::AudioBuf]) {
        let level = {
            let mut generation = self.lock_generation();
            let Some(generation) = generation.as_mut() else {
                return;
            };
            let Some(sample) = generation.builder.build(time, buffers) else {
                return;
            };

            let level = derive_audio_activity_level_from_sample_buf(&sample.sample_buf);
            generation.output.append(&sample);
            level
        };

        let Some(level) = level else {
            return;
        };
        crate::activity::record_delivery(level);

        // Any non-zero sample proves the tap is alive. The probe is the same one
        // the inactivity path reads, so "silent" here means exactly what it means
        // everywhere else in capture.
        if level > 0.0 {
            self.lock_watchdog().observe_sound(now);
        }
    }
}

/// Property listeners on the audio hardware, rebuilt with each tap generation
/// because `DeviceIsAlive` has to be watched on whichever device is current.
struct DeviceChangeListeners {
    device: ca::Device,
    queue: arc::R<dispatch::Queue>,
    default_output: arc::R<ca::PropListenerBlock>,
    device_alive: arc::R<ca::PropListenerBlock>,
}

fn default_output_addr() -> ca::PropAddr {
    ca::PropSelector::HW_DEFAULT_OUTPUT_DEVICE.global_addr()
}

fn device_alive_addr() -> ca::PropAddr {
    ca::PropSelector::DEVICE_IS_ALIVE.global_addr()
}

impl DeviceChangeListeners {
    /// Registration happens *inside* a constructed `Self` so that `Drop` — the
    /// only thing that ever unregisters — covers a half-registered state too.
    /// Returning `Err` between the two `add`s would otherwise strand the first
    /// block on `ca::System::OBJ`, an object that outlives every tap: nothing
    /// would ever remove it, it would hold its `Arc<Shared>` for the life of the
    /// process, and a flapping device would strand one per failed rebuild.
    /// Removing a block that was never added is a logged no-op, so `Drop` needs
    /// no bookkeeping to tell the two apart.
    fn start(shared: &Arc<Shared>, device: ca::Device) -> Result<Self, CaptureErrorResponse> {
        let mut listeners = Self {
            device,
            queue: dispatch::Queue::serial_with_ar_pool(),
            default_output: rebuild_listener_block(
                shared,
                RebuildReason::DefaultOutputDeviceChanged,
                "default output device listener",
            ),
            device_alive: rebuild_listener_block(
                shared,
                RebuildReason::DeviceDied,
                "device liveness listener",
            ),
        };

        ca::System::OBJ
            .add_prop_listener_block(
                &default_output_addr(),
                Some(&listeners.queue),
                &mut listeners.default_output,
            )
            .map_err(|error| listener_error("add default output device listener", error))?;
        listeners
            .device
            .add_prop_listener_block(
                &device_alive_addr(),
                Some(&listeners.queue),
                &mut listeners.device_alive,
            )
            .map_err(|error| listener_error("add device liveness listener", error))?;

        Ok(listeners)
    }
}

impl Drop for DeviceChangeListeners {
    fn drop(&mut self) {
        let default_output = ca::System::OBJ.remove_prop_listener_block(
            &default_output_addr(),
            Some(&self.queue),
            &mut self.default_output,
        );
        let device_alive = self.device.remove_prop_listener_block(
            &device_alive_addr(),
            Some(&self.queue),
            &mut self.device_alive,
        );
        capture_runtime::debug_log!(
            "{LOG_PREFIX} stopped device listeners (default_output={default_output:?}, device_alive={device_alive:?})"
        );
    }
}

fn rebuild_listener_block(
    shared: &Arc<Shared>,
    reason: RebuildReason,
    label: &'static str,
) -> arc::R<ca::PropListenerBlock> {
    let shared = Arc::clone(shared);
    ca::PropListenerBlock::new2(move |_count: u32, _addresses: *const ca::PropAddr| {
        // Core Audio calls this block through a dispatch queue; a panic
        // unwinding back into it would cross an FFI boundary.
        let requested = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            shared.request_rebuild(reason);
        }));
        if requested.is_err() {
            capture_runtime::debug_log!("{LOG_PREFIX} {label} panicked");
        }
    })
}

fn listener_error(context: &str, error: impl std::fmt::Debug) -> CaptureErrorResponse {
    CaptureErrorResponse {
        code: "system_audio_tap_start_failed".to_string(),
        message: format!("{context}: {error:?}"),
    }
}

/// A live system audio capture, across as many tap generations as it takes.
pub struct SystemAudioCaptureSession {
    shared: Arc<Shared>,
    hooks: SystemAudioSegmentHooks,
    excludes: SystemAudioExcludeWatcher,
    tap: Option<SystemAudioTapSession>,
    listeners: Option<DeviceChangeListeners>,
    rebuilds: u64,
    stopped: bool,
}

impl SystemAudioCaptureSession {
    /// Builds the first tap generation and starts its first segment.
    pub fn start(
        excluded_bundle_ids: Vec<String>,
        hooks: SystemAudioSegmentHooks,
    ) -> Result<Self, CaptureErrorResponse> {
        let shared = Arc::new(Shared {
            generation: Mutex::new(None),
            watchdog: Mutex::new(ZeroWatchdog::new(Instant::now())),
            pending_rebuild: Mutex::new(None),
        });

        let signal = Arc::clone(&shared);
        let excludes = SystemAudioExcludeWatcher::start(excluded_bundle_ids, move || {
            signal.request_rebuild(RebuildReason::ExcludeListMoved)
        })?;

        let mut session = Self {
            shared,
            hooks,
            excludes,
            tap: None,
            listeners: None,
            rebuilds: 0,
            stopped: false,
        };
        session.start_generation()?;

        Ok(session)
    }

    /// The privacy-edit hook, forwarded to the exclude watcher. Touches no
    /// Core Audio itself: the edit marks the watcher dirty, and the next
    /// [`Self::poll`] reads the process list, reconciles, and performs any
    /// rebuild the move calls for.
    pub fn set_excluded_bundle_ids(&self, excluded_bundle_ids: Vec<String>) {
        self.excludes.set_excluded_bundle_ids(excluded_bundle_ids);
    }

    /// Closes the in-flight segment and opens the next one, without touching the
    /// tap. This is the seam rotation, inactivity pause, and resume all go
    /// through: pause is `next_output_file` returning `None`, resume is it
    /// returning a path again.
    pub fn advance_segment(&mut self) {
        if self.stopped || !self.has_generation() {
            return;
        }

        let next = (self.hooks.next_output_file)();
        let replaced = {
            let mut generation = self.shared.lock_generation();
            generation.as_mut().map(|generation| {
                std::mem::replace(&mut generation.output, SystemAudioOutputContext::new(next))
            })
        };

        // Outside the lock: finishing an asset writer blocks for as long as the
        // encoder needs, and the IOProc must not wait on that.
        self.finalize(replaced);
    }

    /// Drive from the existing capture tick. Performs any rebuild that came due,
    /// including the watchdog's — which is why this must be called whether or
    /// not system audio is paused for inactivity.
    ///
    /// An `Err` is a failed rebuild, not a dead session: the tap is simply gone
    /// until the next one succeeds. **Keep polling.** With no tap there is no
    /// sound, so the watchdog trips again and retries — paced by its own
    /// backoff, which is what stops a device that is genuinely gone (unplugged
    /// mid-switch) from turning into a retry storm.
    pub fn poll(&mut self) -> Result<(), CaptureErrorResponse> {
        if self.stopped {
            return Ok(());
        }

        // Before the pending request is read, so a reconcile the last Core Audio
        // read left owing gets its rebuild on this tick rather than the next.
        self.excludes.reconcile_if_dirty();

        if self.shared.lock_watchdog().poll(Instant::now()) {
            self.shared.request_rebuild(RebuildReason::ZeroWatchdog);
        }

        let Some(reason) = self.shared.take_pending_rebuild() else {
            return Ok(());
        };
        self.rebuild(reason)
    }

    /// How many tap generations this session has been through. Slice 5 surfaces
    /// it; the soak watches it for rebuild storms.
    pub fn rebuild_count(&self) -> u64 {
        self.rebuilds
    }

    /// Tears the tap down and closes the in-flight segment. Terminal: a stopped
    /// session never rebuilds, so a late tick cannot resurrect a tap the user
    /// asked to be rid of.
    pub fn stop(&mut self) {
        self.stopped = true;
        self.teardown_generation();
        let replaced = self.shared.lock_generation().take();
        self.finalize(replaced.map(|generation| generation.output));
        capture_runtime::debug_log!(
            "{LOG_PREFIX} stopped session after {} rebuild(s)",
            self.rebuild_count()
        );
    }

    fn rebuild(&mut self, reason: RebuildReason) -> Result<(), CaptureErrorResponse> {
        self.rebuilds += 1;
        capture_runtime::debug_log!(
            "{LOG_PREFIX} rebuilding tap generation (reason={}, rebuild={})",
            reason.as_str(),
            self.rebuilds
        );

        // Full teardown of *both* tap and aggregate before anything else: it is
        // the only recovery the macOS-26 wedge responds to, and it is what makes
        // the segment below a clean generation boundary rather than a splice.
        self.teardown_generation();
        let replaced = self.shared.lock_generation().take();
        self.finalize(replaced.map(|generation| generation.output));

        self.start_generation()
    }

    fn start_generation(&mut self) -> Result<(), CaptureErrorResponse> {
        let exclude_list = self.excludes.exclude_list();
        let shared = Arc::clone(&self.shared);
        let tap = SystemAudioTapSession::start(
            exclude_list.process_object_ids(),
            move |time, buffers| shared.deliver(Instant::now(), time, buffers),
        )?;

        let builder = SystemAudioSampleBuilder::new(tap.asbd())?;
        let listeners = DeviceChangeListeners::start(
            &self.shared,
            ca::System::default_output_device()
                .map_err(|error| listener_error("resolve default output device", error))?,
        )?;

        // Taken last, once nothing left can fail: `next_output_file` is not a
        // read but a handover — the caller gives up its pending path and starts
        // tracking it as the live recording file. A generation that failed after
        // taking it would leave the caller publishing a path this session never
        // opened a writer on, and committing a segment for a file that does not
        // exist. Everything above is free to fail; nothing below may.
        let output = SystemAudioOutputContext::new((self.hooks.next_output_file)());

        // The watchdog is deliberately left alone. A rebuild must not reset the
        // backoff — only real sound may, because only real sound proves the
        // rebuild worked. Resetting here would give a permanently wedged tap
        // (and an idle machine, which is silent for hours) a fresh 30-second
        // window forever, which is the rebuild storm the backoff exists to stop.
        // Tripping already restarted the window, so the new tap has its full
        // delay to prove itself.
        *self.shared.lock_generation() = Some(Generation { builder, output });

        self.tap = Some(tap);
        self.listeners = Some(listeners);

        Ok(())
    }

    /// Drops the tap and its listeners, in that order, holding no shared lock:
    /// the tap's teardown waits for any in-flight IOProc, which would otherwise
    /// be waiting on us.
    fn teardown_generation(&mut self) {
        self.tap = None;
        self.listeners = None;
    }

    fn has_generation(&self) -> bool {
        self.shared.lock_generation().is_some()
    }

    fn finalize(&mut self, output: Option<SystemAudioOutputContext>) {
        let Some(finalization) = output.and_then(SystemAudioOutputContext::finalize) else {
            return;
        };
        (self.hooks.segment_finalized)(finalization);
    }
}

impl Drop for SystemAudioCaptureSession {
    fn drop(&mut self) {
        self.teardown_generation();
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::watchdog::ZERO_WATCHDOG_INITIAL_DELAY;
    use crate::writer::fixtures::{asbd, buffers, timestamp};

    use super::*;

    fn secs(seconds: u64) -> Duration {
        Duration::from_secs(seconds)
    }

    /// A generation mid-inactivity-pause: tap alive, no output file, nothing
    /// being written.
    fn paused_generation(start: Instant) -> Shared {
        Shared {
            generation: Mutex::new(Some(Generation {
                builder: SystemAudioSampleBuilder::new(asbd(48_000.0, 2))
                    .expect("sample builder starts"),
                output: SystemAudioOutputContext::new(None),
            })),
            watchdog: Mutex::new(ZeroWatchdog::new(start)),
            pending_rebuild: Mutex::new(None),
        }
    }

    fn poll_watchdog(shared: &Shared, now: Instant) -> bool {
        shared.lock_watchdog().poll(now)
    }

    // A panic on the IOProc's dispatch queue is caught at the FFI boundary, but
    // it still poisons whatever it was holding. The tick reads the same locks
    // from another thread, where an `.expect` would turn one caught panic into a
    // capture-tick panic carrying an unrelated message.
    #[test]
    fn a_poisoned_generation_costs_a_rebuild_not_the_capture_tick() {
        let start = Instant::now();
        let shared = Arc::new(paused_generation(start));

        let poisoner = Arc::clone(&shared);
        let panicked = std::thread::spawn(move || {
            let _generation = poisoner.lock_generation();
            let _watchdog = poisoner.lock_watchdog();
            let _pending = poisoner.lock_pending_rebuild();
            panic!("io proc panicked mid-delivery");
        })
        .join();
        assert!(panicked.is_err());
        assert!(shared.generation.is_poisoned());
        assert!(shared.watchdog.is_poisoned());
        assert!(shared.pending_rebuild.is_poisoned());

        // Everything the tick and the IOProc touch still reads through.
        assert!(shared.lock_generation().is_some());
        assert!(!poll_watchdog(&shared, start));
        shared.request_rebuild(RebuildReason::DeviceDied);
        assert_eq!(
            shared.take_pending_rebuild(),
            Some(RebuildReason::DeviceDied)
        );
        shared.deliver(start, &timestamp(0.0), &buffers(&mut [0.0_f32; 256], 2));
    }

    // The watchdog is fed from the delivery path, never from the writer, so a
    // tap that wedges while system audio is paused for inactivity is still
    // caught. It has to be: the resume trigger is "sound detected", and a wedged
    // tap can never deliver the sound that would wake it — which is exactly how
    // the ScreenCaptureKit bug trapped a live session for 34 minutes.
    #[test]
    fn a_wedged_tap_is_caught_while_the_segment_is_paused() {
        let start = Instant::now();
        let shared = paused_generation(start);
        let mut zeros = vec![0.0_f32; 256];

        for second in 0..30 {
            shared.deliver(
                start + secs(second),
                &timestamp(second as f64 * 128.0),
                &buffers(&mut zeros, 2),
            );
            assert!(!poll_watchdog(&shared, start + secs(second)));
        }

        assert!(poll_watchdog(&shared, start + secs(30)));
    }

    #[test]
    fn sound_during_a_paused_segment_proves_the_tap_alive() {
        let start = Instant::now();
        let shared = paused_generation(start);
        let mut tone = vec![0.25_f32; 256];

        shared.deliver(start + secs(29), &timestamp(0.0), &buffers(&mut tone, 2));

        assert_eq!(
            shared.watchdog.lock().expect("watchdog lock").delay(),
            ZERO_WATCHDOG_INITIAL_DELAY
        );
        assert!(!poll_watchdog(&shared, start + secs(58)));
        assert!(poll_watchdog(&shared, start + secs(59)));
    }

    #[test]
    fn the_first_rebuild_reason_wins() {
        let shared = Shared {
            generation: Mutex::new(None),
            watchdog: Mutex::new(ZeroWatchdog::new(Instant::now())),
            pending_rebuild: Mutex::new(None),
        };

        shared.request_rebuild(RebuildReason::DeviceDied);
        shared.request_rebuild(RebuildReason::ZeroWatchdog);

        assert_eq!(
            shared.take_pending_rebuild(),
            Some(RebuildReason::DeviceDied)
        );
        assert_eq!(shared.take_pending_rebuild(), None);
    }

    // Rebuild reasons are grepped out of `rust.log` during the soak, so they are
    // wire format, not prose.
    #[test]
    fn rebuild_reasons_are_stable_strings() {
        assert_eq!(
            RebuildReason::DefaultOutputDeviceChanged.as_str(),
            "default_output_device_changed"
        );
        assert_eq!(RebuildReason::DeviceDied.as_str(), "device_died");
        assert_eq!(
            RebuildReason::ExcludeListMoved.as_str(),
            "exclude_list_moved"
        );
        assert_eq!(RebuildReason::ZeroWatchdog.as_str(), "zero_watchdog");
    }

    // The session is parked in capture state and driven from the capture tick.
    #[test]
    fn session_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<SystemAudioCaptureSession>();
    }
}
