#!/usr/bin/env bash
# bash scripts/gpui-feel-report.sh [rust.log]
#
# Turns the GPUI app's `mnema_gpui::feel` Debug lines (build with
# `--features feel-metrics`, e.g. scripts/dev-gpui.sh) into the BUILD-PROMPT §5
# budget table: count, p50, p95, max and the budget per metric, plus dropped
# frames. Default log: ~/Library/Logs/day.mnema.gpui-dev/rust.log.
#
# Frame lines are one per drawn+presented frame (GPUI's frame profiler):
#   work_ms  = Window::draw start (render + layout + prepaint + paint) to the
#              end of present (scene handed to Metal);
#   dropped  = gap since the previous frame > 1.5x the display refresh while the
#              window still wanted frames (see apps/desktop-gpui/src/feel.rs).
# When the log has `section start` / `section end` markers (the benches), only
# frames inside the last section count.
set -euo pipefail

log="${1:-${HOME}/Library/Logs/day.mnema.gpui-dev/rust.log}"
[[ -f "$log" ]] || { echo "gpui-feel-report: no log at $log" >&2; exit 1; }

# work_ms: p95 <= 8 (the ProMotion budget, held on every display: §5 allows 16
# off ProMotion, but 8 leaves room on any refresh rate); key_to_paint_ms: <= 1
# frame; summon_ms: Quick Recall warm; first_paint: cold start, <= Tauri.
# (BSD awk takes no newline in -v, so rows are ';'-separated.)
budgets='work_ms|8;draw_ms|-;key_to_paint_ms|16;summon_ms|50;first_paint|-'

grep -F '[mnema_gpui::feel]' "$log" | awk -v budgets="$budgets" '
  / section start/ { sectioned = 1; inside = 1; delete v; frames = 0; drops = 0; refresh = ""; next }
  / section end/ { inside = 0; next }
  sectioned && !inside { next }
  {
    for (i = 1; i <= NF; i++) {
      if ($i ~ /^(work_ms|draw_ms|key_to_paint_ms|summon_ms)=/) { split($i, kv, "="); v[kv[1]] = v[kv[1]] " " kv[2] }
      if ($i ~ /^refresh_ms=/) { split($i, kv, "="); if (kv[2] > 0) refresh = kv[2] }
      if ($i ~ /^dropped=/) { split($i, kv, "="); frames++; drops += kv[2] }
      if ($i == "first_paint") { split($(i + 1), kv, "="); v["first_paint"] = v["first_paint"] " " kv[2] }
    }
  }
  function pct(arr, n, p,   k) { k = int((n - 1) * p + 0.5) + 1; return arr[k] }
  END {
    printf "%-16s %6s %9s %9s %9s %8s  %s\n", "metric", "n", "p50", "p95", "max", "budget", "pass"
    split(budgets, rows, ";")
    for (r = 1; r in rows; r++) {
      split(rows[r], f, "|"); m = f[1]; b = f[2]
      n = split(v[m], xs, " ")
      if (n == 0) { printf "%-16s %6d %9s %9s %9s %8s  %s\n", m, 0, "-", "-", "-", b, "no data"; continue }
      for (i = 1; i <= n; i++) xs[i] += 0
      # insertion sort (small n)
      for (i = 2; i <= n; i++) { t = xs[i]; j = i - 1; while (j > 0 && xs[j] > t) { xs[j + 1] = xs[j]; j-- } xs[j + 1] = t }
      p95 = pct(xs, n, 0.95)
      ok = (b == "-") ? (m == "first_paint" ? "compare with Tauri" : "info") : (p95 <= b ? "PASS" : "FAIL")
      printf "%-16s %6d %9.2f %9.2f %9.2f %8s  %s\n", m, n, pct(xs, n, 0.5), p95, xs[n], b, ok
    }
    if (frames > 0) {
      printf "%-16s %6d %8.1f%% %9s %9s %8s  %s   (of %d frames; refresh %s ms)\n", "dropped", drops, 100 * drops / frames, "-", "-", "0", (drops == 0 ? "PASS" : "FAIL"), frames, refresh
    }
  }'
