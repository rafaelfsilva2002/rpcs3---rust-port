# single_spu_dma_get_any_v1.notes.md

R8.3a — first ANY-wait-mode replay-validated SPU fixture (10th
oracle). Two queued MFC GETs (same shape as R8.2 multi) but with
`WrTagUpdate = ANY` (= 1) instead of ALL (= 2). The SPU embeds
the actual `RdTagStat` returned value into the canonical status,
so the fixture round-trips ANY value the backend produces.
Captured 2026-05-20 from RPCS3 against a CC0 PSL1GHT homebrew
authored for this purpose.

## Origem do homebrew

Autoral. CC0 1.0 (public domain). Source committed at
`behavior-freeze/fixtures/spu/sources/single_spu_dma_get_any_v1/`
with LICENSE.md. Two .c files (PPU `main.c` + SPU
`spu/spu_dma_get_any.c`) + Makefile + README.md. Targets PSL1GHT
runtime.

Comportamento (uma linha): Same as R8.2 except `ch23 = ANY`.
SPU dispatches GET#1 (tag 3, EA1 → LS@0x10000, size 128) and
GET#2 (tag 5, EA2 → LS@0x10100, size 64), then `WrTagMask =
0x28`, `WrTagUpdate = ANY (1)`, `rdch ch24` → captured value.
SPU embeds the returned tag_stat via `(tag_stat << 24)` XOR
into the canonical status:

```
combined = (sum1 << 16) | sum2 = 0x1FC0_1080
status   = combined ^ (tag_stat << 24) ^ 0xBEEFBEAD
```

**Captured RdTagStat = 0x28** (full mask — RPCS3 dispatches DMA
synchronously by the time `rdch ch24` is reached; both tag
completes have already fired).

**Canonical status = `0x892FAE2D`** = `0x1FC0_1080 ^ 0x2800_0000
^ 0xBEEFBEAD`. Future backend changes (real hardware, async
DMA emulator) that produce different tag_stat values would
produce different canonical statuses; that's a backend behavior
change, not a regression — re-document and bump the oracle.

The fixture is the load-bearing R8.3a oracle:
- `status = 0x892fae2d` proves BOTH GETs delivered captured
  bytes AND the backend returned the expected ANY value 0x28
  (which equals the full mask in RPCS3 sync DMA, but the
  embedded XOR proves it wasn't some other value coincidentally
  producing the same status arithmetic).
- Any deviation in `tag_stat` (e.g. real hardware racing to
  return 0x8 or 0x20 first) produces a distinctively wrong
  status (`0xA92FAEAD` or `0x812FAEAD` respectively).

## Toolchain

Same `rpcs3-ps3dev-toolchain:local` Docker image (sha
`ed2167a9ac59…`). Build command:

```
cd behavior-freeze/fixtures/spu/sources/single_spu_dma_get_any_v1
PS3DEV=/opt/ps3dev PSL1GHT=/opt/ps3dev/psl1ght make
```

Output: `build/single_spu_dma_get_any_v1.self` — 940 KB,
sha256 `9710c4e1760ea04dff9f9fe3f8a34bc9d9584f4e53bff9c8cd6203ecab24f8e8`.

## RPCS3 version + capture hooks

Same as R8.2: R6.7 A.1 + R8.1 writer extension active (R8.1
rpcs3.exe handles cmd=0x40 GET capture unchanged; ANY mode
shows up as `ch23 = 1` in the captured wrch event).

Bridge patch sha at R8.3a: `0afda1c6…` (R8.1 baseline
unchanged). Runtime hooks: `1f598d37…` (R8.1). Scaffolding:
`cda976d7…` (unchanged since R6.7 A.1).

`bin/rpcs3.exe` for the capture:
- size 64 MB
- sha256 `3ef63a825f9820373bb1df175bc975d5063f531b98206860fab36a50a8cd95d2`
- (R8.1 baseline; same binary that landed R8.1 + R8.2 — no
  rebuild for R8.3a)

## Capture procedure

Same as R6.7 A.5 / R8.1 / R8.2: `Core: SPU/PPU Decoder`
temporarily set to `Interpreter (static)` in `bin/config/config.yml`
for capture, then restored to `Recompiler (LLVM)`.

Captured artifacts:

- `behavior-freeze/fixtures/spu/traces/single_spu_dma_get_any_v1.jsonl`
  (23 events, ~3 KB)
- `behavior-freeze/fixtures/spu/images/33dc6ca4…85a281.spuimg`
  (262,144 bytes — full LS at thread create; NEW SHA — distinct
  bytecode from R8.2 because `ch23 ALL → ANY` changes the
  emitted SPU code)
- `behavior-freeze/fixtures/spu/dma/471fb943…2be5.dmachunk`
  (128 bytes, counting pattern) — **shared with R6.7 GET v1 +
  R8.1 PUT v1 + R8.2 multi v1**. No new file written.
- `behavior-freeze/fixtures/spu/dma/c422e707…d3ae8.dmachunk`
  (64 bytes, constant 0x42) — **shared with R8.2 multi v1**.
  No new file written.

Two distinct SPU images + zero new `.dmachunk` files is the
peak of content-addressed dedup: R8.3a contributes only the
new spuimg and the new jsonl + notes.

## Trace contents (23 events)

```
seq  0: spu_image          sha=33dc6ca4…  size=0x40000  entry_pc=0
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
seq 17: spu_wrch  ch22=0x28        pc=108  (MFC_WrTagMask)
seq 18: spu_wrch  ch23=0x1         pc=116  (MFC_WrTagUpdate = ANY)
seq 19: spu_rdch  ch24=0x28        pc=120  (MFC_RdTagStat = ANY return)
seq 20: spu_wrch  ch28=0x892FAE2D  pc=252  (OUT_MBOX = canonical status)
seq 21: spu_stop  stop_code=0x101  pc=256
seq 22: final_state  r3=0x28 (= tag_stat returned), r23=0x1FC0 (sum1),
                     r31=0x1080 (sum2), channels={all null}
```

Diff from R8.2 (`single_spu_dma_get_multi_v1.jsonl`):
- seq 18 ch23 = `0x1` here, `0x2` in R8.2 (ANY vs ALL).
- seq 19 ch24 returned value is identical (`0x28`) because
  RPCS3 sync DMA satisfies both modes equivalently.
- seq 20 ch28 differs (R8.2 = `0xE12DEA4E`, R8.3a = `0x892FAE2D`)
  because the SPU embeds tag_stat into the XOR.
- seq 0 spuimg SHA differs because the SPU C source differs (one
  byte: `MFC_TAG_UPDATE_ALL` → `MFC_TAG_UPDATE_ANY`).

## Acceptance criteria (R8.3a contract)

- exactly 1 spu_image event                                              ✓
- exactly 1 target_spu (256)                                             ✓
- exactly 2 spu_mfc_cmd events with cmd=0x40 (GET)                       ✓
- exactly 2 mfc_dma_complete events                                      ✓
- ch16-21 wrch sequence repeated TWICE in canonical order                ✓
- ch22 = 0x28                                                            ✓
- **ch23 = 1 (MFC_TAG_UPDATE_ANY)** — load-bearing R8.3a invariant       ✓
- ch24 rdch = 0x28 — captured-canonical (RPCS3 sync DMA)                 ✓
- spu_wrch ch28 = 0x892FAE2D                                             ✓
- spu_stop with stop_code = 0x101                                        ✓
- final_state r3 = captured tag_stat (proves SPU saw the value)          ✓
- canonical TTY:
  `[dma_get_any_v1] OK cause=0x1 status=0x892fae2d`                      ✓

## Replay-validation

Drives the full pipeline from
`rust/rpcs3-spu-recompiler/tests/single_spu_dma_get_any_v1_replay.rs`:

```
parse_jsonl_trace
  -> captured_events_to_traces_per_spu
  -> build_spu_program_from_captured_image  (seed r3 = EA1, r4 = EA2)
  -> apply_mfc_dma_pre_replay      (both chunks → LS; tag-stat
                                    queue = [captured ch24 value])
  -> replay_per_spu_traces::<InterpreterExecutor>
  -> replay_per_spu_traces_with(|_| RecompilerExecutor::new())
  -> diff_snapshots(interp, jit).is_identical()
```

The R8.3a contract on the replay engine:
- Captured `ch23 = 1` flows through `process_wrch` and sets
  `wr_tag_update = Any` in the state machine.
- `process_rdch_tagstat(captured_value)` validates the value
  in ANY mode. The state machine's invariant is "at least one
  tag in the mask must be completed; return mask of completed-
  and-in-flight tags". For the captured `0x28`, both tags are
  completed, mask is satisfied, oracle returns 0x28 → matches
  captured exactly.

Status: ✅ parser ok / 2 chunks loader ok / ANY pre-replay
state machine ok / interp replay ok / JIT replay ok /
cross-backend snapshot diff identical.

## Engine-side fixes landed for this fixture

**R8.3a implementation: NONE.** Same as R8.2: the 9-oracle
baseline already supports the ANY wait mode end to end:
- Parser accepts `ch23 = 1` (Any) alongside 2 (All) and 0
  (Immediate) — see R6.7 A.2 + parser tests.
- `MfcReplayState::process_rdch_tagstat` handles all three
  modes per the R6.7 A.4 design § 9.3 specification.
- Existing unit tests already cover ANY mode at the synthetic
  level (`mfc_replay::tests` block).
- Bridge ON inherits zero-change support from R8.2: the R7.2
  callback is invoked per ch21 wrch independently of the
  wait mode that comes later.

The 10th oracle is therefore a pure **coverage gain** on the
existing implementation — no new code surface, no new SHA
pins, no new patches, no new `.dmachunk` files (perfect dedup
with R8.2 patterns).

## Stability

Once committed, this trace is a regression sentinel. Do NOT
delete or edit without recording the reason here. The captured
`ch24 = 0x28` value is **load-bearing canonical for this RPCS3
build**; if a future RPCS3 (or real hardware) emulator produces
a different ANY return, the canonical status changes
deterministically and the oracle must be either
re-captured-and-promoted (with explicit notes here) or kept
under a backend-tagged name. Do NOT hand-edit `.jsonl` or
`.spuimg` to make it match a theoretical canonical.
