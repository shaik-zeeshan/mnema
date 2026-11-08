fn main() {
    // Same stamps as `apps/desktop/src-tauri/build.rs`: `option_env!` values bake
    // into the crate that holds the code, so the licensing code that moves here
    // in S2 reads them from app-core's own build script.
    //
    // ponytail: build time ≈ release date — good enough for the update-window gate.
    let build_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    println!("cargo:rustc-env=MNEMA_BUILD_DATE_MS={build_ms}");

    println!("cargo:rerun-if-env-changed=MNEMA_CRL_FLOOR");
    println!("cargo:rerun-if-env-changed=MNEMA_LICENSE_PUBLIC_KEY");
    println!("cargo:rerun-if-env-changed=MNEMA_LICENSE_KID");
    println!("cargo:rerun-if-env-changed=MNEMA_LICENSE_PK_TOKEN");
    println!("cargo:rerun-if-env-changed=MNEMA_LICENSE_BASE_URL");
    println!("cargo:rerun-if-env-changed=MNEMA_TRIAL_LEN_DAYS");
}
