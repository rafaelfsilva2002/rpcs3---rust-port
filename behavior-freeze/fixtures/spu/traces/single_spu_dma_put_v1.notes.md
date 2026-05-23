# single_spu_dma_put_v1.notes.md

R8.1 — first replay-validated SPU DMA PUT fixture (8th oracle).
Symmetric to R6.7 A.5 GET (`single_spu_dma_get_v1`) but inverts
the DMA direction: LS → EA instead of EA → LS. Captured
2026-05-19 from RPCS3 against a CC0 PSL1GHT homebrew authored
for this purpose.

## Origem do homebrew

Autoral. CC0 1.0 (public domain). Source committed at
`behavior-freeze/fixtures/spu/sources/single_spu_dma_put_v1/`
with LICENSE.md. Two .c files (PPU `main.c` + SPU
`spu/spu_dma_put.c`) + Makefile + README.md. Targets PSL1GHT
runtime.

Comportamento (uma linha): PPU allocates a 128-byte BSS buffer
zero-filled, passes EA via `thread_args.arg0`; SPU fills LS at
`lsa=0x10000` with the counting pattern `i & 0xFF`, dispatches
MFC PUT (cmd=0x20) from LS to EA, waits via ch22/23/24, writes
sentinel `0xC0FFEECA` to OUT_MBOX, halts via stop 0x101. PPU
joins, reads EA back, sums (= 8128 = 0x1FC0), XORs with
`0xCAFEBABE` to produce `ea_status = 0xCAFEA57E`.

The fixture is the load-bearing R8.1 oracle:
- `spu = 0xc0ffeeca` (sentinel) proves the SPU reached the
  post-PUT path (rdch ch24 unblocked → tag completed → PUT
  acknowledged by RPCS3 MFC).
- `ea_status = 0xcafea57e` proves the PUT BYTES actually landed
  in EA. A silent fake-PUT (EA stays zero) → `0xCAFEBABE`. A
  wrong-pattern PUT → some other distinctive value.

## Toolchain

Same `rpcs3-ps3dev-toolchain:local` Docker image as the prior
7 oracles (sha256 `ed2167a9ac59…`, content 2.43 GB; backup at
`C:\docker-backup\rpcs3-ps3dev-toolchain-local.tar`).

Build command (in container):

```
cd behavior-freeze/fixtures/spu/sources/single_spu_dma_put_v1
PS3DEV=/opt/ps3dev PSL1GHT=/opt/ps3dev/psl1ght make
```

Output: `build/single_spu_dma_put_v1.self` — 939,475 bytes,
sha256 `761414892bd3757a1a1d8238d6623f7270e5fee49321620b5c47b466e321f3c5`.

## RPCS3 version + capture hooks

RPCS3 build: ToT from this repository at capture time, with the
R5.9c + R5.9e.3 SPU trace writer **plus** R6.7 A.1 DMA writer
extension (committed shas `cda976d7…` scaffolding + `95bdcaae…`
runtime hooks; **R8.1 extends the runtime-hooks patch** to also
snapshot LS bytes when cmd=0x20 PUT, so the writer now records
`spu_mfc_cmd` for both GET and PUT with a content-addressed
`.dmachunk` carrying the source bytes at dispatch time).

Bridge patch sha at R8.1 closure: `0afda1c69…` (superseded by
later R8.x cycles — current HEAD pin is `106ddede…` per R8.4f-b;
see `behavior-freeze/harness/check_patch_separation.py` for the
authoritative current value and the bump history).

`bin/rpcs3.exe` for the capture:
- size 64 MB
- sha256 `3ef63a825f9820373bb1df175bc975d5063f531b98206860fab36a50a8cd95d2`
- (built 2026-05-19 from `rpcs3-upstream-clean` worktree with
  R6.7 A.1 + R7.1 + R7.2 + R8.1 patches applied; the bridge has
  both `bridge_dma_get_callback` + `bridge_dma_put_callback`)

## Capture procedure

Same as R6.7 A.5: `Core: SPU/PPU Decoder` temporarily set to
`Interpreter (static)` in `bin/config/config.yml` for the
capture run (LLVM JIT bypasses the C++ `set_ch_value` hooks
for MFC channels), then restored to `Recompiler (LLVM)`.

Driven by `.r81_run_put_off.bat`:

1. `RPCS3_SPU_TRACE_JSONL` env var points at this fixture's
   canonical JSONL path.
2. `rpcs3.exe --no-gui --headless` invoked on the .self.
3. Trace writer destructor flushes JSONL on group exit.
4. .spuimg + .dmachunk written to per-trace dirs first, then
   moved to canonical pools.

Captured artifacts:

- `behavior-freeze/fixtures/spu/traces/single_spu_dma_put_v1.jsonl`
  (15 events, ~2,300 bytes)
- `behavior-freeze/fixtures/spu/images/331edfe5…ea65.spuimg`
  (262,144 bytes — full LS at thread create)
- `behavior-freeze/fixtures/spu/dma/471fb943…2be5.dmachunk`
  (128 bytes, sum = 8128, counting pattern 0x00..0x7F) —
  **shared with the GET fixture** because both fixtures use the
  same source pattern, so the content-addressed SHA is identical.
  This is the canonical demonstration of the content-addressed
  pool deduplicating across fixtures.

## Trace contents (15 events)

```
seq  0: spu_image          sha=331edfe5…  size=0x40000  entry_pc=0
seq  1: spu_wrch  ch16=0x10000     pc=56   (MFC_LSA — LS source)
seq  2: spu_wrch  ch17=0x0         pc=64   (MFC_EAH)
seq  3: spu_wrch  ch18=0x10011180  pc=68   (MFC_EAL — PPU EA target)
seq  4: spu_wrch  ch19=0x80        pc=76   (MFC_Size = 128)
seq  5: spu_wrch  ch20=0x3         pc=84   (MFC_TagID = 3)
seq  6: spu_wrch  ch21=0x20        pc=92   (MFC_Cmd = PUT)
seq  7: spu_mfc_cmd cmd=0x20 tag=3 size=128 lsa=0x10000 eah=0 eal=0x10011180
                                              ea_chunk_sha256=471fb943…
seq  8: mfc_dma_complete tag=3 transferred_bytes=128
seq  9: spu_wrch  ch22=0x8         pc=100  (MFC_WrTagMask = 1<<3)
seq 10: spu_wrch  ch23=0x2         pc=108  (MFC_WrTagUpdate = ALL)
seq 11: spu_rdch  ch24=0x8         pc=112  (MFC_RdTagStat = 1<<3)
seq 12: spu_wrch  ch28=0xC0FFEECA  pc=124  (OUT_MBOX = sentinel)
seq 13: spu_stop  stop_code=0x101  pc=128
seq 14: final_state  r16=0xC0FFEECA, channels={all null}
```

## Acceptance criteria (R8.1 contract)

- exactly 1 spu_image event                                              ✓
- exactly 1 target_spu (256)                                             ✓
- exactly 1 spu_mfc_cmd event with cmd=0x20 (PUT)                       ✓
- exactly 1 mfc_dma_complete event with same tag (3) and size (128)     ✓
- ch16-23 wrch + ch24 rdch sequence in the parser-mandated order        ✓
- spu_wrch ch28 = 0xC0FFEECA (sentinel)                                  ✓
- spu_stop with stop_code = 0x101                                        ✓
- .dmachunk content matches LS[lsa..lsa+size] at dispatch (SHA = 471fb943…)  ✓
- final_state r16 = 0xC0FFEECA                                           ✓
- canonical TTY:
  `[dma_put_v1] OK cause=0x1 spu=0xc0ffeeca ea_status=0xcafea57e`       ✓

## Replay-validation

Drives the full pipeline from
`rust/rpcs3-spu-recompiler/tests/single_spu_dma_put_v1_replay.rs`:

```
parse_jsonl_trace                  (accepts cmd=0x20 now per R8.1 A.2)
  -> captured_events_to_traces_per_spu
  -> build_spu_program_from_captured_image
  -> apply_mfc_dma_pre_replay      (R8.1 state machine: PUT path
                                    ASSERTS LS bytes match captured
                                    chunk at dispatch — load-bearing
                                    correctness gate)
  -> replay_per_spu_traces::<InterpreterExecutor>
  -> replay_per_spu_traces_with(|_| RecompilerExecutor::new())
  -> diff_snapshots(interp, jit).is_identical()
```

The R8.1 PUT-replay assertion is the inverse of GET-replay:
- GET-replay: write captured chunk INTO LS before SPU runs
  (because there's no real EA in replay).
- PUT-replay: verify the SPU's LS bytes at dispatch MATCH the
  captured chunk (the SPU bytecode writes them as part of the
  prior steps; any divergence is a real correctness gap).

Status: ✅ parser ok / chunk loader ok / pre-replay PUT-assert ok /
interp replay ok / JIT replay ok / cross-backend snapshot diff
identical.

## Engine-side fixes landed for this fixture

R8.1 implementation:

1. **Parser** (`trace_fmt.rs`): accept cmd=0x20 alongside 0x40
   (defensive subset rejection bumped: new canary is 0x44 GETL).
2. **State machine** (`mfc_replay.rs`): switch on cmd in
   `process_mfc_cmd`. GET path writes chunk into LS (unchanged).
   PUT path asserts `ls[lsa..lsa+size] == bytes` and surfaces
   `PutLsBytesMismatch` (new error variant) on divergence.
3. **Interpreter** (`rpcs3-spu-interpreter`): wrch ch21 intercept
   routes by cmd (0x40 → GET callback, 0x20 → PUT callback,
   other → MfcUnsupported).
4. **`SpuChannels` / `SpuThread`** (`rpcs3-spu-thread`): new
   `DmaPutCallback` type + `dma_put_callback: Option<...>`
   field. refuse_mfc gate relaxed when ANY callback is installed.
5. **FFI**: new `rust_spu_set_dma_put_callback` +
   `DmaPutCallbackFn` type. New C header entries.
6. **C++ writer hook** (`SPUThread.cpp`): R6.7 A.1 hook extended
   to also capture cmd=0x20. For PUT, snapshot bytes come from
   `ls + mfc_lsa` (not `vm::_ptr<u8>(mfc_eal)`). The same
   `record_spu_mfc_cmd` + `record_mfc_dma_complete` events fire.
7. **C++ bridge** (`SPURustBridge.cpp`): new
   `bridge_dma_put_callback` that reads `src_ls_ptr` and writes
   to `vm::_ptr<u8>(eal)`. Installed alongside the GET callback
   on every `rust_spu_new`. SUCCESS log:
   `R8.1 DMA PUT dispatched: cmd=0x20 eal=0x... size=N tag=T
    on '...'; real LS/EA path (vm::_ptr<u8>); tag-stat 1<<T
    queued for subsequent rdch ch24`.

## Stability

Once committed, this trace is a regression sentinel. Do NOT
delete or edit without recording the reason here (e.g., RPCS3
trace writer schema change → recapture; SPU C source change →
bump to `single_spu_dma_put_v2`). The `.dmachunk` content is
the load-bearing payload: it MUST hash to the SHA-256 referenced
in the JSONL `spu_mfc_cmd` event AND it MUST contain the
counting pattern. Either invariant breaking should be treated
as suspected corruption — re-capture from a clean RPCS3 build
before editing anything by hand.
