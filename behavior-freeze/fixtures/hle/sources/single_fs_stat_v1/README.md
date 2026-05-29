# single_fs_stat_v1 (VFS slice 2 — stat / fstat / lseek)

Extends the in-memory VFS (slice 1: open/read/close) with `sys_fs_stat` (#808),
`sys_fs_fstat` (#809, fd-based), and `sys_fs_lseek` (#818).

## Behaviour

Against a pre-seeded `/dev_hdd0/test.bin` (bytes 0x01..0x10, 16 bytes):

```c
sysLv2FsStat(path, &st);    // st.st_size == 16          (path-based stat)
sysLv2FsOpen(path, ..&fd..);
sysLv2FsFStat(fd, &fst);    // fst.st_size == 16         (fd-based stat)
sysLv2FsLSeek64(fd, 8, 0, &pos); // pos == 8             (SEEK_SET)
sysLv2FsRead(fd, buf, 8, &nread); // bytes 0x09..0x10, sum = 100 = 0x64
return (all hold) ? 0xC0DE : 0xBADn;
```

No printf / SPU / RSX — pure lv2 fs syscalls.

## How it wires

- `#808 sys_fs_stat` → `rpcs3_lv2_fs::sys_fs_stat(&self.vfs, path)` → `CellFsStat`,
  hand-serialized to the 52-byte BE `sysFSStat` (mode@0, uid@4, gid@8, atime@12,
  mtime@20, ctime@28, size@36, blksize@44 — `__attribute__((packed))`).
- `#809 sys_fs_fstat` edits the prior stdio stub: fd >= 4 resolves fd → handle
  (`FdTable::file_handle`, added to rpcs3-lv2-fs) → path (`MemVfs::stat_handle`) →
  real stat; fd 0..3 keep the S_IFCHR char-device stat.
- `#818 sys_fs_lseek` → `sys_fs_lseek(.., fd, offset, whence)`; new position
  written BE-u64 to `*pos`.

## Result

`EmuCore::run_self` exit status = **0xC0DE** (stat + fstat sizes 16, lseek to 8,
read sum 0x64), vs **0xBAD0** pre-wire / negative control (no seed → ENOENT).

## Consumed by

`rust/rpcs3-emu-core/tests/hle_fs_stat.rs`. The `.self`/`.elf` are built locally
via the PSL1GHT Docker toolchain and gitignored.

CC0 1.0 (public domain) — see LICENSE.md.
