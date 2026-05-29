# single_gcm_viewport_v1 (R13.5d)

Validates the RSX **viewport state registers** against REAL libgcm bytes. Unlike
the surface/texture/draw slices (which surface in `RsxSnapshot`), the viewport is
a *SetState* register group — so the test decodes the captured stream straight
into an `RsxState` and reads the registers directly.

## Behaviour

`rsxInit` → `rsxSetViewport(0, 0, 640, 480, 0.0, 1.0, scale, offset)` → frame
label → `return 0xC0DE`.

PSL1GHT librsx emits `SET_VIEWPORT_HORIZONTAL/VERTICAL` (+ depth range / scale /
offset) state methods. The scale/offset land in registers the test does not
read; only HORIZONTAL/VERTICAL are asserted.

## Result

Observed decode (matches the fixture intent):
`VIEWPORT_HORIZONTAL = 0x02800000` (width 0x280=640, x 0) and
`VIEWPORT_VERTICAL = 0x01E00000` (height 0x1E0=480, y 0) — i.e. each register
packs `(origin | size << 16)`.

## Consumed by

`rust/rpcs3-emu-core/tests/rsx_gcm_viewport.rs` — decodes the stream via
`RsxState::run_and_apply` (needs the `rpcs3-rsx-fifo` dev-dep) and asserts the
viewport registers. The `.self`/`.elf` are built locally via the PSL1GHT Docker
toolchain and gitignored.

CC0 1.0 (public domain) — see LICENSE.md.
