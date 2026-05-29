# single_msgdialog_callback_v1 (guest-PPU callback #2 — cellMsgDialog)

The second consumer of `EmuCore::call_guest_function` (after cellSysutil),
demonstrating the callback unlock on a different, iconic API: `cellMsgDialog`.

## Behaviour

```c
static volatile s32 g_button = -999;
static void on_dialog(msgButton button, void *usrData) { g_button = (s32)button; }

int main(void) {
    if (msgDialogOpen2(MSG_DIALOG_BTN_TYPE_OK, "behavior-freeze",
                       on_dialog, (void*)0xD1A106, 0) != 0) return 0xBAD1;
    return (g_button == MSG_DIALOG_BTN_OK) ? 0x600D : 0xBAD0;
}
```

No printf / SPU / RSX — pure cellMsgDialog + one guest callback.

## How it wires

1. `cellMsgDialogOpen2` (NID captured at runtime; r3=type, r4=msg, r5=callback FD,
   r6=userData) opens the dialog in EmuCore's `DialogManager`.
2. There is no user to press a button, so emu-core **headless-auto-confirms**:
   it immediately closes the dialog (the crate computes the default button per
   type — `BUTTON_TYPE_OK` → `BTN_OK` = 1) and invokes the guest callback via
   `EmuCore::call_guest_function(cb_fd, [button, userData])`. The callback's TOC
   is taken from its OPD descriptor (FD+4), as for all guest callbacks.
3. The guest `on_dialog` records the button; `main` returns `0x600D` iff it ran
   with `OK`.

This is a deliberate headless policy (auto-confirm with the default button); the
real async-on-dismiss flow can be refined later if a title needs it.

## Result

`EmuCore::run_self` exit status = **0x600D** (callback ran with OK), vs **0xBAD0**
pre-wire (Open2 unrouted → callback never invoked).

## Consumed by

`rust/rpcs3-emu-core/tests/hle_msgdialog_callback.rs`. The `.self`/`.elf` are
built locally via the PSL1GHT Docker toolchain and gitignored.

CC0 1.0 (public domain) — see LICENSE.md.
