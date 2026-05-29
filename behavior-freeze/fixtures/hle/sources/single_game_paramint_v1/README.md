# single_game_paramint_v1 (HLE wave — cellGame get-param-int)

Wires `cellGame` into `EmuCore` via a fixed `GameState` provider (`EmuGameConfig`)
— the provider wiring shape (like `cellSysutil`), for PSF parameter lookups.

## Behaviour

A PPU-only homebrew that calls `cellGameGetParamInt(PARENTAL_LEVEL, &v)` (via
PSL1GHT's `sysGameGetParamInt`):

- `ret != 0` → returns `0x0BAD`
- else → returns `v` (`0x55` sentinel pre-wire — OUT untouched; `1` post-wire =
  the configured parental level)

**Param-id numbering gotcha:** PSL1GHT's `sysutil/game.h` omits `APP_VERSION`, so
its `SYS_GAME_PARAMID_*` values are off-by-one vs the real-PS3 / RPCS3 / crate
numbering for ids ≥ 102 (crate: `AppVersion=102`, `ParentalLevel=103`). The
fixture passes the **real id 103** directly. No printf / SPU / RSX.

## How it wires

1. The NID fires (captured at runtime; r3=param id, r4=&value). Confirmed it
   dispatches cleanly with NO BootCheck lifecycle required.
2. The dispatcher routes it to `cell_game_get_param_int(&EmuGameConfig, id)`;
   `EmuGameConfig` impls the 9-method `GameState` trait with RPCS3-style homebrew
   defaults (parental level = 1). The value is written BE to the OUT pointer.

## Result

`EmuCore::run_self` exit status = **1** (configured parental level), vs `0x55`
pre-wire — proving the cellGame provider runs end-to-end.

## Consumed by

`rust/rpcs3-emu-core/tests/hle_game_paramint.rs`. The `.self`/`.elf` are built
locally via the PSL1GHT Docker toolchain and gitignored.

CC0 1.0 (public domain) — see LICENSE.md.
