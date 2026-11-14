#!/usr/bin/env bash
# bash scripts/gpui-surface-bench.sh [--opt] [surface state]...
#
# Frame time per screen of the GPUI app (everything but the Timeline, which
# has scrub-bench): builds apps/desktop-gpui/examples/surface_bench.rs once,
# runs it for each named `surface state` pair (every supported one when none
# are named; `surface-bench --list` prints them), and prints one table: per
# screen and phase (scroll, hover) the frame count, work_ms p50/p95/max, the
# dropped share, and the average layout/prepaint/paint ms of the frames over
# 16.7 ms. Each run pops the bench window to the front for ~10 s.
#
#   --opt      build with opt-level 3 (still a debug-assertions dev build);
#              rebuilds every dependency the first time.
#   BENCH_DIR  where the logs go (default $CARGO_TARGET_DIR/surface-bench);
#              one <surface>-<state>/rust.log per run.
#
# Fixture data only: no app data, no keychain.
set -euo pipefail
cd "$(dirname "$0")/.."

opt=()
if [[ "${1:-}" == "--opt" ]]; then
  opt=(--config 'profile.dev.opt-level=3')
  shift
fi
(($# % 2 == 0)) || { echo "usage: $0 [--opt] [surface state]..." >&2; exit 2; }

. scripts/openblas-build-env.sh >&2
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$PWD/target}"
export MNEMA_TRIPWIRE_NO_KEYCHAIN=1
bench_dir="${BENCH_DIR:-$CARGO_TARGET_DIR/surface-bench}"
mkdir -p "$bench_dir"

cargo build ${opt[@]+"${opt[@]}"} -p mnema-gpui --features fixtures,feel-metrics \
  --example surface-bench >&2
bin="$CARGO_TARGET_DIR/debug/examples/surface-bench"

pairs="$bench_dir/pairs.txt"
if (($# == 0)); then
  "$bin" --list >"$pairs"
else
  : >"$pairs"
  while (($#)); do echo "$1 $2" >>"$pairs"; shift 2; done
fi

rows="$bench_dir/rows.txt"
: >"$rows"
while read -r surface state; do
  dir="$bench_dir/$surface-$state"
  rm -rf "$dir"
  mkdir -p "$dir"
  echo "surface-bench: $surface $state" >&2
  if ! "$bin" "$dir" "$surface" "$state" >"$dir/stdout.log" 2>&1 </dev/null; then
    echo "surface-bench: $surface $state failed; see $dir/stdout.log" >&2
    printf '%s %s failed\n' "$surface" "$state" >>"$rows"
    continue
  fi
  # Frames inside each phase's section; slow = work_ms over 16.7 ms.
  grep -F '[mnema_gpui::feel]' "$dir/rust.log" | awk -v s="$surface" -v st="$state" '
    / surface bench: phase / { phase = $NF; order[++np] = phase; next }
    / section start/ { inside = 1; next }
    / section end/ { inside = 0; next }
    !inside || phase == "" || !/ frame work_ms=/ { next }
    {
      for (i = 1; i <= NF; i++) { split($i, kv, "="); f[kv[1]] = kv[2] + 0 }
      k = ++n[phase]; w[phase, k] = f["work_ms"]; drops[phase] += f["dropped"]
      if (f["work_ms"] > 16.7) {
        slow[phase]++; lay[phase] += f["layout_ms"]; pre[phase] += f["prepaint_ms"]; pnt[phase] += f["paint_ms"]
      }
    }
    function pct(p, c, q) { return xs[int((c - 1) * q + 0.5) + 1] }
    END {
      for (o = 1; o <= np; o++) {
        p = order[o]; c = n[p] + 0
        if (c == 0) { printf "%s %s %s 0 - - - - - - -\n", s, st, p; continue }
        delete xs
        for (i = 1; i <= c; i++) xs[i] = w[p, i]
        for (i = 2; i <= c; i++) { t = xs[i]; j = i - 1; while (j > 0 && xs[j] > t) { xs[j + 1] = xs[j]; j-- } xs[j + 1] = t }
        sl = slow[p] + 0
        printf "%s %s %s %d %.2f %.2f %.2f %.1f", s, st, p, c, pct(p, c, 0.5), pct(p, c, 0.95), xs[c], 100 * drops[p] / c
        if (sl) printf " %.2f %.2f %.2f\n", lay[p] / sl, pre[p] / sl, pnt[p] / sl
        else printf " - - -\n"
      }
    }' >>"$rows"
done <"$pairs"

printf '%-13s %-22s %-6s %6s %8s %8s %8s %8s %10s %10s %10s\n' \
  screen state phase frames p50_ms p95_ms max_ms drop_% slow_lay slow_pre slow_paint
while read -r -a r; do
  if [[ "${r[2]}" == failed ]]; then
    printf '%-13s %-22s %s\n' "${r[0]}" "${r[1]}" "FAILED"
    continue
  fi
  printf '%-13s %-22s %-6s %6s %8s %8s %8s %8s %10s %10s %10s\n' "${r[@]}"
done <"$rows"
echo "(work_ms per frame; slow_* = average ms of frames over 16.7 ms; logs in $bench_dir)"
