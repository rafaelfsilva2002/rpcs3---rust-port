// single_mouse_info_v1 — cellMouse GetInfo (deterministic headless).
// CC0 1.0 (public domain). See LICENSE.md.
//
// ioMouseInit(127) -> ioMouseGetInfo. Headless emu-core has no host mouse
// handler, so connected=0; max=127 (CELL_MAX_MICE). Deterministic.
//   init fails -> 0xBAD0 ; getinfo fails -> 0xBAD1 ; max!=127 -> 0xBAD2 ;
//   connected!=0 -> 0xBAD3 ; all correct -> 0xC0DE

#include <ppu-types.h>
#include <io/mouse.h>
#include <sys/process.h>
#include <string.h>

SYS_PROCESS_PARAM(1001, 0x10000);

int main(void)
{
    if (ioMouseInit(127) != 0) {
        return 0xBAD0;
    }
    mouseInfo info;
    memset(&info, 0xFF, sizeof(info)); // poison
    if (ioMouseGetInfo(&info) != 0) {
        return 0xBAD1;
    }
    if (info.max != 127) {
        return 0xBAD2;
    }
    if (info.connected != 0) {
        return 0xBAD3;
    }
    return 0xC0DE;
}
