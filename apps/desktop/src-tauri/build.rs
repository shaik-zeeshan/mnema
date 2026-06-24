use std::env;
use std::fs;
use std::path::{Path, PathBuf};

/// Sherpa speaker analysis uses `sherpa-onnx`'s `shared` feature on Windows
/// (`crates/speaker-analysis/Cargo.toml`), so `mnema.exe` links
/// `sherpa-onnx-c-api.dll` at load time and the ONNX Runtime DLLs it pulls in.
/// `sherpa-onnx-sys` drops these next to the built binary in the profile dir,
/// which is enough for `cargo run`/`tauri dev`, but a packaged NSIS install only
/// ships what Tauri bundles. Without them beside the installed `mnema.exe`,
/// Windows fails the load-time import and the whole app refuses to start.
///
/// Stage the DLLs into a tracked-but-gitignored dir under `src-tauri` so the
/// Windows-only `tauri.windows.conf.json` can declare them as bundle resources
/// (installed next to the exe). They are copied from the profile dir, where the
/// `sherpa-onnx-sys` build script — run before this dependent crate's build
/// script — has already placed them.
const SHERPA_RUNTIME_DLLS: [&str; 4] = [
    "sherpa-onnx-c-api.dll",
    "sherpa-onnx-cxx-api.dll",
    "onnxruntime.dll",
    "onnxruntime_providers_shared.dll",
];

fn main() {
    link_windows_common_controls_v6_manifest_dependency();
    stage_windows_sherpa_runtime_dlls();
    tauri_build::build()
}

fn stage_windows_sherpa_runtime_dlls() {
    if env::var("CARGO_CFG_WINDOWS").is_err() {
        return;
    }

    let Some(profile_dir) = profile_dir_from_out_dir() else {
        println!(
            "cargo:warning=could not locate the cargo profile dir to stage Sherpa runtime DLLs; \
             speaker analysis may be missing from the bundle"
        );
        return;
    };

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let staging_dir = manifest_dir.join("resources").join("windows");
    if let Err(error) = fs::create_dir_all(&staging_dir) {
        println!("cargo:warning=failed to create Sherpa DLL staging dir {staging_dir:?}: {error}");
        return;
    }

    for dll in SHERPA_RUNTIME_DLLS {
        let source = profile_dir.join(dll);
        // Re-stage whenever the upstream DLL changes (e.g. a sherpa-onnx bump).
        println!("cargo:rerun-if-changed={}", source.display());
        if !source.exists() {
            println!(
                "cargo:warning=expected Sherpa runtime DLL not found at {source:?}; \
                 the packaged Windows app may fail to start (sherpa-onnx-sys should emit it)"
            );
            continue;
        }
        let dest = staging_dir.join(dll);
        if let Err(error) = fs::copy(&source, &dest) {
            println!("cargo:warning=failed to stage Sherpa runtime DLL {source:?} -> {dest:?}: {error}");
        }
    }
}

/// The profile dir (e.g. `target/release` or `target/<triple>/release`) is three
/// levels above `OUT_DIR` (`<profile>/build/<pkg>-<hash>/out`). Deriving it from
/// `OUT_DIR` keeps this correct under custom `CARGO_TARGET_DIR` and `--target`.
fn profile_dir_from_out_dir() -> Option<PathBuf> {
    let out_dir = PathBuf::from(env::var("OUT_DIR").ok()?);
    out_dir
        .ancestors()
        .nth(3)
        .map(Path::to_path_buf)
        .filter(|p| p.is_dir())
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
