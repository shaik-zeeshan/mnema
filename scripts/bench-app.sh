#!/usr/bin/env bash
set -euo pipefail

# Benchmark sandbox: a RELEASE build with its own identity (day.mnema.bench,
# ~/.mnema-bench) so a day-long resource measurement can run without touching
# the installed prod app's data, settings or TCC grants.
#
# Why release and not the debug sandbox (dev-app-signed.sh): in any build with
# debug_assertions on, `app_log_record_allowed` passes every Debug/Trace record
# unconditionally (native_capture_debug_log.rs) — no setting turns that off. A
# debug build therefore writes log all day and measures its own logging. The
# numbers have to come from the same optimisation level users run.
#
# Consequence of release: licensing is ENFORCED (debug builds bypass it), and
# the licensing keychain service is a hardcoded constant shared with prod
# (license_token_store.rs `day.mnema.licensing`). A machine that already holds a
# license reads it and is Licensed; a machine that does not will try to issue a
# trial. Either way capture must not be refused with `capture_refused_read_only`
# — check the window on first launch before starting a measurement run.
#
# Signed with Apple Development like dev-app-signed.sh so the TCC grants
# (screen / mic / system audio) attach to a stable identity and survive
# rebuilds. First launch prompts for all of them — that first-run sequence is
# itself worth watching, it is on the launch checklist.

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# NOTE: repo .env is deliberately NOT sourced (dev-app-signed.sh does source it).
# Its MNEMA_LICENSE_* values point the build at the sandbox licensing instance,
# and those are compile-time `option_env!` constants. A bench build that picks
# them up verifies the real license key against the SANDBOX public key, fails,
# and reports `trialNotStarted` instead of `Licensed` while retrying CRL fetches
# against a dead tunnel. Capture still runs (only ReadOnly/Revoked/lapsed block
# it), but the measured build should carry prod's licensing behaviour, not the
# sandbox's. Set MNEMA_BENCH_LOAD_ENV=1 if you specifically want sandbox
# licensing (e.g. to bench the trial path).
if [[ "${MNEMA_BENCH_LOAD_ENV:-0}" == "1" && -f "${repo_root}/.env" ]]; then
  set -a
  . "${repo_root}/.env"
  set +a
  echo "loaded ${repo_root}/.env (sandbox licensing config)"
fi

export MNEMA_SAVE_DIRECTORY="${MNEMA_SAVE_DIRECTORY:-${HOME}/.mnema-bench}"
export MNEMA_APP_CONFIG_DIR="${MNEMA_APP_CONFIG_DIR:-${HOME}/Library/Application Support/day.mnema.bench}"
mkdir -p "${MNEMA_SAVE_DIRECTORY}" "${MNEMA_APP_CONFIG_DIR}"

echo "mnema benchmark sandbox (release)"
echo "  save dir:   ${MNEMA_SAVE_DIRECTORY}"
echo "  config dir: ${MNEMA_APP_CONFIG_DIR}"

# The sandbox licensing keypair that dev-app-signed.sh loads here is
# deliberately omitted, for the reason given above the .env block: the bench
# build must verify against the same baked-in public key prod does.

identity="${APPLE_SIGNING_IDENTITY:-}"
if [[ -z "${identity}" ]]; then
  identity="$(security find-identity -v -p codesigning | grep 'Apple Development' | head -n 1 | sed -E 's/.*"(.*)"/\1/' || true)"
fi
if [[ -z "${identity}" ]]; then
  echo "No Apple Development signing identity found (see build-macos-local-sign.sh)." >&2
  exit 1
fi
echo "  identity:   ${identity}"

# speakrs/OpenBLAS from-source link path. DYNAMIC_ARCH is deliberately NOT set:
# this build never leaves the machine it was built on, and every arm64 kernel
# would make the first build far slower.
. "${repo_root}/scripts/openblas-build-env.sh"

cd "${repo_root}/apps/desktop"
CI=true APPLE_SIGNING_IDENTITY="${identity}" \
  bun run tauri -- build --bundles app -c src-tauri/tauri.bench.conf.json

# Launch through LaunchServices, not a shell exec: TCC attributes permission
# requests to the *responsible process*, and a shell-exec'd binary inherits the
# terminal as responsible, so prompts never fire. Runtime env rides in
# LSEnvironment because `open` does not pass env, then the outer signature is
# redone so the edited Info.plist stays sealed.
app="${CARGO_TARGET_DIR:-${repo_root}/target}/release/bundle/macos/mnema-bench.app"
plist="${app}/Contents/Info.plist"
/usr/libexec/PlistBuddy -c "Delete :LSEnvironment" "${plist}" 2>/dev/null || true
/usr/libexec/PlistBuddy \
  -c "Add :LSEnvironment dict" \
  -c "Add :LSEnvironment:MNEMA_SAVE_DIRECTORY string ${MNEMA_SAVE_DIRECTORY}" \
  -c "Add :LSEnvironment:MNEMA_APP_CONFIG_DIR string ${MNEMA_APP_CONFIG_DIR}" \
  "${plist}"
codesign --force --sign "${identity}" --options runtime \
  --entitlements "${repo_root}/apps/desktop/src-tauri/Entitlements.plist" "${app}"
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "${app}"
echo "launching ${app} via open"
open "${app}"
