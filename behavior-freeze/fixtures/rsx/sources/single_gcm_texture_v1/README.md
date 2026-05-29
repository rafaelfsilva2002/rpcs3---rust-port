# single_gcm_texture_v1 (R13.5e)

Second Camada-B descriptor struct (`TextureDescriptor`) validated against REAL
libgcm bytes, through the full cellGcm-init'd context (after `SurfaceDescriptor`
in R13.5c).

## Behaviour

`rsxInit` → `rsxLoadTexture(ctx, 0, &tex)` for a 256×128 A8R8G8B8 (linear,
normalized) texture at offset 0x200000 → `rsxTextureControl(ctx, 0, GCM_TRUE,…)`
(enable unit 0) → frame label → `return 0xC0DE`.

PSL1GHT librsx expands these into the `NV4097_SET_TEXTURE_*` method block
(OFFSET, FORMAT, ADDRESS, CONTROL0/1, FILTER, IMAGE_RECT, BORDER_COLOR).
`rpcs3-rsx-state::replay_gcm` parses unit 0 into a `TextureDescriptor`, collected
into `RsxSnapshot.textures`, which the test asserts field-by-field.

Pure command-stream test — no texel data is uploaded; the offset is just the
value emitted into `SET_TEXTURE_OFFSET`. Texture pixel-decode is the deferred
GPU-backend tail (Camada D).

## Result

The texture method addresses in `rpcs3-rsx-state` (`OFFSET=0x1A00`,
`FORMAT=0x1A04`, `CONTROL0=0x1A0C`, `IMAGE_RECT=0x1A18`) already match RPCS3
`gcm_enums.h`, so this slice **confirms** the texture decode is correct against
real bytes (no address bug, unlike R13.5c's surface PITCH_A). Observed decode:
`format_code=0xA5, dimension=TwoD, mipmap_levels=1, width=256, height=128,
location=1 (RSX), offset=0x200000, border=true` (FORMAT bit 3 set by the
`GCM_TEXTURE_FORMAT` flags).

## Consumed by

`rust/rpcs3-emu-core/tests/rsx_gcm_texture.rs`. The `.self`/`.elf` are built
locally via the PSL1GHT Docker toolchain and gitignored.

CC0 1.0 (public domain) — see LICENSE.md.
