// single_netctl_mtu_v1 — cellNetCtl get-info (MTU) HLE fixture.
// CC0 1.0 (public domain). See LICENSE.md.
//
// Calls cellNetCtlInit() then cellNetCtlGetInfo(NET_CTL_INFO_MTU, &info) via
// PSL1GHT's libnetctl. Reuses the cellNetCtl NID-dispatch wiring (NetCtlManager
// field + connected backend) added for single_netctl_state_v1 — only the
// GetInfo NID is new. Pre-wire the return-0 import stub never writes the union,
// so info.mtu stays 0; once routed, the crate returns NetInfo::Mtu(1500) and the
// arm writes 1500 (BE u32) into the union (mtu is at offset 0).
//
//   init != 0 || getinfo != 0 -> return 0x0BAD    (call error)
//   else                      -> return info.mtu   (0 pre-wire; 1500 post-wire)
//
// Behaviour: rsx-free, SPU-free, pure cellNetCtl HLE calls.

#include <ppu-types.h>
#include <net/netctl.h>
#include <sys/process.h>
#include <string.h>

SYS_PROCESS_PARAM(1001, 0x10000);

int main(void)
{
    s32 ret = netCtlInit();

    union net_ctl_info info;
    memset(&info, 0, sizeof(info)); // sentinel: the return-0 stub never writes

    s32 r = netCtlGetInfo(NET_CTL_INFO_MTU, &info);

    if (ret != 0 || r != 0) {
        return 0x0BAD;
    }
    return (s32)info.mtu; // 0 pre-wire (union untouched); 1500 post-wire
}
