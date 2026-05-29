# single_gcm_surface_v1 (R13.5c)

First validation of a whole **Camada-B descriptor struct** (`SurfaceDescriptor`)
against REAL libgcm bytes, through the full cellGcm-init'd context.

## Behaviour

`rsxInit` → `rsxSetSurface(&surface)` with a 640×480 surface (A8R8G8B8 color
into target 0 at offset 0x10000 pitch 2560; Z24S8 depth at offset 0x200000
pitch 2560) → frame label → `return 0xC0DE`.

PSL1GHT librsx expands `rsxSetSurface` inline into the `NV4097_SET_SURFACE_*`
method block (FORMAT, COLOR_TARGET, COLOR_AOFFSET, PITCH_A, ZETA_OFFSET,
PITCH_Z, CLIP_H/V). `rpcs3-rsx-state::replay_gcm` decodes it into
`RsxSnapshot.surface`, which the test asserts field-by-field.

It is a pure command-stream test — no framebuffer is allocated; the offsets are
just the values emitted into the methods. Real rendering is the deferred
GPU-backend tail.

## What it caught

Running this fixture's REAL libgcm bytes through `replay_gcm` exposed a
long-standing **decode bug** in `rpcs3-rsx-state`: `SURFACE_PITCH_A` was wired
to `0x0218` (which is actually `COLOR_BOFFSET`), and `COLOR_BOFFSET` / `PITCH_B`
were likewise off. The correct NV4097 addresses (per RPCS3 `gcm_enums.h`) are
`PITCH_A=0x020C`, `COLOR_BOFFSET=0x0218`, `PITCH_B=0x021C`. The self-referential
unit tests never caught it (they wrote + read at the same wrong constant); the
real-libgcm capture did — exactly the behavior-freeze value proposition.

## Consumed by

`rust/rpcs3-emu-core/tests/rsx_gcm_surface.rs` — boots this `.self` via
`EmuCore::run_self`, captures `[context.begin..current)`, `replay_gcm`, and
asserts the full `SurfaceDescriptor`.

The `.self`/`.elf` are built locally via the PSL1GHT Docker toolchain
(`MSYS_NO_PATHCONV=1 docker run ... bash -lc 'make'`) and gitignored.

CC0 1.0 (public domain) — see LICENSE.md.
