# shellcheck shell=sh
# Sourceable helper that prepares the build environment for speakrs' OpenBLAS.
#
# speakrs (the on-device diarization engine, default-on in apps/desktop/src-tauri)
# links OpenBLAS via the `openblas-static` feature: openblas-src builds OpenBLAS
# (incl. Fortran LAPACK) from source. OpenBLAS's `make all` links its own test
# programs with `-lgfortran`, so the gcc lib dir must be on the linker search
# path (LIBRARY_PATH) or the from-source build fails at its test link. See
# AGENTS.md.
#
# Source this before any cargo/tauri command that builds the desktop crate:
#   . scripts/openblas-build-env.sh
#
# It does NOT set OPENBLAS_DYNAMIC_ARCH — that belongs to *distributable* builds
# only (it builds every arm64 kernel, which is slow and pointless for local dev).
# Distributable build scripts set it themselves.

if ! command -v gfortran >/dev/null 2>&1; then
  echo "gfortran not found — install the Fortran toolchain: brew install gcc" >&2
  exit 1
fi

_openblas_gfortran_libdir="$(dirname "$(gfortran -print-file-name=libgfortran.dylib)")"
export LIBRARY_PATH="${_openblas_gfortran_libdir}${LIBRARY_PATH:+:${LIBRARY_PATH}}"
echo "openblas-build-env: LIBRARY_PATH includes ${_openblas_gfortran_libdir}"
unset _openblas_gfortran_libdir
