# single_fs_readdir_v1 (VFS slice 3 — directory enumeration)

Exercises `sys_fs_opendir` (#805), `sys_fs_readdir` (#806), `sys_fs_closedir`
(#807) against the in-memory VFS.

## Behaviour

The host pre-seeds three files under `/dev_hdd0/d/` (a.bin, b.bin, c.bin):

```c
sysLv2FsOpenDir("/dev_hdd0/d", &fd);
loop: sysLv2FsReadDir(fd, &ent, &nread);  // nread==0 => EOF
      total++; if (ent.d_type == CELL_FS_TYPE_REGULAR/*2*/) regular++;
sysLv2FsCloseDir(fd);
return (total == 3 && regular == 3) ? 0xC0DE : 0xBAD3;
```

No printf / SPU / RSX — pure lv2 fs syscalls.

## How it wires

- `#805 sys_fs_opendir` → `rpcs3_lv2_fs::sys_fs_opendir`; fd written BE to `*fd`.
- `#806 sys_fs_readdir` → on a hit writes the 258-byte `CellFsDirent`
  {d_type@0, d_namlen@1, d_name@2[256]} + `*nread = 258`; at EOF `*nread = 0`.
  **GOTCHA:** lv2-fs `FS_TYPE_*` is inverted vs the real ABI — the arm maps
  crate→real (regular→2, directory→1) when writing `d_type`.
- `#807 sys_fs_closedir` → drops the dir fd.

## Result

`EmuCore::run_self` exit status = **0xC0DE** (3 regular entries enumerated), vs
**0xBAD3** pre-wire (readdir reports EOF immediately) / **0xBAD0** negative
control (no seed → opendir ENOENT).

## Consumed by

`rust/rpcs3-emu-core/tests/hle_fs_readdir.rs`. The `.self`/`.elf` are built
locally via the PSL1GHT Docker toolchain and gitignored.

CC0 1.0 (public domain) — see LICENSE.md.
