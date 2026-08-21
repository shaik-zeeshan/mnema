#!/usr/bin/env bash
set -euo pipefail

# Day-long resource measurement for a running mnema build.
#
#   scripts/bench-day/collect.sh defaults            # run until Ctrl-C
#   scripts/bench-day/collect.sh everything --minutes 480
#
# Collects, into one run directory:
#   meta.txt          hardware, OS, app version + git SHA, the settings in force
#   power.ndjson      macmon: CPU/GPU/ANE/RAM/system watts, cluster frequencies and
#                     utilisation, die temps, RAM+swap. One JSON object per sample.
#   power.log         filtered powermetrics: the same rails as a cross-check PLUS the
#                     thing macmon cannot give — per-COALITION CPU and GPU ms/s.
#   samples.csv       every INTERVAL_SAMPLE seconds: disk by subdir, physical
#                     footprint, battery %, segment counts
#   settings-{start,end}.json   proof the config did not change mid-run
#
# Then `report.py` turns the run directory into the publishable rows.
#
# Two samplers on purpose, because neither is sufficient alone:
#   * macmon is SUDOLESS, emits clean JSON, and is already this repo's power
#     sampler of record (see crates/semantic-search/examples/thermal.rs). It is
#     whole-machine only: it cannot attribute a watt to a process.
#   * powermetrics is the only way to read CPU per COALITION — the app plus the
#     helpers it induces (WindowServer, mediaanalysisd, Apple Speech). A
#     per-process number hides exactly that, and it is what made rem look cheap
#     while mediaanalysisd burned 30-50%. It needs one sudo prompt; skip it with
#     --no-coalitions and the run still produces every other row.
#   * Running both also gives two independent power routes to converge, which is
#     what makes a published wattage believable.
#
# Other deliberate choices:
#   * Memory is `vmmap` "Physical footprint", never `ps` RSS: both the ORT arena
#     and the WebContent graphics strand are invisible to RSS.
#   * Both samplers appear in the coalition table, so their own cost is visible
#     rather than silently folded into the app's.

label="${1:?usage: collect.sh <label> [--minutes N] [--no-coalitions]}"
shift || true
minutes=""
coalitions=true
while [[ $# -gt 0 ]]; do
  case "$1" in
    --minutes) minutes="${2:?--minutes needs a value}"; shift 2 ;;
    --no-coalitions) coalitions=false; shift ;;
    *) echo "unknown arg: $1" >&2; exit 2 ;;
  esac
done

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
save_dir="${MNEMA_SAVE_DIRECTORY:-${HOME}/.mnema-bench}"
config_dir="${MNEMA_APP_CONFIG_DIR:-${HOME}/Library/Application Support/day.mnema.bench}"
# The bundle is mnema-bench.app but the executable inside it is `mnema` — the
# Rust binary target name, which the bench tauri config does not change. So the
# pid must be matched on the bundle PATH (unambiguous: it cannot match a running
# prod app, whose process is also called `mnema`), while the powermetrics filter
# matches on the process/coalition NAME.
app_match="${MNEMA_BENCH_APP_MATCH:-mnema-bench.app/Contents/MacOS/mnema}"
app_proc="${MNEMA_BENCH_PROC:-mnema}"
interval_macmon_ms="${INTERVAL_MACMON_MS:-5000}"
interval_power_ms="${INTERVAL_POWER_MS:-10000}"
interval_sample_s="${INTERVAL_SAMPLE_S:-300}"

command -v macmon >/dev/null || {
  echo "macmon not found — brew install macmon (sudoless power/thermal sampler)" >&2
  exit 1
}

out="${MNEMA_BENCH_RUNS:-${HOME}/mnema-bench-runs}/${label}-$(date +%Y%m%d-%H%M)"
mkdir -p "${out}"

pid="$(pgrep -f "${app_match}" | head -n 1 || true)"
if [[ -z "${pid}" ]]; then
  echo "no running bench app matching '${app_match}' — launch it first (scripts/bench-app.sh)" >&2
  exit 1
fi

echo "run dir: ${out}"
echo "app pid: ${pid}  ·  macmon every ${interval_macmon_ms}ms  ·  samples every ${interval_sample_s}s"
${coalitions} && echo "         powermetrics coalitions every ${interval_power_ms}ms (needs sudo once)"

# ---- meta ----------------------------------------------------------------
{
  echo "label:            ${label}"
  echo "started:          $(date -Iseconds)"
  echo "hardware:         $(sysctl -n hw.model) · $(sysctl -n machdep.cpu.brand_string) · $(( $(sysctl -n hw.memsize) / 1073741824 )) GB"
  echo "cores:            $(sysctl -n hw.perflevel0.physicalcpu)P / $(sysctl -n hw.perflevel1.physicalcpu 2>/dev/null || echo 0)E"
  echo "os:               $(sw_vers -productName) $(sw_vers -productVersion) ($(sw_vers -buildVersion))"
  echo "app binary:       $(ps -o comm= -p "${pid}")"
  echo "app pid:          ${pid}"
  echo "git sha:          $(git -C "${repo_root}" rev-parse --short HEAD 2>/dev/null || echo unknown)"
  echo "git dirty:        $(if [[ -n "$(git -C "${repo_root}" status --porcelain 2>/dev/null)" ]]; then echo yes; else echo no; fi)"
  echo "app version:      $(python3 -c "import json,sys;print(json.load(open('${repo_root}/apps/desktop/src-tauri/tauri.conf.json'))['version'])" 2>/dev/null || echo unknown)"
  echo "save dir:         ${save_dir}"
  echo "config dir:       ${config_dir}"
  echo "macmon:           $(macmon --version 2>/dev/null | head -n 1) @ ${interval_macmon_ms} ms"
  echo "coalitions req:   $(if ${coalitions}; then echo "powermetrics @ ${interval_power_ms} ms"; else echo "disabled (--no-coalitions)"; fi)"
  echo "sample interval:  ${interval_sample_s} s"
  echo "on AC at start:   $(pmset -g batt | head -n 1)"
  echo
  echo "-- other mnema processes at start (a running prod app is also called \`mnema\`;"
  echo "   if one shows up here the power and thermal rows are contaminated) --"
  pgrep -lx "${app_proc}" | grep -v "^${pid} " || echo "  (none — only the bench app)"
  echo
  echo "-- Safari WebContent processes at start (they would inflate the WebContent column) --"
  pgrep -lf "com.apple.WebKit.WebContent" | head -n 5 || echo "  (none)"
} > "${out}/meta.txt"

cp "${config_dir}/recording-settings.json" "${out}/settings-start.json" 2>/dev/null || \
  echo "warning: no recording-settings.json in ${config_dir}" >&2

# ---- macmon (sudoless, primary power/thermal series) ---------------------
macmon pipe -i "${interval_macmon_ms}" > "${out}/power.ndjson" 2> "${out}/macmon-stderr.log" &
macmon_pid=$!
coalitions_active=false

# ---- powermetrics (optional: the only per-coalition attribution) ---------
pm_pid=""
if ${coalitions}; then
  # `sudo -n` never prompts. This machine has an explicit
  # `(root) NOPASSWD: /usr/bin/powermetrics` rule, so the sampler starts silently;
  # `sudo -v` is NOT covered by that rule and would prompt for no reason.
  if sudo -n true 2>/dev/null || sudo -n /usr/bin/powermetrics --help >/dev/null 2>&1; then
    keep='^\*\*\* Sampled system activity|Power:|Power \(|^\s*Name +ID|pressure level|^read:|^write:|GPU HW active|GPU active|E-Cluster|P-Cluster'
    keep="${keep}|${app_proc}|WindowServer|mediaanalysisd|[Ss]peech|WebKit.WebContent|powermetrics"

    # stderr goes to a file, never /dev/null: a silently dead powermetrics is how
    # run 1 lost its per-coalition attribution and nobody noticed until the report.
    sudo -n /usr/bin/powermetrics \
      --samplers cpu_power,gpu_power,ane_power,tasks,thermal,disk \
      --show-process-coalition --show-process-energy --show-process-gpu \
      -i "${interval_power_ms}" \
      2> "${out}/powermetrics-stderr.log" \
      | tee >(awk 'NR<=400' > "${out}/powermetrics-format-sample.txt") \
      | grep -E --line-buffered "${keep}" > "${out}/power.log" &
    pm_pid=$!
    coalitions_active=true
  else
    echo "sudo unavailable for powermetrics — continuing without attribution." >&2
    echo "  (power, thermals, disk, memory and battery are all still collected via macmon.)" >&2
  fi
fi

cleanup() {
  # The trap fires on both INT and EXIT; without this guard the run is stamped
  # "ended" twice and the end-of-run settings are copied twice.
  [[ -n "${cleaned_up:-}" ]] && return
  cleaned_up=1
  echo
  echo "stopping…"
  kill "${macmon_pid}" 2>/dev/null || true
  if [[ -n "${pm_pid}" ]]; then
    sudo -n pkill -f "powermetrics --samplers cpu_power,gpu_power,ane_power,tasks,thermal,disk" 2>/dev/null || true
    kill "${pm_pid}" 2>/dev/null || true
  fi
  cp "${config_dir}/recording-settings.json" "${out}/settings-end.json" 2>/dev/null || true
  {
    echo
    echo "ended:            $(date -Iseconds)"
    echo "on AC at end:     $(pmset -g batt | head -n 1)"
  } >> "${out}/meta.txt"
  echo "run dir: ${out}"
  echo "report:  python3 ${repo_root}/scripts/bench-day/report.py ${out}"
}
trap cleanup EXIT INT TERM

# ---- periodic sampler ----------------------------------------------------
footprint_mb() { # $1 = pid
  vmmap --summary "$1" 2>/dev/null | awk '
    /Physical footprint:/ {
      v = $3
      unit = substr(v, length(v), 1)
      gsub(/[A-Za-z]/, "", v)
      if (unit == "G") v = v * 1024
      else if (unit == "K") v = v / 1024
      printf "%.1f", v
      exit
    }'
}

# Sum of every WebKit WebContent process. One awk at the end rather than a
# spawn per pid: a measurement harness must not be a measurable load itself.
# `vmmap` is still the slow part — measured ~2-3 s per pass on a machine with
# several WebContent processes, i.e. ~1% duty at the default 300 s interval.
# Do not shorten INTERVAL_SAMPLE_S below ~60 s without re-checking that.
webcontent_mb() {
  local p vals=""
  for p in $(pgrep -f "com.apple.WebKit.WebContent" 2>/dev/null); do
    vals="${vals} $(footprint_mb "$p")"
  done
  printf '%s' "${vals}" | awk '{ t = 0; for (i = 1; i <= NF; i++) t += $i; printf "%.1f", t }'
}

# ---- verify the samplers are actually producing, before committing a day ----
sleep 25
mm_ok=false; pm_ok=false
[[ -s "${out}/power.ndjson" ]] && mm_ok=true
[[ -s "${out}/power.log" ]] && pm_ok=true
{
  echo "macmon producing:     ${mm_ok}"
  echo "coalitions producing: ${pm_ok}"
} >> "${out}/meta.txt"

if ! ${mm_ok}; then
  echo "FATAL: macmon wrote nothing in 25 s — no power or thermal data would be collected." >&2
  cat "${out}/macmon-stderr.log" >&2 2>/dev/null || true
  exit 1
fi
if ${coalitions_active} && ! ${pm_ok}; then
  echo >&2
  echo "WARNING: powermetrics wrote nothing in 25 s. The run will have NO per-coalition" >&2
  echo "         attribution — whole-machine power only, which cannot be published as" >&2
  echo "         Mnema's cost. stderr follows:" >&2
  sed 's/^/         /' "${out}/powermetrics-stderr.log" >&2 2>/dev/null || true
  echo "         Ctrl-C now and fix it, or continue for a disk/memory-only run." >&2
  echo >&2
fi
echo "samplers verified: macmon=${mm_ok} coalitions=${pm_ok}"

echo "ts,elapsed_s,disk_total_kb,disk_recordings_kb,disk_db_kb,footprint_mb,webcontent_mb,batt_pct,batt_state,video_segments,audio_segments" > "${out}/samples.csv"

t0="$(date +%s)"
deadline=""
[[ -n "${minutes}" ]] && deadline=$(( t0 + minutes * 60 ))

while :; do
  now="$(date +%s)"
  batt_line="$(pmset -g batt | tail -n 1)"
  batt_pct="$(printf '%s' "${batt_line}" | sed -nE 's/.*[^0-9]([0-9]{1,3})%.*/\1/p')"
  batt_state="$(printf '%s' "${batt_line}" | awk -F';' '{gsub(/^ +| +$/,"",$2); print $2}')"
  printf '%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s\n' \
    "$(date -Iseconds)" \
    "$(( now - t0 ))" \
    "$(du -sk "${save_dir}" 2>/dev/null | awk '{print $1}')" \
    "$(du -sk "${save_dir}/recordings" 2>/dev/null | awk '{print $1}')" \
    "$(du -sk "${save_dir}/db" 2>/dev/null | awk '{print $1}')" \
    "$(footprint_mb "${pid}")" \
    "$(webcontent_mb)" \
    "${batt_pct:-}" \
    "${batt_state:-}" \
    "$(find "${save_dir}/recordings" -type f -name '*.mov' 2>/dev/null | wc -l | tr -d ' ')" \
    "$(find "${save_dir}/recordings" -type f -name '*.m4a' 2>/dev/null | wc -l | tr -d ' ')" \
    >> "${out}/samples.csv"

  if ! kill -0 "${pid}" 2>/dev/null; then
    echo "app process ${pid} exited — stopping collection" >&2
    break
  fi
  [[ -n "${deadline}" && "${now}" -ge "${deadline}" ]] && break
  sleep "${interval_sample_s}"
done
