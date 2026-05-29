// single_sysutil_string_v1 — cellSysutil string-param HLE fixture.
// CC0 1.0 (public domain). See LICENSE.md.
//
// Calls cellSysutilGetSystemParamString(ID_NICKNAME, buf, sizeof buf) via
// PSL1GHT's sysUtilGetSystemParamString. Pre-wire, the permissive return-0 stub
// answers the import with r3=0 but never writes `buf`, so the zero-initialised
// buffer stays empty. Once emu-core routes the NID to
// rpcs3-hle-cellsysutil::cell_sysutil_get_system_param_string (backed by
// EmuSysutilConfig, which now returns a default nickname), the homebrew reads the
// real string.
//
// The return value carries a byte-sum of the buffer so the EmuCore test can
// assert both that the string was written AND its content via the exit code:
//   ret != 0          -> return 0x0BAD                  (call failed)
//   ret == 0, sum(s)  -> return the byte-sum            (0 pre-wire; 363 = "RPCS3" post-wire)
//
// Behaviour: rsx-free, SPU-free, pure cellSysutil HLE call.

#include <ppu-types.h>
#include <sysutil/sysutil.h>
#include <sys/process.h>
#include <string.h>

SYS_PROCESS_PARAM(1001, 0x10000);

int main(void)
{
    char buf[64];
    memset(buf, 0, sizeof(buf)); // sentinel: the return-0 stub leaves this empty

    s32 ret = sysUtilGetSystemParamString(
        SYSUTIL_SYSTEMPARAM_ID_NICKNAME, buf, (u32)sizeof(buf));
    if (ret != 0) {
        return 0x0BAD;
    }

    int sum = 0;
    for (int i = 0; buf[i] != '\0' && i < (int)sizeof(buf); i++) {
        sum += (unsigned char)buf[i];
    }
    return sum; // 0 pre-wire (buf empty); 363 ("RPCS3") once wired
}
