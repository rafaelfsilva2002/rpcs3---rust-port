# single_videoout_resolution_v1 (HLE wave — cellVideoOut)

Wires `cellVideoOut` into `EmuCore` — a **stateless** HLE crate (resolution-id →
width/height table lookup), so no EmuCore state field is needed (unlike
`cellSysModule`).

## Behaviour

A PPU-only homebrew that calls `cellVideoOutGetResolution(VIDEO_RESOLUTION_720,
&res)` (via PSL1GHT's `videoGetResolution`) into a zero-initialised
`videoResolution { u16 width; u16 height; }`, then packs the result:

- `ret != 0` → returns `0x0BAD`
- `ret == 0` → returns `(width << 16) | height` (`0` pre-wire — struct
  untouched; `0x050002D0` = `1280<<16 | 720` once wired)

No printf / SPU / RSX — a clean PPU + cellVideoOut HLE call.

## How it wires

1. The guest call fires the cellVideoOutGetResolution NID (captured at runtime
   from the `[R9.1g.7] unimplemented import` log; r3=2=VIDEO_RESOLUTION_720,
   r4=&res).
2. The dispatcher routes it to
   `rpcs3_hle_cellvideoout::cell_video_out_get_resolution(id)`, then writes
   `width` / `height` as big-endian `u16` into the guest struct.

## Result

`EmuCore::run_self` exit status = **0x050002D0** (1280×720), vs `0` pre-wire
(the stub never wrote the struct) — proving the crate runs end-to-end and BOTH
fields are correct.

## Consumed by

`rust/rpcs3-emu-core/tests/hle_videoout_resolution.rs`. The `.self`/`.elf` are
built locally via the PSL1GHT Docker toolchain and gitignored.

CC0 1.0 (public domain) — see LICENSE.md.
