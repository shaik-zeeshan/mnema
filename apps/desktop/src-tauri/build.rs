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

    // Licensing / ADR 0052: the baked CRL fetch URL comes from `MNEMA_CRL_URL`
    // (read via `option_env!` in `crl_refresh.rs`; release CI sets it to the
    // seller-owned domain). Rebuild the crate when it changes so a re-release
    // with a new URL actually re-bakes it into the binary.
    println!("cargo:rerun-if-env-changed=MNEMA_CRL_URL");

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

/// Force the gfortran/quadmath runtime (dragged in by OpenBLAS LAPACK) to link
/// statically so the binary has no Homebrew dylib dependency. macOS-only.
fn link_static_fortran_runtime() {
    println!("cargo:rerun-if-env-changed=OPENBLAS_FC");
    println!("cargo:rerun-if-env-changed=FC");

    let fc = std::env::var("OPENBLAS_FC")
        .or_else(|_| std::env::var("FC"))
        .unwrap_or_else(|_| "gfortran".to_string());

    let locate = |label: &str, query: &str| -> std::path::PathBuf {
        let output = std::process::Command::new(&fc)
            .arg(query)
            .output()
            .unwrap_or_else(|err| {
                panic!(
                    "speakrs static link: could not run `{fc} {query}` to locate {label}: {err}. \
                     Install a Fortran toolchain (`brew install gcc`) or set OPENBLAS_FC."
                )
            });
        let path = std::path::PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
        // `-print-file-name` echoes the query back verbatim when it cannot find
        // the archive, so require a real file before trusting it.
        if !path.is_file() {
            panic!(
                "speakrs static link: `{fc} {query}` did not resolve {label} to an existing \
                 archive (got {path:?}). Install a Fortran toolchain (`brew install gcc`)."
            );
        }
        path
    };

    let libgfortran = locate("libgfortran.a", "-print-file-name=libgfortran.a");
    let libquadmath = locate("libquadmath.a", "-print-file-name=libquadmath.a");
    let libgcc = locate("libgcc.a", "-print-libgcc-file-name");

    // Order matters: force-load the Fortran runtime first so its symbols become
    // static definitions, then list libgcc *lazily* (plain input, not
    // force-load) so only the members gfortran needs are pulled — chiefly
    // emulated-TLS (`___emutls_get_address`) — without duplicating symbols
    // already provided by Rust's own compiler-builtins.
    println!("cargo:rustc-link-arg=-Wl,-force_load,{}", libgfortran.display());
    println!("cargo:rustc-link-arg=-Wl,-force_load,{}", libquadmath.display());
    println!("cargo:rustc-link-arg={}", libgcc.display());
    // Drop the dynamic libgfortran/libquadmath load commands openblas-src still
    // emits: their symbols are now satisfied statically, so the dylibs are unused.
    println!("cargo:rustc-link-arg=-Wl,-dead_strip_dylibs");
}
