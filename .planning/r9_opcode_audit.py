"""R9 opcode audit — scan PSL1GHT .text and enumerate unique
PowerPC instructions, grouped by primary opcode and (when applicable)
extended opcode XO.

This produces a coverage report that lets R9.1.x slices estimate
the remaining opcode work without trial-and-erroring each gap.

Usage:
    python .planning/r9_opcode_audit.py [SELF_PATH]

Default: single_spu_mailbox_v1.self
"""

from __future__ import annotations
import sys
import struct
from collections import Counter
from pathlib import Path

DEFAULT_SELF = (
    "behavior-freeze/fixtures/spu/sources/"
    "single_spu_mailbox_v1/build/single_spu_mailbox_v1.self"
)

# Opcode primary→name table (subset documented in PowerPC ISA Book I).
PRIMARY = {
    7: "mulli", 8: "subfic", 10: "cmpli", 11: "cmpi",
    12: "addic", 13: "addic.", 14: "addi", 15: "addis",
    16: "bc", 17: "sc", 18: "b",
    19: "branch-cond-ext", 20: "rlwimi", 21: "rlwinm", 23: "rlwnm",
    24: "ori", 25: "oris", 26: "xori", 27: "xoris", 28: "andi.", 29: "andis.",
    30: "rldicl/rldicr/rldic/rldcl/rldcr (MD/MDS-form)",
    31: "X/XO-form (mass)",
    32: "lwz", 33: "lwzu", 34: "lbz", 35: "lbzu", 36: "stw", 37: "stwu",
    38: "stb", 39: "stbu", 40: "lhz", 41: "lhzu", 42: "lha", 43: "lhau",
    44: "sth", 45: "sthu", 46: "lmw", 47: "stmw",
    48: "lfs", 49: "lfsu", 50: "lfd", 51: "lfdu", 52: "stfs", 53: "stfsu",
    54: "stfd", 55: "stfdu",
    58: "ld/ldu/lwa (DS-form)",
    59: "fp-single-ext",
    62: "std/stdu (DS-form)",
    63: "fp-double-ext",
}

# Primary 31 XO subset (X/XO-form). Common ones from PowerPC Book I.
# bits 21..30 (10 bits)
P31_XO = {
    0: "cmp",
    4: "tw",
    8: "subfc",
    10: "addc",
    11: "mulhwu",
    19: "mfcr",
    20: "lwarx",
    21: "ldx",
    23: "lwzx",
    24: "slw",
    26: "cntlzw",
    27: "sld",
    28: "and",
    32: "cmpl",
    40: "subf",
    53: "ldux",
    54: "dcbst",
    55: "lwzux",
    58: "cntlzd",
    60: "andc",
    68: "td",
    73: "mulhd",
    75: "mulhw",
    83: "mfmsr",
    84: "ldarx",
    86: "dcbf",
    87: "lbzx",
    104: "neg",
    119: "lbzux",
    122: "popcntb",
    124: "nor",
    136: "subfe",
    138: "adde",
    144: "mtcrf",
    146: "mtmsr",
    149: "stdx",
    150: "stwcx.",
    151: "stwx",
    181: "stdux",
    183: "stwux",
    200: "subfze",
    202: "addze",
    210: "mtsr",
    214: "stdcx.",
    215: "stbx",
    232: "subfme",
    234: "addme",
    235: "mullw",
    242: "mtsrin",
    246: "dcbtst",
    247: "stbux",
    266: "add",
    278: "dcbt",
    279: "lhzx",
    284: "eqv",
    310: "eciwx",
    311: "lhzux",
    316: "xor",
    339: "mfspr",
    341: "lwax",
    343: "lhax",
    370: "tlbia",
    371: "mftb",
    373: "lwaux",
    375: "lhaux",
    402: "slbmte",
    407: "sthx",
    412: "orc",
    438: "ecowx",
    439: "sthux",
    444: "or",
    457: "divdu",
    459: "divwu",
    467: "mtspr",
    470: "dcbi",
    476: "nand",
    489: "divd",
    491: "divw",
    498: "slbia",
    512: "mcrxr",
    533: "lswx",
    534: "lwbrx",
    535: "lfsx",
    536: "srw",
    539: "srd",
    566: "tlbsync",
    567: "lfsux",
    595: "mfsr",
    597: "lswi",
    598: "sync",
    599: "lfdx",
    631: "lfdux",
    659: "mfsrin",
    661: "stswx",
    662: "stwbrx",
    663: "stfsx",
    695: "stfsux",
    725: "stswi",
    727: "stfdx",
    759: "stfdux",
    790: "lhbrx",
    792: "sraw",
    794: "srad",
    824: "srawi",
    826: "sradi",
    854: "eieio",
    918: "sthbrx",
    922: "extsh",
    954: "extsb",
    982: "icbi",
    983: "stfiwx",
    986: "extsw",
    1014: "dcbz",
}


def primary_of(inst: int) -> int:
    return (inst >> 26) & 0x3F


def xo_p31(inst: int) -> int:
    # bits 21..30, 10 bits
    return (inst >> 1) & 0x3FF


def xo_ds(inst: int) -> int:
    # bits 30..31, 2 bits (primary 58 / 62)
    return inst & 0x3


def xo_md(inst: int) -> int:
    # bits 27..30, 4 bits (primary 30 family rldic*)
    return (inst >> 1) & 0xF


def xo_p19(inst: int) -> int:
    # bits 21..30 for primary 19 (bclr, bcctr, isync, ...)
    return (inst >> 1) & 0x3FF


def main(self_path: str) -> int:
    p = Path(self_path)
    if not p.exists():
        print(f"ERROR: {p} not found", file=sys.stderr)
        return 2

    data = p.read_bytes()

    # SCE header_length is BE u64 at offset 0x10
    header_length = struct.unpack(">Q", data[0x10:0x18])[0]
    elf_start = header_length

    # ELF header at elf_start. PT_LOAD PHDRs at elf_start + e_phoff.
    # e_phoff = BE u64 at ELF +0x20
    e_phoff = struct.unpack(">Q", data[elf_start + 0x20:elf_start + 0x28])[0]
    e_phnum = struct.unpack(">H", data[elf_start + 0x38:elf_start + 0x3A])[0]
    phdr_base = elf_start + e_phoff

    # Find executable PT_LOAD (the .text segment). flags & 1 = X.
    text_segments = []
    for i in range(e_phnum):
        ph = phdr_base + i * 0x38
        p_type = struct.unpack(">I", data[ph:ph + 4])[0]
        p_flags = struct.unpack(">I", data[ph + 4:ph + 8])[0]
        if p_type != 1:  # PT_LOAD
            continue
        if (p_flags & 0x1) == 0:  # not executable
            continue
        p_offset = struct.unpack(">Q", data[ph + 8:ph + 16])[0]
        p_vaddr = struct.unpack(">Q", data[ph + 16:ph + 24])[0]
        p_filesz = struct.unpack(">Q", data[ph + 32:ph + 40])[0]
        abs_offset = elf_start + p_offset
        text_segments.append({
            "file_offset": abs_offset,
            "vaddr": p_vaddr,
            "size": p_filesz,
        })

    print(f"=== {p.name} text segments ===")
    for s in text_segments:
        print(f"  vaddr=0x{s['vaddr']:08x} size={s['size']:#x} "
              f"file=0x{s['file_offset']:x}")
    print()

    # Walk every 4-byte instruction in each text segment. Count by
    # primary opcode and (for primary 31) by XO.
    primary_count: Counter[int] = Counter()
    p31_xo_count: Counter[int] = Counter()
    p19_xo_count: Counter[int] = Counter()
    p58_xo_count: Counter[int] = Counter()
    p62_xo_count: Counter[int] = Counter()
    p59_xo_count: Counter[int] = Counter()
    p63_xo_count: Counter[int] = Counter()
    p30_xo_count: Counter[int] = Counter()
    total_insts = 0

    for s in text_segments:
        bytes_ = data[s["file_offset"]:s["file_offset"] + s["size"]]
        for off in range(0, len(bytes_), 4):
            if off + 4 > len(bytes_):
                break
            inst = struct.unpack(">I", bytes_[off:off + 4])[0]
            if inst == 0:
                continue  # treat all-zero as padding/null
            pr = primary_of(inst)
            primary_count[pr] += 1
            total_insts += 1
            if pr == 31:
                p31_xo_count[xo_p31(inst)] += 1
            elif pr == 19:
                p19_xo_count[xo_p19(inst)] += 1
            elif pr == 58:
                p58_xo_count[xo_ds(inst)] += 1
            elif pr == 62:
                p62_xo_count[xo_ds(inst)] += 1
            elif pr == 59:
                p59_xo_count[xo_p31(inst)] += 1
            elif pr == 63:
                p63_xo_count[xo_p31(inst)] += 1
            elif pr == 30:
                p30_xo_count[xo_md(inst)] += 1

    print(f"Total non-zero insts: {total_insts}")
    print(f"Unique primary opcodes: {len(primary_count)}")
    print()

    print("=== Primary opcode coverage ===")
    for pr in sorted(primary_count.keys()):
        name = PRIMARY.get(pr, f"primary-{pr}")
        print(f"  {pr:3d} {name:40s} count={primary_count[pr]}")
    print()

    if p31_xo_count:
        print("=== Primary 31 XO subset (top 30 by count) ===")
        for xo, n in p31_xo_count.most_common(30):
            name = P31_XO.get(xo, f"XO={xo}")
            print(f"  XO {xo:4d} {name:30s} count={n}")
        print(f"  ... ({len(p31_xo_count)} total unique XOs)")
        print()

    if p19_xo_count:
        print("=== Primary 19 XO subset ===")
        for xo, n in sorted(p19_xo_count.items()):
            print(f"  XO {xo:4d} count={n}")
        print()

    if p62_xo_count:
        print("=== Primary 62 DS-XO ===")
        for xo, n in sorted(p62_xo_count.items()):
            name = {0: "std", 1: "stdu", 2: "stq"}.get(xo, f"XO={xo}")
            print(f"  XO {xo} {name:8s} count={n}")
        print()

    if p58_xo_count:
        print("=== Primary 58 DS-XO ===")
        for xo, n in sorted(p58_xo_count.items()):
            name = {0: "ld", 1: "ldu", 2: "lwa"}.get(xo, f"XO={xo}")
            print(f"  XO {xo} {name:8s} count={n}")
        print()

    if p30_xo_count:
        print("=== Primary 30 MD-XO ===")
        for xo, n in sorted(p30_xo_count.items()):
            print(f"  XO {xo:2d} count={n}")
        print()

    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1] if len(sys.argv) > 1 else DEFAULT_SELF))
