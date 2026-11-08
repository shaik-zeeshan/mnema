//! `cargo run -p app-core --example dump_fixture -- <fixture> <out.json>`
//!
//! Seeds `<fixture>` in a temp dir, calls every read command app-core can reach so
//! far, and writes the `command → JSON` map the parity stub serves to Svelte. The
//! keys are Tauri command names; a command joins this list when its code (and
//! return type) moves into app-core or a crate it depends on (S2/S3).
//! Seed-dir paths in values are written as `$SEED`.

use app_core::fixtures::{seed, FixtureId};
use app_infra::AppInfra;
use serde_json::{Map, Value};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [fixture, out] = args.as_slice() else {
        return Err("usage: dump_fixture <empty|populated> <out.json>".into());
    };
    let id = FixtureId::from_name(fixture).ok_or_else(|| format!("unknown fixture {fixture}"))?;
    let dir = tempfile::tempdir()?;
    let paths = seed(dir.path(), id)?;

    let runtime = tokio::runtime::Runtime::new()?;
    let map = runtime.block_on(async {
        let infra = AppInfra::initialize_read_only(&paths.data).await?;
        let mut map = Map::new();
        map.insert(
            "get_app_infra_status".into(),
            serde_json::to_value(infra.status().await?)?,
        );
        map.insert(
            "list_searchable_apps".into(),
            serde_json::to_value(infra.list_searchable_apps().await?)?,
        );
        Ok::<_, Box<dyn std::error::Error>>(map)
    })?;

    let json = serde_json::to_string_pretty(&Value::Object(map))?
        .replace(&dir.path().display().to_string(), "$SEED");
    std::fs::write(out, json + "\n")?;
    Ok(())
}
