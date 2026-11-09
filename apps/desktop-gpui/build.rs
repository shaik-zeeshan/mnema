fn main() {
    // speakrs links OpenBLAS (Fortran LAPACK) statically, and `openblas-src`
    // re-emits the gfortran/quadmath runtime as dynamic Homebrew dylibs a shipped
    // app can't load (the v0.1.9 launch crash). `rustc-link-arg` doesn't propagate
    // from a lib, so this binary force-links the static runtime itself, exactly as
    // `apps/desktop/src-tauri/build.rs` does. See AGENTS.md (Verification).
    let target_macos = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos");
    if target_macos && std::env::var_os("CARGO_FEATURE_SPEAKER_ANALYSIS_SPEAKRS").is_some() {
        link_static_fortran_runtime();
    }
}

include!("../../scripts/build/fortran_static_link.rs");
