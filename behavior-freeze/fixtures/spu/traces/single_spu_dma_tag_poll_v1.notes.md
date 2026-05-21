# single_spu_dma_tag_poll_v1.notes.md

R8.3b — first repeated-RdTagStat polling oracle (11th oracle).
Two queued GETs (same shape as R8.2 / R8.3a multi) + TWO ch24
reads in the same SPU session with distinct masks (0x08 and
0x20, both ANY mode). Forces persistent `completed_tags`
semantics in `SpuChannels` — the first oracle to exercise a
state that hardware retains across reads. Captured 2026-05-20
from RPCS3 against a CC0 PSL1GHT homebrew authored for this
purpose.

## Origem do homebrew

Autoral. CC0 1.0 (public domain). Source committed at
`behavior-freeze/fixtures/spu/sources/single_spu_dma_tag_poll_v1/`
with LICENSE.md.

Comportamento (uma linha): SPU dispatches GET#1 (tag 3, EA1
→ LS@0x10000, 128 B) and GET#2 (tag 5, EA2 → LS@0x10100, 64 B)
back-to-back, then performs TWO ch24 reads with different
masks. First: `mask=0x08, ANY → 0x08`. Second:
`mask=0x20, ANY → 0x20`. Both completed bits MUST survive the
first read in `completed_tags`. SPU computes
`status = ((sum1 << 16) | sum2) ^ ((tag_stat_1 << 24) |
(tag_stat_2 << 16)) ^ 0xCAFEBADC = 0xDD1E_AA5C`.

**The fixture was authored as the explicit forcing function
for the persistent `completed_tags: u32` refactor.** The R8.3a
drain-clear semantic (which fixed the pop-one-bit divergence
for one-shot reads) would stall the SPU on the second ch24
read because the queue had been drained. Predictions before
the refactor:
- Bridge OFF (pure C++): OK (C++ has persistent
  `completed_tags`).
- Bridge ON (Rust drain-clear): second read stalls; bridge
  falls back to C++; TTY = canonical via C++ resumption (NOT
  full delegation).
- Replay (drain-clear): first read drains queue, second read
  stalls → replay test fails with `Parked{channel:24}`.

All three predictions confirmed empirically before the fix
landed.

## Toolchain

Same `rpcs3-ps3dev-toolchain:local` Docker image (sha
`ed2167a9ac59…`). `.self` 940 KB sha
`c0a61415ff8fe28eda9bfe7eeaa453e6a0f3bed20fffd456aa0b2813ad56ed13`.

## RPCS3 version + capture hooks

Bridge patch sha at R8.3b capture: `0afda1c6…` (R8.1 baseline,
unchanged through R8.2 and R8.3a — the R8.3a/R8.3b fixes are
Rust-core only).
Runtime hooks: `1f598d37…` (R8.1, unchanged).
Scaffolding: `cda976d7…` (R6.7 A.1, unchanged).

`bin/rpcs3.exe` for the capture:
- 63,942,656 bytes
- sha `34ec50d73d22eabb49fc9c6f3ddfebd55d58db76a61ce5e5c6dac14b0d20f851`
- Built 2026-05-20 with R7.1 + R7.2 + R8.1 + R8.3a + R8.3b
  Rust core. C++ side unchanged.

## Capture procedure

Same as R6.7 A.5 / R8.1 / R8.2 / R8.3a. Interpreter (static)
both PPU and SPU decoders during capture; restored to
Recompiler (LLVM) after.

Captured artifacts:

- `behavior-freeze/fixtures/spu/traces/single_spu_dma_tag_poll_v1.jsonl`
  (26 events, ~3 KB)
- `behavior-freeze/fixtures/spu/images/c79584f0…e363.spuimg`
  (262,144 bytes; NEW SHA — SPU C source differs in the second
  ch22/ch23/ch24 sequence + status arithmetic)
- `.dmachunk` files: ZERO new — both chunks (`471fb943…`
  counting pattern, `c422e7070…` constant 0x42) already in
  the canonical pool from R6.7 + R8.1 + R8.2 + R8.3a.

## Trace contents (26 events)

```
seq  0: spu_image          sha=c79584f0…  size=0x40000  entry_pc=0
seq  1-6 : GET #1 setup (ch16=0x10000, ch17=0, ch18=ea1,
                         ch19=128, ch20=3, ch21=0x40)
seq  7  : spu_mfc_cmd cmd=0x40 tag=3 size=128 lsa=0x10000
                       ea_chunk_sha256=471fb943… (counting pattern)
seq  8  : mfc_dma_complete tag=3 transferred_bytes=128
seq  9-14: GET #2 setup (ch16=0x10100, ..., ch21=0x40)
seq 15  : spu_mfc_cmd cmd=0x40 tag=5 size=64 lsa=0x10100
                       ea_chunk_sha256=c422e707… (constant 0x42)
seq 16  : mfc_dma_complete tag=5 transferred_bytes=64
seq 17  : spu_wrch  ch22=0x08      pc=108  (MFC_WrTagMask #1)
seq 18  : spu_wrch  ch23=0x1       pc=116  (MFC_WrTagUpdate = ANY)
seq 19  : spu_rdch  ch24=0x08      pc=120  (RdTagStat read #1)
seq 20  : spu_wrch  ch22=0x20      pc=132  (MFC_WrTagMask #2)
seq 21  : spu_wrch  ch23=0x1       pc=136  (MFC_WrTagUpdate = ANY)
seq 22  : spu_rdch  ch24=0x20      pc=140  (RdTagStat read #2)
seq 23  : spu_wrch  ch28=0xDD1EAA5C pc=288 (OUT_MBOX = canonical)
seq 24  : spu_stop  stop_code=0x101 pc=292
seq 25  : final_state ...
```

The signature R8.3b shape vs R8.3a:
- R8.3a has events seqs 17/18/19 (one ch22/ch23/ch24 sequence).
- R8.3b has events seqs 17-19 AND 20-22 (two ch22/ch23/ch24
  sequences with different masks).

## Acceptance criteria (R8.3b contract)

- exactly 1 spu_image                                                    ✓
- exactly 1 target_spu (256)                                             ✓
- exactly 2 spu_mfc_cmd events with cmd=0x40 (GET)                       ✓
- exactly 2 mfc_dma_complete events                                      ✓
- TWO ch22 writes (0x08, then 0x20) — load-bearing R8.3b invariant       ✓
- TWO ch23 writes (both ANY = 1)                                         ✓
- TWO ch24 reads (0x08, then 0x20) — persistent completed_tags proof     ✓
- spu_wrch ch28 = 0xDD1EAA5C                                             ✓
- spu_stop with stop_code = 0x101                                        ✓
- canonical TTY:
  `[dma_tag_poll_v1] OK cause=0x1 status=0xdd1eaa5c`                     ✓

## Replay-validation

Status: ✅ parser ok / 2 chunks loader ok / persistent
completed_tags via drain-absorb-no-clear / interp replay ok /
JIT replay ok / cross-backend snapshot diff identical.

## Engine-side fixes landed for this fixture

**R8.3b implementation: 1 Rust-core change.**

`rpcs3-spu-thread/src/lib.rs`:

- Added field `completed_tags: u32` on `SpuChannels` (default 0).
  Mirrors the Cell BE / C++ persistent register.
- Modified `read(MFC_RD_TAG_STAT)` to:
  1. Drain `mfc_tag_stat_queue` via bitwise OR into
     `completed_tags`.
  2. Return `Err(WouldStall)` if `completed_tags == 0`.
  3. Return `Ok(completed_tags & mfc_wr_tag_mask)` — NEVER
     clears the register.
- The previous unit test
  `mfc_rdtagstat_drains_queue_and_aggregates_with_mask`
  (R8.3a) was rewritten as
  `mfc_rdtagstat_persistent_completed_tags_with_mask_filtering`
  to cover: empty stall, first-read absorb, second-read
  same-mask (no stall), per-mask filtering between reads,
  additional callback push absorbed into the existing
  `completed_tags`, full-mask read returns aggregate.

**C++ patches DID NOT CHANGE.** SHA pins unchanged. rpcs3.exe
rebuilt (`192dd72f…` → `34ec50d7…` after lib refresh because
cargo cached the build on first attempt — a touch + rebuild
forced the new lib).

## Limitation deferred to R8.4+

The R8.3b refactor adds persistence ACROSS READS but does NOT
implement persistence ACROSS sessions or clearing semantics.
Specifically:

- `MFC_TAG_UPDATE_IMMEDIATE` (= 0) intentionally clears
  `completed_tags` per-bit on read. R8.3b uses ANY mode only;
  Immediate mode is in-scope mechanically but no oracle
  exercises it yet.
- The Cell BE `WrTagUpdate` write semantically does
  "and-not-mask" against `completed_tags` in some
  implementations. R8.3b's drain-absorb-no-clear ignores any
  such write-side clear. A future polling fixture that
  depends on explicit clear would force that addition.

These are documented but NOT implemented in R8.3b per the
empirical-scoping policy.

## Stability

Once committed, this trace is a regression sentinel. Do NOT
delete or edit without recording the reason here. The two
captured ch24 values (0x08 + 0x20) are load-bearing canonical
for this RPCS3 build and SHOULD match real-hardware behavior
(Cell BE's `completed_tags` register persists across reads,
so polling two distinct masks DOES yield two distinct per-mask
subsets). If a future backend produces different ch24 values,
re-document the canonical before bumping the oracle.
