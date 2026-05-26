// single_cond_v1 — PPU-only sys_cond create/signal/broadcast/destroy
// CC0 1.0 (public domain). See LICENSE.md.
//
// Exercises the LV2 sys_cond_* syscall family that is reachable on a
// single PPU. `sysCondWait` (#107) is deliberately NOT called: it
// atomically releases the mutex and parks until another thread
// signals — on a single-PPU EmuCore there is no second thread, so a
// wait would block forever. This fixture validates the
// create / signal-empty / broadcast-empty / destroy arms (R10.3
// CondRegistry).
//
// Behaviour:
//   mutexCreate → condCreate(cond, mutex) → condSignal (no waiters,
//   no-op) → condBroadcast (no waiters, no-op) → condDestroy →
//   mutexDestroy → return 0xC0DE.
//
// No SPU, no DMA, no printf. Status via process exit code (r3).
//
// LV2 syscalls touched:
//   #100 sys_mutex_create   (R10.1.d, reused)
//   #101 sys_mutex_destroy  (R10.1.d, reused)
//   #105 sys_cond_create
//   #106 sys_cond_destroy
//   #108 sys_cond_signal
//   #109 sys_cond_signal_all (broadcast)

#include <ppu-types.h>
#include <sys/mutex.h>
#include <sys/cond.h>
#include <sys/process.h>

SYS_PROCESS_PARAM(1001, 0x10000);

int main(s32 argc, const char *argv[])
{
    (void)argc;
    (void)argv;

    s32 ret;

    sys_mutex_t mutex;
    sys_mutex_attr_t mattr;
    sysMutexAttrInitialize(mattr);
    ret = sysMutexCreate(&mutex, &mattr);
    if (ret) {
        return 1;
    }

    sys_cond_t cond;
    sys_cond_attr_t cattr;
    sysCondAttrInitialize(cattr);
    ret = sysCondCreate(&cond, mutex, &cattr);
    if (ret) {
        return 2;
    }

    // Signal with no waiters → no-op, returns CELL_OK.
    ret = sysCondSignal(cond);
    if (ret) {
        return 3;
    }

    // Broadcast with no waiters → no-op, returns CELL_OK.
    ret = sysCondBroadcast(cond);
    if (ret) {
        return 4;
    }

    ret = sysCondDestroy(cond);
    if (ret) {
        return 5;
    }

    ret = sysMutexDestroy(mutex);
    if (ret) {
        return 6;
    }

    return 0xC0DE;
}
