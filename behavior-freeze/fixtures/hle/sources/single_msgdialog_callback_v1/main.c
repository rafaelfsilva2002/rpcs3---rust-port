// single_msgdialog_callback_v1 — cellMsgDialog guest-callback HLE fixture.
// CC0 1.0 (public domain). See LICENSE.md.
//
// Second consumer of EmuCore::call_guest_function (after cellSysutil). Opens an
// OK message dialog with a callback. With no user to dismiss it, emu-core
// headless-auto-confirms the dialog and invokes the guest callback with the
// default button (OK = 1). `on_dialog` records the button; main reports it.
//
//   open != 0               -> return 0xBAD1   (msgDialogOpen2 failed)
//   g_button == BTN_OK (1)   -> return 0x600D   (callback ran with OK)
//   otherwise               -> return 0xBAD0   (callback never ran / wrong button)
//
// Pre-wire (Open2 NID unrouted) the callback never runs -> 0xBAD0.
// Behaviour: rsx-free, SPU-free, pure cellMsgDialog HLE + one guest callback.

#include <ppu-types.h>
#include <sysutil/msg.h>
#include <sys/process.h>

SYS_PROCESS_PARAM(1001, 0x10000);

static volatile s32 g_button = -999; // sentinel: stays if the callback never runs

static void on_dialog(msgButton button, void *usrData)
{
    (void)usrData;
    g_button = (s32)button;
}

int main(void)
{
    s32 r = msgDialogOpen2(
        MSG_DIALOG_BTN_TYPE_OK, "behavior-freeze", on_dialog, (void *)0xD1A106, 0);
    if (r != 0) {
        return 0xBAD1;
    }

    // emu-core headless-auto-confirms synchronously, invoking on_dialog(OK, ...).
    return (g_button == MSG_DIALOG_BTN_OK) ? 0x600D : 0xBAD0;
}
