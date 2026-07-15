//! The tap exclude list: Mnema's own process plus the privacy-listed apps.
//!
//! Excluding privacy-listed apps is parity, not a feature — ScreenCaptureKit's
//! content filter already silenced their audio (ADR 0052), so the matching rule
//! here deliberately mirrors `plan_privacy_content_filter` in `capture-screen`.

use std::collections::BTreeSet;

#[cfg(target_os = "macos")]
use std::panic::AssertUnwindSafe;
#[cfg(target_os = "macos")]
use std::sync::{Arc, Mutex};

#[cfg(target_os = "macos")]
use capture_types::CaptureErrorResponse;
#[cfg(target_os = "macos")]
use cidre::{arc, core_audio as ca, dispatch};

#[cfg(target_os = "macos")]
use crate::LOG_PREFIX;

/// A Core Audio process object, reduced to the two identities that matter: the
/// object id a tap exclude list is written in, and the bundle id the privacy
/// list is written in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioProcess {
    pub object_id: u32,
    pub bundle_id: Option<String>,
}

/// The set of Core Audio process objects a tap generation must exclude.
///
/// Sorted and deduplicated, so equality is order-independent: Core Audio makes
/// no promise about the order of the process list, and an order-sensitive
/// compare would read a reshuffle as a change and rebuild the tap for nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExcludeList {
    process_object_ids: Vec<u32>,
}

impl ExcludeList {
    /// Ready to hand to [`crate::SystemAudioTapSession::start`].
    pub fn process_object_ids(&self) -> &[u32] {
        &self.process_object_ids
    }

    /// A tap generation only goes stale when the set of process objects it must
    /// exclude moves. This is what bounds the churn: the audio-process list
    /// changes constantly (every app that touches audio joins and leaves it),
    /// but a rebuild is only due when the change lands on an excluded app.
    pub fn rebuild_needed(&self, next: &Self) -> bool {
        self != next
    }
}

/// Maps the privacy list's identity onto Core Audio process objects.
///
/// The privacy list identifies apps by **bundle id** (`ExcludedAppEntry::bundle_id`,
/// resolved to `PrivacyFilterDecision::excluded_bundle_ids` by `evaluate_privacy`),
/// and matching here is exact, case-sensitive bundle-id equality against every
/// process object — the same rule `plan_privacy_content_filter` applies to
/// ScreenCaptureKit's running apps today. Every process object carrying an
/// excluded bundle id is excluded, so an app with several audio processes is
/// covered by all of them; a helper process is covered only when it reports its
/// parent's bundle id, which is exactly ScreenCaptureKit's behavior and so
/// preserves the parity bar either way.
///
/// `own_process_object_id` is `None` only until Core Audio mints an object for
/// our pid; the next reconcile picks it up and rebuilds, so self-exclusion
/// self-heals rather than being lost for the session. A failed read is not that
/// case and never reaches here as `None` — see
/// [`crate::process::own_process_object_id`].
pub fn compute_exclude_list(
    own_process_object_id: Option<u32>,
    excluded_bundle_ids: &[String],
    processes: &[AudioProcess],
) -> ExcludeList {
    // An empty entry would otherwise match a process that reports an empty
    // bundle id and silence it for nobody's benefit.
    let excluded: BTreeSet<&str> = excluded_bundle_ids
        .iter()
        .map(|bundle_id| bundle_id.trim())
        .filter(|bundle_id| !bundle_id.is_empty())
        .collect();

    let mut process_object_ids: Vec<u32> = own_process_object_id.into_iter().collect();
    for process in processes {
        let Some(bundle_id) = process.bundle_id.as_deref() else {
            continue;
        };
        if excluded.contains(bundle_id) {
            process_object_ids.push(process.object_id);
        }
    }
    process_object_ids.sort_unstable();
    process_object_ids.dedup();

    ExcludeList { process_object_ids }
}

#[cfg(target_os = "macos")]
fn exclude_error(context: &str, error: impl std::fmt::Debug) -> CaptureErrorResponse {
    CaptureErrorResponse {
        code: "system_audio_exclude_list_failed".to_string(),
        message: format!("{context}: {error:?}"),
    }
}

#[cfg(target_os = "macos")]
fn process_list_addr() -> ca::PropAddr {
    ca::PropSelector::HW_PROCESS_OBJ_LIST.global_addr()
}

/// An empty audio-process list is a failed read, not a fact about the machine.
///
/// `ca::System::processes()` reports `Ok(vec![])` for a zero-sized property read
/// rather than an error, so the emptiness check is ours to make. It is a trust
/// boundary: an empty list computes an exclude list holding nothing but our own
/// process, which reads exactly like "every privacy-listed app just quit" and
/// would rebuild the tap into one that records them.
///
/// Standing on the wrong side of this is not symmetric — a spurious `Err` keeps
/// the current list (or refuses to start), a spurious `Ok(vec![])` records an
/// app the user asked never to be recorded. Core Audio lists every process
/// holding an audio client, not just the ones making noise, so the list is
/// never legitimately empty for a process that is itself about to tap.
#[cfg(target_os = "macos")]
fn reject_empty_process_list(
    processes: Vec<AudioProcess>,
) -> Result<Vec<AudioProcess>, CaptureErrorResponse> {
    if processes.is_empty() {
        return Err(exclude_error("read audio process list", "empty process list"));
    }
    Ok(processes)
}

/// Reads Core Audio's current audio-process list
/// (`kAudioHardwarePropertyProcessObjectList`).
///
/// Fails loudly rather than reporting an empty list — see
/// [`reject_empty_process_list`].
#[cfg(target_os = "macos")]
pub fn read_audio_processes() -> Result<Vec<AudioProcess>, CaptureErrorResponse> {
    let processes =
        ca::System::processes().map_err(|error| exclude_error("read audio process list", error))?;

    reject_empty_process_list(
        processes
            .into_iter()
            .map(|process| AudioProcess {
                object_id: process.0 .0,
                // ponytail: a process whose bundle id cannot be read is treated as
                // having none, so it never matches — same exposure ScreenCaptureKit
                // has today. Distinguish "no bundle" from "read failed" only if a
                // privacy-listed app is ever seen slipping through this way.
                bundle_id: process.bundle_id().ok().map(|bundle_id| bundle_id.to_string()),
            })
            .collect(),
    )
}

/// The whole Core Audio read a reconcile depends on, as one fallible step.
#[cfg(target_os = "macos")]
fn read_exclude_inputs() -> Result<(Option<u32>, Vec<AudioProcess>), CaptureErrorResponse> {
    Ok((
        crate::process::own_process_object_id()?,
        read_audio_processes()?,
    ))
}

#[cfg(target_os = "macos")]
struct WatcherState {
    excluded_bundle_ids: Vec<String>,
    exclude_list: ExcludeList,
    /// Set when a reconcile could not read Core Audio, so `exclude_list` is
    /// owed a recompute. Sticky on purpose: standing still after a failed read
    /// is only safe while the bundle-id set has not moved, and a privacy edit is
    /// exactly the case where it has. Retried from [`Self::reconcile_if_dirty`]
    /// on the capture tick, which bounds the window to one tick.
    dirty: bool,
}

/// A poisoned watcher lock is not a reason to take the capture tick down with
/// it: the state behind it is a cache of one Core Audio read, and the reconcile
/// that follows recomputes the whole of it.
#[cfg(target_os = "macos")]
fn lock_state(state: &Mutex<WatcherState>) -> std::sync::MutexGuard<'_, WatcherState> {
    state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Owns the live exclude list and signals when a tap rebuild is due.
///
/// One signal covers both dynamic triggers ADR 0052 names — an excluded app
/// launching or quitting an audio process, and a mid-recording privacy-list edit
/// — because both reduce to the same question: has the set of process objects to
/// exclude moved?
#[cfg(target_os = "macos")]
pub struct SystemAudioExcludeWatcher {
    state: Arc<Mutex<WatcherState>>,
    on_rebuild_needed: Arc<dyn Fn() + Send + Sync>,
    listener: arc::R<ca::PropListenerBlock>,
    queue: arc::R<dispatch::Queue>,
}

#[cfg(target_os = "macos")]
impl SystemAudioExcludeWatcher {
    /// Seeds the exclude list from the current process list and starts listening.
    /// `on_rebuild_needed` fires only on later moves, never for this seed — the
    /// caller is about to build its first tap from [`Self::exclude_list`].
    pub fn start(
        excluded_bundle_ids: Vec<String>,
        on_rebuild_needed: impl Fn() + Send + Sync + 'static,
    ) -> Result<Self, CaptureErrorResponse> {
        let exclude_list = compute_exclude_list(
            // A failed own-process read must not refuse the tap the way a failed
            // process-list read does: the list still excludes every
            // privacy-listed app, and self-exclusion self-heals on the next
            // reconcile. Refusing here would trade a few seconds of Mnema
            // hearing itself for a session with no system audio at all.
            crate::process::own_process_object_id().unwrap_or(None),
            &excluded_bundle_ids,
            &read_audio_processes()?,
        );
        let state = Arc::new(Mutex::new(WatcherState {
            excluded_bundle_ids,
            exclude_list,
            dirty: false,
        }));
        let on_rebuild_needed: Arc<dyn Fn() + Send + Sync> = Arc::new(on_rebuild_needed);

        let queue = dispatch::Queue::serial_with_ar_pool();
        let listener_state = Arc::clone(&state);
        let listener_callback = Arc::clone(&on_rebuild_needed);
        let mut listener = ca::PropListenerBlock::new2(
            move |_count: u32, _addresses: *const ca::PropAddr| {
                // Core Audio calls this block through a dispatch queue; a panic
                // unwinding back into it would cross an FFI boundary.
                let reconciled = std::panic::catch_unwind(AssertUnwindSafe(|| {
                    reconcile(&listener_state, listener_callback.as_ref());
                }));
                if reconciled.is_err() {
                    capture_runtime::debug_log!("{LOG_PREFIX} process list listener panicked");
                }
            },
        );

        ca::System::OBJ
            .add_prop_listener_block(&process_list_addr(), Some(&queue), &mut listener)
            .map_err(|error| exclude_error("add audio process list listener", error))?;

        Ok(Self {
            state,
            on_rebuild_needed,
            listener,
            queue,
        })
    }

    /// The exclude list as of the last reconcile — what the next tap generation
    /// must be built with.
    pub fn exclude_list(&self) -> ExcludeList {
        lock_state(&self.state).exclude_list.clone()
    }

    /// The privacy-edit hook: parity with the screen side's live
    /// `update_content_filter_ch` path. A mid-recording privacy-list edit lands
    /// here and signals a rebuild when it actually moves the exclude list.
    ///
    /// A Core Audio read that fails under an edit does not lose it: the reconcile
    /// leaves the watcher dirty and [`Self::reconcile_if_dirty`] retries.
    pub fn set_excluded_bundle_ids(&self, excluded_bundle_ids: Vec<String>) {
        lock_state(&self.state).excluded_bundle_ids = excluded_bundle_ids;
        reconcile(&self.state, self.on_rebuild_needed.as_ref());
    }

    /// Re-attempts a reconcile that a failed Core Audio read left owing.
    ///
    /// Driven from the capture tick rather than a timer, and a no-op unless a
    /// read actually failed — reconciling every tick would put the process-list
    /// read (a synchronous round-trip to coreaudiod, and a slow one exactly when
    /// it is failing) on the tick for nothing.
    pub fn reconcile_if_dirty(&self) {
        if !lock_state(&self.state).dirty {
            return;
        }
        reconcile(&self.state, self.on_rebuild_needed.as_ref());
    }
}

#[cfg(target_os = "macos")]
impl Drop for SystemAudioExcludeWatcher {
    fn drop(&mut self) {
        let removed = ca::System::OBJ.remove_prop_listener_block(
            &process_list_addr(),
            Some(&self.queue),
            &mut self.listener,
        );
        capture_runtime::debug_log!("{LOG_PREFIX} stopped process list listener ({removed:?})");
    }
}

/// Swaps in the freshly read exclude list, reporting whether it moved.
///
/// Split from [`reconcile`] as the half that holds the lock and touches no
/// Core Audio: the compare-and-swap is the whole privacy decision, and it is
/// worth being able to test it over plain data.
#[cfg(target_os = "macos")]
fn reconcile_state(
    state: &mut WatcherState,
    own_process_object_id: Option<u32>,
    processes: &[AudioProcess],
) -> bool {
    let next = compute_exclude_list(own_process_object_id, &state.excluded_bundle_ids, processes);
    let moved = state.exclude_list.rebuild_needed(&next);
    if moved {
        state.exclude_list = next;
    }
    state.dirty = false;
    moved
}

/// Recomputes the exclude list and reports a rebuild only when it moved.
///
/// A failed Core Audio read keeps the current list — reconciling to a shorter
/// one would rebuild the tap into one that records a privacy-listed app — but
/// keeping it is only *safe* while the bundle-id set the list was computed from
/// still holds. Under a privacy edit it does not: the user just named an app the
/// live tap does not exclude. So a failed read leaves the watcher dirty rather
/// than merely logging, and the tick retries it.
#[cfg(target_os = "macos")]
fn reconcile(state: &Mutex<WatcherState>, on_rebuild_needed: &(dyn Fn() + Send + Sync)) {
    reconcile_with(state, read_exclude_inputs(), on_rebuild_needed);
}

/// [`reconcile`] with the Core Audio read handed in, so both halves of the
/// contract — what a failed read does, and what the retry after it does — are
/// exercisable without a tap.
#[cfg(target_os = "macos")]
fn reconcile_with(
    state: &Mutex<WatcherState>,
    inputs: Result<(Option<u32>, Vec<AudioProcess>), CaptureErrorResponse>,
    on_rebuild_needed: &(dyn Fn() + Send + Sync),
) {
    let (own_process_object_id, processes) = match inputs {
        Ok(inputs) => inputs,
        Err(error) => {
            lock_state(state).dirty = true;
            capture_runtime::debug_log!(
                "{LOG_PREFIX} keeping exclude list after failed process list read, retrying on the next tick: {}",
                error.message
            );
            return;
        }
    };

    // The lock covers the pure compare-and-swap only; the callback runs outside
    // it so a rebuild can read the fresh list back without deadlocking.
    let moved = reconcile_state(&mut lock_state(state), own_process_object_id, &processes);

    if moved {
        capture_runtime::debug_log!("{LOG_PREFIX} exclude list moved, rebuild needed");
        on_rebuild_needed();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OWN: u32 = 10;

    fn process(object_id: u32, bundle_id: &str) -> AudioProcess {
        AudioProcess {
            object_id,
            bundle_id: Some(bundle_id.to_string()),
        }
    }

    fn excluded(bundle_ids: &[&str]) -> Vec<String> {
        bundle_ids.iter().map(|id| id.to_string()).collect()
    }

    fn list(excluded_bundle_ids: &[&str], processes: &[AudioProcess]) -> ExcludeList {
        compute_exclude_list(Some(OWN), &excluded(excluded_bundle_ids), processes)
    }

    #[test]
    fn own_process_is_always_excluded() {
        assert_eq!(list(&[], &[]).process_object_ids(), &[OWN]);
        assert_eq!(
            list(&["com.secret"], &[process(2, "com.unrelated")]).process_object_ids(),
            &[OWN]
        );
    }

    #[test]
    fn own_process_is_absent_until_core_audio_mints_one() {
        let list = compute_exclude_list(None, &excluded(&["com.secret"]), &[process(2, "com.secret")]);
        assert_eq!(list.process_object_ids(), &[2]);
    }

    // Self-exclusion self-heals: the process-list listener sees our own process
    // object appear and the resulting move rebuilds the tap.
    #[test]
    fn own_process_appearing_needs_a_rebuild() {
        let before = compute_exclude_list(None, &excluded(&[]), &[]);
        let after = compute_exclude_list(Some(OWN), &excluded(&[]), &[]);
        assert!(before.rebuild_needed(&after));
    }

    #[test]
    fn excluded_app_is_matched_by_bundle_id() {
        let list = list(
            &["com.secret"],
            &[process(2, "com.unrelated"), process(3, "com.secret")],
        );
        assert_eq!(list.process_object_ids(), &[3, OWN]);
    }

    #[test]
    fn excluded_app_appearing_needs_a_rebuild() {
        let before = list(&["com.secret"], &[process(2, "com.unrelated")]);
        let after = list(
            &["com.secret"],
            &[process(2, "com.unrelated"), process(3, "com.secret")],
        );
        assert!(before.rebuild_needed(&after));
    }

    #[test]
    fn excluded_app_disappearing_needs_a_rebuild() {
        let before = list(&["com.secret"], &[process(3, "com.secret")]);
        let after = list(&["com.secret"], &[]);
        assert!(before.rebuild_needed(&after));
    }

    // The reason a departing excluded app must rebuild rather than be ignored:
    // Core Audio can hand its object id to an unrelated process later, and a
    // stale exclude list would then silence the wrong app.
    #[test]
    fn recycled_object_id_does_not_keep_an_unrelated_app_excluded() {
        let before = list(&["com.secret"], &[process(3, "com.secret")]);
        let after = list(&["com.secret"], &[process(3, "com.unrelated")]);
        assert!(before.rebuild_needed(&after));
        assert_eq!(after.process_object_ids(), &[OWN]);
    }

    #[test]
    fn unrelated_app_churn_needs_no_rebuild() {
        let before = list(&["com.secret"], &[process(3, "com.secret")]);
        for after in [
            // An unrelated app starts playing audio.
            list(
                &["com.secret"],
                &[process(3, "com.secret"), process(4, "com.unrelated")],
            ),
            // ...and stops again, while another comes and goes.
            list(
                &["com.secret"],
                &[process(5, "com.other"), process(3, "com.secret")],
            ),
        ] {
            assert!(
                !before.rebuild_needed(&after),
                "unrelated churn must not rebuild the tap"
            );
        }
    }

    #[test]
    fn empty_privacy_list_only_rebuilds_for_our_own_process() {
        let before = list(&[], &[process(2, "com.unrelated")]);
        let after = list(&[], &[process(3, "com.other"), process(4, "com.another")]);
        assert_eq!(before.process_object_ids(), &[OWN]);
        assert!(!before.rebuild_needed(&after));
    }

    #[test]
    fn process_list_order_is_not_a_change() {
        let before = list(
            &["com.a", "com.b"],
            &[process(3, "com.a"), process(4, "com.b")],
        );
        let after = list(
            &["com.a", "com.b"],
            &[process(4, "com.b"), process(3, "com.a")],
        );
        assert!(!before.rebuild_needed(&after));
    }

    // Every process object carrying the excluded bundle id is excluded, so a
    // multi-process app is covered in full.
    #[test]
    fn every_process_sharing_an_excluded_bundle_id_is_excluded() {
        let list = list(
            &["com.google.Chrome"],
            &[
                process(3, "com.google.Chrome"),
                process(4, "com.google.Chrome"),
                process(5, "com.unrelated"),
            ],
        );
        assert_eq!(list.process_object_ids(), &[3, 4, OWN]);
    }

    // The open assumption in the plan: a helper reporting its OWN bundle id is
    // not excluded — matching is exact, exactly as ScreenCaptureKit's filter
    // matches today, so this is parity rather than a regression.
    #[test]
    fn helper_reporting_its_own_bundle_id_is_not_excluded() {
        let list = list(
            &["com.google.Chrome"],
            &[
                process(3, "com.google.Chrome"),
                process(4, "com.google.Chrome.helper"),
            ],
        );
        assert_eq!(list.process_object_ids(), &[3, OWN]);
    }

    #[test]
    fn bundle_id_matching_is_exact() {
        let list = list(
            &["com.secret"],
            &[
                process(3, "com.Secret"),
                process(4, "com.secret.extra"),
                process(5, "secret"),
                process(6, "com.secret"),
            ],
        );
        assert_eq!(list.process_object_ids(), &[6, OWN]);
    }

    #[test]
    fn blank_privacy_entries_never_match() {
        let list = list(
            &["", "   "],
            &[
                AudioProcess {
                    object_id: 3,
                    bundle_id: Some(String::new()),
                },
                AudioProcess {
                    object_id: 4,
                    bundle_id: None,
                },
            ],
        );
        assert_eq!(list.process_object_ids(), &[OWN]);
    }

    // `evaluate_privacy` trims before it ever reaches us; the pure function does
    // not rely on that being the only caller.
    #[test]
    fn privacy_entries_are_trimmed_before_matching() {
        let list = list(&["  com.secret  "], &[process(3, "com.secret")]);
        assert_eq!(list.process_object_ids(), &[3, OWN]);
    }

    #[test]
    fn duplicate_privacy_entries_do_not_duplicate_process_objects() {
        let list = list(&["com.secret", "com.secret"], &[process(3, "com.secret")]);
        assert_eq!(list.process_object_ids(), &[3, OWN]);
    }

    #[test]
    fn privacy_edit_moves_the_exclude_list() {
        let processes = [process(3, "com.secret"), process(4, "com.unrelated")];
        let before = list(&[], &processes);
        let after = list(&["com.secret"], &processes);
        assert!(before.rebuild_needed(&after));
        assert_eq!(after.process_object_ids(), &[3, OWN]);

        // ...and removing the rule moves it back.
        assert!(after.rebuild_needed(&before));
    }

    #[test]
    fn privacy_edit_for_an_app_with_no_audio_process_needs_no_rebuild() {
        let processes = [process(3, "com.unrelated")];
        let before = list(&[], &processes);
        let after = list(&["com.secret"], &processes);
        assert!(!before.rebuild_needed(&after));
    }

    // The watcher is parked in capture state and reconciled from a Core Audio
    // dispatch queue, so losing `Send` would only surface in the integrating
    // slice.
    #[cfg(target_os = "macos")]
    #[test]
    fn watcher_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<SystemAudioExcludeWatcher>();
    }

    // Core Audio reports a zero-sized property read as `Ok(vec![])`, so without
    // this check an empty read computes `[OWN]` — a tap that excludes Mnema and
    // records every privacy-listed app.
    #[cfg(target_os = "macos")]
    #[test]
    fn an_empty_process_list_is_a_failed_read_not_an_empty_exclude_list() {
        let error = reject_empty_process_list(vec![]).expect_err("an empty read must fail closed");
        assert_eq!(error.code, "system_audio_exclude_list_failed");

        let processes = reject_empty_process_list(vec![process(3, "com.secret")])
            .expect("a non-empty read passes through");
        assert_eq!(processes, vec![process(3, "com.secret")]);
    }

    #[cfg(target_os = "macos")]
    fn watcher_state(excluded_bundle_ids: &[&str], processes: &[AudioProcess]) -> WatcherState {
        WatcherState {
            excluded_bundle_ids: excluded(excluded_bundle_ids),
            exclude_list: list(excluded_bundle_ids, processes),
            dirty: false,
        }
    }

    // A privacy edit whose reconcile cannot read Core Audio must not be lost:
    // the live tap does not exclude the app the user just added, so standing
    // still is fail-open, and nothing else would come back for it — the caller
    // advances its bundle-id set regardless, and a rebuild reads only the cached
    // list. `killall coreaudiod` mid-edit is one of ADR 0052's own drills.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_privacy_edit_survives_a_failed_read_and_rebuilds_on_the_retry() {
        let processes = vec![process(3, "com.secret"), process(4, "com.unrelated")];
        let state = Mutex::new(watcher_state(&[], &processes));
        let rebuilds = Arc::new(Mutex::new(0_u32));
        let on_rebuild_needed = {
            let rebuilds = Arc::clone(&rebuilds);
            move || *rebuilds.lock().expect("counter") += 1
        };
        let rebuild_count = || *rebuilds.lock().expect("counter");

        // The user adds com.secret; the read behind the edit fails.
        lock_state(&state).excluded_bundle_ids = excluded(&["com.secret"]);
        reconcile_with(
            &state,
            Err(exclude_error("read audio process list", "coreaudiod restarting")),
            &on_rebuild_needed,
        );
        assert_eq!(rebuild_count(), 0);
        assert_eq!(
            lock_state(&state).exclude_list.process_object_ids(),
            &[OWN],
            "a failed read must not reconcile to a shorter list"
        );
        assert!(
            lock_state(&state).dirty,
            "the edit is owed a retry, not dropped"
        );

        // The tick retries against a Core Audio that answers this time.
        reconcile_with(&state, Ok((Some(OWN), processes)), &on_rebuild_needed);
        assert_eq!(rebuild_count(), 1, "the retry must rebuild the tap");
        assert_eq!(
            lock_state(&state).exclude_list.process_object_ids(),
            &[3, OWN]
        );
        assert!(!lock_state(&state).dirty);
    }

    // The retry only fires while one is owed — the process-list read is a
    // synchronous round-trip to coreaudiod and does not belong on every tick.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_successful_reconcile_leaves_nothing_owing() {
        let processes = [process(3, "com.secret")];
        let mut state = watcher_state(&["com.secret"], &processes);
        state.dirty = true;

        let moved = reconcile_state(&mut state, Some(OWN), &processes);
        assert!(!moved, "an unchanged list must not rebuild the tap");
        assert!(!state.dirty);
    }

    // `Ok(None)` is "Core Audio has not minted our object yet" and self-heals;
    // a failed read is neither, and must not be laundered into it.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_failed_own_process_read_is_not_a_missing_own_process() {
        let processes = [process(3, "com.secret")];
        let mut state = watcher_state(&["com.secret"], &processes);

        // What a `.ok()?` would have handed the compare-and-swap: self-exclusion
        // silently dropped, a rebuild spent on it, and another once it returns.
        let moved = reconcile_state(&mut state, None, &processes);
        assert!(moved);
        assert_eq!(state.exclude_list.process_object_ids(), &[3]);

        // Which is why the read error keeps the current list instead. The
        // reconcile that reads it back rebuilds self-exclusion in.
        let moved = reconcile_state(&mut state, Some(OWN), &processes);
        assert!(moved);
        assert_eq!(state.exclude_list.process_object_ids(), &[3, OWN]);
    }
}
