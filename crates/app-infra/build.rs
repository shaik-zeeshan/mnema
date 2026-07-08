use std::{env, fs, path::PathBuf};

fn main() {
    // Licensing per-env keypair split: `license_verify::license_public_key()`
    // reads this via `option_env!` to bake a dev/staging public key instead of
    // the production default. Rebuild when it changes (unset on release builds).
    println!("cargo:rerun-if-env-changed=MNEMA_LICENSE_PUBLIC_KEY");
    println!("cargo:rerun-if-changed=../../apps/desktop/src-tauri/tauri.conf.json");
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let config_path = manifest_dir
        .join("../../apps/desktop/src-tauri/tauri.conf.json")
        .clean();
    let identifier = fs::read_to_string(config_path)
        .ok()
        .and_then(|raw| extract_identifier(&raw))
        .unwrap_or_else(|| "com.shaikzeeshan.mnema".to_string());
    println!("cargo:rustc-env=MNEMA_APP_IDENTIFIER={identifier}");
}

fn extract_identifier(raw: &str) -> Option<String> {
    let key = "\"identifier\"";
    let after_key = raw.split_once(key)?.1;
    let after_colon = after_key.split_once(':')?.1.trim_start();
    let value = after_colon.strip_prefix('"')?;
    let end = value.find('"')?;
    Some(value[..end].to_string())
}

trait CleanPath {
    fn clean(self) -> Self;
}

impl CleanPath for PathBuf {
    fn clean(self) -> Self {
        let mut cleaned = PathBuf::new();
        for component in self.components() {
            match component {
                std::path::Component::ParentDir => {
                    cleaned.pop();
                }
                other => cleaned.push(other.as_os_str()),
            }
        }
        cleaned
    }
}
