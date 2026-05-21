# single_spu_dma_tag_immediate_v1 — R8.3c fixture

**R8.3c first IMMEDIATE wait-mode oracle (12th oracle).**
Same shape as R8.3b but `WrTagUpdate = IMMEDIATE` (= 0) and
distinct masks (0x08 then 0x28). Probes the IMMEDIATE +
clearing semantics. CC0 1.0 public domain.

## Behaviour

1-3. Identical to R8.3b (PPU fills ea_buf1+ea_buf2, SPU
     dispatches GET#1+GET#2).
4. SPU writes `WrTagMask = 0x08`, `WrTagUpdate = IMMEDIATE`,
   reads ch24 → captured `ts1 = 0x08`.
5. SPU writes `WrTagMask = 0x28`, `WrTagUpdate = IMMEDIATE`,
   reads ch24 → captured `ts2 = 0x28` (load-bearing).
6. SPU computes:
   - combined = (sum1 << 16) | sum2 = 0x1FC0_1080
   - packed = (ts1 << 24) | (ts2 << 16) = 0x0828_0000
   - status = combined ^ packed ^ 0xCAFE5A1E = 0xDD16_4A9E
7. SPU writes status to OUT_MBOX, halts via stop 0x101.

## Canonical TTY

```
[dma_tag_immediate_v1] OK cause=0x1 status=0xdd164a9e
```

`ts2 == 0x28` (where mask covers BOTH tag bits) proves
IMMEDIATE does NOT clear `completed_tags` on read — confirms
Cell BE persistent register semantic.

## Predicted alternatives (not observed in this RPCS3)

| Semantic | ts2 | status |
|---|---|---|
| **No-clear (Cell BE, captured)** | 0x28 | `0xDD164A9E` ✓ |
| Per-bit clear on IMMEDIATE | 0x20 | `0xDD1E4A9E` |
| Full clear on IMMEDIATE | 0x00 | `0xDD3E4A9E` |

If a future RPCS3 / hardware backend produces a different
value, re-capture and document; do NOT hand-edit the JSONL.

## Build

Same Docker image as the other R8.x PSL1GHT fixtures:

```bash
docker run --rm -v "$PWD":/work \
  -e PS3DEV=/opt/ps3dev -e PSL1GHT=/opt/ps3dev/psl1ght \
  -w /work/behavior-freeze/fixtures/spu/sources/single_spu_dma_tag_immediate_v1 \
  rpcs3-ps3dev-toolchain:local \
  bash -lc 'make clean && make V=1'
```

Output: `single_spu_dma_tag_immediate_v1.self` (940 KB; sha
`84809807fbe5e34566012dcba292f3123ded6b2887235b8fc2bde5b90ad97b00`).

## Capture trace (R8.3c)

Standard procedure (Interpreter (static) both decoders):

```cmd
set RPCS3_SPU_TRACE_JSONL=behavior-freeze\fixtures\spu\traces\single_spu_dma_tag_immediate_v1.jsonl
R:\bin\rpcs3.exe --no-gui ^
   behavior-freeze\fixtures\spu\sources\single_spu_dma_tag_immediate_v1\build\single_spu_dma_tag_immediate_v1.self
```

26 events captured (mirror of R8.3b with ch23 = 0). Both
`.dmachunk` files dedup with canonical pool (zero new files
written).

## Hard rules

- No fake IMMEDIATE behavior. Captured values are canonical
  for THIS RPCS3 build.
- Status MUST embed both ch24 reads (carry forward of the
  R8.3a "embed every observed value" rule).
- No manual JSONL editing.
