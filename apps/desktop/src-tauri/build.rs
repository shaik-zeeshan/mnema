fn main() {
    // Tauri codegen + asset/permission embedding.
    tauri_build::build();

    // Licensing slice 9: stamp the build time so the running binary can compare
    // its own release date against a License's Update Window at launch (the
    // fresh-install-after-lapse edge). Read via `option_env!("MNEMA_BUILD_DATE_MS")`.
    // ponytail: build time ≈ release date — good enough for the window gate; wire
    // the real updater-manifest `pub_date` if reproducible builds ever need exactness.
    let build_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    println!("cargo:rustc-env=MNEMA_BUILD_DATE_MS={build_ms}");

    // Licensing / ADR 0052: the fresh-install CRL floor comes from
    // `MNEMA_CRL_FLOOR` (read via `option_env!` in `crl_refresh.rs`; release CI
    // fetches the live prod CRL and exports the signed wire). Rebuild when it
    // changes so a re-release actually re-bakes the floor into the binary.
    println!("cargo:rerun-if-env-changed=MNEMA_CRL_FLOOR");
    // licensegate config (`option_env!` in `crates/app-core/src/licensing/adapter.rs`): release CI
    // bakes the prod pubkey/kid/publishable token/base URL; dev builds bake the
    // sandbox equivalents. Rebuild when any of them changes.
    println!("cargo:rerun-if-env-changed=MNEMA_LICENSE_PUBLIC_KEY");
    println!("cargo:rerun-if-env-changed=MNEMA_LICENSE_KID");
    println!("cargo:rerun-if-env-changed=MNEMA_LICENSE_PK_TOKEN");
    println!("cargo:rerun-if-env-changed=MNEMA_LICENSE_BASE_URL");
    println!("cargo:rerun-if-env-changed=MNEMA_TRIAL_LEN_DAYS");

    // The speakrs on-device diarization engine pulls in OpenBLAS, built from
    // source and linked statically via speakrs' `openblas-static` feature.
    // OpenBLAS's LAPACK is Fortran, so `openblas-src` re-emits the gfortran /
    // quadmath runtime as *dynamic* `-l` flags that resolve to Homebrew's
    // `/opt/homebrew/.../libgfortran.5.dylib` (and friends). A shipped,
    // hardened-runtime app can't load those — they're missing on clean Macs, and
    // where present they fail library validation on a Team-ID mismatch. That was
    // the v0.1.9 launch crash (`Library not loaded: .../libopenblas.0.dylib`).
    //
    // Fix: force-load the *static* Fortran runtime archives into this binary, so
    // the gfortran/quadmath symbols become regular static definitions, then
    // `-dead_strip_dylibs` so the now-unused dynamic libgfortran/libquadmath load
    // commands are dropped. Result: zero Homebrew dylib dependencies. Archive
    // paths are discovered at build time from the Fortran compiler — never baked
    // in, since they are toolchain/version specific (see CLAUDE.md).
    let target_macos = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos");
    if target_macos && std::env::var_os("CARGO_FEATURE_SPEAKER_ANALYSIS_SPEAKRS").is_some() {
        link_static_fortran_runtime();
    }
}

// Shared with the GPUI binary's build.rs: `rustc-link-arg` doesn't propagate from
// a lib, so each final binary links the static Fortran runtime itself.
include!("../../../scripts/build/fortran_static_link.rs");
