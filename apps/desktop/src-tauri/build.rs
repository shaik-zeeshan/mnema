use std::env;

fn main() {
    link_windows_common_controls_v6_manifest_dependency();

    // Tauri codegen + asset/permission embedding.
    tauri_build::build();

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
    //
    // This Fortran/OpenBLAS static-link is macOS-only: it exists because the
    // macOS speakrs Execution Backend (CoreML) uses OpenBLAS, whose LAPACK drags
    // in gfortran/quadmath. Windows speakrs runs the CPU backend on
    // `intel-mkl-static` (no OpenBLAS, no Fortran), so the `target_macos` guard
    // keeps the Windows build free of any Fortran toolchain requirement.
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

fn link_windows_common_controls_v6_manifest_dependency() {
    if env::var("CARGO_CFG_WINDOWS").is_err() {
        return;
    }

    println!(
        "cargo:rustc-link-arg=/MANIFESTDEPENDENCY:type='win32' \
         name='Microsoft.Windows.Common-Controls' version='6.0.0.0' \
         processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'"
    );
}
