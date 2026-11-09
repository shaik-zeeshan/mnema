// `include!`d by each desktop binary's build.rs (`apps/desktop/src-tauri/build.rs`
// and `apps/desktop-gpui/build.rs`). Build scripts only: it prints cargo directives.
//
// Why: OpenBLAS (speakrs' diarization engine) is built from source with a Fortran
// LAPACK, and `openblas-src` re-emits the gfortran/quadmath runtime as dynamic
// `-l` flags that resolve to Homebrew dylibs a shipped app can't load (the v0.1.9
// launch crash). See the caller's comment and AGENTS.md.

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
