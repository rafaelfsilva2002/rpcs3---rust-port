# single_fs_read_v1 (VFS — first lv2 filesystem oracle)

The first behavior-freeze oracle for the in-memory VFS: a PSL1GHT homebrew opens
and reads a pre-seeded file via the **raw lv2 fs syscalls** (these are numbered
`sc`s — sys_fs_open #801 / read #802 / close #804 — NOT cell* PRX imports).

## Behaviour

```c
s32 fd; u64 nread; unsigned char buf[16];
if (sysFsOpen("/dev_hdd0/test.bin", SYS_O_RDONLY, &fd, 0, 0) != 0) return 0xBAD0;
if (sysFsRead(fd, buf, 16, &nread) != 0)                          return 0xBAD1;
sysFsClose(fd);
if (nread != 16)                                                  return 0xBAD2;
sum = Σ buf[i];                       // content 0x01..0x10 -> 136 = 0x88
return (sum == 0x88) ? 0xC0DE : 0xBAD3;
```

No printf / SPU / RSX — pure lv2 fs syscalls.

## How it wires

1. The host test pre-seeds the VFS BEFORE `run_self`:
   `core.vfs_add_file("/dev_hdd0/test.bin", vec![1..=16])` — a deterministic
   stand-in for on-disk content (the key MUST equal the guest path byte-for-byte).
2. `sysFsOpen` issues sys_fs_open (#801) → emu-core routes it to
   `rpcs3_lv2_fs::sys_fs_open(&mut self.vfs, &mut self.fd_table, path, flags)`,
   writes the allocated fd (BE u32) to `*fd`. `sysFsRead` (#802) reads into the
   guest buffer + writes the byte count (BE u64) to `*nread`. `sysFsClose` (#804)
   drops the fd. The MemVfs (src/vfs.rs) backs it all in RAM.

## Result

`EmuCore::run_self` exit status = **0xC0DE** (read the seeded 16 bytes, sum 0x88),
vs **0xBAD0** pre-wire (open unrouted) / negative control (no pre-seed → ENOENT).

## Consumed by

`rust/rpcs3-emu-core/tests/hle_fs_read.rs`. The `.self`/`.elf` are built locally
via the PSL1GHT Docker toolchain and gitignored.

CC0 1.0 (public domain) — see LICENSE.md.
