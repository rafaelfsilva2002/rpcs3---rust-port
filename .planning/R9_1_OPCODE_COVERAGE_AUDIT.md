# R9.1 — PSL1GHT opcode coverage audit

**Status:** AUDIT (read-only analysis + strategy doc, 2026-05-23
post R9.1c).
**Inputs:** [`R9_1_AUDIT_FINDINGS.md`](./R9_1_AUDIT_FINDINGS.md),
[`.planning/r9_opcode_audit.py`](./r9_opcode_audit.py), live state
of `rust/rpcs3-ppu-interpreter/src/lib.rs`.
**Output:** opcode gap matrix + recommended R9.1.x strategy.

## Headline

PSL1GHT-compiled `single_spu_mailbox_v1.self` uses **59 unique
primary opcodes** and **46 unique primary-31 XO subfields**.
The `rpcs3-ppu-interpreter` currently implements ~30 primary
opcodes and ~30 primary-31 XOs. **Estimated remaining gap: 20-25
primary opcodes + 15-20 primary-31 XOs + assorted DS/MD/X-form
sub-coverage**.

R9.1.x per-opcode slicing (10-40 LOC each, ~30 commits) is
tractable but slow. The audit recommends **R9.1d-batch**: pick
the top-10 highest-usage gaps + ship them in one or two larger
commits, then re-audit and decide whether to keep going or pivot
to `main()`-stub bypass.

## Raw audit data (mailbox_v1.self, 28,188 non-zero insts)

### Primary opcodes used

| Primary | Name | Count | Impl? | Notes |
|---|---|---|---|---|
| 0 | reserved/padding | 1161 | n/a | Likely .rodata trailing in .text scan |
| 1 | attn? | 6 | ✗ | Need verify |
| 4 | VX | 6 | ✓ | (Altivec subset) |
| 5 | reserved | 1 | n/a | |
| 6 | vector? | 4 | ✗ | |
| 7 | mulli | 33 | ✓ | Already in match |
| 8 | subfic | 42 | **✗** | **R9.1e candidate (high count)** |
| 9 | reserved | 4 | n/a | |
| 10 | cmpli | 210 | ✓ | |
| 11 | cmpi | 1773 | ✓ | |
| 12 | addic | 4 | **✗** | low priority |
| 13 | addic. | 16 | **✗** | low priority |
| 14 | addi | 3310 | ✓ | |
| 15 | addis | 236 | ✓ | |
| 16 | bc | 2430 | ✓ | |
| 17 | sc | 58 | ✓ | |
| 18 | b | 1774 | ✓ | |
| 19 | bcext | 730 | ✓ | (P19 XO coverage below) |
| 20 | rlwimi | 8 | **✗** | low priority |
| 21 | rlwinm | 602 | ✓ | |
| 23 | rlwnm | 1 | **✗** | trivial |
| 24 | ori | 1610 | ✓ | |
| 25 | oris | 40 | ✓ | |
| 26 | xori | 6 | ✓ | |
| 27 | xoris | 1 | ✓ | |
| 28 | andi. | 7 | ✓ | |
| 29 | andis. | 3 | ✓ | |
| 30 | rldic-family | 509 | ✓ | (P30 MD-XO coverage below) |
| 31 | XO_ALU | 5772 | ✓ | (P31 XO coverage below) |
| 32 | lwz | 1118 | ✓ | |
| 33 | lwzu | 27 | **✗** | **R9.1e candidate** |
| 34 | lbz | 325 | ✓ | |
| 35 | lbzu | 45 | **✗** | **R9.1e candidate (high count)** |
| 36 | stw | 527 | ✓ | |
| 37 | stwu | 12 | **✗** | low priority |
| 38 | stb | 164 | ✓ | |
| 39 | stbu | 37 | **✗** | **R9.1e candidate** |
| 40 | lhz | 107 | ✓ | |
| 41 | lhzu | 1 | **✗** | trivial |
| 42 | lha | 1 | ✓ | |
| 43 | lhau | 5 | **✗** | trivial |
| 44 | sth | 73 | ✓ | |
| 45 | sthu | 1 | **✗** | trivial |
| 46 | lmw | 2 | **✗** | low priority |
| 47 | stmw | 1 | **✗** | low priority |
| 48 | lfs | 51 | ✓ | |
| 49 | lfsu | 1 | **✗** | trivial |
| 50 | lfd | 85 | ✓ | |
| 52 | stfs | 5 | ✓ | |
| 54 | stfd | 51 | ✓ | |
| 55 | stfdu | 3 | **✗** | trivial |
| 56 | reserved | 2 | n/a | likely data |
| 57 | reserved | 4 | n/a | likely data |
| 58 | ld/ldu/lwa | 2633 | ✓ | (XO subset below) |
| 59 | FPS (fp-single) | 3 | ✓ | |
| 60 | reserved | 1 | n/a | |
| 61 | reserved | 2 | n/a | |
| 62 | std/stdu | 2394 | ✓ | (XO subset below) |
| 63 | FPD (fp-double) | 150 | ✓ | |

**Primary opcode gap count: ~12** (subfic, addic, addic., rlwimi,
rlwnm, lwzu, lbzu, stwu, stbu, lhzu, lhau, sthu, lmw, stmw, lfsu,
stfdu — most are "with-update" variants of existing implemented
ops).

### Primary 31 (XO_ALU) sub-coverage

Top-40 by occurrence, **bold = NOT YET IMPLEMENTED**:

| XO | Name | Count | Impl? |
|---|---|---|---|
| 444 | or | 2110 | ✓ |
| 986 | extsw | 1168 | ✓ |
| 467 | mtspr | 613 | ✓ |
| 266 | add | 450 | ✓ |
| 339 | mfspr | 332 | ✓ |
| 40 | subf | 231 | ✓ |
| 0 | cmp | 188 | ✓ |
| 32 | cmpl | 168 | ✓ |
| 922 | extsh | 72 | ✓ |
| 28 | and | 46 | ✓ |
| 824 | srawi | 43 | ✓ |
| 104 | neg | 27 | ✓ |
| **144** | **mtcrf** | 27 | **✗** |
| **202** | **addze** | 27 | **✗** ← R9.1e top of list |
| 60 | andc | 25 | ✓ |
| 24 | slw | 24 | ✓ |
| **149** | **stdx** | 23 | **✗** |
| **87** | **lbzx** | 23 | **✗** |
| **23** | **lwzx** | 21 | **✗** |
| **124** | **nor** | 18 | **✗** |
| **21** | **ldx** | 16 | **✗** |
| **233** | (?) | 16 | **✗** |
| **19** | **mfcr** | 15 | **✗** |
| **599** | **lfdx** | 14 | **✗** |
| 826 | sradi | 13 | ✓ (R9.1d) |
| 536 | srw | 13 | ✓ |
| 457 | divdu | 11 | ✓ |
| 235 | mullw | 8 | ✓ |
| **983** | **stfiwx** | 7 | **✗** |
| 27 | sld | 6 | ✓ |
| 827 | sradi+ | 6 | ✓ (R9.1d, via 826\|827) |
| ... | ... | ... | ... |

**Primary 31 XO gap count: ~15** of the top-30 (estimate ~20
across all 46 unique XOs).

### Primary 58 DS-XO (LD family)

| XO | Name | Count | Impl? |
|---|---|---|---|
| 0 | ld | 2618 | ✓ |
| 1 | ldu | 14 | **✗** |
| 3 | reserved | 1 | n/a |

### Primary 62 DS-XO (STD family)

| XO | Name | Count | Impl? |
|---|---|---|---|
| 0 | std | 2063 | ✓ |
| 1 | stdu | 331 | ✓ (R9.1c) |

### Primary 30 MD-XO (RLDI family)

| XO | Likely op | Count | Impl? |
|---|---|---|---|
| 0 | rldicl | 182 | ?  |
| 1 | rldicr | 85 | ? |
| 2 | rldic | 154 | ? |
| 3 | rldimi | 58 | ? |
| 4 | rldcl | 16 | ? |
| 6 | rldicl+ | 7 | ? |
| 7 | rldicr+ | 6 | ? |
| 12 | reserved | 1 | n/a |

Need to verify which MD-XO variants the `RLDI` arm handles.

### Primary 19 XO (BCEXT family)

| XO | Likely op | Count | Impl? |
|---|---|---|---|
| 0 | mcrf | 12 | ? |
| 16 | bclr | 538 | ? |
| 356 | (?) | 1 | ? |
| 449 | (?) | 3 | ? |
| 528 | bcctr | 176 | ? |

## R9.1.x strategy options

### Option A — One opcode per slice (R9.1d-style)

Pros: Each commit small (10-40 LOC), easy to review/revert,
empirical progression. Pros for code quality.

Cons: 25-30 commits to finish PSL1GHT `_start` coverage. Each
commit's smoke run + cargo test cycle = ~20 seconds. Slow.

### Option B — Batch by usage tier

Group the gaps and ship them in batches:
- **R9.1e**: top-10 highest-count P31 XOs (addze, mtcrf, stdx,
  lbzx, lwzx, nor, ldx, mfcr, lfdx, stfiwx) — ~200 LOC, 1 commit.
- **R9.1f**: with-update load/store family (lbzu, lwzu, stbu,
  stfdu, ldu, etc.) — these are trivial variants of implemented
  ops, just adding the "RA := EA" update step. ~150 LOC, 1 commit.
- **R9.1g**: subfic + addic + addic. + rlwimi (primary opcodes
  10-13 + 20). ~80 LOC, 1 commit.
- **R9.1h**: indexed load/store family (lbzx, lhzx, etc. in
  P31). ~150 LOC, 1 commit.
- **R9.1i**: re-audit remaining gaps, ship cleanup.

Pros: 4-5 commits total. Faster wall-clock progression.

Cons: Larger commits, harder to bisect if a regression sneaks in.

### Option C — Stub bypass to `main()`

Instead of running through `_start`, parse the ELF symbol table
to locate `main()` and:
- Allocate stack as in R9.1c.
- Set CIA = main() directly.
- Marshal argc / argv into r3 / r4 manually (per PS3 ABI).
- Skip PSL1GHT module init / TLS setup.

Pros: Bypasses ~80% of opcode work (most of the missing ops are
used in init, less in main()).

Cons: Skips PSL1GHT runtime setup. Many syscalls that need TOC
or TLS state may fail. Test fidelity is lower (the integration
doesn't mirror real lv2 boot).

### Option D — Pause + measure

Run R9.1d (sradi, already shipped this audit) + R9.1e (addze)
and measure: how much CIA progress per opcode added? If the
ratio is "each new opcode unlocks ~100 instructions of
execution", Option A pays off in ~30 slices. If "each opcode
unlocks ~5 instructions", reconsider Option C.

## Recommendation

**Ship Option B with the order R9.1e → R9.1f → R9.1g → R9.1h**,
then re-audit. Expected outcomes:
- R9.1e (top P31 XOs): unblock ~200+ instructions of `_start`.
- R9.1f (with-update L/S): unblock module init / TLS area.
- R9.1g (immediate arithmetic): cleanup tail.
- R9.1h (indexed L/S): final coverage gap.

After R9.1h, expect PSL1GHT `_start` to either complete or hit
the first unimplemented syscall (sys_spu_initialize). That's the
natural transition point to R9.1i (SPU group syscall arms).

If after R9.1e+f progress stalls (each commit unlocks < 10 new
instructions), pivot to Option C (main() bypass).

R9.1d (sradi, 13 occurrences) ships in this audit's companion
commit as proof-of-progress for Option A's per-slice path. The
next concrete deliverable is R9.1e (batched top-10 P31 XOs).
