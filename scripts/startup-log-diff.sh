#!/usr/bin/env bash
set -euo pipefail

# Startup-order check for docs/gpui/PLAN.md S4: run the Tauri dev binary of a
# base commit and of this working tree for RUN_SECONDS each, keep the Info log
# lines from launch to END_MARKER, strip timestamps, and diff them.
#
#   bash scripts/startup-log-diff.sh [base-ref]      # default 824c20bc
#
# Never touches real data or the keychain. Each run gets a fresh scratch copy
# of ~/.mnema-dev and its own HOME, config dir and license-token dir:
#   - the copy drops `secrets.vault` + `dev-master-key`, so no stored provider
#     key can be used (nothing is sent anywhere with the founder's keys);
#   - the config dir is empty except the dev DB key (copied, so the DB copy
#     opens), so onboarding reads as not done: no capture auto-start, no update
#     check, no not-recording reminder, nothing that asks for TCC;
#   - MNEMA_DEV_MASTER_KEY_FILE / MNEMA_CAPTURE_INDEX_KEY_DIR /
#     MNEMA_LICENSE_TOKEN_DIR point into the scratch dir and
#     MNEMA_TRIPWIRE_NO_KEYCHAIN=1 turns any vault keychain read into a panic;
#   - devUrl points at a closed port, so no other worktree's Vite frontend loads.
# It refuses to run while another Mnema app is running: deferred startup reaps
# every other process's system-audio aggregate device (it would break that
# app's recording), and the single-instance plugin would hand off to it.
#
# Env: RUN_SECONDS (30), END_MARKER ("startup maintenance finished"; empty =
# the whole run), SOURCE_DATA (~/.mnema-dev), SOURCE_KEYS (the dev sandbox's
# capture-index-keys), STARTUP_LOG_DIFF_WORK (scratch root), CARGO_TARGET_DIR.
# Exit status: 0 when the Info messages match, 1 when they differ.

base_ref="${1:-824c20bc}"
run_seconds="${RUN_SECONDS:-30}"
end_marker="${END_MARKER-startup maintenance finished}"
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
work="${STARTUP_LOG_DIFF_WORK:-${TMPDIR:-/tmp}/mnema-startup-log-diff}"
source_data="${SOURCE_DATA:-${HOME}/.mnema-dev}"
source_keys="${SOURCE_KEYS:-${HOME}/Library/Application Support/day.mnema.dev/capture-index-keys}"

running="$(pgrep -fl '(/Contents/MacOS/[Mm]nema[^/]*|/debug/mnema)( |$)' || true)"
if [[ -n "${running}" ]]; then
  echo "refusing: a Mnema app is running (quit it first):" >&2
  echo "${running}" >&2
  exit 2
fi
for path in "${source_data}" "${source_keys}"; do
  if [[ ! -d "${path}" ]]; then
    echo "missing ${path}" >&2
    exit 2
  fi
done

. "${repo_root}/scripts/openblas-build-env.sh"
mkdir -p "${work}"

# The dev sandbox identity (day.mnema.dev), with devUrl on a closed port.
tauri_config="$(python3 -c '
import json, sys
config = json.load(open(sys.argv[1]))
config.setdefault("build", {})["devUrl"] = "http://127.0.0.1:9"
config["build"].pop("beforeBuildCommand", None)
print(json.dumps(config))
' "${repo_root}/apps/desktop/src-tauri/tauri.dev.conf.json")"

# build <source tree> <label>: the debug binary, copied to $work/mnema-<label>.
build() {
  local tree="$1" label="$2"
  echo "building ${label} (${tree})"
  (
    cd "${tree}"
    [[ -f apps/desktop/THIRD_PARTY_LICENSES.md ]] || touch apps/desktop/THIRD_PARTY_LICENSES.md
    bash scripts/prepare-mnema-cli-sidecar.sh debug >/dev/null
    TAURI_CONFIG="${tauri_config}" cargo build --quiet \
      --manifest-path apps/desktop/src-tauri/Cargo.toml --bin mnema
  )
  local target="${CARGO_TARGET_DIR:-${tree}/target}"
  cp "${target}/debug/mnema" "${work}/mnema-${label}"
}

# run <label>: start the binary on a fresh scratch copy, kill it after
# $run_seconds, and write the normalised Info lines to $work/<label>.info.
run() {
  local label="$1" run_dir="${work}/run"
  rm -rf "${run_dir}"
  mkdir -p "${run_dir}/home" "${run_dir}/config" "${run_dir}/license-tokens"
  cp -R "${source_data}" "${run_dir}/data"
  rm -f "${run_dir}/data/secrets.vault" "${run_dir}/data/secrets.vault.tmp" "${run_dir}/data/dev-master-key"
  cp -R "${source_keys}" "${run_dir}/config/capture-index-keys"

  echo "running ${label} for ${run_seconds}s"
  set -m
  env -u MNEMA_LICENSE_ENFORCE -u MNEMA_DEV_PUBLIC_KEY_FILE \
    HOME="${run_dir}/home" \
    MNEMA_SAVE_DIRECTORY="${run_dir}/data" \
    MNEMA_APP_CONFIG_DIR="${run_dir}/config" \
    MNEMA_DEV_MASTER_KEY_FILE="${run_dir}/dev-master-key" \
    MNEMA_CAPTURE_INDEX_KEY_DIR="${run_dir}/config/capture-index-keys" \
    MNEMA_LICENSE_TOKEN_DIR="${run_dir}/license-tokens" \
    MNEMA_TRIPWIRE_NO_KEYCHAIN=1 \
    "${work}/mnema-${label}" >"${work}/${label}.log" 2>&1 &
  local pid=$!
  set +m
  sleep "${run_seconds}"
  kill -9 -- "-${pid}" 2>/dev/null || kill -9 "${pid}" 2>/dev/null || true
  wait "${pid}" 2>/dev/null || true

  # `[date][time][target][INFO] message` → `[target][INFO] message`, launch to marker.
  sed -E 's/\x1b\[[0-9;]*m//g' "${work}/${label}.log" \
    | grep -E '^\[[0-9-]+\]\[[0-9:.]+\]\[[^]]+\]\[INFO\]' \
    | sed -E 's/^\[[0-9-]+\]\[[0-9:.]+\]//; s/ in [0-9]+ ?ms/ in N ms/g; s/ in [0-9.]+s/ in Ns/g' \
    | awk -v marker="${end_marker}" '{ print } marker != "" && index($0, marker) { exit }' \
      >"${work}/${label}.info"
  sed -E 's/^\[[^]]+\]\[INFO\] //' "${work}/${label}.info" >"${work}/${label}.messages"
  if [[ -n "${end_marker}" ]] && ! grep -qF "${end_marker}" "${work}/${label}.info"; then
    echo "warning: ${label} never logged '${end_marker}' within ${run_seconds}s" >&2
  fi
  echo "  $(wc -l <"${work}/${label}.info" | tr -d ' ') Info lines -> ${work}/${label}.info"
}

base_tree="${work}/base-${base_ref}"
if [[ ! -d "${base_tree}" ]]; then
  mkdir -p "${base_tree}"
  git -C "${repo_root}" archive "${base_ref}" | tar -x -C "${base_tree}"
fi
build "${base_tree}" base
build "${repo_root}" head
run base
run head

echo
echo "targets (moved code logs under app_core::…; informational):"
diff "${work}/base.info" "${work}/head.info" || true
echo
echo "messages:"
if diff "${work}/base.messages" "${work}/head.messages"; then
  echo "same Info messages, same order ($(wc -l <"${work}/head.messages" | tr -d ' ') lines)"
else
  exit 1
fi
