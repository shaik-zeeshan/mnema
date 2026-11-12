//! The background workers' shared control: a shutdown signal, a retention
//! schedule signal, and the handles a graceful exit aborts or awaits.

use super::*;

// Grace period for cooperative (non-abortable) workers to observe the shutdown
// signal and exit at their next between-jobs check. Queue-backed processing
// workers are aborted immediately (their in-flight job is reclaimed), so this no
// longer needs to cover a multi-minute transcription. A worker that's idle or
// between operations responds in well under a second; one blocked inside a long
// op (e.g. a user-context LLM distillation) never finishes within any practical
// window, so we give a short grace and then abort rather than stall quit for it.
// Every cooperative worker is idempotent/resumable and writes transactionally,
// so an abort here rolls back cleanly and the work re-runs on next launch.
const BACKGROUND_WORKER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

/// Runs its closure when dropped: a worker's "stopped" line still lands when a
/// graceful exit aborts it mid-job (the abort drops the worker's future).
pub(super) struct OnStop<F: FnOnce()>(Option<F>);

impl<F: FnOnce()> OnStop<F> {
    pub(super) fn new(f: F) -> Self {
        Self(Some(f))
    }
}

impl<F: FnOnce()> Drop for OnStop<F> {
    fn drop(&mut self) {
        if let Some(f) = self.0.take() {
            f();
        }
    }
}

#[derive(Clone)]
pub struct BackgroundWorkersControl {
    inner: Arc<BackgroundWorkersControlInner>,
}

pub(super) struct BackgroundWorkersControlInner {
    shutdown_requested: AtomicBool,
    shutdown_tx: watch::Sender<bool>,
    retention_schedule_version: AtomicU64,
    retention_schedule_tx: watch::Sender<u64>,
    tasks: Mutex<Vec<JoinHandle<()>>>,
    // Workers whose in-flight unit is a queue-backed Processing Job that
    // **Processing Job Reclamation** requeues on the next launch. At quit there
    // is nothing to drain cooperatively — letting them finish the current job
    // only stalls quit by seconds (a multi-minute transcription up to the
    // shutdown timeout) for work that gets requeued anyway. These are aborted
    // immediately instead of awaited.
    abortable_tasks: Mutex<Vec<JoinHandle<()>>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct BackgroundWorkerShutdownSummary {
    tracked_tasks: usize,
    timed_out_tasks: usize,
    aborted_immediately_tasks: usize,
}

impl Default for BackgroundWorkersControl {
    fn default() -> Self {
        let (shutdown_tx, _) = watch::channel(false);
        let (retention_schedule_tx, _) = watch::channel(0);
        Self {
            inner: Arc::new(BackgroundWorkersControlInner {
                shutdown_requested: AtomicBool::new(false),
                shutdown_tx,
                retention_schedule_version: AtomicU64::new(0),
                retention_schedule_tx,
                tasks: Mutex::new(Vec::new()),
                abortable_tasks: Mutex::new(Vec::new()),
            }),
        }
    }
}

impl BackgroundWorkersControl {
    pub fn subscribe(&self) -> watch::Receiver<bool> {
        self.inner.shutdown_tx.subscribe()
    }

    pub fn subscribe_retention_schedule(&self) -> watch::Receiver<u64> {
        self.inner.retention_schedule_tx.subscribe()
    }

    pub fn notify_retention_schedule_changed(&self) {
        let version = self
            .inner
            .retention_schedule_version
            .fetch_add(1, Ordering::SeqCst)
            .saturating_add(1);
        let _ = self.inner.retention_schedule_tx.send(version);
    }

    pub fn track(&self, handle: JoinHandle<()>) {
        if self.inner.shutdown_requested.load(Ordering::SeqCst) {
            handle.abort();
            return;
        }

        let mut tasks = self
            .inner
            .tasks
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        tasks.push(handle);
    }

    /// Track a worker whose in-flight unit is a queue-backed Processing Job. At
    /// graceful shutdown these are aborted immediately rather than awaited,
    /// since whatever they were mid-flight on is requeued by **Processing Job
    /// Reclamation** on the next launch. See `abortable_tasks`.
    pub fn track_abortable(&self, handle: JoinHandle<()>) {
        if self.inner.shutdown_requested.load(Ordering::SeqCst) {
            handle.abort();
            return;
        }

        let mut tasks = self
            .inner
            .abortable_tasks
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        tasks.push(handle);
    }

    fn begin_shutdown(&self) {
        if self.inner.shutdown_requested.swap(true, Ordering::SeqCst) {
            return;
        }

        let _ = self.inner.shutdown_tx.send(true);
    }

    async fn shutdown(&self, timeout: Duration) -> BackgroundWorkerShutdownSummary {
        self.begin_shutdown();

        // Abort queue-backed processing workers up front: their current job is
        // requeued by reclamation, so there is nothing to drain — awaiting them
        // only stalls quit while a job (up to a multi-minute transcription)
        // finishes. abort() cancels at the next await point; we still await the
        // join so nothing is executing when reclamation runs afterward.
        let mut abortable_tasks = {
            let mut tasks = self
                .inner
                .abortable_tasks
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            std::mem::take(&mut *tasks)
        };
        let aborted_immediately_tasks = abortable_tasks.len();
        for handle in abortable_tasks.drain(..) {
            handle.abort();
            let _ = handle.await;
        }

        let mut tasks = {
            let mut tasks = self
                .inner
                .tasks
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            std::mem::take(&mut *tasks)
        };
        let tracked_tasks = tasks.len();
        let deadline = tokio::time::Instant::now() + timeout;
        let mut timed_out_tasks = 0usize;

        for mut handle in tasks.drain(..) {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                timed_out_tasks = timed_out_tasks.saturating_add(1);
                handle.abort();
                let _ = handle.await;
                continue;
            }

            match tokio::time::timeout(remaining, &mut handle).await {
                Ok(_) => {}
                Err(_) => {
                    timed_out_tasks = timed_out_tasks.saturating_add(1);
                    handle.abort();
                    let _ = handle.await;
                }
            }
        }

        BackgroundWorkerShutdownSummary {
            tracked_tasks,
            timed_out_tasks,
            aborted_immediately_tasks,
        }
    }
}

pub async fn shutdown_background_workers_for_app_exit(core: &AppCore) {
    let Some(background_workers) = core.try_state::<BackgroundWorkersState>() else {
        crate::debug_log::log_warn(
            "background workers state was not initialized during app exit; skipping shutdown",
        );
        return;
    };
    let background_workers = background_workers.as_ref().clone();

    crate::debug_log::log_info(format!(
        "requesting app infrastructure background worker shutdown (timeout_ms={})",
        BACKGROUND_WORKER_SHUTDOWN_TIMEOUT.as_millis()
    ));

    let summary = background_workers
        .shutdown(BACKGROUND_WORKER_SHUTDOWN_TIMEOUT)
        .await;

    crate::debug_log::log_info(format!(
        "app infrastructure background worker shutdown completed (tracked_tasks={}, timed_out_tasks={}, aborted_immediately_tasks={})",
        summary.tracked_tasks, summary.timed_out_tasks, summary.aborted_immediately_tasks
    ));

    // Workers are now aborted and awaited, so nothing is executing: reclaim any job a worker was
    // mid-flight on (Processing Job Reclamation) so a normal quit requeues it for the next launch
    // rather than stranding it as an Orphaned Processing Job. We keep the short shutdown timeout
    // and requeue-and-exit rather than block quit on a multi-minute transcription.
    crate::startup::step(core, "requeue_orphaned_jobs");
    match core.try_state::<AppInfraState>() {
        Some(infra) => match infra.reconcile_orphaned_processing_jobs().await {
            Ok(reclamation) => crate::debug_log::log_info(format!(
                "graceful shutdown reclaimed in-flight processing jobs (requeued={}, failed_on_ceiling={})",
                reclamation.requeued, reclamation.failed_on_ceiling
            )),
            Err(error) => crate::debug_log::log_warn(format!(
                "graceful shutdown failed to reclaim in-flight processing jobs: {error}"
            )),
        },
        None => crate::debug_log::log_warn(
            "app infrastructure state was not initialized during app exit; skipping processing job reclamation",
        ),
    }
}

pub async fn shutdown_aware_sleep(
    shutdown_rx: &mut watch::Receiver<bool>,
    duration: Duration,
) -> bool {
    if *shutdown_rx.borrow() {
        return true;
    }

    match tokio::time::timeout(duration, shutdown_rx.changed()).await {
        Ok(Ok(())) => *shutdown_rx.borrow(),
        Ok(Err(_)) => true,
        Err(_) => false,
    }
}

/// Runs `work` until it finishes or shutdown is requested, whichever is first;
/// `None` means shutdown won and `work` was dropped mid-flight. For a worker
/// whose unit of work (an AI call) is redone next launch, so quit never waits on it.
pub async fn until_shutdown<T>(
    shutdown_rx: &mut watch::Receiver<bool>,
    work: impl std::future::Future<Output = T>,
) -> Option<T> {
    if *shutdown_rx.borrow() {
        return None;
    }
    tokio::select! {
        output = work => Some(output),
        _ = shutdown_rx.changed() => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetentionSleepOutcome {
    Elapsed,
    ScheduleChanged,
    Shutdown,
}

pub async fn retention_schedule_aware_sleep(
    shutdown_rx: &mut watch::Receiver<bool>,
    retention_schedule_rx: &mut watch::Receiver<u64>,
    duration: Duration,
) -> RetentionSleepOutcome {
    if *shutdown_rx.borrow() {
        return RetentionSleepOutcome::Shutdown;
    }

    let shutdown_changed = shutdown_rx.changed();
    let retention_schedule_changed = retention_schedule_rx.changed();
    let sleep = tokio::time::sleep(duration);
    pin_mut!(shutdown_changed, retention_schedule_changed, sleep);

    match select(shutdown_changed, select(retention_schedule_changed, sleep)).await {
        Either::Left((Ok(()), _)) => RetentionSleepOutcome::Shutdown,
        Either::Left((Err(_), _)) => RetentionSleepOutcome::Shutdown,
        Either::Right((Either::Left((Ok(()), _)), _)) => RetentionSleepOutcome::ScheduleChanged,
        Either::Right((Either::Left((Err(_), _)), _)) => RetentionSleepOutcome::Elapsed,
        Either::Right((Either::Right(((), _)), _)) => RetentionSleepOutcome::Elapsed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn until_shutdown_drops_in_flight_work_at_once() {
        tokio::runtime::Runtime::new()
            .expect("test runtime starts")
            .block_on(async {
                let (tx, mut rx) = watch::channel(false);
                assert_eq!(until_shutdown(&mut rx, async { 7 }).await, Some(7));

                let started = Instant::now();
                let stop = tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    let _ = tx.send(true);
                });
                let hung = until_shutdown(&mut rx, std::future::pending::<()>()).await;
                assert_eq!(hung, None);
                assert!(started.elapsed() < Duration::from_secs(1));
                stop.await.unwrap();
                assert_eq!(until_shutdown(&mut rx, async { 7 }).await, None);
            });
    }

    // Quit must not stall while a processing worker finishes its current job.
    // An abortable (queue-backed) worker stuck in a long unit of work has to be
    // aborted immediately — its in-flight job is reclaimed on next launch — while
    // a cooperative worker that watches the shutdown signal still drains cleanly.
    #[test]
    fn shutdown_aborts_abortable_workers_without_awaiting_their_current_job() {
        tokio::runtime::Runtime::new()
            .expect("test runtime starts")
            .block_on(async {
                let control = BackgroundWorkersControl::default();

                // Cooperative worker: observes the shutdown signal and exits promptly.
                let mut rx = control.subscribe();
                let cooperative_exited = Arc::new(AtomicBool::new(false));
                let cooperative_flag = cooperative_exited.clone();
                control.track(tokio::spawn(async move {
                    let _ = rx.changed().await;
                    cooperative_flag.store(true, Ordering::SeqCst);
                }));

                // Abortable worker: models a processing worker stuck inside a long
                // `process_once` that never reaches the top-of-loop shutdown check.
                // Only abort() ends it.
                let abortable_finished = Arc::new(AtomicBool::new(false));
                let abortable_flag = abortable_finished.clone();
                // Its "stopped" line (an `OnStop`) must still run, before shutdown returns.
                let stop_logged = Arc::new(AtomicBool::new(false));
                let stop_flag = stop_logged.clone();
                let stop = OnStop::new(move || stop_flag.store(true, Ordering::SeqCst));
                control.track_abortable(tokio::spawn(async move {
                    let _stop = stop;
                    tokio::time::sleep(Duration::from_secs(3600)).await;
                    abortable_flag.store(true, Ordering::SeqCst);
                }));

                let started = Instant::now();
                let summary = control.shutdown(Duration::from_secs(15)).await;
                let elapsed = started.elapsed();

                assert!(
                elapsed < Duration::from_secs(1),
                "shutdown must not block on the abortable worker's current job: took {elapsed:?}"
            );
                assert_eq!(summary.aborted_immediately_tasks, 1);
                assert_eq!(summary.tracked_tasks, 1);
                assert_eq!(summary.timed_out_tasks, 0);
                assert!(
                    !abortable_finished.load(Ordering::SeqCst),
                    "abortable worker must be cancelled mid-job, not run to completion"
                );
                assert!(
                    stop_logged.load(Ordering::SeqCst),
                    "an aborted worker still logs its stop line before shutdown returns"
                );
                assert!(
                    cooperative_exited.load(Ordering::SeqCst),
                    "cooperative worker should observe the shutdown signal and exit"
                );
            });
    }
}
