# single_spu_dma_put_v1 — R8.1 fixture

**R8.1 first replay-validated DMA PUT fixture (8th oracle).**
Symmetric to R6.7 A.5 `single_spu_dma_get_v1` but inverts the
DMA direction (LS → EA). CC0 1.0 public domain.

## Behaviour

1. PPU allocates a 128-byte EA buffer, ZERO-FILLED.
2. PPU passes EA via `thread_args.arg0` and starts the SPU group.
3. SPU fills LS at `lsa=0x10000` with the counting pattern
   `buf[i] = i & 0xFF` for `i in 0..128` (sum = 8128 = 0x1FC0).
4. SPU dispatches `MFC PUT` (cmd=0x20) from LS to EA, size 128.
5. SPU waits for the tag via `WrTagMask=1<<3`,
   `WrTagUpdate=ALL`, then blocks on `rdch ch24 (RdTagStat)`.
6. SPU writes sentinel `0xC0FFEECA` to OUT_MBOX, halts via
   `stop 0x101 (SYS_SPU_THREAD_STOP_GROUP_EXIT)`.
7. PPU joins. lv2 reads OUT_MBOX as the group-exit status
   (= sentinel).
8. PPU reads EA, sums all 128 bytes, XORs with `0xCAFEBABE`,
   prints both numbers.

## Canonical TTY

```
[dma_put_v1] OK cause=0x1 spu=0xc0ffeeca ea_status=0xcafea57e
```

Three independent invariants on one line:

- `cause=0x1` (= `GROUP_EXIT`): SPU stopped cleanly.
- `spu=0xc0ffeeca` (the sentinel): SPU reached the post-PUT path
  (i.e. the `rdch ch24` unblocked → tag was completed → MFC
  acknowledged the PUT).
- `ea_status=0xcafea57e`: the PUT BYTES actually landed in EA.
  Computed as `sum(ea_after_put) ^ 0xCAFEBABE`. For the canonical
  source pattern, `sum = 8128 = 0x1FC0` and `ea_status =
  0x1FC0 ^ 0xCAFEBABE = 0xCAFEA57E`.

## Failure mode catalogue

| Failure | Observable |
|---|---|
| SPU never ran | no TTY at all (PPU stuck) |
| SPU ran but PUT never dispatched | `spu=0xc0ffeeca ea_status=0xcafebabe` (sentinel OK, EA stayed zero) |
| PUT dispatched with wrong bytes | `spu=0xc0ffeeca ea_status=<other>` (sentinel OK, EA wrong) |
| Tag never completed (PUT silently dropped) | SPU hangs on rdch ch24 → no sentinel |
| Correct | `spu=0xc0ffeeca ea_status=0xcafea57e` |

## Build

Same Docker image as the other R5/R6/R7 PSL1GHT fixtures:

```bash
docker run --rm -v "$PWD":/work \
  -w /work/behavior-freeze/fixtures/spu/sources/single_spu_dma_put_v1 \
  rpcs3-ps3dev-toolchain:local \
  bash -lc 'make clean && make V=1'
```

Output: `single_spu_dma_put_v1.self`. Move to `build/` after
verification.

## Capture trace (R8.1)

After RPCS3 OFF reproduces the canonical TTY, capture with the
R6.7 A.1 + R8.1 writer extension active (R7-aware rpcs3.exe with
the R8.1 PUT writer hook):

```cmd
set RPCS3_SPU_TRACE_JSONL=behavior-freeze\fixtures\spu\traces\single_spu_dma_put_v1.jsonl
R:\bin\rpcs3.exe --no-gui ^
   behavior-freeze\fixtures\spu\sources\single_spu_dma_put_v1\build\single_spu_dma_put_v1.self
```

Capture-time requirements (same as R6.7 A.5):

- `Core: SPU Decoder: Interpreter (static)` AND
- `Core: PPU Decoder: Interpreter (static)`

LLVM JIT bypasses the C++ `set_ch_value()` MFC hooks, so the
trace would only contain `spu_image` / `spu_wrch ch28` / `spu_stop`
without the load-bearing `spu_mfc_cmd` + `mfc_dma_complete` events.

The capture produces:

- `behavior-freeze/fixtures/spu/traces/single_spu_dma_put_v1.jsonl`
  — JSONL events.
- `<jsonl>.dma/<sha>.dmachunk` — the captured SPU LS-source bytes
  at dispatch time. For PUT, this is what the SPU PRODUCED (not
  what it received); the replay state machine asserts that the
  Rust-replayed SPU's LS bytes at dispatch byte-match the chunk.
- `<jsonl>.images/<sha>.spuimg` — full SPU LS at thread-create.

## Move artifacts to canonical locations

```bash
mv <jsonl>.images/*.spuimg behavior-freeze/fixtures/spu/images/
mv <jsonl>.dma/*.dmachunk behavior-freeze/fixtures/spu/dma/
mkdir -p build && mv ../single_spu_dma_put_v1.self build/
```

## Replay test

`rust/rpcs3-spu-recompiler/tests/single_spu_dma_put_v1_replay.rs`
mirrors the R6.7 A.5 GET replay test:

- Parses the JSONL via `parse_jsonl_trace`.
- Builds the SPU program from `.spuimg` via
  `build_spu_program_from_captured_image`.
- Runs `apply_mfc_dma_pre_replay` (R6.7 C.3 helper; the A.4 state
  machine now distinguishes GET vs PUT: GET writes the chunk into
  LS before replay, PUT asserts the SPU's LS bytes at dispatch
  match the chunk).
- Replays through Interpreter AND Recompiler via
  `replay_per_spu_traces_with`.
- Asserts:
  - `Finished{stop_code: 0x101}` on both backends.
  - `diff_snapshots(interp, jit).is_identical() == true`.
  - The SPU's OUT_MBOX = `0xC0FFEECA` (sentinel) — proves the
    post-PUT path ran.
  - For the captured `spu_mfc_cmd` event: cmd=0x20, size=128, tag=3.
  - `.dmachunk` SHA256 + size validate against the event.

## Hard rules carried forward

- No fake PUT (LS bytes flow through real RPCS3 vm:: memory).
- No manual JSONL edits.
- The captured `.dmachunk` IS the SPU's PUT output at dispatch
  time; any replay-time divergence is a real correctness gap.
