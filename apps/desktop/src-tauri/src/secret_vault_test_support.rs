//! Test-only secret vault: pins the process-global vault slot to one shared,
//! file-key-backed scratch vault so tests exercising the `app_infra` secret
//! store free functions never touch the real OS keychain.
//!
//! One shared vault per test process (not per test): the slot is process-wide,
//! so per-test installs would race under the parallel test runner. Tests must
//! therefore use accounts/ids unique to themselves.

use std::sync::{Arc, OnceLock};

pub(crate) fn install_shared_test_secret_vault() {
    static INSTALLED: OnceLock<()> = OnceLock::new();
    INSTALLED.get_or_init(|| {
        let dir =
            std::env::temp_dir().join(format!("mnema-desktop-test-vault-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("shared test vault dir should be created");
        app_infra::install_process_secret_vault(app_infra::SecretVaultHandle::with_source(
            &dir,
            Arc::new(app_infra::FileMasterKeySource::new(dir.join("master.key"))),
        ));
    });
}
