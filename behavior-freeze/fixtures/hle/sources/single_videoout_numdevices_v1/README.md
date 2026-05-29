# single_videoout_numdevices_v1 (HLE wave — cellVideoOut device count)

Second `cellVideoOut` function (after `GetResolution`), reusing the same
`rpcs3-hle-cellvideoout` dep — a stateless count, returned directly in `r3`
(no OUT pointer).

## Behaviour

A PPU-only homebrew that calls `cellVideoOutGetNumberOfDevice(VIDEO_PRIMARY)`
(via PSL1GHT's `videoGetNumberOfDevice`) and returns the count:

- `ret < 0` → returns `0x0BAD`
- `ret == n` → returns `n` (`0` pre-wire — return-0 stub; `1` post-wire = the
  primary port reports one connected device)

No printf / SPU / RSX — a clean PPU + cellVideoOut HLE call.

## How it wires

1. The guest call fires the cellVideoOutGetNumberOfDevice NID (captured at
   runtime from the `[R9.1g.7] unimplemented import` log; r3=0=VIDEO_PRIMARY).
2. The dispatcher routes it to
   `rpcs3_hle_cellvideoout::cell_video_out_get_number_of_device(port)`
   (`Ok(1)` for the primary port) and returns it in `r3`.

## Result

`EmuCore::run_self` exit status = **1** (primary device connected), vs `0`
pre-wire (the return-0 stub) — proving the call runs end-to-end.

## Consumed by

`rust/rpcs3-emu-core/tests/hle_videoout_numdevices.rs`. The `.self`/`.elf` are
built locally via the PSL1GHT Docker toolchain and gitignored.

CC0 1.0 (public domain) — see LICENSE.md.
