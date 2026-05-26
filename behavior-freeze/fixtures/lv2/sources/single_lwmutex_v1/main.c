// single_lwmutex_v1 — PPU-only lwmutex round-trip
// CC0 1.0 (public domain). See LICENSE.md.
//
// Minimal end-to-end exercise of the PSL1GHT lwmutex syscalls:
//   create → lock → unlock → destroy → return canonical status.
//
// No SPU code, no DMA, no printf (PSL1GHT TTY emit is gated by the
// newlib _reent linkage we don't have — R9 closure deferral). Status
// is communicated via the process exit code:
//
//   0xC0DE — full round-trip success (the canonical happy-path
//            sentinel the Rust smoke test asserts on).
//   1..=4  — step-specific failure codes; preserved in r3 so the
//            smoke test can distinguish which syscall faulted.
//
// PSL1GHT calls under the hood (sys/lwmutex.h):
//   sysLwMutexCreate   → NID 0x2f85c0ef  _sys_lwmutex_create
//   sysLwMutexLock     → NID 0x1573dc3f  _sys_lwmutex_lock
//   sysLwMutexUnlock   → NID 0x1bc200f4  _sys_lwmutex_unlock
//   sysLwMutexDestroy  → NID 0xc3476d0c  _sys_lwmutex_destroy
//
// All four NIDs are wired in rpcs3-emu-core::EmuCore (R10.1.b for
// create/lock/unlock; destroy still falls into the permissive
// catch-all and returns CELL_OK, which is fine for this fixture
// because Lv2SyncState retains the entry until process exit).

#include <ppu-types.h>
#include <sys/lwmutex.h>
#include <sys/process.h>

SYS_PROCESS_PARAM(1001, 0x10000);

int main(s32 argc, const char *argv[])
{
    (void)argc;
    (void)argv;

    sys_lwmutex_t lw;
    sys_lwmutex_attribute_t attr;
    sysLwMutexAttributeInitialize(attr);
    sysLwMutexAttributeName(attr, "lwm1");

    s32 ret;

    ret = sysLwMutexCreate(&lw, &attr);
    if (ret) {
        return 1;
    }

    // Infinite timeout — single-threaded program; the lock is
    // guaranteed free.
    ret = sysLwMutexLock(&lw, 0);
    if (ret) {
        return 2;
    }

    ret = sysLwMutexUnlock(&lw);
    if (ret) {
        return 3;
    }

    ret = sysLwMutexDestroy(&lw);
    if (ret) {
        return 4;
    }

    return 0xC0DE;
}
