# single_pad_info_v1 (HLE backlog — cellPad GetInfo2, deterministic headless)

`ioPadInit(7) -> ioPadGetInfo2`. RPCS3's `cellPadGetInfo2` reports `max_connect`
(the Init value) + `now_connect` (connected-pad count from the host controller
handler). emu-core is headless (no host pad handler) → every port disconnected →
`max=7, connected=0`. Deterministic, no callbacks/FS/decode.

## How it wires (emu-core arms, NIDs captured at runtime)

- `cellPadInit` -> `rpcs3_hle_cellpad::cell_pad_init(max)` (CELL_OK; gate).
- `cellPadGetInfo2` -> `cell_pad_get_info2(&pad, &NoPads)` (NoPads backend = 0
  connected), then the 124-byte `CellPadInfo2` is BE-serialized: max@0=7,
  now@4=0, system@8, port_status[7]@12, port_setting[7]@40, capability[7]@68,
  type[7]@96 (all 0 with no pads).

## Result

`EmuCore::run_self` exit = **0xC0DE** (max==7 && connected==0), vs 0xBAD2/0xBAD3
on a wrong field (the homebrew poisons the struct with 0xFF first, so the arm
must overwrite it).

## Consumed by

`rust/rpcs3-emu-core/tests/hle_pad_info.rs`. `.self`/`.elf` built via Docker +
gitignored.

CC0 1.0 (public domain) — see LICENSE.md.
