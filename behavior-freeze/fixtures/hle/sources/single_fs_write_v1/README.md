# single_fs_write_v1 (VFS slice 4 — write round-trip + O_CREAT)

Exercises `sys_fs_write` (#803, fd>=4 branch) plus the REAL-octal open-flag
decode for `O_CREAT | O_WRONLY`.

## Behaviour

The host pre-creates the parent dir `/dev_hdd0` (`vfs_add_dir`); the file is NOT
pre-seeded — it is created and written by the homebrew, then re-read:

```c
sysLv2FsOpen("/dev_hdd0/w.bin", 0101 /*O_CREAT|O_WRONLY*/, &fd, 0666, 0, 0);
sysLv2FsWrite(fd, w, 8, &nwritten);   // w = 0x11,0x22,..,0x88
sysLv2FsClose(fd);
sysLv2FsOpen("/dev_hdd0/w.bin", 0 /*O_RDONLY*/, &fd2, 0, 0, 0);
sysLv2FsRead(fd2, r, 8, &nread); sysLv2FsClose(fd2);
return (r == w) ? 0xC0DE : 0xBADn;
```

No printf / SPU / RSX — pure lv2 fs syscalls.

## How it wires

- The #801 open arm now translates the guest's **REAL octal** oflags
  (`O_CREAT=0o100`, `O_TRUNC=0o1000`, `O_APPEND=0o2000`) into the lv2-fs flag
  space (whose frozen bit values are POSIX-style, not the octal ABI) via
  `translate_fs_oflags` — RDONLY (0) is unchanged, so prior slices are unaffected.
- The #803 write arm branches `fd >= 4` → `rpcs3_lv2_fs::sys_fs_write`, writing
  the bytes-written count BE-u64 to `*pwritten`; fd 1/2 keep the TTY path.

## Result

`EmuCore::run_self` exit status = **0xC0DE** (write 8 bytes, read them back
identical), vs **0xBADn** pre-wire (write unrouted, round-trip mismatches).

## Consumed by

`rust/rpcs3-emu-core/tests/hle_fs_write.rs`. The `.self`/`.elf` are built locally
via the PSL1GHT Docker toolchain and gitignored.

CC0 1.0 (public domain) — see LICENSE.md.
