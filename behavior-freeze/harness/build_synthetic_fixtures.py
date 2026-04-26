#!/usr/bin/env python3
"""
build_synthetic_fixtures.py — generate the canonical SPU smoke fixtures.

Run this whenever the encoder format or the program shape needs to
change. Output is committed under `behavior-freeze/fixtures/spu/`.

Each fixture exercises a different family of opcodes, so that running
all of them through `spu-runner` covers a meaningful slice of the
interpreter without depending on any external homebrew.
"""

from __future__ import annotations

import struct
import sys
from pathlib import Path


# ----- ELF builder ----------------------------------------------------

EI_NIDENT = 16
ELF_HEADER_SIZE = 52
PHDR_SIZE = 32
ET_EXEC = 2
EM_SPU = 0x17
PT_LOAD = 1


def build_minimal_spu_elf(load_lsa: int, code: list[int]) -> bytes:
    out = bytearray()
    out += b'\x7fELF'
    out += bytes([1, 2, 1, 0])
    out += bytes(EI_NIDENT - len(out))
    out += struct.pack('>HHIIIIIHHHHHH',
                       ET_EXEC, EM_SPU, 1,
                       load_lsa,
                       ELF_HEADER_SIZE, 0, 0,
                       ELF_HEADER_SIZE, PHDR_SIZE, 1,
                       0, 0, 0)
    code_size = len(code) * 4
    code_offset = ELF_HEADER_SIZE + PHDR_SIZE
    out += struct.pack('>IIIIIIII',
                       PT_LOAD, code_offset, load_lsa, load_lsa,
                       code_size, code_size, 5, 4)
    for inst in code:
        out += struct.pack('>I', inst)
    return bytes(out)


# ----- SPU encoders (mirrored from rpcs3-spu-interpreter::encode) -----

def il(rt: int, imm16: int) -> int:
    return ((0x081 & 0x1FF) << 23) | ((imm16 & 0xFFFF) << 7) | (rt & 0x7F)

def ila(rt: int, imm18: int) -> int:
    return ((0x21) << 25) | ((imm18 & 0x3FFFF) << 7) | (rt & 0x7F)

def ai(rt: int, ra: int, imm10: int) -> int:
    return ((0x1C & 0xFF) << 24) | ((imm10 & 0x3FF) << 14) | ((ra & 0x7F) << 7) | (rt & 0x7F)

def stop(code: int) -> int:
    # Primary 0x000 occupies bits 0..10 (all zero); the 14-bit code
    # sits at MSB bits 18..31 → LSB bits 0..13 (no shift).
    return code & 0x3FFF

def nop() -> int:
    return 0x4020_0000

def pack_rr(primary: int, rt: int, ra: int, rb: int) -> int:
    return ((primary & 0x7FF) << 21) | ((rb & 0x7F) << 14) | ((ra & 0x7F) << 7) | (rt & 0x7F)

def pack_ri7(primary: int, rt: int, ra: int, imm7: int) -> int:
    return ((primary & 0x7FF) << 21) | ((imm7 & 0x7F) << 14) | ((ra & 0x7F) << 7) | (rt & 0x7F)

def a(rt, ra, rb): return pack_rr(0x0C0, rt, ra, rb)  # canonical SPU primary 0xC0
def sf(rt, ra, rb): return pack_rr(0x040, rt, ra, rb)  # canonical 0x40
def shl(rt, ra, rb): return pack_rr(0x05B, rt, ra, rb)
def rot(rt, ra, rb): return pack_rr(0x058, rt, ra, rb)
def or_(rt, ra, rb): return pack_rr(0x041, rt, ra, rb)
def xor_(rt, ra, rb): return pack_rr(0x241, rt, ra, rb)
def and_(rt, ra, rb): return pack_rr(0x0C1, rt, ra, rb)  # canonical 0xC1
def fa(rt, ra, rb): return pack_rr(0x2C4, rt, ra, rb)
def fm(rt, ra, rb): return pack_rr(0x2C6, rt, ra, rb)
def shli(rt, ra, imm): return pack_ri7(0x07B, rt, ra, imm)
def roti(rt, ra, imm): return pack_ri7(0x078, rt, ra, imm)
def br(imm16: int) -> int:
    # primary 0x064 (9-bit), imm16 at bits 9..24
    return (0x064 << 23) | ((imm16 & 0xFFFF) << 7)
def brnz(rt: int, imm16: int) -> int:
    return (0x042 << 23) | ((imm16 & 0xFFFF) << 7) | (rt & 0x7F)
def brsl(rt: int, imm16: int) -> int:
    return (0x066 << 23) | ((imm16 & 0xFFFF) << 7) | (rt & 0x7F)
def bi(ra: int) -> int:
    # 11-bit primary 0x1A8 (RR-form unary)
    return (0x1A8 << 21) | ((ra & 0x7F) << 7)
def ceqi(rt, ra, imm10): return ((0x7C & 0xFF) << 24) | ((imm10 & 0x3FF) << 14) | ((ra & 0x7F) << 7) | (rt & 0x7F)
def ah(rt, ra, rb): return pack_rr(0x0C8, rt, ra, rb)
def cg(rt, ra, rb): return pack_rr(0x0C2, rt, ra, rb)
def orx(rt, ra): return (0x1F0 << 21) | ((ra & 0x7F) << 7) | (rt & 0x7F)
def lqd(rt: int, ra: int, imm10: int) -> int:
    # 8-bit primary 0x34, imm10 / 16 (the ISA stores the dword index)
    return ((0x34 & 0xFF) << 24) | ((imm10 & 0x3FF) << 14) | ((ra & 0x7F) << 7) | (rt & 0x7F)
def stqd(rt: int, ra: int, imm10: int) -> int:
    return ((0x24 & 0xFF) << 24) | ((imm10 & 0x3FF) << 14) | ((ra & 0x7F) << 7) | (rt & 0x7F)
def lqx(rt, ra, rb): return pack_rr(0x1C4, rt, ra, rb)
def stqx(rt, ra, rb): return pack_rr(0x144, rt, ra, rb)
def ila_(rt, imm18): return (0x21 << 25) | ((imm18 & 0x3FFFF) << 7) | (rt & 0x7F)
def ilh(rt: int, imm16: int) -> int:
    return ((0x083 & 0x1FF) << 23) | ((imm16 & 0xFFFF) << 7) | (rt & 0x7F)
def shlhi(rt: int, ra: int, imm7: int) -> int:
    return ((0x07F & 0x7FF) << 21) | ((imm7 & 0x7F) << 14) | ((ra & 0x7F) << 7) | (rt & 0x7F)
def rothmi(rt: int, ra: int, imm7: int) -> int:
    return ((0x07D & 0x7FF) << 21) | ((imm7 & 0x7F) << 14) | ((ra & 0x7F) << 7) | (rt & 0x7F)
def rothi(rt: int, ra: int, imm7: int) -> int:
    return ((0x07C & 0x7FF) << 21) | ((imm7 & 0x7F) << 14) | ((ra & 0x7F) << 7) | (rt & 0x7F)


# ----- Programs -------------------------------------------------------

def program_il_stop() -> list[int]:
    """Original sentinel: il r3, 0x1234; stop 0."""
    return [il(3, 0x1234), stop(0)]


def program_arith() -> list[int]:
    """Exercise ALU + bitwise + shift + float lanes.

    r3 = 5
    r4 = 3
    r5 = r3 + r4   = 8
    r6 = r3 - r4   = 2 (sf computes rb-ra so sf rt,ra,rb -> rb-ra; we want 5-3 → swap order)
    r7 = r3 << 2   = 20
    r8 = r3 ^ r4   = 6
    r9 = r3 | r4   = 7
    r10 = r3 & r4  = 1
    stop 0x4242
    """
    return [
        il(3, 5),
        il(4, 3),
        a(5, 3, 4),
        sf(6, 4, 3),       # sf rt,ra,rb = rb-ra → r3-r4 = 2
        shli(7, 3, 2),
        xor_(8, 3, 4),
        or_(9, 3, 4),
        and_(10, 3, 4),
        stop(0x4242),
    ]


def program_loop() -> list[int]:
    """Counted loop: sums 1+2+3+...+10 into r3, exits with code = sum.

    Uses brnz back-edge with negative offset.

    Layout (PC offsets from entry 0x100):
      0x100  ila r3, 0          ; accumulator
      0x104  ila r4, 1          ; counter starts at 1
      0x108: a r3, r3, r4       ; r3 += r4   (loop top)
      0x10C  ai r4, r4, 1       ; r4 += 1
      0x110  ceqi r5, r4, 11    ; r5 = (r4 == 11) ? 0xFFFFFFFF : 0
      0x114  brnz r5, +2        ; if r5 != 0 (preferred lane), skip back-branch
      0x118  br -4              ; jump back 4 instructions to loop top (offset -16/4 = -4)
      0x11C  stop 0x55          ; final stop

    Expected: r3 = 1+2+...+10 = 55 (== 0x37) broadcast to all lanes.
    """
    # Compute relative offsets in word-units from each branch's PC.
    # br offset is in bytes-units? No — encoded as imm16 then *4. We
    # supply the imm16 directly. brnz: imm16 (signed) is added to PC
    # then PC = (PC + imm16*4). We want to skip the next instruction
    # (br) so we jump +2 words = imm16 = +2 → offset +8 bytes.
    # br jumps back 4 words (16 bytes) → imm16 = -4.
    return [
        ila(3, 0),                # 0x100
        ila(4, 1),                # 0x104
        a(3, 3, 4),               # 0x108: loop top
        ai(4, 4, 1),              # 0x10C
        ceqi(5, 4, 11),           # 0x110
        brnz(5, 2),               # 0x114: if r5 nonzero (preferred lane), branch +2 words → 0x11C
        br(-4),                   # 0x118: branch back to 0x108
        stop(0x55),               # 0x11C
    ]


def program_float_dot() -> list[int]:
    """Compute a 4-lane float dot product into r3 lane 0.

    Setup:
      r4 = [1.0, 2.0, 3.0, 4.0]   broadcast/setup is hard with il alone, so
                                   we use il + iohl + ila on individual lanes
                                   — for simplicity, reuse il which broadcasts
                                   so each lane gets the same value.
      r5 = [2.0, 2.0, 2.0, 2.0]

    r6 = fm r4, r5  → [2.0, 2.0, 2.0, 2.0] (dot doesn't really compute)

    Since `il` broadcasts an integer immediate, encoding distinct floats
    per lane requires more setup than fits in a smoke fixture. So this
    program instead exercises fa/fm with broadcast values:
      r3 = 0x40000000 (float 2.0) broadcast via il+ila
      r4 = same
      r5 = fa r3, r4   → 4.0 each lane
      r6 = fm r3, r4   → 4.0 each lane
      r7 = fa r5, r6   → 8.0 each lane
      stop 0x66

    The stored bit pattern of r7 lane 0 = 0x41000000 (float 8.0).
    """
    # ila takes 18-bit immediate → only loads up to 0x3FFFF. We need
    # 0x40000000. Workaround: ila r3, 0; shli r3, r3, 0 → r3 = 0;
    # then ai with negative imm... — simpler: il loads sign-extended
    # 16-bit. il(3, 0x4000) gives 0x00004000 broadcast. shli r3, r3, 16
    # → 0x40000000. That's float 2.0.
    return [
        il(3, 0x4000),            # r3 = 0x00004000 broadcast
        shli(3, 3, 16),           # r3 = 0x40000000 (= float 2.0)
        a(4, 3, 0),               # r4 = r3 + r0(=0) = same
        fa(5, 3, 4),              # r5 = 2.0 + 2.0 = 4.0 (0x40800000)
        fm(6, 3, 4),              # r6 = 2.0 * 2.0 = 4.0
        fa(7, 5, 6),              # r7 = 4.0 + 4.0 = 8.0 (0x41000000)
        stop(0x66),
    ]


def program_loadstore() -> list[int]:
    """Round-trip a known pattern through LS via stqd then lqd.

    Steps:
      r3 = il 0x5A5A (broadcast)             ; pattern across 4 lanes
      r4 = ila 0x40                          ; base address (LSA)
      stqd r3, 0x40(r4=0x40)? => offset 0x40 + 1*16 = 0x50
        Actually stqd uses signed imm10 << 4. We pass imm10=1 so the
        offset is 16 bytes, stored at LSA = r4 lane0 + 16 = 0x50.
      r5 = lqd 1(r4)                         ; load the same address
      stop 0xAB

    Expected: r5 == r3 (== 0x5A5A broadcast).
    """
    return [
        il(3, 0x5A5A),                # 0x100
        ila_(4, 0x40),                # 0x104  r4 = 0x40 broadcast
        stqd(3, 4, 1),                # 0x108  store r3 at LSA 0x50
        lqd(5, 4, 1),                 # 0x10C  load from LSA 0x50 into r5
        stop(0xAB),                   # 0x110
    ]


def program_shifts() -> list[int]:
    """Exercise word shift family (immediate variants):
      r3 = il 1
      r4 = shli r3, 5                ; r4 = 1 << 5 = 32 (broadcast)
      r5 = il 0xFF00 then iohl 0xFFFF  -> r5 = 0xFFFFFF00 (broadcast)
      r6 = rotmi r5, -8              ; r6 = r5 >> 8 = 0x00FFFFFF
      r7 = rotmai r5, -8             ; signed shr by 8 → sign fill = 0xFFFFFFFF
      r8 = roti r3, 4                ; r8 = rotate-left 1 by 4 = 16
      stop 0x77
    """
    iohl = lambda rt, imm: ((0x0C1 & 0x1FF) << 23) | ((imm & 0xFFFF) << 7) | (rt & 0x7F)
    return [
        il(3, 1),
        shli(4, 3, 5),
        il(5, 0xFF00 - 0x10000),     # signed 16-bit -> 0xFFFFFF00 broadcast (sign-extended)
        pack_ri7(0x079, 6, 5, (-8) & 0x7F),  # rotmi r6, r5, -8
        pack_ri7(0x07A, 7, 5, (-8) & 0x7F),  # rotmai r7, r5, -8
        roti(8, 3, 4),
        stop(0x77),
    ]


def program_brsl_ret() -> list[int]:
    """Function-call style: brsl to a small subroutine that adds 7,
    then `bi` back through the link register.

    Layout (offsets from entry 0x100):
      0x100  il r3, 10              ; argument
      0x104  brsl r5, +3            ; jump to 0x100+3*4 = 0x110, link in r5
      0x108  stop 0x99              ; final stop after return
      0x10C  nop                    ; padding
      0x110  ai r3, r3, 7           ; subroutine: r3 += 7
      0x114  bi r5                  ; return via link
    """
    return [
        il(3, 10),                    # 0x100
        brsl(5, 3),                   # 0x104  → 0x110, link = 0x108
        stop(0x99),                   # 0x108
        nop(),                        # 0x10C  padding
        ai(3, 3, 7),                  # 0x110  r3 += 7 → 17
        bi(5),                        # 0x114  return
    ]


def program_halfword_shifts() -> list[int]:
    """Exercise per-halfword shift family.

      r3 = ilh 0x00FF              ; broadcasts 0x00FF to all 8 halves
      r4 = shlhi r3, 4             ; each half = 0x00FF << 4 = 0x0FF0
      r5 = rothmi r3, -4           ; each half = 0x00FF >> 4 = 0x000F
      r6 = rothi r3, 8             ; each half = rotate-left 0x00FF by 8 = 0xFF00
      stop 0xDD
    """
    return [
        ilh(3, 0x00FF),
        shlhi(4, 3, 4),
        rothmi(5, 3, -4),
        rothi(6, 3, 8),
        stop(0xDD),
    ]


def program_orx_collapse() -> list[int]:
    """Test orx (or-across-word-lanes).

      r3 = il 0x1234
      r4 = il 0x5678
      r5 = ah r3, r4   ; per-halfword add
      r6 = orx r5      ; collapse to preferred slot
      stop 0xCC
    """
    return [
        il(3, 0x1234),
        il(4, 0x5678),
        ah(5, 3, 4),
        orx(6, 5),
        stop(0xCC),
    ]


# ----- Main -----------------------------------------------------------

def main() -> int:
    here = Path(__file__).resolve().parent
    out_dir = here.parent / 'fixtures' / 'spu'
    out_dir.mkdir(parents=True, exist_ok=True)

    fixtures = {
        'synthetic_il_stop.elf': program_il_stop(),
        'synthetic_arith.elf': program_arith(),
        'synthetic_loop.elf': program_loop(),
        'synthetic_float_dot.elf': program_float_dot(),
        'synthetic_loadstore.elf': program_loadstore(),
        'synthetic_shifts.elf': program_shifts(),
        'synthetic_brsl_ret.elf': program_brsl_ret(),
        'synthetic_orx_collapse.elf': program_orx_collapse(),
        'synthetic_halfword_shifts.elf': program_halfword_shifts(),
    }

    for name, code in fixtures.items():
        elf = build_minimal_spu_elf(0x100, code)
        path = out_dir / name
        path.write_bytes(elf)
        print(f'wrote {path.relative_to(here.parent)} ({len(elf)} bytes, {len(code)} insts)')

    return 0


if __name__ == '__main__':
    sys.exit(main())
