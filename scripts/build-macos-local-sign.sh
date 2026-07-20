#!/bin/zsh

set -euo pipefail

if [[ "${OSTYPE}" != darwin* ]]; then
  print -u2 "This script only runs on macOS."
  exit 1
fi

debug=false
env_name=""
while (( $# > 0 )); do
  case "$1" in
    --debug) debug=true ;;
    --env)
      if (( $# < 2 )); then
        print -u2 "--env requires a name (e.g. dev, prod)"
        exit 1
      fi
      env_name="$2"
      shift
      ;;
    *)
      print -u2 "Unknown option: $1"
      print -u2 "Usage: $0 [--debug] [--env <name>]  (loads .env.<name> instead of .env)"
      exit 1
      ;;
  esac
  shift
done

identity="${APPLE_SIGNING_IDENTITY:-}"

if [[ -z "${identity}" ]]; then
  identity="$(security find-identity -v -p codesigning | grep 'Apple Development' | head -n 1 | sed -E 's/.*\"(.*)\"/\1/' || true)"
fi

if [[ -z "${identity}" ]]; then
  print -u2 "No Apple Development signing identity found."
  print -u2 "Create one in Xcode: Settings > Accounts > Apple ID > Manage Certificates > + > Apple Development."
  print -u2 "Then re-run this script, or set APPLE_SIGNING_IDENTITY explicitly."
  exit 1
fi

print "Using signing identity: ${identity}"
script_dir="$(cd -- "$(dirname -- "$0")" && pwd)"
repo_root="$(cd -- "${script_dir}/.." && pwd)"

# Repo-root .env (gitignored): TAURI_SIGNING_PRIVATE_KEY + licensing build env.
# Same allexport source as dev-app.sh; the env file wins over the inherited
# shell env. --env <name> selects .env.<name> (must exist); default is .env.
env_file="${repo_root}/.env${env_name:+.${env_name}}"
if [[ -n "${env_name}" && ! -f "${env_file}" ]]; then
  print -u2 "Env file not found: ${env_file}"
  exit 1
fi
if [[ -f "${env_file}" ]]; then
  set -a
  . "${env_file}"
  set +a
  print "loaded ${env_file}"
fi

if [[ "${debug}" == true ]]; then
  profile_dir="debug"
  print "Build profile: debug"
else
  profile_dir="release"
fi
dmg_dir="${repo_root}/target/${profile_dir}/bundle/dmg"

# speakrs links OpenBLAS via the `openblas-static` feature, built from source:
# put the gcc lib dir on the linker search path (shared helper), and — because
# this signed build is handed to others — build all-generation arm64 kernels so
# it runs on every Apple Silicon, not just this machine's core. See AGENTS.md.
source "${repo_root}/scripts/openblas-build-env.sh"
export OPENBLAS_DYNAMIC_ARCH=1
print "OpenBLAS: static + DYNAMIC_ARCH (all Apple Silicon generations)"

# Regenerate the third-party attribution file so the freshly-built version is
# bundled into the .app (tauri.conf.json bundle.resources references it).
print "Generating third-party license attribution…"
"${repo_root}/scripts/generate-third-party-licenses.sh"

tauri_args=(build)
if [[ "${debug}" == true ]]; then
  tauri_args+=(--debug)
fi

cd "${repo_root}/apps/desktop"
CI=true APPLE_SIGNING_IDENTITY="${identity}" bun run tauri -- "${tauri_args[@]}"

# Tauri does not entitle sidecars (ADR 0057): re-sign the bundled mnema-cli
# with its own entitlements, then re-sign the app so the outer signature seals
# the new nested code — sidecar first, or the app signature breaks.
app_path="${repo_root}/target/${profile_dir}/bundle/macos/mnema.app"
tauri_dir="${repo_root}/apps/desktop/src-tauri"
print "Re-signing mnema-cli sidecar with entitlements…"
codesign --force --options runtime --sign "${identity}" \
  --entitlements "${tauri_dir}/Entitlements.mnema-cli.plist" \
  "${app_path}/Contents/MacOS/mnema-cli"
codesign --force --options runtime --sign "${identity}" \
  --entitlements "${tauri_dir}/Entitlements.plist" \
  "${app_path}"
codesign --verify --deep --strict "${app_path}"

dmg_path="$(ls -t "${dmg_dir}"/*.dmg 2>/dev/null | head -n 1 || true)"

if [[ -n "${dmg_path}" ]]; then
  # Tauri built the DMG before the re-sign above, so it embeds the
  # un-entitled app — rebuild it from the re-signed bundle.
  print "Rebuilding DMG from re-signed app…"
  staging="$(mktemp -d)"
  ditto "${app_path}" "${staging}/mnema.app"
  ln -s /Applications "${staging}/Applications"
  hdiutil create -volname "mnema" -srcfolder "${staging}" -ov -format UDZO \
    "${dmg_path}" >/dev/null
  rm -rf "${staging}"
  print "Opening DMG: ${dmg_path}"
  open "${dmg_path}"
else
  print -u2 "Build succeeded, but no DMG was found in ${dmg_dir}."
  exit 1
fi
