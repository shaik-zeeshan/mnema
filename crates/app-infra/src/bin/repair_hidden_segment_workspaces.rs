use std::path::PathBuf;

fn main() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("repair runtime should build");

    if let Err(error) = runtime.block_on(run()) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let Some(base_dir_arg) = args.next() else {
        return Err(
            "usage: cargo run -p app-infra --bin repair_hidden_segment_workspaces -- <base-dir>"
                .to_string(),
        );
    };
    if args.next().is_some() {
        return Err(
            "usage: cargo run -p app-infra --bin repair_hidden_segment_workspaces -- <base-dir>"
                .to_string(),
        );
    }

    let base_dir = PathBuf::from(base_dir_arg);
    let recordings_root = base_dir.join("recordings");
    let infra = app_infra::AppInfra::initialize(&base_dir)
        .await
        .map_err(|error| {
            format!(
                "failed to initialize app infra at {}: {error}",
                base_dir.display()
            )
        })?;
    let result = infra
        .repair_hidden_segment_workspaces(&recordings_root)
        .await
        .map_err(|error| {
            format!(
                "failed to repair hidden segment workspaces under {}: {error}",
                recordings_root.display()
            )
        })?;

    println!(
        "scanned={} removed={} skipped={}",
        result.scanned_workspace_count,
        result.removed_workspace_count,
        result.skipped_workspace_count
    );

    Ok(())
}
