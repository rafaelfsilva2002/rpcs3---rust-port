# single_spu_dma_tag_immediate_v1.notes.md

R8.3c — first IMMEDIATE wait-mode oracle (12th oracle). Two
queued GETs + TWO ch24 reads in the same SPU session with
`WrTagUpdate = IMMEDIATE` (= 0) and distinct masks (0x08 then
0x28). Captures real RPCS3 behavior for IMMEDIATE / clearing
semantics. Captured 2026-05-20 from RPCS3 against a CC0
PSL1GHT homebrew authored for this purpose.

## Captured RPCS3 behavior

- `ts1` (mask 0x08, IMMEDIATE) = **0x08**
- `ts2` (mask 0x28, IMMEDIATE) = **0x28** ← load-bearing

`ts2 == 0x28` (with mask 0x28 covering both tags) PROVES that
RPCS3 does NOT clear bits from `completed_tags` on IMMEDIATE
read. If clearing had happened, ts2 would have been 0x20 (only
tag 5 retained) or 0x00 (full clear). Real Cell BE semantic
confirmed: `completed_tags` is a persistent register;
IMMEDIATE just disables the wait. Behaviourally identical to
ANY mode for these reads (both return the snapshot without
waiting and without clearing).

## Origem do homebrew

Autoral. CC0 1.0 (public domain). Source committed at
`behavior-freeze/fixtures/spu/sources/single_spu_dma_tag_immediate_v1/`
with LICENSE.md.

Comportamento (uma linha): same as R8.3b but `WrTagUpdate =
IMMEDIATE` instead of ANY; SPU embeds ts1 + ts2 in canonical
`((sum1 << 16) | sum2) ^ ((ts1 << 24) | (ts2 << 16)) ^
0xCAFE5A1E = 0xDD164A9E`.

## Engine-side fix landed for this fixture

**R8.3c implementation: 1 Rust-core change in
`rpcs3-spu-differential/src/mfc_replay.rs`.**

The pre-existing `process_rdch_tagstat` cleared the observed
bits from `completed_tags` after the read:

```rust
// LEGACY (R6.7 A.4):
self.completed_tags &= !observed_now;
Ok(observed_now)
```

This clear behavior was a legacy from R6.7 A.4 when no oracle
exercised repeated reads. R8.3b (separate ANY-mode masks
0x08 + 0x20, NO overlap) didn't surface it because each read
cleared only bits unique to its mask, so the state-machine
oracle still matched captured. R8.3c uses overlapping masks
(first 0x08 ⊂ second 0x28) — the tag-3 bit cleared after the
first read makes the second oracle return 0x20 instead of
0x28, triggering `TagStatMismatch{captured:0x28, oracle:0x20,
wr_tag_mask:0x28, mode:Immediate}`.

Fix: remove the clear. `process_rdch_tagstat` now returns
`observed_now` without mutating `completed_tags`. Matches
Cell BE persistent register semantic, aligns with R8.3b's
`SpuChannels::read(MFC_RD_TAG_STAT)` runtime fix (which also
does not clear). The two paths are now consistent.

A unit test in `mfc_replay::tests` previously asserted
"After observation, the bit is cleared." — rewritten to
assert "After observation, completed_tags is unchanged" plus
a re-read returns the same value.

**rpcs3.exe does NOT require rebuild.** The fix lives in
`rpcs3-spu-differential`, which is consumed by replay tests
(`rpcs3-spu-recompiler`) but not by the runtime bridge path
through `rpcs3-spu-ffi.lib`. The runtime bridge already had
persistent semantics from R8.3b via the executor's
`SpuChannels::read` — the legacy clear was only in the
replay layer's oracle validator.

## Toolchain

Same `rpcs3-ps3dev-toolchain:local` Docker image (sha
`ed2167a9ac59…`). `.self` 940 KB sha
`84809807fbe5e34566012dcba292f3123ded6b2887235b8fc2bde5b90ad97b00`.

## RPCS3 version + capture hooks

Bridge patch sha: `0afda1c6…` (R8.1, unchanged).
Runtime hooks: `1f598d37…` (R8.1, unchanged).
Scaffolding: `cda976d7…` (R6.7 A.1, unchanged).

`bin/rpcs3.exe`: sha `34ec50d7…` (R8.3b binary, unchanged).
R8.3c is a Rust-core-only fix in the replay layer; rpcs3.exe
does NOT need to relink.

## Capture procedure

Same as R6.7 A.5 / R8.1 / R8.2 / R8.3a / R8.3b. Interpreter
(static) both PPU and SPU decoders for capture; restored to
Recompiler (LLVM).

Captured artifacts:

- `behavior-freeze/fixtures/spu/traces/single_spu_dma_tag_immediate_v1.jsonl`
  (26 events, same shape as R8.3b but ch23 = 0 instead of 1)
- `behavior-freeze/fixtures/spu/images/f8175729…1340.spuimg`
  (262,144 bytes; NEW SHA — SPU C source differs by one byte:
  `MFC_TAG_UPDATE_ANY` → `MFC_TAG_UPDATE_IMMEDIATE`)
- `.dmachunk` files: ZERO new (both already in pool from
  R6.7 / R8.1 / R8.2 / R8.3a / R8.3b — content-addressed
  dedup at peak).

## Trace contents (26 events)

```
seq  0-16: Same as R8.3b — spu_image + 2 GET dispatch sequences
           + 2 mfc_dma_complete + 2 sets of ch16-21 wrch
seq 17: spu_wrch ch22=0x08      pc=108
seq 18: spu_wrch ch23=0x0       pc=112  (IMMEDIATE — R8.3c diff)
seq 19: spu_rdch ch24=0x08      pc=116
seq 20: spu_wrch ch22=0x28      pc=128
seq 21: spu_wrch ch23=0x0       pc=132  (IMMEDIATE — load-bearing)
seq 22: spu_rdch ch24=0x28      pc=136  (NO-CLEAR proof)
seq 23: spu_wrch ch28=0xDD164A9E pc=288 (canonical)
seq 24: spu_stop stop_code=0x101
seq 25: final_state
```

Diff vs R8.3b: ch23 = 0 (IMMEDIATE) instead of 1 (ANY).
ch24 second read returns 0x28 (full mask, persistent) instead
of 0x20 (R8.3b's per-mask subset).

## Acceptance criteria (R8.3c contract)

- exactly 2 spu_mfc_cmd events with cmd=0x40 (GET)                       ✓
- exactly 2 mfc_dma_complete events                                      ✓
- TWO ch22 writes (0x08, 0x28)                                           ✓
- TWO ch23 writes (both IMMEDIATE = 0)                                   ✓
- TWO ch24 reads (0x08, 0x28) — second mask covers BOTH tags             ✓
- ts2 == 0x28 proves no-clear semantic                                   ✓
- spu_wrch ch28 = 0xDD164A9E                                             ✓
- spu_stop stop_code = 0x101                                             ✓
- canonical TTY:
  `[dma_tag_immediate_v1] OK cause=0x1 status=0xdd164a9e`                ✓

## Replay-validation

Status: ✅ parser ok / chunks loader ok / pre-replay state
machine post-fix (no-clear) / interp replay ok / JIT replay ok /
cross-backend snapshot diff identical.

## Triple-symmetry

| Path | Result |
|---|---|
| bridge OFF TTY | canonical ✓ |
| bridge ON  TTY | canonical ✓; `DELEGATED EXECUTION OK total_steps=1594` |
| replay oracle  | diff_snapshots is_identical ✓ |

## Stability

Once committed, this trace is a regression sentinel. The
captured (ts1=0x08, ts2=0x28) pair is load-bearing canonical
for the no-clear-on-IMMEDIATE semantic. If a future backend
implements per-bit clear on IMMEDIATE read (some Cell BE
variants), the captured values would change deterministically
and the canonical status would shift — re-document before
bumping the oracle.

## Out of R8.3c scope (deferred)

- Explicit per-bit clear via WrTagUpdate write (alternative
  Cell BE clearing mechanism in some implementations).
- DMA list cmds (GETL/PUTL/GETLB/PUTLB).
- Atomic primitives.
- MFC barriers / fence bits.
- Multi-SPU DMA races on shared EA.
- SPURS production support.
