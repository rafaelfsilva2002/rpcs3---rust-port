# single_videoout_state_v1 (HLE wave — cellVideoOut get-state)

Sixth cellVideoOut function, reusing the `VideoOutManager` field — only the
`cellVideoOutGetState` NID is new (one more arm, no new dep/field). Demonstrates
serialising a multi-field nested struct (`videoState` + `videoDisplayMode`) into
guest memory.

## Behaviour

A PPU-only homebrew that calls `cellVideoOutGetState(VIDEO_PRIMARY, 0, &state)`
(via PSL1GHT's `videoGetState`) into a zero-initialised `videoState`:

- `ret != 0` → returns `0x0BAD`
- else → returns `(colorSpace << 8) | displayMode.resolution` (`0` pre-wire —
  struct untouched; `0x102` post-wire = colorSpace 1, resolution 720p=2)

The `state` byte is ENABLED=0 (not distinguishable from the stub), so the proof
keys off colorSpace + resolution. No printf / SPU / RSX.

## Struct layout

`videoState`: `state`@0, `colorSpace`@1, `padding[6]`, `displayMode`@8.
`videoDisplayMode`: `resolution`@8, `scanMode`@9, `conversion`@10, `aspect`@11,
`padding[2]`, `refreshRates`@14.

## How it wires

1. The NID fires (captured at runtime; r3=videoOut, r4=deviceIndex, r5=&state).
2. The dispatcher routes it to `cell_video_out_get_state(&self.videoout, port,
   device_index)` and serialises state@0/colorSpace@1/resolution@8/scanMode@9/
   aspect@11 (the conversion + refreshRates fields stay zero).

## Result

`EmuCore::run_self` exit status = **0x102** (258), vs `0` pre-wire — proving the
sixth cellVideoOut function runs end-to-end off the shared field.

## Consumed by

`rust/rpcs3-emu-core/tests/hle_videoout_state.rs`. The `.self`/`.elf` are built
locally via the PSL1GHT Docker toolchain and gitignored.

CC0 1.0 (public domain) — see LICENSE.md.
