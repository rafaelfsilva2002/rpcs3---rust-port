# single_videoout_resavail_v1 (HLE wave — cellVideoOut resolution availability)

Third cellVideoOut function. Introduces a **stateful** `VideoOutManager` field on
`EmuCore` (the prior two — GetResolution / GetNumberOfDevice — were stateless), so
the availability answer comes from the manager's `supported_resolutions` table.

## Behaviour

A PPU-only homebrew that calls
`cellVideoOutGetResolutionAvailability(VIDEO_PRIMARY, VIDEO_RESOLUTION_720,
VIDEO_ASPECT_16_9, 0)` (via PSL1GHT's `videoGetResolutionAvailability`):

- `ret == n` → returns `n` (`0` pre-wire — return-0 stub; `1` post-wire = 720p is
  in the default supported-resolution set)

No printf / SPU / RSX — a clean PPU + cellVideoOut HLE call.

## How it wires

1. The NID fires (captured at runtime; r3=videoOut, r4=resolutionId, r5=aspect,
   r6=option).
2. `rpcs3-emu-core` gains a `videoout: VideoOutManager` field (init in `new()`).
   The dispatcher routes the call to
   `cell_video_out_get_resolution_availability(&self.videoout, port, res, aspect)`
   → `1` for the supported 720p, returned in r3. (The field also unlocks
   GetState / GetConfiguration / GetDeviceInfo as future one-arm additions.)

## Result

`EmuCore::run_self` exit status = **1** (720p available), vs `0` pre-wire —
proving the stateful cellVideoOut path runs end-to-end.

## Consumed by

`rust/rpcs3-emu-core/tests/hle_videoout_resavail.rs`. The `.self`/`.elf` are built
locally via the PSL1GHT Docker toolchain and gitignored.

CC0 1.0 (public domain) — see LICENSE.md.
