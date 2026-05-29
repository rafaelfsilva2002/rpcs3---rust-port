# single_gcm_multidraw_v1 (R13.5a)

Validates MULTIPLE `DrawCall` records in one frame against REAL libgcm bytes —
the draw oracle so far only ever produced a single DrawCall.

## Behaviour

`rsxInit` → `rsxDrawVertexArray(TRIANGLES, 0, 3)` → `rsxDrawVertexArray(TRIANGLES,
10, 6)` → frame label → `return 0xC0DE`.

Each `rsxDrawVertexArray` emits `SET_BEGIN_END(5)` + `DRAW_ARRAYS` +
`SET_BEGIN_END(0)`, so `rpcs3-rsx-state`'s `DrawTracker` finalizes one DrawCall
per call → two DrawCalls in the snapshot.

Pure command-stream test — no vertex buffers uploaded; the ranges are the values
emitted into `DRAW_ARRAYS`.

## Result

Observed decode (matches the fixture intent):
`draw_calls = [ {primitive:5, kind:Arrays, ranges:[(0,3)]},
{primitive:5, kind:Arrays, ranges:[(10,6)]} ]`.

## Consumed by

`rust/rpcs3-emu-core/tests/rsx_gcm_multidraw.rs`. The `.self`/`.elf` are built
locally via the PSL1GHT Docker toolchain and gitignored.

CC0 1.0 (public domain) — see LICENSE.md.
