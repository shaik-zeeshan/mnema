# Contributing to Mnema

Thanks for your interest! Issues and pull requests are welcome.

## Prerequisites

- macOS on Apple Silicon (the capture stack is macOS-native; see [SUPPORTS.md](SUPPORTS.md))
- [Bun](https://bun.sh) and [Rust](https://rustup.rs) (stable)
- `brew install gcc cmake` — gfortran is needed to build OpenBLAS for speaker analysis; cmake for local Whisper

## Building

All commands run from the repo root.

```sh
bun install
bash scripts/prepare-mnema-cli-sidecar.sh debug   # required once per clean checkout
bash scripts/dev-app.sh                            # run the desktop app in dev mode
```

Heads-up: the first build compiles OpenBLAS from source and is slow; later builds are incremental.

## Checks before a PR

- Frontend: `bun run check`
- Single crate: `cargo check -p <crate>` / `cargo test -p <crate>`
- Desktop Rust: `cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml`

Please don't run broad `cargo fmt` — the tree isn't rustfmt-clean and it churns unrelated files. Format only the lines you touch.

## Conventions

- Commit messages follow [Conventional Commits](https://www.conventionalcommits.org) (`feat:`, `fix:`, `chore:`, with an optional scope like `feat(insights):`).
- Architecture decisions live in `docs/adr/`; agent-facing invariants in `CLAUDE.md` / `AGENTS.md`. If a change contradicts an ADR, discuss in an issue first.
- Database schema changes go in `crates/app-infra/migrations` as new migration files.
- Platform-specific behavior changes should update [SUPPORTS.md](SUPPORTS.md).

## Reporting bugs

Use the issue templates. For anything security- or privacy-sensitive, see [SECURITY.md](SECURITY.md) instead of opening a public issue.
