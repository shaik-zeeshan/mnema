//! Scratch dirs and runtimes for the app_infra tests.

use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub(crate) struct TestDir {
    path: PathBuf,
}

impl TestDir {
    pub(crate) fn new(label: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("desktop-tauri-{label}-{unique}"));

        fs::create_dir_all(&path).expect("test directory should be created");

        Self { path }
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

pub(crate) fn run_async_test(test: impl std::future::Future<Output = ()>) {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime should build")
        .block_on(test);
}
