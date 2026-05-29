# single_sysutil_callback_v1 (guest-PPU callback — first re-entrant oracle)

The first behavior-freeze fixture where **emu-core calls back into guest PPU
code**. It validates `EmuCore::call_guest_function` end-to-end against real
PSL1GHT bytes via the cellSysutil callback dispatch.

## Behaviour

```c
static volatile u32 g_observed = 0;
static void my_cb(u64 status, u64 param, void *usrdata) { g_observed = (u32)status; }

int main(void) {
    if (sysUtilRegisterCallback(0, my_cb, (void*)0xABCD1234) != 0) return 0xBAD1;
    sysUtilCheckCallback();                 // host pre-seeded event 0x0101
    return (g_observed == 0x0101) ? 0x600D : 0xBAD0;
}
```

No printf / SPU / RSX — pure cellSysutil + one guest callback.

## How it wires

1. `cellSysutilRegisterCallback` (NID captured at runtime) stores
   `{fn_addr = &my_cb's FD, user_data = 0xABCD1234}` in slot 0 of EmuCore's
   `sysutil_callbacks` table.
2. The **host test pre-seeds** one pending event: `core.sysutil_queue.push(0x0101,
   0)` before `run_self` — a deterministic stand-in for the system event source
   (no auto-injection, so real-game behaviour is unaffected).
3. `cellSysutilCheckCallback` (NID captured at runtime) drains the queue and, per
   pending dispatch, calls `EmuCore::call_guest_function(my_cb_fd, [status=0x0101,
   param=0, userdata=0xABCD1234])`. That snapshots the PPU frame, runs `my_cb`
   (which stores `0x0101` to `g_observed`), then restores the frame so
   CheckCallback resumes and `main` reaches its `return`.

## Result

`EmuCore::run_self` exit status = **0x600D** (callback ran with status 0x0101),
vs **0xBAD0** pre-wire (callback never invoked — the proof it was the re-entrant
call, not luck, that produced success). `g_observed` in guest memory reads
`0x0101` after the run.

## Consumed by

`rust/rpcs3-emu-core/tests/hle_sysutil_callback.rs`. The `.self`/`.elf` are built
locally via the PSL1GHT Docker toolchain and gitignored.

CC0 1.0 (public domain) — see LICENSE.md.
