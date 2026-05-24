# R9.1f — Bug investigation: PSL1GHT runtime init missing

**Status:** INVESTIGATION (read + 1 diag-only commit landed
2026-05-23).
**Trigger:** R9.1e batch landed deep PPU progression, but
control flow ended at CIA=0x7C0802A4 (unmapped) — initial
suspicion was an opcode-handler bug in the new batch.
**Conclusion:** The 10 new opcodes are correct. The crash is
caused by **missing lv2 runtime initialization** that PSL1GHT
binaries expect to run BEFORE `_start`.

## Diagnostic findings

R9.1f extended `tests/run_self_smoke.rs` to dump CTR + r0-r12
+ LR at run termination. Output at the unmapped CIA:

```
CIA = 0x7C0802A4
LR  = 0x000000000002AB88 (deep in _start)
CTR = 0x000000007C0802A6 ← matches the `mflr r0` inst encoding
r0  = 0x000000007C0802A6 ← same value as CTR
r1  = 0xCFFFFCF0 (multiple frames pushed)
r2  = 0xF8010010 ← matches `std r0, 0x10(r1)` inst encoding
r11 = 0x000108480003A548 ← {code=0x10848, toc=0x3A548} as u64
r12 = 0x000000000002AB60 (current function)
```

The CTR=0x7C0802A6 → bcctr masks low 2 bits → CIA=0x7C0802A4.
The "primary 31 XO=90" reason was a misdirection — the actual
fault is bcctr to unmapped.

## Root cause analysis

The function at vaddr 0x2AB60 has this prologue (disassembled
from .self):

```
0x2AB60: mflr  r0                  ; save LR
0x2AB64: std   r0,  0x10(r1)       ; spill LR
0x2AB68: std   r2,  0x28(r1)       ; spill TOC
0x2AB6C: stdu  r1, -0x80(r1)       ; alloc frame
0x2AB70: addis r12, 0, 3           ; r12 = 0x30000 (literal)
0x2AB74: lwz   r12, 0x108(r12)     ; r12 = mem32[0x30108]
0x2AB78: lwz   r0,  0(r12)         ; r0  = mem32[r12]
0x2AB7C: lwz   r2,  4(r12)         ; r2  = mem32[r12+4]
0x2AB80: mtctr r0                  ; CTR = r0
0x2AB84: bcctrl                    ; INDIRECT CALL via CTR ← crash
```

This is the standard ELFv1 PPC32 indirect-call pattern via
**function descriptor table** at `0x30108`:
- The table holds FD pointers (one per imported function)
- Each FD = `{ u32 code, u32 toc }`
- Caller: `r12 = mem[table_entry]; r0 = mem[r12+0]; r2 = mem[r12+4]; CTR=r0; bcctrl`

What we observe:
- `mem[0x30108]` = `0x0002AB60` (= the function we're INSIDE!)
- `mem[0x2AB60]` = `0x7C0802A6` (the `mflr r0` opcode at the
  function's entry, NOT a code address)
- `mem[0x2AB64]` = `0xF8010010` (the next `std` opcode)

So the FD pointer table at 0x30108 contains **CODE OFFSETS,
not FD pointers**. The PSL1GHT runtime expects the table to
have been **rewritten by the lv2 loader** (or by an earlier
init routine in crt0) to convert offsets-to-FDs into pointers-
to-real-FDs before `_start` runs.

## Why our R9.1e opcodes are NOT to blame

Cross-checked the 10 new handlers (addze, mtcrf, stdx, lbzx,
lwzx, nor, ldx, mulld, mfcr, lfdx) against PowerPC ISA:
- All field extractions are correct (op.rd(), op.ra(), op.rb(),
  op.rs() all map to bits 6-10 / 11-15 / 16-20 as expected).
- Memory addresses computed via `ra_or_zero(ra) + gpr[rb]` for
  X-form indexed.
- `mtcrf` FXM bit ordering verified (FXM MSB controls CR0).
- `mfcr` uses `ppu.cr.pack()` which is the canonical packer.

The R9.1e batch causes the PPU to reach this indirect-call
site successfully (it didn't reach this depth before R9.1e).
The crash there is **inherent to the missing runtime init**,
not a regression introduced by R9.1e.

Confirmed by: cargo test --workspace --tests still 264 / 0
fails; 20 oracles untouched; 136 ppu-interpreter unit tests
unchanged. R9.1e is sound.

## Strategy revision: Option C (main() bypass) is now warranted

The R9 plan's § 3 originally listed 5 slice tiers ending at
"R9.1e" — the audit's Option B has shipped (R9.1d+R9.1e). The
empirical crash reveals that completing PSL1GHT's `_start`
boot requires either:

**Path A: Implement the lv2 process startup runtime**
- Pre-populate the .opd / FD-pointer tables (~hundreds of FDs
  per binary)
- Resolve dynamic relocations
- Initialize TLS
- Estimated scope: 200-500 LOC of "loader patch" code + audit
  of all PSL1GHT runtime tables

**Path B: Locate `main()` and skip `_start`**
- Parse the ELF symbol table to find `main`'s vaddr
- Set CIA = main's code address (via FD deref)
- Marshal argc/argv into r3/r4
- Skip PSL1GHT's crt0 entirely
- Estimated scope: 100-150 LOC (symbol table parser + entry)

Path B is **significantly cheaper** and aligns better with
the R9 goal of "drive the 20 existing CC0 oracles end-to-end."
The oracles' `main()` is what produces the canonical TTY and
exercises SPU group syscalls — that's what we actually care
about validating.

## R9.1f outcome

This commit ships only the diagnostic extension to
`run_self_smoke.rs` (CTR + GPR0-12 dump) — no code-path changes
to load_elf / run_self / interpreter. The investigation
artifact is this document.

**Recommended next slice: R9.1g — Path B: main() symbol lookup
+ direct entry**. After landing, we can skip the runtime-init
gap and reach `main()` → SPU group syscalls → integration
test passes.

**Validation:**
- cargo test --workspace --tests --release: 264 blocks, 0 fails
- Smoke test still passes (Memory error in accepted outcomes)
- 20 SPU oracles ✓
- No regression
