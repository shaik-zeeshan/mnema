//! Slow synchronous OCR work runs off the async runtime, at utility QoS.

use crate::{OcrError, OcrResult};

/// Runs `work` on the blocking pool at utility QoS, so a slow recognition never
/// holds a runtime worker (a quit's abort only lands at an `.await`).
pub(crate) async fn run_blocking<T: Send + 'static>(
    work: impl FnOnce() -> OcrResult<T> + Send + 'static,
) -> OcrResult<T> {
    tokio::task::spawn_blocking(move || with_utility_qos(work))
        .await
        .map_err(|error| OcrError::Provider(format!("OCR worker failed to join: {error}")))?
}

/// Runs `work` at utility QoS on this thread, then puts the thread's previous
/// class back: blocking-pool threads are reused by latency-sensitive work.
#[cfg(target_os = "macos")]
fn with_utility_qos<T>(work: impl FnOnce() -> T) -> T {
    struct Restore(Option<(libc::qos_class_t, libc::c_int)>);
    impl Drop for Restore {
        fn drop(&mut self) {
            if let Some((class, relative_priority)) = self.0 {
                // SAFETY: changes only the calling thread's QoS.
                unsafe { libc::pthread_set_qos_class_self_np(class, relative_priority) };
            }
        }
    }

    let mut class = libc::qos_class_t::QOS_CLASS_UNSPECIFIED;
    let mut relative_priority = 0;
    // SAFETY: per-thread QoS FFI; the out-pointers are valid stack locals.
    let read = unsafe {
        libc::pthread_get_qos_class_np(libc::pthread_self(), &mut class, &mut relative_priority)
    };
    let _restore = Restore((read == 0).then_some((class, relative_priority)));
    // SAFETY: changes only the calling thread's QoS.
    unsafe { libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_UTILITY, 0) };
    work()
}

#[cfg(not(target_os = "macos"))]
fn with_utility_qos<T>(work: impl FnOnce() -> T) -> T {
    work()
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use libc::qos_class_t::{QOS_CLASS_UNSPECIFIED, QOS_CLASS_USER_INITIATED, QOS_CLASS_UTILITY};

    fn current_qos() -> u32 {
        let mut class = QOS_CLASS_UNSPECIFIED;
        let mut relative_priority = 0;
        unsafe {
            libc::pthread_get_qos_class_np(libc::pthread_self(), &mut class, &mut relative_priority)
        };
        class as u32
    }

    #[test]
    fn utility_qos_lasts_only_for_the_work() {
        std::thread::spawn(|| {
            unsafe { libc::pthread_set_qos_class_self_np(QOS_CLASS_USER_INITIATED, 0) };
            assert_eq!(
                super::with_utility_qos(current_qos),
                QOS_CLASS_UTILITY as u32
            );
            assert_eq!(current_qos(), QOS_CLASS_USER_INITIATED as u32);
        })
        .join()
        .expect("qos thread");
    }

    /// The quit-hang fix: aborting a task stuck in slow OCR returns at once,
    /// because the slow call sits on the blocking pool, not in the task.
    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn aborting_a_slow_recognition_returns_at_once() {
        let task = tokio::spawn(super::run_blocking(|| {
            std::thread::sleep(std::time::Duration::from_secs(5));
            Ok(())
        }));
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let started = std::time::Instant::now();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
    }
}
