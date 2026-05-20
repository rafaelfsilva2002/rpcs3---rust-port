#!/usr/bin/env python3
"""Triple-symmetry regression gate for SPU DMA fixtures.

Asserts that all three execution paths converge on the same
canonical OUT_MBOX status for a given DMA fixture:

    1. bridge OFF real binary   — rpcs3.exe running the .self
                                  with the bare C++ executor
    2. bridge ON  real binary   — rpcs3.exe with
                                  RPCS3_SPU_RUST_BRIDGE=1; the Rust
                                  bridge delegates end-to-end via
                                  the runtime DMA path
    3. replay oracle            — `cargo test
                                  single_spu_dma_<dir>_v1_replay`
                                  byte-identical across Interpreter
                                  and Recompiler

R7.3 introduced this gate for `single_spu_dma_get_v1` (cs =
`0xdeada12f`). R8.1 extends it to `single_spu_dma_put_v1`
(spu sentinel = `0xc0ffeeca`, ea_status = `0xcafea57e`).

Two fixtures are supported via `--fixture {get,put}` (default
`get` for backwards compatibility).

This gate is the load-bearing R7.3 + R8.1 acceptance and is
documented in `docs/PROJECT_STATUS.md` and
`docs/SPU_DMA_MFC_R6_7_DESIGN.md` § 14.

Environmental prerequisites:

- `rpcs3-upstream-clean/bin/rpcs3.exe` is the R7.2/R8.1-aware build
  (sha pinned via `check_patch_separation.py` indirectly through
  the bridge patch sha).
- The fixture `.self` exists at
  `behavior-freeze/fixtures/spu/sources/<name>/build/<name>.self`.
- Windows host with `subst R:` active for the build configuration
  (only relevant if rpcs3.exe needs rebuilding; the test itself
  only RUNS the binary).
- Rust toolchain installed for the replay-test stage.

Exit codes:
  0 — all three paths converge.
  1 — at least one path diverges or rpcs3.exe / .self is missing.

Usage:
  python behavior-freeze/harness/check_triple_symmetry.py [--fixture get|put]

The script is intentionally Windows-specific: the runtime bridge
needs RPCS3-on-Windows + the static `rpcs3_spu_ffi.lib`. The
replay path is platform-agnostic but the runtime paths are not.
"""

from __future__ import annotations

import argparse
import os
import re
import shutil
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import List, Tuple


REPO_ROOT = Path(__file__).resolve().parent.parent.parent
UPSTREAM = REPO_ROOT.parent / "rpcs3-upstream-clean"
RPCS3_EXE = UPSTREAM / "bin" / "rpcs3.exe"
TTY_LOG = UPSTREAM / "bin" / "log" / "TTY.log"
RPCS3_LOG = UPSTREAM / "bin" / "log" / "RPCS3.log"
RUST_WORKSPACE = REPO_ROOT / "rust"


@dataclass(frozen=True)
class FixtureSpec:
    """All fixture-specific knobs in one place."""

    name: str
    rust_test_target: str
    canonical_tty_substr: str
    canonical_status_summary: str
    delegation_log_marker: str
    rust_log_intro: str

    @property
    def self_path(self) -> Path:
        return (
            REPO_ROOT
            / "behavior-freeze"
            / "fixtures"
            / "spu"
            / "sources"
            / self.name
            / "build"
            / f"{self.name}.self"
        )


# R7.3 — GET fixture (cs = 0xdeada12f).
GET_FIXTURE = FixtureSpec(
    name="single_spu_dma_get_v1",
    rust_test_target="single_spu_dma_get_v1_replay",
    canonical_tty_substr="[dma_get_v1] OK cause=0x1 status=0xdeada12f",
    canonical_status_summary=(
        "r20 (= OUT_MBOX = cs = sum(0..127) ^ 0xDEADBEEF) = 0xDEADA12F"
    ),
    delegation_log_marker="DMA GET dispatched",
    rust_log_intro="R7.2 DMA GET",
)

# R8.1 — PUT fixture (spu sentinel = 0xc0ffeeca, ea_status =
# 0xcafea57e). The PPU prints both on one line.
PUT_FIXTURE = FixtureSpec(
    name="single_spu_dma_put_v1",
    rust_test_target="single_spu_dma_put_v1_replay",
    canonical_tty_substr="[dma_put_v1] OK cause=0x1 spu=0xc0ffeeca ea_status=0xcafea57e",
    canonical_status_summary=(
        "spu sentinel = 0xC0FFEECA; ea_status = sum(0..127) ^ 0xCAFEBABE = 0xCAFEA57E"
    ),
    delegation_log_marker="DMA PUT dispatched",
    rust_log_intro="R8.1 DMA PUT",
)


FIXTURES = {
    "get": GET_FIXTURE,
    "put": PUT_FIXTURE,
}


def fail(msg: str) -> int:
    print(f"FAIL: {msg}", file=sys.stderr)
    return 1


def run_rpcs3(self_bin: Path, bridge_on: bool, timeout_s: int = 60) -> Tuple[str, str]:
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
                str(self_bin),
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


def run_replay_test(test_target: str, timeout_s: int = 600) -> Tuple[bool, str]:
    """Run `cargo test <test_target> --release` and return
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
                test_target,
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
    lines = result.stdout.splitlines()
    tail = "\n".join(lines[-8:]) if lines else ""
    passed = any(
        "test result: ok. 1 passed" in line and "0 failed" in line
        for line in lines
    )
    return passed, tail


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--fixture",
        choices=sorted(FIXTURES.keys()),
        default="get",
        help="DMA fixture to gate (default: get, kept for R7.3 backwards-compat)",
    )
    args = parser.parse_args()

    fixture = FIXTURES[args.fixture]
    violations: List[str] = []

    # ----- Phase 0: prerequisites -----
    if not RPCS3_EXE.exists():
        return fail(f"rpcs3.exe not found at {RPCS3_EXE}")
    if not fixture.self_path.exists():
        return fail(f"{fixture.name}.self not found at {fixture.self_path}")

    print(f"OK: prerequisites — rpcs3.exe = {RPCS3_EXE}")
    print(f"                  .self        = {fixture.self_path}")
    print(f"                  fixture      = {fixture.name}")

    # ----- Phase 1: bridge OFF -----
    print("\n[1/3] Running real binary, bridge OFF ...")
    tty_off, _ = run_rpcs3(fixture.self_path, bridge_on=False)
    if fixture.canonical_tty_substr not in tty_off:
        violations.append(
            f"bridge OFF: TTY did not contain canonical line.\n"
            f"  expected substring: {fixture.canonical_tty_substr!r}\n"
            f"  got TTY:\n    {tty_off!r}"
        )
    else:
        print(f"  OK: TTY contains canonical: {fixture.canonical_tty_substr!r}")

    # ----- Phase 2: bridge ON -----
    print("\n[2/3] Running real binary, bridge ON ...")
    tty_on, rust_lines = run_rpcs3(fixture.self_path, bridge_on=True)
    if fixture.canonical_tty_substr not in tty_on:
        violations.append(
            f"bridge ON: TTY did not contain canonical line.\n"
            f"  expected substring: {fixture.canonical_tty_substr!r}\n"
            f"  got TTY:\n    {tty_on!r}"
        )
    else:
        print(f"  OK: TTY contains canonical: {fixture.canonical_tty_substr!r}")

    # Bridge ON must DELEGATE end-to-end (R7.2 / R8.1 runtime DMA
    # path) — no MfcUnsupported / fallback in the Rust bridge log
    # lines.
    if "MFC/DMA detected" in rust_lines or "falling back honestly" in rust_lines:
        violations.append(
            f"bridge ON: {fixture.rust_log_intro} expects DELEGATED EXECUTION OK "
            "(no fallback). Found a fallback log line. Rust bridge log:\n"
            f"  {rust_lines!r}"
        )
    elif "DELEGATED EXECUTION OK" not in rust_lines:
        violations.append(
            "bridge ON: expected 'DELEGATED EXECUTION OK' in Rust bridge log "
            "but did not find it. Rust bridge log:\n"
            f"  {rust_lines!r}"
        )
    else:
        # Extract total_steps + dispatch parameters for the report.
        m = re.search(r"total_steps=(\d+)", rust_lines)
        ts = m.group(1) if m else "<unknown>"
        marker = re.escape(fixture.delegation_log_marker)
        m2 = re.search(
            marker + r".*?eal=(0x[0-9a-fA-F]+).*?size=(\d+).*?tag=(\d+)",
            rust_lines,
        )
        if m2:
            print(
                f"  OK: {fixture.rust_log_intro} dispatched "
                f"(eal={m2.group(1)} size={m2.group(2)} tag={m2.group(3)}); "
                f"DELEGATED EXECUTION OK (total_steps={ts})"
            )
        else:
            print(f"  OK: DELEGATED EXECUTION OK (total_steps={ts})")

    # ----- Phase 3: replay oracle -----
    print("\n[3/3] Running replay-oracle test ...")
    passed, tail = run_replay_test(fixture.rust_test_target)
    if not passed:
        violations.append(
            f"replay oracle: {fixture.rust_test_target} did NOT pass.\n"
            "  cargo test tail:\n" + "    " + tail.replace("\n", "\n    ")
        )
    else:
        print(
            "  OK: cargo test -p rpcs3-spu-recompiler "
            f"--test {fixture.rust_test_target} passed"
        )

    # ----- Verdict -----
    print()
    if violations:
        print(
            f"FAIL: {len(violations)} violation(s) in triple-symmetry contract",
            file=sys.stderr,
        )
        for v in violations:
            print(f"  - {v}", file=sys.stderr)
        return 1

    print(f"OK: triple-symmetry verified for {fixture.name}")
    print(f"  bridge OFF TTY  : {fixture.canonical_tty_substr}")
    print(f"  bridge ON  TTY  : {fixture.canonical_tty_substr}")
    print("  replay oracle    : diff_snapshots(interp, jit).is_identical() == true")
    print(f"  canonical status: {fixture.canonical_status_summary}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
