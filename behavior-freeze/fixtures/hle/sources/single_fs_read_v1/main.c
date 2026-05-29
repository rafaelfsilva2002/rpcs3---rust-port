// single_fs_read_v1 — lv2 filesystem (VFS) read HLE fixture.
// CC0 1.0 (public domain). See LICENSE.md.
//
// First consumer of the in-memory VFS. Opens a pre-seeded file via the raw lv2
// fs syscalls (sysFsOpen/Read/Close issue sys_fs_open #801 / read #802 /
// close #804 directly — NOT cell* PRX imports), reads 16 bytes, and returns a
// constant derived from their byte-sum. The host (emu-core test) pre-seeds
// "/dev_hdd0/test.bin" = bytes 0x01..0x10 (sum = 136 = 0x88) before run_self.
//
//   open  != 0          -> return 0xBAD0   (sys_fs_open failed / file absent)
//   read  != 0          -> return 0xBAD1
//   nread != 16         -> return 0xBAD2
//   sum == 0x88          -> return 0xC0DE   (read the seeded content)
//   otherwise           -> return 0xBAD3
//
// Pre-wire (syscalls unrouted, permissive no-op) open leaves fd unset -> 0xBAD0.
// Behaviour: rsx-free, SPU-free, pure lv2 fs syscalls.

#include <ppu-types.h>
#include <sys/file.h>
#include <sys/process.h>

SYS_PROCESS_PARAM(1001, 0x10000);

int main(void)
{
    s32 fd = -1;
    u64 nread = 0;
    unsigned char buf[16];

    // oflags=0 (SYS_O_RDONLY), mode=0, arg=NULL, argsize=0. sysLv2Fs* are
    // inline LV2_SYSCALL stubs (issue the lv2 `sc` directly; no lib dependency).
    if (sysLv2FsOpen("/dev_hdd0/test.bin", 0, &fd, 0, 0, 0) != 0) {
        return 0xBAD0;
    }
    if (sysLv2FsRead(fd, buf, sizeof(buf), &nread) != 0) {
        return 0xBAD1;
    }
    sysLv2FsClose(fd);
    if (nread != sizeof(buf)) {
        return 0xBAD2;
    }

    unsigned int sum = 0;
    for (unsigned i = 0; i < sizeof(buf); i++) {
        sum += buf[i];
    }
    return (sum == 0x88) ? 0xC0DE : 0xBAD3;
}
