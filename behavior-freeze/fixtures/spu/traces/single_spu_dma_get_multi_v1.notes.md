# single_spu_dma_get_multi_v1.notes.md

R8.2 — first multi-DMA replay-validated SPU fixture (9th oracle).
Two queued MFC GETs (distinct tags 3 + 5, distinct EAs, distinct
sizes 128 + 64, distinct LSAs 0x10000 + 0x10100) + WrTagUpdate=ALL
wait. Captured 2026-05-20 from RPCS3 against a CC0 PSL1GHT
homebrew authored for this purpose.

## Origem do homebrew

Autoral. CC0 1.0 (public domain). Source committed at
`behavior-freeze/fixtures/spu/sources/single_spu_dma_get_multi_v1/`
with LICENSE.md. Two .c files (PPU `main.c` + SPU
`spu/spu_dma_get_multi.c`) + Makefile + README.md. Targets PSL1GHT
runtime.

Comportamento (uma linha): PPU allocates two BSS buffers
(ea_buf1=128B counting pattern, ea_buf2=64B constant 0x42),
passes both EAs via `thread_args.arg0` + `arg1`; SPU dispatches
GET #1 (tag 3, EA1 → LS@0x10000, size 128) and GET #2 (tag 5,
EA2 → LS@0x10100, size 64) back-to-back, then waits via
WrTagMask=0x28 + WrTagUpdate=ALL + RdTagStat=0x28. SPU computes
combined status = ((sum1 << 16) | sum2) ^ 0xFEEDFACE = 0xE12DEA4E,
writes OUT_MBOX, halts via stop 0x101.

The fixture is the load-bearing R8.2 oracle:
- `status = 0xe12dea4e` proves BOTH GETs delivered the captured
  bytes AND the SPU waited for both tags (ALL semantics) before
  reading.
- Either GET silently dropping → distinctively wrong status
  (e.g. `0xe12dface` if GET#2 dropped, `0xfeedea4e` if GET#1
  dropped, `0xfeedface` if both dropped).

## Toolchain

Same `rpcs3-ps3dev-toolchain:local` Docker image as the prior
8 oracles (sha256 `ed2167a9ac59…`, content 2.43 GB; backup at
`C:\docker-backup\rpcs3-ps3dev-toolchain-local.tar`).

Build command (in container):

```
cd behavior-freeze/fixtures/spu/sources/single_spu_dma_get_multi_v1
PS3DEV=/opt/ps3dev PSL1GHT=/opt/ps3dev/psl1ght make
```

Output: `build/single_spu_dma_get_multi_v1.self` — 940 KB,
sha256 `7eb545af47a2c51e064b4d79090e2930d1cd6058edbd9d29032785d0ad535659`.

## RPCS3 version + capture hooks

RPCS3 build: ToT from this repository at capture time, with the
R5.9c + R5.9e.3 SPU trace writer **plus** R6.7 A.1 DMA writer
extension **plus** R8.1 PUT writer extension (committed shas
`cda976d7…` scaffolding + `1f598d37…` runtime hooks; **R8.2
does NOT extend the writer** — captures cmd=0x40 GET via the
existing R6.7 A.1 hook unchanged. Both back-to-back GETs trigger
two `record_spu_mfc_cmd` + two `record_mfc_dma_complete`
emissions independently).

Bridge patch sha at R8.2 capture: `0afda1c6…` (R8.1 baseline
unchanged — bridge already supports cmd=0x40 GET via R7.2 +
cmd=0x20 PUT via R8.1, and refuse_mfc is relaxed for either
callback).

`bin/rpcs3.exe` for the capture:
- size 64 MB
- sha256 `3ef63a825f9820373bb1df175bc975d5063f531b98206860fab36a50a8cd95d2`
- (R8.1 baseline; same binary that landed R8.1 — no rebuild for R8.2)

## Capture procedure

Same as R6.7 A.5 / R8.1: `Core: SPU/PPU Decoder` temporarily set
to `Interpreter (static)` in `bin/config/config.yml` for the
capture run (LLVM JIT bypasses the C++ `set_ch_value` hooks for
MFC channels), then restored to `Recompiler (LLVM)`.

Driven by the standard capture invocation:

1. `RPCS3_SPU_TRACE_JSONL` env var points at this fixture's
   canonical JSONL path.
2. `rpcs3.exe --no-gui --headless` invoked on the .self.
3. Trace writer destructor flushes JSONL on group exit.
4. .spuimg + .dmachunk written to per-trace dirs first, then
   moved to canonical pools.

Captured artifacts:

- `behavior-freeze/fixtures/spu/traces/single_spu_dma_get_multi_v1.jsonl`
  (23 events, ~3 KB)
- `behavior-freeze/fixtures/spu/images/a092b2e5…ea855e0.spuimg`
  (262,144 bytes — full LS at thread create)
- `behavior-freeze/fixtures/spu/dma/471fb943…2be5.dmachunk`
  (128 bytes, counting pattern 0x00..0x7F) — **shared with the
  R6.7 GET v1 + R8.1 PUT v1 fixtures**. Content-addressed pool
  deduplicates: same source pattern → same SHA → no new file.
- `behavior-freeze/fixtures/spu/dma/c422e707…d3ae8.dmachunk`
  (64 bytes, constant 0x42) — new content, new SHA, fresh
  canonical pool entry.

## Trace contents (23 events)

```
seq  0: spu_image          sha=a092b2e5…  size=0x40000  entry_pc=0
seq  1: spu_wrch  ch16=0x10000     pc=20   (MFC_LSA #1)
seq  2: spu_wrch  ch17=0x0         pc=28   (MFC_EAH #1)
seq  3: spu_wrch  ch18=0x10011180  pc=36   (MFC_EAL #1)
seq  4: spu_wrch  ch19=0x80        pc=44   (MFC_Size #1 = 128)
seq  5: spu_wrch  ch20=0x3         pc=52   (MFC_TagID #1 = 3)
seq  6: spu_wrch  ch21=0x40        pc=64   (MFC_Cmd #1 = GET)
seq  7: spu_mfc_cmd cmd=0x40 tag=3 size=128 lsa=0x10000 eah=0
        eal=0x10011180 ea_chunk_sha256=471fb943… (counting pattern)
seq  8: mfc_dma_complete tag=3 transferred_bytes=128
seq  9: spu_wrch  ch16=0x10100     pc=72   (MFC_LSA #2)
seq 10: spu_wrch  ch17=0x0         pc=76   (MFC_EAH #2)
seq 11: spu_wrch  ch18=0x10011200  pc=84   (MFC_EAL #2)
seq 12: spu_wrch  ch19=0x40        pc=88   (MFC_Size #2 = 64)
seq 13: spu_wrch  ch20=0x5         pc=96   (MFC_TagID #2 = 5)
seq 14: spu_wrch  ch21=0x40        pc=100  (MFC_Cmd #2 = GET)
seq 15: spu_mfc_cmd cmd=0x40 tag=5 size=64 lsa=0x10100 eah=0
        eal=0x10011200 ea_chunk_sha256=c422e707… (constant 0x42)
seq 16: mfc_dma_complete tag=5 transferred_bytes=64
seq 17: spu_wrch  ch22=0x28        pc=108  (MFC_WrTagMask = (1<<3)|(1<<5))
seq 18: spu_wrch  ch23=0x2         pc=116  (MFC_WrTagUpdate = ALL)
seq 19: spu_rdch  ch24=0x28        pc=120  (MFC_RdTagStat = mask)
seq 20: spu_wrch  ch28=0xE12DEA4E  pc=248  (OUT_MBOX = canonical status)
seq 21: spu_stop  stop_code=0x101  pc=252
seq 22: final_state  r21=0x1FC0 (sum1), r29=0x1080 (sum2),
                     r35=0xE12DEA4E (final status), channels={all null}
```

## Acceptance criteria (R8.2 contract)

- exactly 1 spu_image event                                              ✓
- exactly 1 target_spu (256)                                             ✓
- exactly 2 spu_mfc_cmd events with cmd=0x40 (GET) and distinct tags     ✓
- exactly 2 mfc_dma_complete events matching tags + sizes                ✓
- ch16-21 wrch sequence repeated TWICE in canonical order                ✓
- ch22 = 0x28 (= (1<<3)|(1<<5)) — multi-bit tag mask                    ✓
- ch23 = 2 (MFC_TAG_UPDATE_ALL) — wait for BOTH tags                    ✓
- ch24 rdch = 0x28 (ALL mode returns mask exactly when both complete)   ✓
- spu_wrch ch28 = 0xE12DEA4E (canonical multi-GET status)                ✓
- spu_stop with stop_code = 0x101                                        ✓
- .dmachunk content matches captured chunks for both tags                ✓
- final_state r35 = 0xE12DEA4E                                           ✓
- canonical TTY:
  `[dma_get_multi_v1] OK cause=0x1 status=0xe12dea4e`                    ✓

## Replay-validation

Drives the full pipeline from
`rust/rpcs3-spu-recompiler/tests/single_spu_dma_get_multi_v1_replay.rs`:

```
parse_jsonl_trace                  (accepts 2 cmd=0x40 events)
  -> captured_events_to_traces_per_spu
  -> build_spu_program_from_captured_image
  -> seed r3 = EA1 lane 1, r4 = EA2 lane 1 (PSL1GHT arg0/arg1)
  -> apply_mfc_dma_pre_replay      (both .dmachunk loaded into LS
                                    at captured LSAs; tag-stat queue
                                    pre-populated for ALL mode)
  -> replay_per_spu_traces::<InterpreterExecutor>
  -> replay_per_spu_traces_with(|_| RecompilerExecutor::new())
  -> diff_snapshots(interp, jit).is_identical()
```

Status: ✅ parser ok / 2 chunks loader ok / pre-replay state
machine handles 2 in-flight tags + ALL wait / interp replay ok /
JIT replay ok / cross-backend snapshot diff identical.

## Engine-side fixes landed for this fixture

**R8.2 implementation: NONE.** The 8-oracle baseline already
supports everything R8.2 exercises:
- Parser accepts cmd=0x40 GET (R6.7 A.2).
- State machine `process_mfc_cmd` handles GETs back-to-back via
  the in-flight set (R6.7 A.4) — verified across 2 dispatches
  by the multi-tag tests in `mfc_replay::tests::
  mfc_replay_handles_wr_tag_mask_update_basic` (R6.7 A.4
  unit test that already covered 2-tag ALL mode).
- Chunk loader resolves 2 distinct SHA-256s independently
  (R6.7 A.3).
- Executor wiring (Phase C) handles `tag_stat_queue` with
  multiple entries already (queue is a `VecDeque<u32>`).
- Bridge ON (R7.2 + R8.1) installs both callbacks; refuse_mfc
  relaxed for either; each ch21 wrch invokes its callback in
  turn.

The 9th oracle is therefore a pure **coverage gain** on the
existing implementation — no new code surface, no new SHA pins,
no new patches required.

## Stability

Once committed, this trace is a regression sentinel. Do NOT
delete or edit without recording the reason here (e.g., RPCS3
trace writer schema change → recapture; SPU C source change →
bump to `single_spu_dma_get_multi_v2`). Both `.dmachunk` files
MUST hash to the SHAs referenced in the JSONL `spu_mfc_cmd`
events AND must contain the documented byte patterns. Either
invariant breaking should be treated as suspected corruption —
re-capture from a clean RPCS3 build before editing anything by
hand.
