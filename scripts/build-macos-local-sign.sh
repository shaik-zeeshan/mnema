#!/bin/zsh

set -euo pipefail

if [[ "${OSTYPE}" != darwin* ]]; then
  print -u2 "This script only runs on macOS."
  exit 1
fi

debug=false
for arg in "$@"; do
  case "${arg}" in
    --debug) debug=true ;;
    *)
      print -u2 "Unknown option: ${arg}"
      print -u2 "Usage: $0 [--debug]"
      exit 1
      ;;
  esac
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
# Same allexport source as dev-app.sh; .env wins over the inherited shell env.
if [[ -f "${repo_root}/.env" ]]; then
  set -a
  . "${repo_root}/.env"
  set +a
  print "loaded ${repo_root}/.env"
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

dmg_path="$(ls -t "${dmg_dir}"/*.dmg 2>/dev/null | head -n 1 || true)"

if [[ -n "${dmg_path}" ]]; then
  print "Opening DMG: ${dmg_path}"
  open "${dmg_path}"
else
  print -u2 "Build succeeded, but no DMG was found in ${dmg_dir}."
  exit 1
fi
