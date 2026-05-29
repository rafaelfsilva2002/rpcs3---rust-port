// single_netctl_state_v1 — cellNetCtl init + get-state HLE fixture.
// CC0 1.0 (public domain). See LICENSE.md.
//
// Calls cellNetCtlInit() then cellNetCtlGetState(&state) via PSL1GHT's libnetctl.
// Pre-wire both fire the permissive return-0 import stub: init returns 0 and
// get-state returns 0 WITHOUT writing the OUT pointer, so `state` keeps its
// sentinel. Once emu-core routes the NIDs to rpcs3-hle-cellnetctl backed by a
// StubConnectedBackend, the runtime reports a connected network and writes
// CELL_NET_CTL_STATE_IPOBTAINED (3) into the OUT pointer.
//
//   init != 0 || getstate != 0 -> return 0x0BAD     (call error)
//   else                       -> return state       (0x55 sentinel pre-wire; 3 post-wire)
//
// Behaviour: rsx-free, SPU-free, pure cellNetCtl HLE calls.

#include <ppu-types.h>
#include <net/netctl.h>
#include <sys/process.h>

SYS_PROCESS_PARAM(1001, 0x10000);

int main(void)
{
    s32 ret = netCtlInit();

    s32 state = 0x55; // sentinel: the return-0 stub never writes this
    s32 r = netCtlGetState(&state);

    if (ret != 0 || r != 0) {
        return 0x0BAD;
    }
    return state; // 0x55 pre-wire (OUT untouched); 3 (IPOBTAINED) post-wire
}
