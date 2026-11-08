use std::{sync::atomic::AtomicUsize, thread};

use super::*;
use crate::app_infra::test_helpers::{run_async_test, run_multithread_async_test, TestDir};

mod host;
mod intervals;
mod preview_cache;
mod preview_extract;

struct TestVideoPreviewExtractorGuard;

impl TestVideoPreviewExtractorGuard {
    /// Waits for any other test's extractor to be removed first, so the tests
    /// that install one run one at a time.
    fn install(extractor: Arc<TestVideoPreviewExtractor>) -> Self {
        loop {
            {
                let mut state = test_video_preview_extractor_state()
                    .lock()
                    .expect("test video preview extractor poisoned");
                if state.is_none() {
                    *state = Some(extractor);
                    return Self;
                }
            }
            thread::sleep(Duration::from_millis(5));
        }
    }
}

impl Drop for TestVideoPreviewExtractorGuard {
    fn drop(&mut self) {
        let mut state = test_video_preview_extractor_state()
            .lock()
            .expect("test video preview extractor poisoned");
        *state = None;
    }
}
