#!/usr/bin/env bash
# bash scripts/parity.sh <surface> [state] [theme]
#
# GPUI | Tauri | mockup composites for one surface x state (x theme; both when omitted) at
# target/parity/<surface>/<state>-<theme>/composite.png. Surfaces and states:
# apps/desktop/parity/surfaces.ts. State defaults to the surface's first.
#
# GPUI side: once apps/desktop-gpui/tests/visual.rs exists, this runs
#   PARITY_DIR=<out dir> PARITY_STATE=<state> PARITY_THEME=<theme> \
#     cargo test -p mnema-gpui --test visual -- <surface>
# and the test writes <out dir>/gpui.png. Set CARGO_TARGET_DIR to your build slot first.
# Until then (or if the test writes nothing) the composite shows a grey "GPUI: not built" panel.
set -euo pipefail

if [[ $# -lt 1 ]]; then
  sed -n '2,12p' "$0" | sed 's/^# \{0,1\}//'
  exit 2
fi

repo="$(cd "$(dirname "$0")/.." && pwd)"
surface="$1" state="${2:-}" theme="${3:-}"

# Stop the Vite that shoot.ts starts if :1420 was free, so it can't serve this tree to another worktree's run.
if ! lsof -tiTCP:1420 -sTCP:LISTEN >/dev/null 2>&1; then
  trap 'kill $(lsof -tiTCP:1420 -sTCP:LISTEN) 2>/dev/null || true' EXIT
fi

dirs="$(cd "$repo/apps/desktop" && bun parity/shoot.ts "$surface" "$state" "$theme")"

gpui_test="$repo/apps/desktop-gpui/tests/visual.rs"
if [[ -f "$gpui_test" ]]; then
  # shellcheck source=/dev/null
  . "$repo/scripts/openblas-build-env.sh"
fi

while IFS= read -r dir; do
  [[ -n "$dir" ]] || continue
  rm -f "$dir/gpui.png"
  if [[ -f "$gpui_test" ]]; then
    leaf="$(basename "$dir")" # <state>-<theme>
    (cd "$repo" && PARITY_DIR="$dir" PARITY_STATE="${leaf%-*}" PARITY_THEME="${leaf##*-}" \
      cargo test -p mnema-gpui --test visual -- "$surface") ||
      echo "parity: GPUI visual test failed for $surface ($leaf); composite shows it as not built" >&2
  fi
  (cd "$repo/apps/desktop" && bun parity/compose.ts "$dir")
done <<<"$dirs"
