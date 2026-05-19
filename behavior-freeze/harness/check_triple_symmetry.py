#!/usr/bin/env python3
"""R7.3 — Triple-symmetry regression gate for `single_spu_dma_get_v1`.

Asserts that all three execution paths converge on the canonical
status `0xdeada12f`:

    1. bridge OFF real binary   — rpcs3.exe running the .self
                                  with the bare C++ executor
    2. bridge ON  real binary   — rpcs3.exe with
                                  RPCS3_SPU_RUST_BRIDGE=1; the Rust
                                  bridge delegates end-to-end via
                                  R7.2 runtime DMA GET (no fallback)
    3. replay oracle            — `cargo test
                                  single_spu_dma_get_v1_replay`
                                  byte-identical across Interpreter
                                  and Recompiler

Triple-symmetric means: all three produce the SPU OUT_MBOX value
`0xdeada12f` (the canonical post-DMA sum + XOR). The lv2 kernel
reads OUT_MBOX as the group-exit status, the PPU prints it via
sys_tty_write, and the host TTY shows
`[dma_get_v1] OK cause=0x1 status=0xdeada12f`. The replay test
asserts the same value internally via `diff_snapshots(interp,
jit).is_identical()` and a captured-event check.

This gate is the load-bearing R7.3 acceptance and is documented
in `docs/PROJECT_STATUS.md` (R7 closure) and
`docs/SPU_DMA_MFC_R6_7_DESIGN.md` § 14.

Environmental prerequisites:

- `rpcs3-upstream-clean/bin/rpcs3.exe` is the R7.2-aware build
  (sha pinned via `check_patch_separation.py` indirectly through
  the bridge patch sha).
- The fixture `single_spu_dma_get_v1.self` exists at
  `behavior-freeze/fixtures/spu/sources/single_spu_dma_get_v1/build/`.
- Windows host with `subst R:` active for the build configuration
  (only relevant if rpcs3.exe needs rebuilding; the test itself
  only RUNS the binary).
- Rust toolchain installed for the replay-test stage.

Exit codes:
  0 — all three converge on `0xdeada12f`.
  1 — at least one path diverges or the rpcs3.exe / .self is missing.

Usage:
  python behavior-freeze/harness/check_triple_symmetry.py

The script is intentionally Windows-specific: the runtime bridge
needs RPCS3-on-Windows + the static `rpcs3_spu_ffi.lib`. The
replay path is platform-agnostic but the runtime paths are not.
"""

from __future__ import annotations

import os
import re
import shutil
import subprocess
import sys
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parent.parent.parent
UPSTREAM = REPO_ROOT.parent / "rpcs3-upstream-clean"
RPCS3_EXE = UPSTREAM / "bin" / "rpcs3.exe"
TTY_LOG = UPSTREAM / "bin" / "log" / "TTY.log"
RPCS3_LOG = UPSTREAM / "bin" / "log" / "RPCS3.log"
SELF_BIN = (
    REPO_ROOT
    / "behavior-freeze"
    / "fixtures"
    / "spu"
    / "sources"
    / "single_spu_dma_get_v1"
    / "build"
    / "single_spu_dma_get_v1.self"
)
RUST_WORKSPACE = REPO_ROOT / "rust"

CANONICAL_TTY = "[dma_get_v1] OK cause=0x1 status=0xdeada12f"
CANONICAL_STATUS = "0xdeada12f"


def fail(msg: str) -> int:
    print(f"FAIL: {msg}", file=sys.stderr)
    return 1


def run_rpcs3(bridge_on: bool, timeout_s: int = 60) -> tuple[str, str]:
    """Run rpcs3.exe on the .self and return (tty_content, rust_bridge_log_lines).

    Returns ("", "") on subprocess failure / timeout (caller surfaces as
    a triple-symmetry violation).
    """
    env = os.environ.copy()
    if bridge_on:
        env["RPCS3_SPU_RUST_BRIDGE"] = "1"
    else:
        env.pop("RPCS3_SPU_RUST_BRIDGE", None)

    # Clean prior logs.
    for p in (TTY_LOG, RPCS3_LOG):
        try:
            p.unlink()
        except FileNotFoundError:
            pass

    try:
        subprocess.run(
            [
                str(RPCS3_EXE),
                "--no-gui",
                "--headless",
                "--stdout",
                str(SELF_BIN),
            ],
            env=env,
            timeout=timeout_s,
            check=False,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
    except (OSError, subprocess.TimeoutExpired) as exc:
        print(f"  WARN: rpcs3.exe invocation failed: {exc}", file=sys.stderr)
        return "", ""

    tty = TTY_LOG.read_text(encoding="utf-8", errors="replace") if TTY_LOG.exists() else ""
    rust_lines = ""
    if RPCS3_LOG.exists():
        rpcs3_log = RPCS3_LOG.read_text(encoding="utf-8", errors="replace")
        rust_lines = "\n".join(
            line for line in rpcs3_log.splitlines() if "RustSPU" in line
        )
    return tty, rust_lines


def run_replay_test(timeout_s: int = 600) -> tuple[bool, str]:
    """Run `cargo test single_spu_dma_get_v1_replay --release` and return
    (passed, last_lines_of_output).
    """
    if not shutil.which("cargo"):
        return False, "cargo not on PATH"
    try:
        result = subprocess.run(
            [
                "cargo",
                "test",
                "--release",
                "-p",
                "rpcs3-spu-recompiler",
                "--test",
                "single_spu_dma_get_v1_replay",
            ],
            cwd=str(RUST_WORKSPACE),
            timeout=timeout_s,
            check=False,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
        )
    except (OSError, subprocess.TimeoutExpired) as exc:
        return False, str(exc)
    # Look for the canonical "test result: ok. 1 passed" tail.
    lines = result.stdout.splitlines()
    tail = "\n".join(lines[-8:]) if lines else ""
    passed = any(
        "test result: ok. 1 passed" in line and "0 failed" in line
        for line in lines
    )
    return passed, tail


def main() -> int:
    violations: list[str] = []

    # ----- Phase 0: prerequisites -----
    if not RPCS3_EXE.exists():
        return fail(f"rpcs3.exe not found at {RPCS3_EXE}")
    if not SELF_BIN.exists():
        return fail(f"single_spu_dma_get_v1.self not found at {SELF_BIN}")

    print(f"OK: prerequisites — rpcs3.exe = {RPCS3_EXE}")
    print(f"                  .self        = {SELF_BIN}")

    # ----- Phase 1: bridge OFF -----
    print("\n[1/3] Running real binary, bridge OFF ...")
    tty_off, _ = run_rpcs3(bridge_on=False)
    if CANONICAL_TTY not in tty_off:
        violations.append(
            f"bridge OFF: TTY did not contain canonical line.\n"
            f"  expected substring: {CANONICAL_TTY!r}\n"
            f"  got TTY:\n    {tty_off!r}"
        )
    else:
        print(f"  OK: TTY contains canonical: {CANONICAL_TTY!r}")

    # ----- Phase 2: bridge ON -----
    print("\n[2/3] Running real binary, bridge ON ...")
    tty_on, rust_lines = run_rpcs3(bridge_on=True)
    if CANONICAL_TTY not in tty_on:
        violations.append(
            f"bridge ON: TTY did not contain canonical line.\n"
            f"  expected substring: {CANONICAL_TTY!r}\n"
            f"  got TTY:\n    {tty_on!r}"
        )
    else:
        print(f"  OK: TTY contains canonical: {CANONICAL_TTY!r}")

    # Bridge ON must DELEGATE end-to-end (R7.2 runtime DMA path) —
    # no MfcUnsupported / fallback in the Rust bridge log lines.
    if "MFC/DMA detected" in rust_lines or "falling back honestly" in rust_lines:
        violations.append(
            "bridge ON: R7.2 expects DELEGATED EXECUTION OK (no fallback). "
            "Found a fallback log line. Rust bridge log:\n"
            f"  {rust_lines!r}"
        )
    elif "DELEGATED EXECUTION OK" not in rust_lines:
        violations.append(
            "bridge ON: expected 'DELEGATED EXECUTION OK' in Rust bridge log "
            "but did not find it. Rust bridge log:\n"
            f"  {rust_lines!r}"
        )
    else:
        # Extract total_steps for the report.
        m = re.search(r"total_steps=(\d+)", rust_lines)
        ts = m.group(1) if m else "<unknown>"
        m2 = re.search(r"DMA GET dispatched.*?eal=(0x[0-9a-fA-F]+).*?size=(\d+).*?tag=(\d+)", rust_lines)
        if m2:
            print(
                f"  OK: R7.2 DMA GET dispatched (eal={m2.group(1)} size={m2.group(2)} "
                f"tag={m2.group(3)}); DELEGATED EXECUTION OK (total_steps={ts})"
            )
        else:
            print(f"  OK: DELEGATED EXECUTION OK (total_steps={ts})")

    # ----- Phase 3: replay oracle -----
    print("\n[3/3] Running replay-oracle test ...")
    passed, tail = run_replay_test()
    if not passed:
        violations.append(
            "replay oracle: single_spu_dma_get_v1_replay did NOT pass.\n"
            "  cargo test tail:\n" + "    " + tail.replace("\n", "\n    ")
        )
    else:
        print("  OK: cargo test -p rpcs3-spu-recompiler "
              "--test single_spu_dma_get_v1_replay passed")

    # ----- Verdict -----
    print()
    if violations:
        print(f"FAIL: {len(violations)} violation(s) in triple-symmetry contract",
              file=sys.stderr)
        for v in violations:
            print(f"  - {v}", file=sys.stderr)
        return 1

    print("OK: triple-symmetry verified for single_spu_dma_get_v1")
    print(f"  bridge OFF TTY  : {CANONICAL_TTY}")
    print(f"  bridge ON  TTY  : {CANONICAL_TTY}")
    print(f"  replay oracle    : diff_snapshots(interp, jit).is_identical() == true")
    print(f"  canonical status: {CANONICAL_STATUS}")
    print(f"  r20 (= OUT_MBOX = cs = sum(0..127) ^ 0xDEADBEEF) = 0xDEADA12F")
    return 0


if __name__ == "__main__":
    sys.exit(main())
