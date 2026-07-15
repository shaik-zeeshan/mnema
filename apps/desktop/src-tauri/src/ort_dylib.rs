//! Windows dynamic-ORT runtime wiring (#137, Slice 1).
//!
//! On Windows the binary is built with `ort/load-dynamic` (see
//! `crates/audio-transcription/Cargo.toml`), so ONNX Runtime is loaded from a
//! `onnxruntime.dll` at runtime rather than statically linked. `ort` resolves the
//! library from the `ORT_DYLIB_PATH` env var on first use (falling back to the
//! bare name `onnxruntime.dll`, looked up against `current_exe().parent()` and
//! then the OS search path), and rejects any runtime whose MINOR version is < 24.
//!
//! We bundle a version-locked `onnxruntime.dll` flat next to the exe (staged by
//! `scripts/prepare-ort-dylibs.mjs`, bundled via `tauri.windows.conf.json`). This
//! helper pins `ORT_DYLIB_PATH` to that exe-adjacent copy so the load is
//! unambiguous — it can never be hijacked by an unrelated `onnxruntime.dll` that
//! happens to sit earlier on `PATH`.
//!
//! It MUST run before the first ORT use in BOTH process roles, which both enter
//! through `main()`:
//!   - the main app (parakeet transcription runs in-process, `Session::builder()`
//!     in `audio-transcription`), and
//!   - the speaker-analysis helper, a re-invocation of THIS exe
//!     (`maybe_run_subprocess_helper_and_exit`) that runs speakrs. Because the
//!     helper is the same `mnema.exe`, its `current_exe().parent()` is the same
//!     install dir holding the DLLs.
//!
//! It is idempotent (only sets the var when unset) and side-effect-light, so it
//! is safe to call from several entry points defensively.

/// Point `ORT_DYLIB_PATH` at the bundled, exe-adjacent `onnxruntime.dll` if it is
/// present and the var is not already set. Windows-only.
#[cfg(windows)]
pub fn ensure_ort_dylib_path() {
    // Respect an existing override (operator/CI may point at a specific DLL, e.g.
    // when debugging a different ONNX Runtime build). `ort` reads this lazily on
    // first use, so setting it now — at process start, before any worker threads —
    // is in time and free of the env-var data race that mid-run mutation risks.
    if std::env::var_os("ORT_DYLIB_PATH").is_some_and(|value| !value.is_empty()) {
        return;
    }

    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let Some(exe_dir) = exe.parent() else {
        return;
    };

    // Primary: flat next to the exe (where `tauri.windows.conf.json` bundles it
    // and where the dev/run staging copy lands). The `ort/` subdir is a defensive
    // fallback in case a future packaging step nests the resource.
    for candidate in [
        exe_dir.join("onnxruntime.dll"),
        exe_dir.join("ort").join("onnxruntime.dll"),
    ] {
        if candidate.is_file() {
            // Edition 2021: `set_var` is safe. We are at process start (no other
            // threads observe the env yet).
            std::env::set_var("ORT_DYLIB_PATH", &candidate);
            return;
        }
    }
    // No bundled DLL found: leave `ORT_DYLIB_PATH` unset and let `ort` fall back to
    // its default `onnxruntime.dll` search. A genuinely missing runtime surfaces as
    // `ort`'s own clear load error on first use, not a silent misconfiguration here.
}

/// No-op off Windows: macOS parakeet links ONNX Runtime statically
/// (`download-binaries`) so there is no dylib to resolve, and macOS speakrs is
/// native CoreML (no `ort`). There is nothing to point at.
#[cfg(not(windows))]
pub fn ensure_ort_dylib_path() {}
