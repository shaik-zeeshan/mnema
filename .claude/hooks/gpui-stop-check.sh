#!/usr/bin/env bash
# Stop: cargo check app-core + mnema-gpui, only when they exist and have uncommitted changes.
input=$(cat)
[ "$(jq -r '.stop_hook_active // false' <<<"$input")" = "true" ] && exit 0
cd "$(git rev-parse --show-toplevel)" || exit 0
pkgs=()
[ -f crates/app-core/Cargo.toml ] && [ -n "$(git status --porcelain -- crates/app-core)" ] && pkgs+=(-p app-core)
[ -f apps/desktop-gpui/Cargo.toml ] && [ -n "$(git status --porcelain -- apps/desktop-gpui)" ] && pkgs+=(-p mnema-gpui)
[ ${#pkgs[@]} -eq 0 ] && exit 0
unset RUSTUP_TOOLCHAIN  # an inherited 1.92 override would beat rust-toolchain.toml
. scripts/openblas-build-env.sh >/dev/null 2>&1
out=$(cargo check "${pkgs[@]}" --message-format short 2>&1) && exit 0
echo "cargo check ${pkgs[*]} failed:" >&2
grep -E '^(error|warning: unused)' <<<"$out" | head -30 >&2
exit 2
