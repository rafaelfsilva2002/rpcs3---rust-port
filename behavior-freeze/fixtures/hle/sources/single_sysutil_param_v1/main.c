// single_sysutil_param_v1 — first HLE-crate integration fixture.
// CC0 1.0 (public domain). See LICENSE.md.
//
// Calls cellSysutilGetSystemParamInt(ID_LANG, &lang) via PSL1GHT's
// sysUtilGetSystemParamInt wrapper. This fires the cellSysutil NID import,
// which emu-core currently answers with the permissive return-0 stub (r3=0, no
// write) — so `lang` keeps its sentinel. Once emu-core routes that NID to
// rpcs3-hle-cellsysutil::cell_sysutil_get_system_param_int (backed by a fixed
// system-config provider), the homebrew sees the real LANG value.
//
// The return value carries the result so the EmuCore test can assert it via the
// process exit code (no printf — keep the newlib exit path clean):
//   ret != 0           -> return 0x0BAD              (call failed)
//   ret == 0, lang=X   -> return X                   (X = sentinel pre-wire, real param post-wire)
//
// Behaviour: rsx-free, SPU-free, pure cellSysutil HLE call.

#include <ppu-types.h>
#include <sysutil/sysutil.h>
#include <sys/process.h>

SYS_PROCESS_PARAM(1001, 0x10000);

int main(void)
{
    s32 lang = -12345; // sentinel: the return-0 stub leaves this unwritten
    s32 ret = sysUtilGetSystemParamInt(SYSUTIL_SYSTEMPARAM_ID_LANG, &lang);
    if (ret != 0) {
        return 0x0BAD;
    }
    return lang; // sentinel pre-wire; the real system LANG param once wired
}
