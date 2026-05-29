# single_gcm_indexdraw_v1 (R13.5b)

First INDEXED-draw path validated against REAL libgcm bytes, through the full
cellGcm-init'd context (the draw oracle `single_gcm_draw_v1` only covered
`DrawKind::Arrays`).

## Behaviour

`rsxInit` → `rsxDrawIndexArray(ctx, GCM_TYPE_TRIANGLES, offset 0x10000, count 3,
GCM_INDEX_TYPE_16B, GCM_LOCATION_RSX)` → frame label → `return 0xC0DE`.

PSL1GHT librsx expands `rsxDrawIndexArray` into `SET_INDEX_ARRAY_ADDRESS` +
`SET_INDEX_ARRAY_DMA` + `SET_BEGIN_END(5)` + `DRAW_INDEX_ARRAY` +
`SET_BEGIN_END(0)`. `rpcs3-rsx-state` recognises the trio as a
`DrawKind::Indexed` `DrawCall` and parses the `IndexArray` descriptor.

Pure command-stream test — no index/vertex buffers are uploaded; the offset is
just the value emitted into `SET_INDEX_ARRAY_ADDRESS`.

## Result

Observed decode (all match the fixture intent — no decode bug; the index-array
method addresses already matched RPCS3 `gcm_enums.h`):
`DrawCall { primitive: 5, kind: Indexed, ranges: [(0, 3)] }` and
`IndexArray { address: 0x10000, index_type: U16, location: 0 (RSX) }`.

## Consumed by

`rust/rpcs3-emu-core/tests/rsx_gcm_indexdraw.rs`. The `.self`/`.elf` are built
locally via the PSL1GHT Docker toolchain and gitignored.

CC0 1.0 (public domain) — see LICENSE.md.
