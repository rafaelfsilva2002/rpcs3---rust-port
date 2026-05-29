// single_sysutil_callback_v1 — guest-PPU callback HLE fixture.
// CC0 1.0 (public domain). See LICENSE.md.
//
// The first fixture that proves emu-core can CALL BACK INTO guest PPU code.
// Registers a sysutil callback, then calls cellSysutilCheckCallback. The host
// (emu-core test) pre-seeds one pending system event (status = 0x0101); on
// CheckCallback emu-core drains it and invokes `my_cb` via call_guest_function.
// `my_cb` writes the status it observed into a global; main reports success.
//
//   register != 0          -> return 0xBAD1   (RegisterCallback failed)
//   g_observed == 0x0101    -> return 0x600D   (callback ran with the right status)
//   otherwise               -> return 0xBAD0   (callback never ran / wrong value)
//
// Pre-wire (callback NIDs unrouted) the callback never runs -> 0xBAD0.
// Behaviour: rsx-free, SPU-free, pure cellSysutil HLE + one guest callback.

#include <ppu-types.h>
#include <sysutil/sysutil.h>
#include <sys/process.h>

SYS_PROCESS_PARAM(1001, 0x10000);

static volatile u32 g_observed = 0; // sentinel: stays 0 if the callback never runs

static void my_cb(u64 status, u64 param, void *usrdata)
{
    (void)param;
    (void)usrdata;
    g_observed = (u32)status; // record what the system passed us
}

int main(void)
{
    s32 r = sysUtilRegisterCallback(0, my_cb, (void *)0xABCD1234);
    if (r != 0) {
        return 0xBAD1;
    }

    // Host pre-seeded one event (status = 0x0101); this drains + dispatches it,
    // calling my_cb(0x0101, ...) through emu-core's call_guest_function.
    sysUtilCheckCallback();

    return (g_observed == 0x0101) ? 0x600D : 0xBAD0;
}
