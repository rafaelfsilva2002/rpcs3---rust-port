# single_sysmodule_v1 (HLE wave — cellSysModule integration)

Second HLE-crate wired into `EmuCore` (after `cellSysutil`, R13.6). Establishes
the **stateful** HLE pattern: a `SysmoduleManager` field lives on `EmuCore` and
survives across guest calls, so the module load lifecycle is observable.

## Behaviour

A PPU-only homebrew that exercises the cellSysModule load lifecycle via PSL1GHT:

```c
before = sysModuleIsLoaded(SYSMODULE_GCM_SYS); // NOT loaded yet -> UNLOADED
load   = sysModuleLoad(SYSMODULE_GCM_SYS);     // CELL_OK
after  = sysModuleIsLoaded(SYSMODULE_GCM_SYS); // LOADED -> CELL_OK
```

No printf / SPU / RSX — pure cellSysModule HLE calls.

## Exit-code encoding

The return value packs three observations so the EmuCore test can assert them:

| bit | meaning |
| --- | --- |
| `0x1` | module reported **not loaded** before the load call (real impl) |
| `0x2` | the load call returned `CELL_OK` |
| `0x4` | module reported **loaded** after the load call |

- **Pre-wire** (permissive return-0 stub answers every import with `r3=0`):
  `before=0`, `load=0`, `after=0` → `0x2 | 0x4` = **`0x6`** (the "is loaded?"
  before the load wrongly reads loaded).
- **Post-wire** (real `rpcs3-hle-cellsysmodule` + `SysmoduleManager`):
  `before=UNLOADED`, `load=CELL_OK`, `after=LOADED` → `0x1 | 0x2 | 0x4` = **`0x7`**.

## How it wires (the stateful pattern)

1. The guest calls fire the cellSysModule NIDs (load / is-loaded), captured at
   runtime from the `[R9.1g.7] unimplemented import` log.
2. `rpcs3-emu-core` depends on `rpcs3-hle-cellsysmodule`; `EmuCore` gains a
   `sysmodule: SysmoduleManager` field (init in `new()`), mirroring
   `lv2_sync_state`.
3. The `match nid` dispatcher routes load → `cell_sysmodule_load_module`,
   is-loaded → `cell_sysmodule_is_loaded` against that field, returns the result
   in `r3`.

## Consumed by

`rust/rpcs3-emu-core/tests/hle_sysmodule.rs`. The `.self`/`.elf` are built
locally via the PSL1GHT Docker toolchain and gitignored.

CC0 1.0 (public domain) — see LICENSE.md.
