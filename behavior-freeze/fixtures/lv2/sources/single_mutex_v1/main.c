// single_mutex_v1 — PPU-only kernel sys_mutex round-trip
// CC0 1.0 (public domain). See LICENSE.md.
//
// Minimal end-to-end exercise of the LV2 sys_mutex_* syscalls
// (kernel mutex, NOT lightweight mutex). PSL1GHT exposes these via
// <sys/mutex.h>; lwmutex is internal to PSL1GHT runtime + lib code
// and isn't called directly from homebrew, so this fixture targets
// the kernel mutex layer (R10.2 + the syscall arms wired by this
// same slice).
//
// Behaviour:
//   create → lock → unlock → trylock → unlock → destroy → return
//   0xC0DE on full success. Each step has its own failure code so
//   the Rust smoke test can pinpoint where it broke.
//
// No SPU, no DMA, no printf (TTY emit is deferred from R9). Status
// communicated via the process exit code (r3).
//
// LV2 syscalls touched:
//   #100 sys_mutex_create
//   #101 sys_mutex_destroy
//   #102 sys_mutex_lock
//   #103 sys_mutex_trylock
//   #104 sys_mutex_unlock

#include <ppu-types.h>
#include <sys/mutex.h>
#include <sys/process.h>

SYS_PROCESS_PARAM(1001, 0x10000);

int main(s32 argc, const char *argv[])
{
    (void)argc;
    (void)argv;

    sys_mutex_t mutex;
    sys_mutex_attr_t attr;
    sysMutexAttrInitialize(attr);

    s32 ret;

    ret = sysMutexCreate(&mutex, &attr);
    if (ret) {
        return 1;
    }

    // Non-contended lock (single thread).
    ret = sysMutexLock(mutex, 0);
    if (ret) {
        return 2;
    }

    ret = sysMutexUnlock(mutex);
    if (ret) {
        return 3;
    }

    // Trylock on a free mutex must succeed.
    ret = sysMutexTryLock(mutex);
    if (ret) {
        return 4;
    }

    ret = sysMutexUnlock(mutex);
    if (ret) {
        return 5;
    }

    ret = sysMutexDestroy(mutex);
    if (ret) {
        return 6;
    }

    return 0xC0DE;
}
