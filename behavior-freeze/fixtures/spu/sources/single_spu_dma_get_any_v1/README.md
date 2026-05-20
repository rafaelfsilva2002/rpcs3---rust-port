# single_spu_dma_get_any_v1 — R8.3a fixture

**R8.3a first ANY-wait-mode replay-validated fixture (10th
oracle).** Two queued MFC GETs (same shape as R8.2) but with
`WrTagUpdate = ANY` (= 1) instead of ALL (= 2). The SPU embeds
the actual `RdTagStat` return into the canonical OUT_MBOX
status, so the fixture is resilient to whatever the backend
chooses. CC0 1.0 public domain.

## Behaviour

1. PPU allocates two distinct EA buffers (same patterns + sizes
   as R8.2): `ea_buf1` (128 B, counting pattern `i & 0xFF`) →
   sum1 = 0x1FC0; `ea_buf2` (64 B, constant 0x42) →
   sum2 = 0x1080.
2. PPU passes EA1 via `thread_args.arg0` and EA2 via `arg1`,
   starts the SPU group.
3. SPU dispatches GET #1 (tag 3, EA1 → LS@0x10000, size 128)
   and GET #2 (tag 5, EA2 → LS@0x10100, size 64) back-to-back
   BEFORE any tag wait.
4. SPU writes `WrTagMask = 0x28`, `WrTagUpdate = ANY`,
   `rdch ch24`. ANY returns a non-zero subset of the completed
   tags ∩ mask. In RPCS3 sync DMA both tags are already
   completed at the moment of rdch, so ANY returns the full
   mask 0x28.
5. SPU embeds the returned tag_stat into the canonical status:
   `combined = (sum1 << 16) | sum2 = 0x1FC0_1080`
   `status = combined ^ (tag_stat << 24) ^ 0xBEEFBEAD`
   For tag_stat = 0x28: `status = 0x892F_AE2D`.
6. SPU writes status to OUT_MBOX, halts via `stop 0x101`.
7. PPU joins; lv2 reads OUT_MBOX as the group-exit status.

## Canonical TTY (expected for RPCS3 sync DMA backend)

```
[dma_get_any_v1] OK cause=0x1 status=0x892fae2d
```

Three invariants:

- `cause=0x1` (= GROUP_EXIT): SPU stopped cleanly.
- `status=0x892fae2d`: SPU saw BOTH LS regions correctly
  populated AND the backend's ch24 returned the full mask
  (0x28). Either DMA failing → distinctively wrong status;
  the backend returning a smaller mask → also distinctively
  wrong status (each choice round-trips through the high-byte
  XOR).

**Note:** the canonical value is FIXED by the first capture
from a known-good RPCS3 build. If a future capture against a
different backend (or real hardware) produces a different
status, that's a backend behavior change, not a regression —
investigate and document the new canonical (and update this
README + the replay test).

## Failure mode catalogue

| Failure | Observable status |
|---|---|
| SPU never ran | no TTY at all (PPU stuck) |
| Both DMAs silently dropped (LS zero-fill), ch24 = 0x28 | `0 ^ 0x2800_0000 ^ 0xBEEFBEAD = 0x96EFBEAD` |
| Both DMAs OK, ch24 broken returning 0 | `0x1FC0_1080 ^ 0x0 ^ 0xBEEFBEAD = 0xA12FAEAD` |
| Both DMAs OK, ch24 returned 0x8 only (partial ANY) | `0x1FC0_1080 ^ 0x0800_0000 ^ 0xBEEFBEAD = 0xA92FAEAD` |
| Both DMAs OK, ch24 returned 0x20 only (partial ANY) | `0x1FC0_1080 ^ 0x2000_0000 ^ 0xBEEFBEAD = 0x812FAEAD` |
| Both DMAs OK, ch24 returned 0x28 (full mask, RPCS3 sync) | `0x1FC0_1080 ^ 0x2800_0000 ^ 0xBEEFBEAD = 0x892FAE2D` (canonical) |
| One GET silently dropped (LS partial zero), ch24 = 0x28 | distinct combined sum → distinct status |

## Build

Same Docker image as the other R5/R6/R7/R8.1/R8.2 PSL1GHT
fixtures:

```bash
docker run --rm -v "$PWD":/work \
  -e PS3DEV=/opt/ps3dev -e PSL1GHT=/opt/ps3dev/psl1ght \
  -w /work/behavior-freeze/fixtures/spu/sources/single_spu_dma_get_any_v1 \
  rpcs3-ps3dev-toolchain:local \
  bash -lc 'make clean && make V=1'
```

Output: `single_spu_dma_get_any_v1.self`. Move to `build/`
after verification.

## Capture trace (R8.3a)

After RPCS3 OFF reproduces the canonical TTY, capture with the
R6.7 A.1 + R8.1 writer extension active (R8.1 rpcs3.exe handles
cmd=0x40 GET capture unchanged; the ANY mode shows up in the
captured ch23 value = 1 instead of 2):

```cmd
set RPCS3_SPU_TRACE_JSONL=behavior-freeze\fixtures\spu\traces\single_spu_dma_get_any_v1.jsonl
R:\bin\rpcs3.exe --no-gui ^
   behavior-freeze\fixtures\spu\sources\single_spu_dma_get_any_v1\build\single_spu_dma_get_any_v1.self
```

Capture-time requirements (same as R6.7 A.5 / R8.1 / R8.2):

- `Core: SPU Decoder: Interpreter (static)` AND
- `Core: PPU Decoder: Interpreter (static)`

LLVM JIT bypasses the C++ `set_ch_value()` MFC hooks; restore
to `Recompiler (LLVM)` after capture.

The capture produces:

- `behavior-freeze/fixtures/spu/traces/single_spu_dma_get_any_v1.jsonl`
  — JSONL events. Expected 23 events (same as R8.2).
- `<jsonl>.dma/<sha1>.dmachunk` for GET #1 (128 bytes counting
  pattern). SHA `471fb943aa23c511f6f72f8d1652d9c880cfa392ad80503120547703e56a2be5`
  — already in the canonical pool from R6.7 / R8.1 / R8.2.
- `<jsonl>.dma/<sha2>.dmachunk` for GET #2 (64 bytes of 0x42).
  SHA `c422e7070cb1cb455b5de9afee0d975e303d0239c72030cd7414ab5c382d3ae8`
  — already in the canonical pool from R8.2.
- `<jsonl>.images/<sha>.spuimg` — full SPU LS at thread-create.
  This will be a NEW SHA (the SPU code base differs from R8.2's
  ch23 ALL → ANY).

**Expected zero new `.dmachunk` files** because the patterns
are identical to R8.2's. Content-addressed dedup at its best.

## Move artifacts to canonical locations

```bash
mv <jsonl>.images/*.spuimg behavior-freeze/fixtures/spu/images/
mv <jsonl>.dma/*.dmachunk behavior-freeze/fixtures/spu/dma/  # both already there, deletions OK
mkdir -p build && mv ../single_spu_dma_get_any_v1.self build/
```

## Replay test

`rust/rpcs3-spu-recompiler/tests/single_spu_dma_get_any_v1_replay.rs`
extends the R8.2 multi-DMA shape to ANY:

- Parses the JSONL via `parse_jsonl_trace`.
- Asserts exactly 2 `spu_mfc_cmd` events (cmd=0x40, tags 3+5,
  sizes 128+64, EAs distinct).
- Asserts exactly 2 `mfc_dma_complete` events.
- Asserts `ch22 = 0x28`, **`ch23 = 1 (ANY)`**, `ch24 rdch =
  <captured-value>` (the load-bearing ANY assertion).
- Asserts ch28 carries the captured canonical status.
- Asserts stop 0x101.
- Builds SpuProgram from `.spuimg`, seeds r3 = EA1 lane 1 +
  r4 = EA2 lane 1.
- Runs `apply_mfc_dma_pre_replay` (both chunks land in LS;
  tag-stat queue gets one entry = the captured ANY return).
- Replays through Interpreter AND Recompiler;
  `diff_snapshots.is_identical()`.
- Post-replay LS verification mirrors R8.2 + R8.1 shape.

## Hard rules carried forward

- No fake DMA (both LS regions populated via real RPCS3 vm::
  memory).
- No fake RdTagStat (the captured ch24 value IS the canonical
  ANY return for this backend — replay must reproduce exactly).
- No manual JSONL edits.
- Captured `.dmachunk` files must byte-match the PPU's pre-DMA
  EA contents.
