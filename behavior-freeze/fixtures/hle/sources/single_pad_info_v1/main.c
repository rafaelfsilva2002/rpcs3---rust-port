// single_pad_info_v1 — cellPad GetInfo2 (deterministic headless).
// CC0 1.0 (public domain). See LICENSE.md.
//
// cellPad Init -> GetInfo2. RPCS3's cellPadGetInfo2 (cellPad.cpp:911-equivalent)
// reports max_connect (the Init value) + now_connect (count of connected pads,
// from the host controller handler). emu-core is headless — no host pad handler,
// so every port is disconnected: max=7, connected=0. Deterministic.
//
//   ioPadInit fails        -> 0xBAD0
//   ioPadGetInfo2 fails    -> 0xBAD1
//   max != 7               -> 0xBAD2
//   connected != 0         -> 0xBAD3
//   all correct            -> 0xC0DE

#include <ppu-types.h>
#include <io/pad.h>
#include <sys/process.h>
#include <string.h>

SYS_PROCESS_PARAM(1001, 0x10000);

int main(void)
{
    if (ioPadInit(7) != 0) {
        return 0xBAD0;
    }

    padInfo2 info;
    memset(&info, 0xFF, sizeof(info)); // poison: the arm must overwrite max/connected
    if (ioPadGetInfo2(&info) != 0) {
        return 0xBAD1;
    }

    if (info.max != 7) {
        return 0xBAD2;
    }
    if (info.connected != 0) {
        return 0xBAD3;
    }

    return 0xC0DE;
}
