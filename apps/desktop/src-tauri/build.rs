use std::env;

fn main() {
    link_windows_common_controls_v6_manifest_dependency();
    tauri_build::build()
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
