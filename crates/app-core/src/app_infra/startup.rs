//! Deferred startup (after the window opens, on `mnema-deferred-startup`): the
//! maintenance and repair passes, then every background worker. The workspace
//! repair reads the live capture session.

use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, OnceLock,
    },
    time::Instant,
};

use super::worker_spawn::*;
use crate::app_infra::*;
use crate::{AppCore, AppInfraState};

/// What the last deferred startup repaired, for the developer window's Startup
/// row. Data only; the row's copy lives with the health rows. Stored once, at
/// the end of [`run_deferred_startup_blocking`], so it is `None` while startup
/// is still running and for a run a quit cut short. The aggregate-device reap
/// runs afterwards in `lib.rs` and is not part of it.
#[derive(Debug, Clone, Default)]
pub struct StartupReport {
    pub duration_ms: u64,
    pub vault_unlocked: bool,
    /// `"vault_denied"` or `"vault_error"`. Never the message: it can quote a secret.
    pub vault_error_code: Option<&'static str>,
    /// `None` when maintenance failed (`errors` then holds `"startup_maintenance"`).
    pub maintenance: Option<::app_infra::StartupMaintenanceSummary>,
    /// `None` when the workspace repair failed or never ran.
    pub workspaces_removed: Option<u64>,
    /// Queued OCR jobs failed because OCR is off; `None` while OCR is on.
    pub ocr_disabled_jobs_failed: Option<u64>,
    /// Short labels of the passes that failed, in run order.
    pub errors: Vec<&'static str>,
}

static STARTUP_REPORT: OnceLock<StartupReport> = OnceLock::new();

/// Set when deferred startup unwound: the report then never lands and no
/// workers spawned, which "still running" would otherwise disguise.
static STARTUP_PANICKED: AtomicBool = AtomicBool::new(false);

pub fn startup_report() -> Option<StartupReport> {
    STARTUP_REPORT.get().cloned()
}

pub fn startup_panicked() -> bool {
    STARTUP_PANICKED.load(Ordering::Relaxed)
}

/// Flags [`STARTUP_PANICKED`] if dropped during an unwind.
struct StartupPanicFlag;

impl Drop for StartupPanicFlag {
    fn drop(&mut self) {
        if std::thread::panicking() {
            STARTUP_PANICKED.store(true, Ordering::Relaxed);
        }
    }
}

/// Runs every startup task that does not need to complete before the window is
/// shown: the [`::app_infra::AppInfra::run_startup_maintenance`] passes, the
/// filesystem repair/conversion scans, OCR-disabled reconciliation, the
/// audio/speaker transcription backfill, and finally spawning the background
/// processing/retention/repair workers.
///
/// This is intended to be invoked from a dedicated background thread *after* the
/// window has opened (see `lib.rs` setup). Ordering matters and is preserved from
/// the previous synchronous path: maintenance and hidden-segment repair complete
/// before any worker (or capture auto-start) runs, and orphaned-job reconciliation
/// (inside `run_startup_maintenance`) completes before the processing workers
/// spawn — it is only safe while nothing is executing those jobs (ADR 0020).
pub fn run_deferred_startup_blocking(core: &Arc<AppCore>) {
    let _panic_flag = StartupPanicFlag;
    // Defense in depth against a quit requested before deferred startup even
    // began (see also the gate in `run_deferred_startup`): if a graceful exit is
    // already in progress, there is nothing to spin up. Bail before running any
    // maintenance/repair or spawning workers so we never start new background
    // work while the app is tearing down. Interrupted maintenance is safe to skip
    // here — it is re-run (idempotently) on the next launch.
    if crate::exit::is_graceful_exit_in_progress(core) {
        crate::debug_log::log_info(
            "graceful exit already in progress; skipping deferred startup before it began",
        );
        return;
    }

    // NOTE on #11 (duplicated state-lookup boilerplate): these two
    // `try_state ... else { log; return }` blocks superficially resemble the
    // pair in `shutdown_background_workers_for_app_exit`, but they differ in log
    // level (error vs warn), message, and the fact that shutdown only needs the
    // background-workers state while startup needs both states with different
    // failure copy. A shared helper would be more indirection than the two-line
    // pattern saves, so they are intentionally left inline.
    let Some(infra) = core.try_state::<AppInfraState>() else {
        crate::debug_log::log_error(
            "app infrastructure state was not initialized; skipping deferred startup",
        );
        return;
    };
    let infra = infra.as_ref().clone();
    let Some(background_workers) = core.try_state::<BackgroundWorkersState>() else {
        crate::debug_log::log_error(
            "background workers state was not initialized; skipping deferred startup",
        );
        return;
    };
    let background_workers = background_workers.as_ref().clone();
    let base_dir = infra.base_dir().to_path_buf();
    let started = Instant::now();
    let mut report = StartupReport::default();

    // Unlock the secret vault exactly once per process, here on the deferred
    // thread so a first-run keychain prompt never blocks the window opening.
    // A denied unlock is cached: features degrade with a distinct
    // SecretVaultDenied error instead of re-prompting per read.
    match infra.secret_vault().unlock_now() {
        Ok(()) => {
            report.vault_unlocked = true;
            crate::debug_log::log_info("secret vault unlocked");
        }
        Err(error) => {
            report.vault_error_code = Some(match error {
                ::app_infra::AppInfraError::SecretVaultDenied(_) => "vault_denied",
                _ => "vault_error",
            });
            report.errors.push("vault_unlock");
            crate::debug_log::log_error(format!(
                "secret vault unlock failed; AI/MCP secret access is disabled for this run: {error}"
            ));
        }
    }

    // #3: This runs on a detached background thread, so we cannot cleanly
    // fail-fast the app from here (the old synchronous path could). We
    // deliberately keep startup best-effort: a maintenance failure is logged at
    // ERROR level and startup proceeds to spawn the workers anyway. Aborting or
    // skipping worker spawn on a maintenance error would strand processing far
    // worse than re-running the (idempotent, SQLite-WAL-safe) maintenance on the
    // next launch. The ADR-0020 ordering still holds: orphaned-job reconciliation
    // lives inside `run_startup_maintenance` and completes (or this thread is
    // gone via a panic — see below) before the workers spawn.
    match core.block_on(infra.run_startup_maintenance()) {
        Ok(summary) => report.maintenance = Some(summary),
        Err(error) => {
            report.errors.push("startup_maintenance");
            crate::debug_log::log_error(format!(
                "startup maintenance failed (base_dir='{}'): {error}",
                base_dir.display()
            ));
        }
    }

    // If a quit was requested during maintenance, stop here rather than running
    // the remaining filesystem/backfill passes or spawning workers. We only
    // short-circuit *between* major passes, never mid-transaction, so already
    // committed work stays consistent and the rest is re-run on next launch.
    if crate::exit::is_graceful_exit_in_progress(core) {
        crate::debug_log::log_info(
            "graceful exit requested during startup maintenance; skipping remaining deferred startup",
        );
        return;
    }

    // #4: The passes below are *not* ordering-critical for ADR-0020 (the
    // reconcile already completed inside `run_startup_maintenance` above). Wrap
    // them in `catch_unwind` so a panic in the filesystem repair / sidecar
    // conversion / OCR-disabled reconciliation / audio backfill is logged and
    // still lets the processing/retention workers spawn — otherwise a bug in a
    // non-critical pass would leave the app with no background workers at all. A
    // panic *inside* `run_startup_maintenance` (which contains the reconcile)
    // intentionally remains uncaught: it unwinds this thread, is recorded by the
    // installed panic hook, and prevents worker spawn because the reconcile may
    // not have completed.
    let post_maintenance = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // FIRST, and deliberately so. `run_startup_maintenance` has just cleared every lock row
        // older than the stale threshold, which by construction includes the previous session's
        // long-lived `absent` locks. This closure's panic path intentionally still spawns the
        // workers, so any pass ahead of this one that panics would leave the absent locks cleared,
        // never re-taken, and the workers running — every job frozen onto a missing model burning
        // its three attempts for the whole session. It depends on none of the passes below.
        core.block_on(crate::model_download_lock::reconcile_absent_model_locks(
            core,
        ));
        crate::app_infra::frame_preview::run_generated_frame_preview_cache_startup_pass(core);
        report.workspaces_removed =
            run_hidden_segment_workspace_repair_startup_pass(&infra, &base_dir, core);
        if report.workspaces_removed.is_none() {
            report.errors.push("workspace_repair");
        }
        if let Ok(settings) = core
            .state::<crate::recording_settings::RecordingSettingsState>()
            .lock()
        {
            if !settings.settings.ocr.enabled {
                match core.block_on(infra.fail_queued_ocr_jobs_because_disabled()) {
                    // Counted in the "startup maintenance finished" line below.
                    Ok(failed_count) => report.ocr_disabled_jobs_failed = Some(failed_count),
                    Err(error) => {
                        report.errors.push("ocr_disabled_reconcile");
                        crate::debug_log::log_warn(format!(
                            "startup failed to mark queued OCR jobs failed while OCR is disabled: {error}"
                        ));
                    }
                }
            }
        } else {
            report.errors.push("ocr_disabled_reconcile");
            crate::debug_log::log_warn(
                "failed to read recording settings during OCR disabled startup reconciliation",
            );
        }
        run_audio_transcription_backfill_startup_pass(&infra, core);
        // Before the User Context worker spawns below, so no distillation races it.
        crate::user_context::conclusion_guard::run_dedup_startup_pass(core, &infra);
        run_lock_screen_activity_heal_startup_pass(&infra, core);
    }));
    if post_maintenance.is_err() {
        report.errors.push("post_maintenance_panic");
        // The panic itself is already recorded by the installed panic hook; we
        // intentionally continue to spawn workers because these passes are not
        // ordering-critical and stranding processing is worse than skipping them.
        crate::debug_log::log_error(
            "deferred startup post-maintenance passes panicked; continuing to spawn background workers",
        );
    }

    spawn_retention_cleanup_worker(Arc::clone(&infra), core.clone(), background_workers.clone());

    crate::user_context::spawn_user_context_worker(
        Arc::clone(&infra),
        core,
        background_workers.clone(),
    );

    // Semantic Search startup reconciliation (self-heal): if a past model switch
    // left the vec0 table holding a model that disagrees with the selection (e.g. a
    // rebuild that failed under DB contention), every search degrades to
    // keyword-only and the worker idles forever — recovery cannot come from
    // re-selecting the same model (the UI early-returns on an unchanged pick). Run
    // once here on the deferred-startup seam, BEFORE the backfill worker spawns, to
    // rebuild a stuck table under the selected model. Idempotent: a matching table
    // is left untouched (the common case). Best-effort — a failure is logged and the
    // worker still spawns (it will idle on the mismatch rather than error-loop).
    //
    // Running BEFORE the worker also matters for a fresh install: migration 0039's
    // table carries no model stamp, and the worker's store gate requires one, so
    // this pass is what stamps it before the first sweep runs. An unstamped table is
    // REBUILT, not adopted in place (its embedding recipe is unknowable — see
    // `reconcile_vectors_table`); on a fresh install that rebuild discards nothing
    // because migration 0039's table is empty.
    //
    // The selection is passed as a READER, not a snapshot: this pass queues behind an
    // in-flight Settings model switch, and that switch is exactly what changes the
    // selection while we wait. Reading it here would decide from a value the switch
    // has already superseded — re-DROPping the table it just rebuilt and stamping the
    // OLD model on it, which the write gate then rejects for the whole session.
    {
        core.block_on(
            crate::semantic_search_models::reconcile_semantic_search_index_on_startup(
                &infra,
                || crate::semantic_search_worker::effective_semantic_search_settings(core),
            ),
        );
    }

    // Semantic Index Backfill sweep-loop (issue #123): derives a Semantic Search
    // Vector for direct anchors lacking one — live capture + historical backfill
    // in one self-healing newest-first query, resumable across restarts, inert
    // until a Semantic Search Model is installed. Spawned here on the
    // deferred-startup seam (after the window opens), never on the capture hot path.
    crate::semantic_search_worker::spawn_semantic_index_backfill_worker(
        Arc::clone(&infra),
        core,
        background_workers.clone(),
    );

    // Subject Vector Backfill sweep-loop (slice 4): embeds every distinct,
    // non-dismissed User-Context Subject into the `user_context_subject_vectors`
    // table so slice 5's pre-retrieval candidate selection has vectors to KNN
    // against. Shares the one installed Semantic Search Model with the search
    // backfill above; inert (silent idle-poll) until a model is installed, then
    // self-starts retroactively. Spawned here on the deferred-startup seam, never
    // on the capture hot path.
    crate::user_context::spawn_user_context_subject_vector_worker(
        Arc::clone(&infra),
        core,
        background_workers.clone(),
    );

    // Licensing gate (ADR 0044/0045): compute the offline trial/license status,
    // cache it for synchronous reads (capture-stop, status bar), and emit the
    // `license_status` event the frontend listens for. Runs here on the
    // deferred-startup seam (after the window opens) — it never gates launch.
    crate::licensing::run_license_gate(core);

    spawn_processing_worker(infra, base_dir, core.clone(), background_workers);

    report.duration_ms = started.elapsed().as_millis() as u64;
    let summary = report.maintenance.unwrap_or_default();
    crate::debug_log::log_info(format!(
        "startup maintenance finished in {} ms (requeued={}, failed={}, workspaces_removed={}, orphaned_app_jobs={}, stale_model_locks={}, frame_equivalence={}, batches_closed={}, batches_open={}, ocr_disabled_failed={}, errors={:?})",
        report.duration_ms,
        summary.orphaned_processing_requeued,
        summary.orphaned_processing_failed,
        report.workspaces_removed.unwrap_or(0),
        summary.orphaned_app_jobs,
        summary.stale_model_locks_cleared,
        summary.frame_equivalence_backfilled,
        summary.closed_batches_reconciled,
        summary.open_batches_reconciled,
        report.ocr_disabled_jobs_failed.unwrap_or(0),
        report.errors,
    ));
    let _ = STARTUP_REPORT.set(report);
}

pub(super) fn run_audio_transcription_backfill_startup_pass(
    infra: &::app_infra::AppInfra,
    core: &AppCore,
) {
    core.block_on(
        crate::audio_transcription_backfill::run_audio_transcription_backfill_pass(
            infra, core, "startup",
        ),
    );
}

pub(super) fn run_hidden_segment_workspace_repair_startup_pass(
    infra: &::app_infra::AppInfra,
    base_dir: &Path,
    core: &AppCore,
) -> Option<u64> {
    let recordings_root =
        crate::managed_storage_layout::ManagedStorageLayout::from_base_dir(base_dir.to_path_buf())
            .recordings_root();
    let recordings_root_display = recordings_root.display().to_string();

    // This pass now runs on the deferred-startup thread *after* the window is
    // open, by which point a manual `start_native_capture` (or the recording
    // global shortcut) may already be live. Exclude any workspace the running
    // capture is using — mirroring the periodic repair worker — so we never
    // delete the just-created (still empty) workspace of an active recording.
    // Returns an empty set when no capture is running.
    let active_workspace_dirs = active_workspace_dirs_for_hidden_workspace_repair(core);
    match core.block_on(repair_hidden_segment_workspaces_once(
        infra,
        &recordings_root,
        &active_workspace_dirs,
        hidden_workspace_repair_retention_cutoff(core),
    )) {
        Ok(result) => {
            // Debug: `removed` is repeated in the "startup maintenance finished" Info line.
            crate::debug_log::log(format!(
                "startup hidden segment workspace repair completed (recordings_root='{}', scanned={}, removed={}, skipped={})",
                recordings_root_display,
                result.scanned_workspace_count,
                result.removed_workspace_count,
                result.skipped_workspace_count
            ));
            Some(result.removed_workspace_count)
        }
        Err(error) => {
            crate::debug_log::log_warn(format!(
                "startup hidden segment workspace repair failed (recordings_root='{}'): {error}",
                recordings_root_display
            ));
            None
        }
    }
}

/// Deferred-startup heal: delete Activities written entirely from lock-screen
/// frames before derivation learned to skip them (lock screen = time away).
/// Idempotent and a no-op once clean; a failure is retried on the next launch.
pub(super) fn run_lock_screen_activity_heal_startup_pass(
    infra: &::app_infra::AppInfra,
    core: &AppCore,
) {
    match core.block_on(infra.user_context().delete_lock_screen_only_activities()) {
        Ok(summary) if summary.deleted_activities > 0 => {
            crate::debug_log::log_info(format!(
                "user context removed {} lock-screen-only activit(ies), {} conclusion(s), {} digest(s)",
                summary.deleted_activities, summary.deleted_conclusions, summary.deleted_digests
            ));
        }
        Ok(_) => {}
        Err(error) => crate::debug_log::log_warn(format!(
            "lock-screen activity heal failed; retrying next launch: {error}"
        )),
    }
}
