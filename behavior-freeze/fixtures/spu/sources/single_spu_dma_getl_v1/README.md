# single_spu_dma_getl_v1 — R8.4b fixture

**R8.4b first MFC GETL list-DMA capture (13th oracle target).**
CC0 1.0 public domain.

## Behaviour

1. PPU fills two EA buffers (ea_buf1 = 128 B counting pattern,
   ea_buf2 = 64 B constant 0x42) and passes both EAs via
   `thread_args.arg0` / `arg1`.
2. SPU builds a 2-element list_element[] in LS at a static
   offset:
   ```
   list[0] = { sb=0, pad=0, ts=128, ea=EA1 }
   list[1] = { sb=0, pad=0, ts= 64, ea=EA2 }
   ```
3. SPU dispatches MFC GETL (cmd=0x44) with ch16-21:
   - LSA      = 0x10000 (destination base)
   - EAH      = 0
   - EAL      = LS offset of the descriptor list (NOT data EA)
   - Size     = 16 (= 2 elements × 8 bytes)
   - TagID    = 3
   - Cmd      = 0x44 GETL
4. RPCS3 walks the descriptors, copies element 0 (128 B) into
   LS[0x10000..0x10080] and element 1 (64 B) into
   LS[0x10080..0x100C0].
5. SPU waits via ch22/ch23/ch24 (mask=0x08, ALL).
6. SPU sums both regions, packs as combined = (sum1 << 16) | sum2
   = 0x1FC0_1080, XORs with 0xC0DEFADA → canonical status
   0xDF1E_EA5A.
7. SPU writes status to OUT_MBOX, halts via stop 0x101.

## Canonical TTY

```
[dma_getl_v1] OK cause=0x1 status=0xdf1eea5a
```

## R8.4b acceptance status

- **Bridge OFF (LLVM C++ executor)**: ✓ produces canonical
  (verified 2026-05-21).
- **Real JSONL capture (R8.4b writer extension)**: ✓ 15 events
  including `spu_mfc_cmd cmd=0x44` with the 5 additive list
  fields (`descriptor_sha256`, `descriptor_size`,
  `element_chunks`, `element_sizes`, `element_eals`).
- **`.dmalistdesc` side-file**: ✓ new file kind landed in
  canonical pool (16 bytes; sha `79238773…`).
- **Element `.dmachunk` side-files**: ✓ both dedup with
  existing pool (zero new files).
- **Replay test**: ⛔ NOT YET LANDED. Parser deserializes the
  additive fields but `MfcReplayState` rejects with
  `UnsupportedMfcListCmd` (R8.4a canary preserved). R8.4c
  lifts the canary AND adds the replay state machine + the
  13th oracle promotion in one coherent delivery.
- **Bridge ON runtime delegation**: ⛔ NOT YET LANDED. The
  Rust bridge has no GETL callback installed; bridge ON will
  hit `MfcUnsupported` and fall back to C++. R8.4d adds the
  `rust_spu_set_dma_getl_callback` FFI + bridge handler.

## Build

```bash
docker run --rm -v "$PWD":/work \
  -e PS3DEV=/opt/ps3dev -e PSL1GHT=/opt/ps3dev/psl1ght \
  -w /work/behavior-freeze/fixtures/spu/sources/single_spu_dma_getl_v1 \
  rpcs3-ps3dev-toolchain:local \
  bash -lc 'make clean && make V=1'
```

Output: `single_spu_dma_getl_v1.self`. Move to `build/`.

## Capture (R8.4b)

After rpcs3.exe with the R8.4b writer extension is built
(sha `3f2348de…` or later), capture with Interpreter
(static) decoders:

```cmd
set RPCS3_SPU_TRACE_JSONL=behavior-freeze\fixtures\spu\traces\single_spu_dma_getl_v1.jsonl
R:\bin\rpcs3.exe --no-gui ^
   behavior-freeze\fixtures\spu\sources\single_spu_dma_getl_v1\build\single_spu_dma_getl_v1.self
```

The capture produces the JSONL trace + `.dmalistdesc` +
2 `.dmachunk` (dedup) side-files. See the corresponding
`single_spu_dma_getl_v1.notes.md` for full schema and
acceptance criteria.

## Hard rules

- No fake descriptor — the `.dmalistdesc` content MUST be
  the actual bytes the SPU wrote to LS at the dispatch
  moment.
- No fake element chunk — each `.dmachunk` MUST be the
  actual EA bytes per element at the dispatch moment.
- No manual JSONL editing.
- No stall-and-notify (`sb` bit 0x80 in descriptor) — R8.5+
  scope.
- No PUTL / GETLB / GETLF — out of R8.4b scope.
