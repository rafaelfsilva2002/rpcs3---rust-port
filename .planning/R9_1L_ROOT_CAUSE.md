# R9.1l — root cause traced to PPC64 FD-pointer convention mismatch

> **SUPERSEDED 2026-05-25 by R9.1m + R9.1n.** This document's
> diagnosis (claiming `.ctors` u32/u64 entry-size mismatch
> prevented the constructor walker from running) was proven
> INCORRECT by R9.1m. The R9.1m scan over PHDR[3]'s full
> p_memsz=0x414D8 (R9.1k incorrectly scanned only p_filesz=0x14D8)
> found 31 sequential u64 BE FD pointers at vaddr `0x100511A0`
> — the `__syscalls` table IS populated. R9.1n further mapped
> slot[04] → `__librt_write_r @ 0x11168`. The actual blocker
> is newlib's `_reent._write_r` linkage (separate from
> `__syscalls_init`), not the constructor chain. See
> `.planning/R9_FINAL_CLOSURE.md` for the corrected analysis.
> File retained as a record of the investigation arc.

**Status:** ROOT CAUSE IDENTIFIED + reproducible from raw .self.
**Fix path:** Requires interpreter-level changes (out of read-only scope of this slice).

## Investigation method

R9.1l disassembled the boot path from `_start` to the
constructor walkers entirely from the raw `.self` bytes
without running the binary, then cross-checked against PPU
state at runtime via R9.1k's `.data` scan.

## Boot chain mapped exhaustively

```
__start FD @ 0x302B0 → code=0x103F8 toc=0x3A548 env=0
  ↓
_start @ 0x103F8
  ↓ bl 0x105D0
_initialize @ 0x105D0
  ↓ bl -0x3E4 → 0x10200
_init @ 0x10200
  ↓ bl +0x1B4 → 0x103C0 (.init_array walker)
  ↓ bl +0x19C3C → 0x29E50 (.ctors walker)
_init returns to _initialize
  ↓ bl +0x1C → 0x10610 = main
  ↓ bl +0x324C → 0x13848 = exit
```

## Finding #1 — .init_array is empty

The walker at 0x103C0 loads:
- `r9 = TOC[-0x7FB0]` = mem[0x32598] = **0**

Since `r9 == 0`, the `beq` skips the loop body. The
walker then tail-calls 0x10288 (the `.ctors`-aware walker
sister function), which checks:
- `start = TOC[-0x8000]` = mem[0x32548] = **0x10011098**
- `end   = TOC[-0x7FF0]` = mem[0x32558] = **0x10011098**
- `start == end` → **.init_array literally empty.**

PSL1GHT's lv2.ld does not register any constructors via
`.init_array`. The actual constructors live in `.ctors`.

## Finding #2 — .ctors walker reads u64 from u32-aligned entries

`_init`'s second `bl` (→ 0x29E50) is the `.ctors` walker.
It loads:
- `r31 = TOC[-0x7F78]` = mem[0x325D0] = **0x00030028**
  (= the start of `.ctors` end, walker walks backward)

The walker body does `ld r9, -8(r31)` — an 8-byte load.

The `.ctors` array at vaddr 0x30000..0x30028 contains **u32
entries** (4 bytes each):

```
0x30000: 0x0002a2e0   ← function ptr to first ctor
0x30004: 0x0002a320
0x30008: 0x0002a360
0x3000C: 0x0002a3a0
...
0x30024: 0x0002a520
```

The walker's 8-byte `ld` reads pairs of u32 entries as a
single u64. Result: r9 = garbage u64 like `0x0002a4e0_0002a520`.

Since the cmpdi vs -1 fails (garbage != -1), the walker
proceeds to call. The bctrl jumps via the low 32 bits of
the garbage value, which happens to be a valid `.text`
address — but the FD deref at `lwz r0, 0(r9)` reads
random text bytes as "code address".

## Finding #3 — recursive PPC64 FD-pointer convention mismatch

Each ctor at 0x2A2E0, 0x2A320, etc. is a **thunk** with this
pattern:

```
mflr r0; std r0, 16(r1)
std r2, 40(r1)            ; save TOC
stdu r1, -128(r1)
addis r12, 0, 3            ; r12 = 0x30000
lwz r12, 0x80(r12)        ; r12 = mem[0x30080]
lwz r0, 0(r12)             ; r0 = mem[r12]      ← expects FD code
lwz r2, 4(r12)             ; r2 = mem[r12+4]   ← expects FD toc
mtctr r0
bcctrl                     ; CALL CTR
```

`mem[0x30080]` = **0x0002aae0** (a `.text` vaddr).

The thunk treats `mem[0x30080]` as a **pointer to an 8-byte
FD struct** (`{u32 code, u32 toc}`). It then reads:
- `mem[0x2aae0]` = `7c 08 02 a6` = the **first instruction
  (`mflr r0`)** of the function at vaddr `0x2aae0`, NOT an
  FD code field.

CTR is set to `0x7C0802A6` (mflr r0 instruction encoding),
`bcctrl` jumps to that address (unmapped), the PPU faults.

## Root cause summary

PSL1GHT compiles binaries with a **compact FD model** where
function pointers in `.data`/`.ctors` tables are **direct
4-byte code addresses** (matching the `__start` FD R9.1b
discovery). But the compiler-emitted indirect-call thunks
follow the **PPC64 ELFv1 convention** that the value loaded
is an **FD pointer** to a separate `{code, toc}` struct.

This is **exactly the R9.1f bug** repeating at a deeper
level. R9.1b fixed `e_entry` (the loader-side FD deref) but
runtime `bcctrl` thunks still hit the mismatch.

## What R9.1l proves

- `.init_array` walker visits 0 entries (confirmed).
- `.ctors` walker hits the FD-vs-code-address mismatch
  immediately on the first entry.
- `__syscalls_init` is one of those ctors, but it is never
  reached because the walker faults at iteration 1.
- The mismatch is **endemic** to every indirect call through
  a `.data` pointer table in PSL1GHT — not specific to
  constructors.

This explains why our PPU still progressed past the
constructor walker without raising an explicit error: the
PPU interpreter's `bcctrl` masks CTR with `!0x3`, so the
garbage CTR `0x7C0802A6` becomes `0x7C0802A4` (we observed
this in R9.1f), which falls inside a region without page
flags → MissingFlags. But the smoke test treats memory
faults as accepted outcomes, so the failure was silently
absorbed and execution continued into other paths that
happened to terminate at sys_process_exit.

## Required fix

To make ANY indirect call through a `.data` table work,
the loader or interpreter must reconcile the two FD
conventions:

**Option (a) — load-time fixup.** At `load_elf` time,
scan `.data` (PHDR[1] + PHDR[3]) for u32 values that fall
in `.text` range and treat them as direct code addresses.
For each, allocate a 2-word synthetic FD `{code=value,
toc=default}` in a side region, and replace the original
u32 with the synthetic-FD pointer. Then runtime FD derefs
work.

Risk: false positives. Some u32 values in `.data` that
happen to be in `.text` range may be data (struct offsets,
etc.), not function pointers. Without symbols this can't
be disambiguated 100%.

**Option (b) — interpreter recognition.** In
`bcctr/bcctrl`, after `mtctr` fires, inspect the value:
if it doesn't decode as a valid PPC instruction at that
address (or the address has no X flag), trace back to the
preceding `lwz` and recover the original 32-bit value as
the actual target. This is fragile.

**Option (c) — recompile fixtures with full FDs.** Patch
PSL1GHT's lv2.ld or GCC config so the linker emits full
8-byte FD structs into `.data` tables. This requires
rebuilding all 20 oracle `.self` files, which conflicts
with behavior-freeze (we'd be changing the fixture
binaries).

## Recommendation

Since the user constraint forbids fake TTY output, manual
fixture edits, or random NID stubs, the right path is
**Option (a)**: a load-time `.data` scan + FD synthesis.

This is an interpreter-level architectural change with
~150-300 LOC + careful boundary testing. The risk of
false-positive FD synthesis on real data needs mitigation
(e.g., only treat values as code pointers if they appear
in patterns the compiler reliably emits).

R9.1l honest closure: **root cause traced exhaustively;
fix requires a new architectural slice (R9.1m) for
load-time FD-pointer synthesis, which is significant work
in its own right**.

## R9 wave honest summary (post-R9.1l)

What we have:
- Full architectural integration of LV2/PPU/SPU.
- PSL1GHT main() reaches sysSpuImageImport, group_join, fstat.
- 119 import stubs, 20 NID handlers, 7 lv2 syscall arms.
- 12 new PPC opcodes.
- 264 cargo tests pass throughout.
- 20 SPU oracles preserved.

What's blocked:
- TTY emit from printf, because PSL1GHT's compact-FD-pointer
  layout in `.data` tables disagrees with the compiler's
  PPC64 ELFv1 FD-deref thunks. Constructor walker is
  the FIRST point this hits, blocking `__syscalls_init`
  from populating newlib's syscall table, which then makes
  printf silently fail.

What R9.1m (next slice) would need to do:
- Implement load-time scan of `.data` for "u32 values that
  look like code pointers" with a constrained heuristic
  (e.g., only in segments aligned to PHDR boundaries known
  to hold linkage tables).
- Synthesize side-table FD structs `{code, toc}` for each.
- Patch the `.data` entries to point to the synthetic FDs.
- Verify by running smoke and confirming `__syscalls_init`
  populates the `__syscalls` global.
- Confirm `tty_ch3` becomes non-empty.

Estimated R9.1m scope: ~250 LOC + several smoke iterations
to tune the heuristic.

## Validation status

- cargo test --workspace --tests --release: 264 result blocks,
  0 failures.
- 20 SPU oracle replay tests still green.
- behavior-freeze contract preserved.
- No code changes in R9.1l; only investigation + doc.
