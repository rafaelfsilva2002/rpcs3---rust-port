# single_spu_dma_tag_poll_v1 — R8.3b fixture

**R8.3b first repeated-RdTagStat polling oracle (11th oracle).**
Two queued GETs (same shape as R8.3a multi) + TWO ch24 reads in
the same SPU session with distinct masks (0x08 then 0x20, both
ANY mode). Forces persistent `completed_tags` semantics in
`SpuChannels`. CC0 1.0 public domain.

## Behaviour

1. PPU allocates ea_buf1 (128 B, counting pattern `i & 0xFF`)
   and ea_buf2 (64 B, constant 0x42).
2. PPU passes EA1 via `thread_args.arg0`, EA2 via `arg1`.
3. SPU dispatches GET#1 (tag 3, EA1 → LS@0x10000, size 128)
   and GET#2 (tag 5, EA2 → LS@0x10100, size 64).
4. SPU writes `WrTagMask = 0x08`, `WrTagUpdate = ANY (1)`,
   reads `RdTagStat` → captured `tag_stat_1 = 0x08` (tag 3
   completed bit, mask-filtered).
5. SPU writes `WrTagMask = 0x20`, `WrTagUpdate = ANY (1)`,
   reads `RdTagStat` → captured `tag_stat_2 = 0x20` (tag 5
   completed bit, mask-filtered — `completed_tags = 0x28`
   retained across reads).
6. SPU computes:
   - sum1 = 0x1FC0, sum2 = 0x1080
   - combined = (sum1 << 16) | sum2 = 0x1FC0_1080
   - packed = (tag_stat_1 << 24) | (tag_stat_2 << 16) = 0x0820_0000
   - status = combined ^ packed ^ 0xCAFEBADC = 0xDD1E_AA5C
7. SPU writes status to OUT_MBOX, halts via stop 0x101.

## Canonical TTY

```
[dma_tag_poll_v1] OK cause=0x1 status=0xdd1eaa5c
```

## Failure mode catalogue

| Failure | Observable status |
|---|---|
| Both DMAs zero-filled, both reads = 0x28 (hardware-style) | distinctively wrong |
| Both DMAs OK, second read stalls (drain-clear bug) | bridge ON: stall fallback; replay: parked at pc=140 |
| Both DMAs OK, both reads return 0x28 (mask not filtering) | `0x1FC0_1080 ^ 0x2828_0000 ^ 0xCAFEBADC = 0xF53EAA5C` |
| Both DMAs OK, persistent state working correctly | `0xDD1EAA5C` (canonical) |

## Why this matters

The R8.3a closure note explicitly documented:

> The drain-semantic empties the queue. A future fixture
> performing MULTIPLE ch24 reads in the same SPU session
> would see the first read consume all pending bits and
> subsequent reads stall. R8.4+ that requires polling forces
> a refactor to persistent `completed_tags: u32` on
> `SpuChannels`.

R8.3b is exactly that fixture, written intentionally to force
the refactor. The replay test was authored BEFORE the fix and
predicted to fail at the second ch24 read (queue empty post-
first-drain). Both predicted failures (replay + bridge ON)
were empirically observed before the persistent-state landed.

## Build

Same Docker image as the other R8.x PSL1GHT fixtures:

```bash
docker run --rm -v "$PWD":/work \
  -e PS3DEV=/opt/ps3dev -e PSL1GHT=/opt/ps3dev/psl1ght \
  -w /work/behavior-freeze/fixtures/spu/sources/single_spu_dma_tag_poll_v1 \
  rpcs3-ps3dev-toolchain:local \
  bash -lc 'make clean && make V=1'
```

Output: `single_spu_dma_tag_poll_v1.self`. Move to `build/`.

## Capture trace (R8.3b)

After RPCS3 bridge OFF reproduces the canonical TTY (LLVM or
Interpreter both work for the binary; capture requires
Interpreter):

```cmd
set RPCS3_SPU_TRACE_JSONL=behavior-freeze\fixtures\spu\traces\single_spu_dma_tag_poll_v1.jsonl
R:\bin\rpcs3.exe --no-gui ^
   behavior-freeze\fixtures\spu\sources\single_spu_dma_tag_poll_v1\build\single_spu_dma_tag_poll_v1.self
```

Capture-time requirements (carry forward):
- `Core: SPU Decoder: Interpreter (static)`
- `Core: PPU Decoder: Interpreter (static)`

The capture produces 26 events (vs 23 for single-read DMA
fixtures). Both `.dmachunk` files dedup with the canonical
pool — zero new files written.

## Replay test

`rust/rpcs3-spu-recompiler/tests/single_spu_dma_tag_poll_v1_replay.rs`
extends R8.3a's shape:

- TWO `ch22` writes (distinct masks).
- TWO `ch23` writes (both ANY).
- TWO `ch24` reads (captured `0x08` then `0x20`).
- Canonical OUT_MBOX = `0xDD1EAA5C`.

The replay engine uses the persistent `completed_tags` register
introduced by R8.3b: first read absorbs the pre-replay queue
into `completed_tags`; second read returns the same persistent
state with the new mask applied.

## Hard rules carried forward

- No fake DMA. Both LS regions populated via real RPCS3 vm::
  memory.
- No fake RdTagStat. Both captured ch24 values are load-bearing
  canonical.
- Status must embed BOTH ch24 values via XOR (the R8.3a +
  R8.3b lesson on canonical-embedding).
