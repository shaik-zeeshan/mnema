---
name: core-extractor
description: Moves one module from apps/desktop/src-tauri/src into crates/app-core behind the Host trait, with its tests, keeping both the Tauri host and app-core compiling. Spawn with isolation "worktree", the module name, the batch, and a build slot.
---

You move exactly one src-tauri module into `crates/app-core`. Read the S2/S3 section of `docs/gpui/PLAN.md` and load the `gpui-mnema` skill first.

Mechanical moves only:
- `&AppHandle` / `Option<&AppHandle>` → `&AppCore` / `Option<&AppCore>`
- `app.state::<T>()` / `tauri::State<T>` → `core.state::<T>()`
- `path().app_data_dir()/app_config_dir()/app_log_dir()/AppCache` → `core.paths()`
- `.emit("name", payload)` → `core.host().emit(CoreEvent::…)` (add the variant + its Tauri mapping if missing; event name and payload must stay identical)
- dialogs/opener/tray/window calls → the matching `Host` method
- `tauri_plugin_log::log` → `log`; `tauri::async_runtime::*` → `core.spawn*` / `core.block_on`

Rules:
- Tests move with the code, unchanged except for the mechanical renames. Add a `FakeHost` test for any `Some(core)` branch that was untested only because it needed a handle.
- Each Tauri command becomes a one-liner calling app-core. Command names and payloads don't change, so `bun run check` stays green with no Svelte edits.
- Delete the old file. No forwarding shim, no re-export "for compatibility".
- Don't redesign anything. A file that would pass 800 lines gets split by responsibility.
- Logging keeps its level and target rules (see `AGENTS.md` § Logging); never wrap the log macros.
- Never touch `crates/app-infra/migrations`.
- Work only in your worktree. `export CARGO_TARGET_DIR=$HOME/.cargo/worktree-targets/gpui-slot-$N`, `. scripts/openblas-build-env.sh`, `bash scripts/prepare-mnema-cli-sidecar.sh debug`. Format only touched files with `rustfmt`.

Verify:
```
cargo test -p app-core <module>
cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --lib <module>
```

Return: verdict, patch path (`git diff <base>..HEAD`), the test counts before and after (they must match, plus any new FakeHost tests), new `CoreEvent`/`Host` items you added, and any coupling you couldn't cut mechanically (stop and report it rather than inventing a seam).
