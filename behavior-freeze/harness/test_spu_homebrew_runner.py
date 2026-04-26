#!/usr/bin/env python3
"""
test_spu_homebrew_runner.py — self-test for the SPU homebrew harness.

Builds a synthetic SPU ELF, runs it through `spu-runner`, then runs it
again into a second dir and verifies the diff is empty (proves the
diff path works end-to-end with an identical-source baseline).

Usage:
    python3 test_spu_homebrew_runner.py [--rust-runner PATH]

Default `--rust-runner` resolves to the workspace's release build.
"""

from __future__ import annotations

import argparse
import os
import struct
import subprocess
import sys
import tempfile
from pathlib import Path


def build_minimal_spu_elf(load_lsa: int, code: list[int]) -> bytes:
    EI_NIDENT = 16
    ELF_HEADER_SIZE = 52
    PHDR_SIZE = 32
    ET_EXEC = 2
    EM_SPU = 0x17
    PT_LOAD = 1

    out = bytearray()
    # ELF identification
    out += b'\x7fELF'
    out += bytes([1, 2, 1, 0])           # ELFCLASS32, ELFDATA2MSB, EV_CURRENT, OSABI=NONE
    out += bytes(EI_NIDENT - len(out))   # pad

    # ELF header (BE)
    out += struct.pack('>HHIIIIIHHHHHH',
                       ET_EXEC, EM_SPU, 1,       # e_type, e_machine, e_version
                       load_lsa,                 # e_entry
                       ELF_HEADER_SIZE, 0, 0,    # e_phoff, e_shoff, e_flags
                       ELF_HEADER_SIZE, PHDR_SIZE, 1,  # ehsize, phentsize, phnum
                       0, 0, 0)                  # shentsize, shnum, shstrndx

    # Program header (PT_LOAD)
    code_size = len(code) * 4
    code_offset = ELF_HEADER_SIZE + PHDR_SIZE
    out += struct.pack('>IIIIIIII',
                       PT_LOAD, code_offset, load_lsa, load_lsa,
                       code_size, code_size, 5, 4)

    # Code (BE)
    for inst in code:
        out += struct.pack('>I', inst)

    return bytes(out)


def il(rt: int, imm16: int) -> int:
    return ((0x081 & 0x1FF) << 23) | ((imm16 & 0xFFFF) << 7) | (rt & 0x7F)


def stop(code: int) -> int:
    # Primary 0x000 in bits 0..10 (all zero); 14-bit code sits at the
    # low 14 bits (MSB bits 18..31).
    return code & 0x3FFF


def find_default_runner() -> Path:
    here = Path(__file__).resolve().parent
    workspace = here.parent.parent / 'rust'
    candidates = [
        workspace / 'target' / 'release' / 'spu-runner.exe',
        workspace / 'target' / 'release' / 'spu-runner',
    ]
    for c in candidates:
        if c.exists():
            return c
    raise FileNotFoundError(
        'spu-runner not built. Run: cargo build -p spu-runner --release')


def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument('--rust-runner', type=Path, default=None)
    args = p.parse_args()

    rust_runner = args.rust_runner or find_default_runner()
    print(f'using runner: {rust_runner}')

    here = Path(__file__).resolve().parent
    harness = here / 'spu_homebrew_runner.py'

    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        elf = build_minimal_spu_elf(0x100, [il(3, 0x1234), stop(0)])
        elf_path = tmp / 'hello.elf'
        elf_path.write_bytes(elf)

        run_a = tmp / 'run_a'
        run_b = tmp / 'run_b'

        for out in (run_a, run_b):
            rc = subprocess.call([
                sys.executable, str(harness),
                '--elf', str(elf_path),
                '--rust-runner', str(rust_runner),
                '--output', str(out),
            ])
            if rc != 0:
                print(f'rust execution failed (rc={rc})', file=sys.stderr)
                return 1

        # Diff identical runs — should be IDENTICAL.
        rc = subprocess.call([
            sys.executable, str(harness),
            '--diff', str(run_a / 'rust'), str(run_b / 'rust'),
        ])
        if rc != 0:
            print('FAIL: diff between two identical runs returned non-zero',
                  file=sys.stderr)
            return 1

        # Sanity-check the dump shape.
        gpr = (run_a / 'rust' / 'gpr.csv').read_text().splitlines()
        assert len(gpr) == 128, f'expected 128 GPR lines, got {len(gpr)}'
        assert gpr[3].endswith('00001234000012340000123400001234'), gpr[3]
        ls_size = (run_a / 'rust' / 'ls.bin').stat().st_size
        assert ls_size == 256 * 1024, f'expected 256 KB LS, got {ls_size}'

        print('OK: end-to-end harness self-test passed.')
        return 0


if __name__ == '__main__':
    sys.exit(main())
