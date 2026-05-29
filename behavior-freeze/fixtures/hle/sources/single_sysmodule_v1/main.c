// single_sysmodule_v1 — cellSysModule HLE-crate integration fixture.
// CC0 1.0 (public domain). See LICENSE.md.
//
// Drives the cellSysModule NID imports (load / is-loaded) into emu-core via
// PSL1GHT's sysModuleLoad / sysModuleIsLoaded wrappers. Pre-wire, the permissive
// return-0 stub answers every import with r3=0, so "is loaded?" wrongly reports
// loaded even before the load call. Once emu-core routes the NIDs to
// rpcs3-hle-cellsysmodule (backed by a SysmoduleManager state field on EmuCore),
// the module-load lifecycle becomes observable.
//
// The exit code packs three observations so the EmuCore test can assert them:
//   bit0 (0x1): module reported NOT loaded *before* the load call (real impl)
//   bit1 (0x2): the load call returned CELL_OK
//   bit2 (0x4): module reported loaded *after* the load call
// => pre-wire stub (all-zero returns): 0x2 | 0x4         = 0x6
// => post-wire real lifecycle:         0x1 | 0x2 | 0x4   = 0x7
//
// Behaviour: rsx-free, SPU-free, pure cellSysModule HLE calls.

#include <ppu-types.h>
#include <sysmodule/sysmodule.h>
#include <sys/process.h>

SYS_PROCESS_PARAM(1001, 0x10000);

int main(void)
{
    const sysModuleId mod = SYSMODULE_GCM_SYS; // 0x10 — a real, loadable module

    s32 before = sysModuleIsLoaded(mod); // real impl: UNLOADED (nonzero)
    s32 load   = sysModuleLoad(mod);     // CELL_OK on success
    s32 after  = sysModuleIsLoaded(mod); // real impl: LOADED (0)

    s32 code = 0;
    if (before != 0) code |= 0x1; // not loaded before
    if (load == 0)   code |= 0x2; // load succeeded
    if (after == 0)  code |= 0x4; // loaded after
    return code;
}
