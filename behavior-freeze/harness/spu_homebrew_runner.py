#!/usr/bin/env python3
"""
spu_homebrew_runner.py — execute a SPU ELF via the Rust port and (when
provided) RPCS3 C++, then diff the resulting state dumps.

Modes:

  1) Single-runtime dump:
       python3 spu_homebrew_runner.py \
           --elf fixtures/spu/hello.elf \
           --rust-runner ../../rust/target/release/spu-runner \
           --output baselines/spu_hello/rust/

  2) Differential against C++ (when RPCS3 supports standalone SPU
     headless execution — see HOMEBREW_PLAN P5):
       python3 spu_homebrew_runner.py \
           --elf fixtures/spu/hello.elf \
           --rust-runner ../../rust/target/release/spu-runner \
           --rpcs3-binary /path/to/rpcs3 \
           --output baselines/spu_hello/

  3) Diff two existing dump dirs (no execution):
       python3 spu_homebrew_runner.py --diff dump_a/ dump_b/

Outputs (mode 1 / 2):
  <output>/rust/{gpr.csv,pc.txt,ls.bin,summary.txt}
  <output>/rpcs3/{...}                  (mode 2 only)
  <output>/diff.txt                     (mode 2: textual diff report)
"""

from __future__ import annotations

import argparse
import subprocess
import sys
from pathlib import Path


# --------------------------------------------------------------------
# Subprocess wrapper around `spu-runner`
# --------------------------------------------------------------------

def run_rust(rust_runner: Path, elf: Path, out: Path, max_steps: int) -> int:
    out.mkdir(parents=True, exist_ok=True)
    cmd = [str(rust_runner), str(elf),
           '--out-dir', str(out),
           '--max-steps', str(max_steps)]
    print(f'$ {" ".join(cmd)}', file=sys.stderr)
    return subprocess.call(cmd)


def run_rpcs3(_rpcs3: Path, _elf: Path, _out: Path) -> int:
    """Placeholder for RPCS3 standalone SPU dump.

    Upstream RPCS3 does not currently expose a `--spu-test <ELF>` flag
    that produces compatible {gpr,ls,pc} dumps. Capturing this requires
    either:
      * a small RPCS3 patch adding the dump CLI, OR
      * driving a full PPU process that loads the SPU image via
        sys_spu_thread_initialize and dumps via gdb / save state.

    See HOMEBREW_PLAN.md P5 for the decision tree. Until then, mode 2
    falls back to mode 1.
    """
    print('warning: RPCS3 standalone SPU dump not yet implemented '
          '(see HOMEBREW_PLAN P5).', file=sys.stderr)
    return 1


# --------------------------------------------------------------------
# Diff
# --------------------------------------------------------------------

def diff_dumps(a: Path, b: Path) -> tuple[bool, str]:
    """Return (equal, report)."""
    report: list[str] = []
    eq = True

    # GPR diff (line-by-line — order matters and lines are short).
    a_gpr = (a / 'gpr.csv').read_text().splitlines()
    b_gpr = (b / 'gpr.csv').read_text().splitlines()
    for i, (la, lb) in enumerate(zip(a_gpr, b_gpr)):
        if la != lb:
            eq = False
            report.append(f'  GPR mismatch r{i}:\n    A: {la}\n    B: {lb}')
    if len(a_gpr) != len(b_gpr):
        eq = False
        report.append(f'  GPR line count differs: {len(a_gpr)} vs {len(b_gpr)}')

    # PC diff.
    pa = (a / 'pc.txt').read_text()
    pb = (b / 'pc.txt').read_text()
    if pa != pb:
        eq = False
        report.append(f'  PC differs:\n    A: {pa.strip()}\n    B: {pb.strip()}')

    # LS diff: byte-wise. Report first 5 differing 16-byte windows.
    la_ls = (a / 'ls.bin').read_bytes()
    lb_ls = (b / 'ls.bin').read_bytes()
    if len(la_ls) != len(lb_ls):
        eq = False
        report.append(f'  LS size differs: {len(la_ls)} vs {len(lb_ls)}')
    else:
        diffs = 0
        for off in range(0, len(la_ls), 16):
            chunk_a = la_ls[off:off + 16]
            chunk_b = lb_ls[off:off + 16]
            if chunk_a != chunk_b:
                eq = False
                if diffs < 5:
                    report.append(
                        f'  LS @ 0x{off:05x}:\n'
                        f'    A: {chunk_a.hex()}\n'
                        f'    B: {chunk_b.hex()}'
                    )
                diffs += 1
        if diffs > 5:
            report.append(f'  ... {diffs - 5} more LS chunks differ')

    if eq:
        return True, 'IDENTICAL — every byte matches.'
    return False, '\n'.join(report)


# --------------------------------------------------------------------
# CLI
# --------------------------------------------------------------------

def main() -> int:
    p = argparse.ArgumentParser(description=__doc__.split('\n\n', 1)[0])
    p.add_argument('--elf', type=Path)
    p.add_argument('--rust-runner', type=Path)
    p.add_argument('--rpcs3-binary', type=Path, default=None)
    p.add_argument('--output', type=Path)
    p.add_argument('--max-steps', type=int, default=1_000_000)
    p.add_argument('--diff', nargs=2, type=Path, metavar=('DUMP_A', 'DUMP_B'),
                   help='diff two existing dump directories')
    args = p.parse_args()

    if args.diff:
        a, b = args.diff
        eq, report = diff_dumps(a, b)
        print(report)
        return 0 if eq else 1

    if not args.elf or not args.rust_runner or not args.output:
        p.error('--elf, --rust-runner, --output are required (or use --diff)')

    if not args.elf.exists():
        print(f'error: ELF missing: {args.elf}', file=sys.stderr)
        return 2
    if not args.rust_runner.exists():
        print(f'error: rust runner missing: {args.rust_runner}', file=sys.stderr)
        print('Build it: `cargo build -p spu-runner --release`', file=sys.stderr)
        return 2

    rust_dir = args.output / 'rust'
    rc_rust = run_rust(args.rust_runner, args.elf, rust_dir, args.max_steps)

    if args.rpcs3_binary:
        rpcs3_dir = args.output / 'rpcs3'
        rpcs3_dir.mkdir(parents=True, exist_ok=True)
        rc_cpp = run_rpcs3(args.rpcs3_binary, args.elf, rpcs3_dir)
        if rc_cpp == 0:
            eq, report = diff_dumps(rust_dir, rpcs3_dir)
            (args.output / 'diff.txt').write_text(report + '\n')
            print(report)
            return 0 if eq else 1

    return rc_rust


if __name__ == '__main__':
    sys.exit(main())
