// single_sema_v1 — PPU-only kernel sys_semaphore round-trip
// CC0 1.0 (public domain). See LICENSE.md.
//
// Minimal end-to-end exercise of the LV2 sys_semaphore_* syscalls.
// Targets R10.4 (SyncTable sema impl) via the syscall arms wired
// by this same slice.
//
// Behaviour:
//   create(initial=1, max=10) → wait → post(1) → trywait →
//   get_value (expect 0) → post(2) → get_value (expect 2) →
//   destroy → return 0xC0DE.
//
// Single-PPU: the first `wait` succeeds without blocking because
// initial=1, then we keep value > 0 for every subsequent wait via
// post() between operations. The fixture is deliberately
// non-contended.
//
// No SPU, no DMA, no printf (TTY emit is deferred from R9). Status
// communicated via the process exit code (r3).
//
// LV2 syscalls touched:
//   #90  sys_semaphore_create
//   #91  sys_semaphore_destroy
//   #92  sys_semaphore_wait
//   #93  sys_semaphore_trywait
//   #94  sys_semaphore_post
//   #114 sys_semaphore_get_value

#include <ppu-types.h>
#include <sys/sem.h>
#include <sys/process.h>

SYS_PROCESS_PARAM(1001, 0x10000);

int main(s32 argc, const char *argv[])
{
    (void)argc;
    (void)argv;

    sys_sem_t sem;
    sys_sem_attr_t attr;

    // PSL1GHT doesn't ship a sysSemAttrInitialize macro; zero the
    // struct and set the default priority protocol explicitly.
    attr.attr_protocol = SYS_SEM_ATTR_PROTOCOL;
    attr.attr_pshared = SYS_SEM_ATTR_PSHARED;
    attr.key = 0;
    attr.flags = 0;
    attr.pad = 0;
    for (int i = 0; i < 8; i++) attr.name[i] = 0;

    s32 ret;

    ret = sysSemCreate(&sem, &attr, 1, 10);
    if (ret) {
        return 1;
    }

    // value=1 → Acquired immediately (no parking needed).
    ret = sysSemWait(sem, 0);
    if (ret) {
        return 2;
    }

    // value back to 1.
    ret = sysSemPost(sem, 1);
    if (ret) {
        return 3;
    }

    // value back to 0.
    ret = sysSemTryWait(sem);
    if (ret) {
        return 4;
    }

    s32 val = -1;
    ret = sysSemGetValue(sem, &val);
    if (ret) {
        return 5;
    }
    if (val != 0) {
        return 6;
    }

    ret = sysSemPost(sem, 2);
    if (ret) {
        return 7;
    }

    ret = sysSemGetValue(sem, &val);
    if (ret) {
        return 8;
    }
    if (val != 2) {
        return 9;
    }

    ret = sysSemDestroy(sem);
    if (ret) {
        return 10;
    }

    return 0xC0DE;
}
