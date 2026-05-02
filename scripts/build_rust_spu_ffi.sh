#!/usr/bin/env bash
# R6.1 — Build the rpcs3-spu-ffi staticlib that the C++ bridge links.
#
# Idempotent: re-runnable without side-effects. After completion the
# CMake build of rpcs3_emu will pick up the staticlib via the EXISTS
# check in `rpcs3/Emu/CMakeLists.txt` (added by the
# `spu_rust_bridge.patch`).
#
# Usage:
#   scripts/build_rust_spu_ffi.sh                       # release build
#   scripts/build_rust_spu_ffi.sh --debug               # debug build (dev only)
#   scripts/build_rust_spu_ffi.sh --dest-root /r        # auto-copy to /r/rust/
#   RPCS3_RUST_DEST_ROOT=/r scripts/build_rust_spu_ffi.sh
#
# Outputs:
#   rust/target/release/rpcs3_spu_ffi.lib       (Windows MSVC staticlib)
#   rust/target/release/librpcs3_spu_ffi.a      (Unix-style staticlib)
#   rust/rpcs3-spu-ffi/include/rpcs3_spu_ffi.h  (cbindgen-generated header)
#
# When --dest-root is supplied (or RPCS3_RUST_DEST_ROOT is set), the
# .lib/.a + header are also copied into:
#   <dest>/rust/target/<mode>/rpcs3_spu_ffi.{lib,a}
#   <dest>/rust/rpcs3-spu-ffi/include/rpcs3_spu_ffi.h
# (R6.1c — added so a separate rpcs3 source tree picks up the staticlib
# without manual copying.)
#
# Exit codes:
#   0 — staticlib + header produced (and copied if requested).
#   1 — cargo build failed.
#   2 — cbindgen missing (install via `cargo install cbindgen`).
#   3 — header generation failed.
#   4 — dest-root copy failed.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RUST_DIR="${REPO_ROOT}/rust"
FFI_DIR="${RUST_DIR}/rpcs3-spu-ffi"

MODE="release"
DEST_ROOT="${RPCS3_RUST_DEST_ROOT-}"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --debug)
            MODE="debug"
            shift
            ;;
        --dest-root)
            DEST_ROOT="${2-}"
            shift 2
            ;;
        --dest-root=*)
            DEST_ROOT="${1#--dest-root=}"
            shift
            ;;
        *)
            echo "[build_rust_spu_ffi] unknown arg: $1" >&2
            exit 2
            ;;
    esac
done

echo "[build_rust_spu_ffi] mode=${MODE}"
echo "[build_rust_spu_ffi] cargo build -p rpcs3-spu-ffi --${MODE} ..."

if [[ "${MODE}" == "release" ]]; then
    (cd "${RUST_DIR}" && cargo build --release -p rpcs3-spu-ffi) || exit 1
else
    (cd "${RUST_DIR}" && cargo build -p rpcs3-spu-ffi) || exit 1
fi

echo "[build_rust_spu_ffi] regenerating header via cbindgen..."
if ! command -v cbindgen >/dev/null 2>&1; then
    echo "[build_rust_spu_ffi] ERROR: cbindgen not found in PATH" >&2
    echo "[build_rust_spu_ffi] install via: cargo install cbindgen" >&2
    exit 2
fi

(cd "${FFI_DIR}" && cbindgen --config cbindgen.toml --crate rpcs3-spu-ffi --output include/rpcs3_spu_ffi.h) || exit 3

echo "[build_rust_spu_ffi] OK"
echo "[build_rust_spu_ffi]   staticlib: ${RUST_DIR}/target/${MODE}/rpcs3_spu_ffi.lib (or .a on Unix)"
echo "[build_rust_spu_ffi]   header:    ${FFI_DIR}/include/rpcs3_spu_ffi.h"

# R6.1c — optional auto-copy to a destination rpcs3 source tree.
if [[ -n "${DEST_ROOT}" ]]; then
    if [[ ! -d "${DEST_ROOT}" ]]; then
        echo "[build_rust_spu_ffi] ERROR: --dest-root '${DEST_ROOT}' does not exist" >&2
        exit 4
    fi
    DEST_LIB_DIR="${DEST_ROOT}/rust/target/${MODE}"
    DEST_HDR_DIR="${DEST_ROOT}/rust/rpcs3-spu-ffi/include"
    mkdir -p "${DEST_LIB_DIR}" "${DEST_HDR_DIR}" || exit 4
    # Copy whichever staticlib(s) exist (MSVC .lib on Windows, .a on Unix; both on cross builds).
    copied_any=0
    for ext in lib a; do
        src="${RUST_DIR}/target/${MODE}/rpcs3_spu_ffi.${ext}"
        # The Unix-style staticlib follows the librpcs3_spu_ffi.a convention.
        if [[ "${ext}" == "a" ]]; then
            src="${RUST_DIR}/target/${MODE}/librpcs3_spu_ffi.a"
        fi
        if [[ -f "${src}" ]]; then
            cp -f "${src}" "${DEST_LIB_DIR}/" || exit 4
            echo "[build_rust_spu_ffi] copied ${src##*/} to ${DEST_LIB_DIR}/"
            copied_any=1
        fi
    done
    # PDB on Windows builds, helpful for the MSVC linker.
    if [[ -f "${RUST_DIR}/target/${MODE}/rpcs3_spu_ffi.pdb" ]]; then
        cp -f "${RUST_DIR}/target/${MODE}/rpcs3_spu_ffi.pdb" "${DEST_LIB_DIR}/" || exit 4
    fi
    if [[ "${copied_any}" -eq 0 ]]; then
        echo "[build_rust_spu_ffi] WARN: no staticlib found to copy" >&2
        exit 4
    fi
    cp -f "${FFI_DIR}/include/rpcs3_spu_ffi.h" "${DEST_HDR_DIR}/" || exit 4
    echo "[build_rust_spu_ffi] copied rpcs3_spu_ffi.h to ${DEST_HDR_DIR}/"
fi

echo ""
echo "Next step: re-run cmake configure on rpcs3_emu so the EXISTS check"
echo "in rpcs3/Emu/CMakeLists.txt picks up the staticlib and defines"
echo "RPCS3_HAS_SPU_RUST_BRIDGE=1. The bridge stays runtime-gated by"
echo "the env var RPCS3_SPU_RUST_BRIDGE=1 (default OFF)."
