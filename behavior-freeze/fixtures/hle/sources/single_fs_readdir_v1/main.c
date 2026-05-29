// single_fs_readdir_v1 — lv2 fs directory enumeration HLE fixture (VFS slice 3).
// CC0 1.0 (public domain). See LICENSE.md.
//
// Exercises sys_fs_opendir (#805), sys_fs_readdir (#806), sys_fs_closedir (#807)
// against the in-memory VFS. The host pre-seeds three files under
// "/dev_hdd0/d/" (a.bin, b.bin, c.bin) before run_self; opendir+readdir should
// enumerate exactly 3 regular entries.
//
//   opendir != 0                    -> return 0xBAD0  (dir absent)
//   readdir != 0                    -> return 0xBAD1
//   total == 3 && regular == 3       -> return 0xC0DE  (all three files seen)
//   otherwise                       -> return 0xBAD3
//
// Pre-wire (syscalls unrouted, permissive no-op) readdir reports EOF immediately
// -> total 0 -> 0xBAD3. Behaviour: rsx-free, SPU-free, pure lv2 fs syscalls.

#include <ppu-types.h>
#include <sys/file.h>
#include <sys/process.h>

SYS_PROCESS_PARAM(1001, 0x10000);

#define CELL_FS_TYPE_REGULAR 2 /* real lv2 ABI: directory=1, regular=2 */

int main(void)
{
    s32 fd = -1;
    if (sysLv2FsOpenDir("/dev_hdd0/d", &fd) != 0) {
        return 0xBAD0;
    }

    int total = 0;
    int regular = 0;
    for (;;) {
        sysFSDirent ent;
        u64 nread = 0;
        if (sysLv2FsReadDir(fd, &ent, &nread) != 0) {
            return 0xBAD1;
        }
        if (nread == 0) {
            break; // EOF
        }
        total++;
        if (ent.d_type == CELL_FS_TYPE_REGULAR) {
            regular++;
        }
        if (total > 64) {
            return 0xBAD2; // runaway guard
        }
    }
    sysLv2FsCloseDir(fd);

    return (total == 3 && regular == 3) ? 0xC0DE : 0xBAD3;
}
