// single_fs_stat_v1 — lv2 fs stat / fstat / lseek HLE fixture (VFS slice 2).
// CC0 1.0 (public domain). See LICENSE.md.
//
// Exercises sys_fs_stat (#808), sys_fs_fstat (#809), and sys_fs_lseek (#818)
// against the in-memory VFS (atop the slice-1 open/read/close). The host
// pre-seeds "/dev_hdd0/test.bin" = bytes 0x01..0x10 (16 bytes) before run_self.
//
//   stat.st_size  == 16             (path-based stat)
//   fstat.st_size == 16             (fd-based stat — fd>=4 resolves to the path)
//   lseek SET 8   -> pos == 8       (seek)
//   read 8 bytes from offset 8      -> bytes 0x09..0x10, sum = 100 = 0x64
//   all hold -> return 0xC0DE ; any mismatch -> 0xBADn
//
// Pre-wire (syscalls unrouted, permissive no-op) stat leaves st unset -> 0xBAD1.
// Behaviour: rsx-free, SPU-free, pure lv2 fs syscalls.

#include <ppu-types.h>
#include <sys/file.h>
#include <sys/process.h>

SYS_PROCESS_PARAM(1001, 0x10000);

int main(void)
{
    sysFSStat st;
    if (sysLv2FsStat("/dev_hdd0/test.bin", &st) != 0) {
        return 0xBAD0;
    }
    if (st.st_size != 16) {
        return 0xBAD1;
    }

    s32 fd = -1;
    if (sysLv2FsOpen("/dev_hdd0/test.bin", 0, &fd, 0, 0, 0) != 0) {
        return 0xBAD2;
    }

    sysFSStat fst;
    if (sysLv2FsFStat(fd, &fst) != 0) {
        return 0xBAD3;
    }
    if (fst.st_size != 16) {
        return 0xBAD4;
    }

    u64 pos = 0;
    if (sysLv2FsLSeek64(fd, 8, 0 /* SEEK_SET */, &pos) != 0) {
        return 0xBAD5;
    }
    if (pos != 8) {
        return 0xBAD6;
    }

    u64 nread = 0;
    unsigned char buf[8];
    if (sysLv2FsRead(fd, buf, sizeof(buf), &nread) != 0) {
        return 0xBAD7;
    }
    sysLv2FsClose(fd);
    if (nread != sizeof(buf)) {
        return 0xBAD8;
    }

    unsigned int sum = 0;
    for (unsigned i = 0; i < sizeof(buf); i++) {
        sum += buf[i]; // bytes 0x09..0x10 -> 100 = 0x64
    }

    return (st.st_size == 16 && fst.st_size == 16 && sum == 0x64) ? 0xC0DE : 0xBAD9;
}
