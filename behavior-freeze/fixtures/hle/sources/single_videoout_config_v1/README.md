# single_videoout_config_v1 (HLE wave — cellVideoOut get-configuration)

Fourth cellVideoOut function, reusing the `VideoOutManager` field from
`single_videoout_resavail_v1` — only the `cellVideoOutGetConfiguration` NID is
new (one more arm, no new dep/field).

## Behaviour

A PPU-only homebrew that calls `cellVideoOutGetConfiguration(VIDEO_PRIMARY, &cfg,
NULL)` (via PSL1GHT's `videoGetConfiguration`) into a zero-initialised
`videoConfiguration`:

- `getconfig != 0` → returns `0x0BAD`
- else → returns `cfg.resolution` (`0` pre-wire — struct untouched; `2`
  = `VIDEO_RESOLUTION_720` post-wire = the primary port's default config)

`videoConfiguration` layout: `resolution`@0 (u8), `format`@1, `aspect`@2,
`padding[9]`, `pitch`@12 (u32). No printf / SPU / RSX.

## How it wires

1. The NID fires (captured at runtime; r3=videoOut, r4=&cfg, r5=option).
2. The dispatcher routes it to `cell_video_out_get_configuration(&self.videoout,
   port)` and serialises resolution/format/aspect (bytes @0..2) + pitch (BE u32
   @offset 12) into the guest struct.

## Result

`EmuCore::run_self` exit status = **2** (720p), vs `0` pre-wire — proving the
fourth cellVideoOut function runs end-to-end off the shared field.

## Consumed by

`rust/rpcs3-emu-core/tests/hle_videoout_config.rs`. The `.self`/`.elf` are built
locally via the PSL1GHT Docker toolchain and gitignored.

CC0 1.0 (public domain) — see LICENSE.md.
