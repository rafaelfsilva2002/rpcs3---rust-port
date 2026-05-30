// single_kb_info_v1 — cellKb GetInfo (deterministic headless).
// CC0 1.0 (public domain). See LICENSE.md.
//
// ioKbInit(127) -> ioKbGetInfo. Headless emu-core has no host keyboard handler,
// so connected=0; max=127 (CELL_KB_MAX_KEYBOARDS). Deterministic.
//   init fails -> 0xBAD0 ; getinfo fails -> 0xBAD1 ; max!=127 -> 0xBAD2 ;
//   connected!=0 -> 0xBAD3 ; all correct -> 0xC0DE

#include <ppu-types.h>
#include <io/kb.h>
#include <sys/process.h>
#include <string.h>

SYS_PROCESS_PARAM(1001, 0x10000);

int main(void)
{
    if (ioKbInit(127) != 0) {
        return 0xBAD0;
    }
    KbInfo info;
    memset(&info, 0xFF, sizeof(info)); // poison
    if (ioKbGetInfo(&info) != 0) {
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
