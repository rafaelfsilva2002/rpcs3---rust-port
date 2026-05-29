// single_net_inet_addr_v1 — sys_net inet_addr HLE fixture.
// CC0 1.0 (public domain). See LICENSE.md.
//
// Calls inet_addr("1.2.3.4") via PSL1GHT's libnet (arpa/inet.h). On real PS3
// firmware sys_net_inet_addr is a STUB that unconditionally returns
// INET_ADDR_NONE (0xFFFFFFFF) — rpcs3-hle-sys-net-user::inet_addr_stub mirrors
// this byte-exact (cpp:65 `return 0xffffffff`). Pre-wire, the permissive
// return-0 import stub returns r3=0; once emu-core routes the NID, r3=0xFFFFFFFF.
//
//   inet_addr == 0xFFFFFFFF -> return 1   (post-wire firmware-stub behaviour)
//   else                    -> return 0   (pre-wire return-0 stub)
//
// Behaviour: rsx-free, SPU-free, pure sys_net HLE call.

#include <ppu-types.h>
#include <arpa/inet.h>
#include <sys/process.h>

SYS_PROCESS_PARAM(1001, 0x10000);

int main(void)
{
    in_addr_t a = inet_addr("1.2.3.4");
    return (a == 0xFFFFFFFFu) ? 1 : 0;
}
