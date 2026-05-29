# single_sysutil_param_v1 (R13.6 — first HLE-crate integration)

The first of the 137 `rpcs3-hle-*` crates wired into `EmuCore` and validated
end-to-end through the boot path. Establishes the **HLE NID-dispatch pattern**
(dep + state provider + `match nid` arm) the rest of the wave reuses.

## Behaviour

A PPU-only homebrew that calls `cellSysutilGetSystemParamInt(ID_LANG, &lang)`
(via PSL1GHT's `sysUtilGetSystemParamInt`) and returns the value:
- `ret != 0` → returns `0x0BAD`
- `ret == 0` → returns `lang` (a `-12345` sentinel pre-wire; the real param post-wire)

No printf / SPU / RSX — a clean PPU + cellSysutil HLE call.

## How it wires (the pattern)

1. The guest call fires NID **`0x40e895d3`** (`cellSysutil`, confirmed at runtime
   from the `[R9.1g.7] unimplemented import` log; r3=0x111=ID_LANG, r4=&lang).
2. `rpcs3-emu-core` now depends on `rpcs3-hle-cellsysutil` and its `match nid`
   dispatcher routes `0x40e895d3` to
   `rpcs3_hle_cellsysutil::cell_sysutil_get_system_param_int(&EmuSysutilConfig, id)`.
3. `EmuSysutilConfig` (a fixed `SysutilState` provider in emu-core) returns
   RPCS3-faithful defaults — `Lang = 1` (CELL_SYSUTIL_LANG_ENGLISH_US).
4. The arm writes the value (BE) to the OUT pointer and returns CELL_OK.

## Result

`EmuCore::run_self` exit status = **1** (the LANG value the homebrew read back),
vs the `-12345` sentinel before wiring — proving the HLE crate actually ran.

## Consumed by

`rust/rpcs3-emu-core/tests/hle_sysutil_param.rs`. The `.self`/`.elf` are built
locally via the PSL1GHT Docker toolchain and gitignored.

CC0 1.0 (public domain) — see LICENSE.md.
