# single_spu_dma_get_multi_v1 — R8.2 fixture

**R8.2 first multi-DMA replay-validated fixture (9th oracle).**
Two queued MFC GETs (distinct tags, EAs, sizes, LSAs) + ALL
wait mode. Exercises multi-tag in-flight state and the ALL wait
semantics with multi-bit mask, on top of the GET runtime path
established in R6.7 A.5 + R7.2. CC0 1.0 public domain.

## Behaviour

1. PPU allocates two distinct EA buffers:
   - `ea_buf1` (128 bytes, aligned 128): filled with counting
     pattern `i & 0xFF` → sum1 = 8128 = 0x1FC0.
   - `ea_buf2` (64 bytes, aligned 128): filled with constant
     0x42 → sum2 = 64 * 0x42 = 4224 = 0x1080.
2. PPU passes EA1 via `thread_args.arg0` and EA2 via
   `thread_args.arg1`, starts the SPU group.
3. SPU dispatches GET #1 (tag=3, EA1 → LS@0x10000, size 128)
   and GET #2 (tag=5, EA2 → LS@0x10100, size 64) back-to-back
   BEFORE any tag wait — both in flight simultaneously.
4. SPU writes `WrTagMask = 0x28`, `WrTagUpdate = ALL`,
   `rdch ch24` blocks until both completions fire and returns
   0x28 exactly.
5. SPU computes:
   - sum1 = sum(LS[0x10000..0x10080]) = 0x1FC0
   - sum2 = sum(LS[0x10100..0x10140]) = 0x1080
   - combined = (sum1 << 16) | sum2 = 0x1FC0_1080
   - status = combined ^ 0xFEEDFACE = 0xE12D_EA4E
6. SPU writes status to `OUT_MBOX`, halts via `stop 0x101
   (SYS_SPU_THREAD_STOP_GROUP_EXIT)`.
7. PPU joins; lv2 reads OUT_MBOX as the group-exit status
   (= canonical 0xE12DEA4E).

## Canonical TTY

```
[dma_get_multi_v1] OK cause=0x1 status=0xe12dea4e
```

Three independent invariants on one line:

- `cause=0x1` (= `GROUP_EXIT`): SPU stopped cleanly.
- `status=0xe12dea4e`: BOTH DMAs completed AND the SPU saw the
  correct byte content in both LS regions before computing
  the combined checksum.

## Failure mode catalogue

| Failure | Observable |
|---|---|
| SPU never ran | no TTY at all (PPU stuck) |
| Both DMAs silently dropped (LS zero-filled) | `status=0xfeedface` (= 0 ^ 0xFEEDFACE) |
| Only GET #1 succeeded (GET #2 dropped) | `status=0xe12dface` (= 0x1FC0_0000 ^ 0xFEEDFACE — leading half canonical, trailing half mask only) |
| Only GET #2 succeeded (GET #1 dropped) | `status=0xfeedea4e` (= 0x0000_1080 ^ 0xFEEDFACE — leading half mask only, trailing half canonical) |
| Tag swap (GET #1 byte content landed at LSA #2 region) | `status` arithmetic differs distinctively because the SPU reads from fixed LSAs and gets the wrong content in each region |
| RdTagStat returned early (only one tag completed) | SPU reads partial / pre-DMA bytes → `status` arithmetic is distinctively wrong, not the canonical |
| Correct | `status=0xe12dea4e` |

## Build

Same Docker image as the other R5/R6/R7/R8.1 PSL1GHT fixtures:

```bash
docker run --rm -v "$PWD":/work \
  -e PS3DEV=/opt/ps3dev -e PSL1GHT=/opt/ps3dev/psl1ght \
  -w /work/behavior-freeze/fixtures/spu/sources/single_spu_dma_get_multi_v1 \
  rpcs3-ps3dev-toolchain:local \
  bash -lc 'make clean && make V=1'
```

Output: `single_spu_dma_get_multi_v1.self`. Move to `build/`
after verification.

## Capture trace (R8.2)

After RPCS3 OFF reproduces the canonical TTY, capture with the
R6.7 A.1 + R8.1 writer extension active (R8.1 rpcs3.exe with
the PUT writer hook — same `record_spu_mfc_cmd` +
`record_mfc_dma_complete` events fire for both cmd=0x40 GET
dispatches):

```cmd
set RPCS3_SPU_TRACE_JSONL=behavior-freeze\fixtures\spu\traces\single_spu_dma_get_multi_v1.jsonl
R:\bin\rpcs3.exe --no-gui ^
   behavior-freeze\fixtures\spu\sources\single_spu_dma_get_multi_v1\build\single_spu_dma_get_multi_v1.self
```

Capture-time requirements (same as R6.7 A.5 / R8.1):

- `Core: SPU Decoder: Interpreter (static)` AND
- `Core: PPU Decoder: Interpreter (static)`

LLVM JIT bypasses the C++ `set_ch_value()` MFC hooks, so the
trace would only contain `spu_image` / `spu_wrch ch28` / `spu_stop`
without the load-bearing `spu_mfc_cmd` + `mfc_dma_complete` events.

The capture produces:

- `behavior-freeze/fixtures/spu/traces/single_spu_dma_get_multi_v1.jsonl`
  — JSONL events. Expected ~25 events (vs ~15 for single-DMA
  fixtures): `spu_image` + 6 wrch (GET #1 params) + `spu_mfc_cmd` +
  `mfc_dma_complete` + 6 wrch (GET #2 params) + `spu_mfc_cmd` +
  `mfc_dma_complete` + 2 wrch (ch22/ch23) + 1 rdch ch24 + 1 wrch
  ch28 + `spu_stop` + `final_state`.
- `<jsonl>.dma/<sha1>.dmachunk` for GET #1 (128 bytes counting
  pattern). SHA `471fb943aa23c511f6f72f8d1652d9c880cfa392ad80503120547703e56a2be5`
  — already in the canonical pool from R6.7 / R8.1.
- `<jsonl>.dma/<sha2>.dmachunk` for GET #2 (64 bytes of 0x42).
  Brand-new SHA — will be added to the canonical pool.
- `<jsonl>.images/<sha>.spuimg` — full SPU LS at thread-create.

## Move artifacts to canonical locations

```bash
mv <jsonl>.images/*.spuimg behavior-freeze/fixtures/spu/images/
mv <jsonl>.dma/*.dmachunk behavior-freeze/fixtures/spu/dma/
mkdir -p build && mv ../single_spu_dma_get_multi_v1.self build/
```

## Replay test

`rust/rpcs3-spu-recompiler/tests/single_spu_dma_get_multi_v1_replay.rs`
extends the R6.7 A.5 GET replay test for 2 dispatches:

- Parses the JSONL via `parse_jsonl_trace`.
- Asserts exactly 2 `spu_mfc_cmd` events (cmd 0x40, tags 3 + 5,
  sizes 128 + 64, EAs distinct, LSAs distinct).
- Asserts exactly 2 `mfc_dma_complete` events (one per tag,
  matching sizes).
- Asserts WrTagMask = 0x28, WrTagUpdate = ALL, RdTagStat = 0x28.
- Asserts ch28 carries `0xE12DEA4E`.
- Asserts stop 0x101.
- Builds the SPU program from `.spuimg`, seeds r3 with EA1 and
  r4 with EA2 (PSL1GHT arg0 + arg1 in lane 1 of each register).
- Runs `apply_mfc_dma_pre_replay` (both GETs land their captured
  chunks into LS at the correct LSAs).
- Replays through Interpreter AND Recompiler via
  `replay_per_spu_traces_with`.
- Asserts:
  - `Finished{stop_code: 0x101}` on both backends.
  - `diff_snapshots(interp, jit).is_identical() == true`.
  - The SPU's final LS at both regions matches the captured
    chunks (deferred verification redundant for GET, but
    exercised to match the R8.1 PUT structure).
  - For both `spu_mfc_cmd` events: cmd=0x40, GET parameters
    match capture.

## Hard rules carried forward

- No fake DMA (both LS regions populated via real RPCS3 vm::
  memory).
- No manual JSONL edits.
- Both captured `.dmachunk` files MUST be byte-for-byte equal to
  what the PPU wrote to their respective EAs at thread-create
  time; any replay-time divergence is a real correctness gap.
