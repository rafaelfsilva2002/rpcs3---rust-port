// single_fs_write_v1 — lv2 fs write round-trip HLE fixture (VFS slice 4).
// CC0 1.0 (public domain). See LICENSE.md.
//
// Exercises O_CREAT|O_WRONLY open + sys_fs_write (#803) + read-back against the
// in-memory VFS. The host pre-creates the parent dir "/dev_hdd0" (via
// vfs_add_dir) so the O_CREAT of a fresh file succeeds; the file itself is NOT
// pre-seeded — it is created and written by this homebrew, then re-read.
//
// Open-flags are the REAL octal lv2 ABI: O_RDONLY=0, O_WRONLY=01, O_CREAT=0100.
//   create+write fails  -> 0xBAD0/0xBAD1/0xBAD2
//   reopen+read fails    -> 0xBAD3/0xBAD4/0xBAD5
//   bytes differ         -> 0xBAD6
//   round-trip OK        -> 0xC0DE
//
// Pre-wire (write unrouted) the round-trip can't match -> 0xBADn.
// Behaviour: rsx-free, SPU-free, pure lv2 fs syscalls.

#include <ppu-types.h>
#include <sys/file.h>
#include <sys/process.h>

SYS_PROCESS_PARAM(1001, 0x10000);

int main(void)
{
    unsigned char w[8];
    for (int i = 0; i < 8; i++) {
        w[i] = (unsigned char)(0x11 * (i + 1)); // 0x11,0x22,...,0x88
    }

    s32 fd = -1;
    // 0101 octal = O_CREAT(0100) | O_WRONLY(01); mode 0666.
    if (sysLv2FsOpen("/dev_hdd0/w.bin", 0101, &fd, 0666, 0, 0) != 0) {
        return 0xBAD0;
    }
    u64 nwritten = 0;
    if (sysLv2FsWrite(fd, w, sizeof(w), &nwritten) != 0) {
        return 0xBAD1;
    }
    sysLv2FsClose(fd);
    if (nwritten != sizeof(w)) {
        return 0xBAD2;
    }

    s32 fd2 = -1;
    if (sysLv2FsOpen("/dev_hdd0/w.bin", 0 /* O_RDONLY */, &fd2, 0, 0, 0) != 0) {
        return 0xBAD3;
    }
    unsigned char r[8];
    for (int i = 0; i < 8; i++) {
        r[i] = 0;
    }
    u64 nread = 0;
    if (sysLv2FsRead(fd2, r, sizeof(r), &nread) != 0) {
        return 0xBAD4;
    }
    sysLv2FsClose(fd2);
    if (nread != sizeof(r)) {
        return 0xBAD5;
    }

    for (int i = 0; i < 8; i++) {
        if (r[i] != w[i]) {
            return 0xBAD6;
        }
    }
    return 0xC0DE;
}
