# In-Memory VFS Implementation Blueprint (RPCS3 → Rust port)

Status: DESIGN (pre-implementation). Date: 2026-05-29.
Scope authority: this doc plans wiring `sys_fs_*` numbered syscalls into `EmuCore` against an in-memory backend. Canonical project state lives in `docs/PROJECT_STATUS.md`.

---

## 1. Goal and scope

**Goal.** Stand up a deterministic, in-memory VFS so PSL1GHT homebrew (and later real titles) can `sysFsOpen → sysFsRead → … → sysFsClose` against pre-seeded byte content driven through `EmuCore::run_self`. This unlocks the file-backed HLE families that currently dead-end at the permissive-unknown fallback: **cellFs** (HLE wrappers over the lv2 fs syscalls), **savedata** (read/write of `PARAM.SFO` + data files), and **cellFont** (`OpenFontFile` opens-then-fstats for size).

**Reuse, do not reinvent.** The engine already exists:
- `rust/rpcs3-lv2-fs/src/lib.rs` — `FileSystem` trait, `FdTable` (fds start at 4, reserving 0..3 for stdio), and the generic `sys_fs_*` free functions. **All wiring reuses these.**
- `rust/rpcs3-hle-cellfs/src/lib.rs` — thin cellFs HLE wrapper (octal→lv2 flag translation, `validate_path`). **Optional / later** — PSL1GHT `sysFs*` issue raw lv2 syscalls, so the numbered-syscall path can call `rpcs3_lv2_fs::sys_fs_*` directly and bypass cellFs entirely for the MVP.
- `rust/rpcs3-vfs-paths/src/lib.rs` — pure PS3-path canonicalization (`get_path_root_and_trail`). Optional, only if canonical-key normalization is wanted.
- `rust/rpcs3-vfs-mount/src/lib.rs` — `MountTable`, resolves a guest path to a **host disk path**. **EXPLICITLY NOT USED** for the in-memory goal: it targets an on-disk backend. The in-memory store is keyed directly by the guest path string.

**Out of scope for slice 1:** host-disk I/O, mount-point host-path resolution, write-back persistence, directory enumeration, cellFs/savedata/font HLE. Those are later slices (§6).

**Key constraint (behavior-freeze).** The deterministic oracle is a byte-sum: a homebrew reads fixed pre-seeded content and returns a constant derived from it. Pre-seed and C-source content must be byte-identical.

---

## 2. The in-memory FileSystem backend

### Where it lives
A new public module `rust/rpcs3-emu-core/src/vfs.rs` exposing `pub struct MemVfs`. **Origin:** lift the already-proven `MemFs` reference impl from `rpcs3-lv2-fs/src/lib.rs:393-587` (currently behind `#[cfg(test)] mod tests`, so it is NOT compiled into the rlib and cannot be imported). Copy it near-verbatim into emu-core as `MemVfs` — it already satisfies the `FileSystem` trait and passes 30+ unit tests. (Alternative: promote `MemFs` to a `pub` lib type in `rpcs3-lv2-fs`; copying into emu-core is the lower-coupling path and keeps the crate's public surface clean.)

### Struct shape
```rust
// rust/rpcs3-emu-core/src/vfs.rs
pub struct MemVfs {
    files: HashMap<String, Vec<u8>>,            // guest path -> bytes
    dirs:  HashSet<String>,                      // seed with "/"; ancestors auto-inserted
    next_handle: u64,                            // starts at 1
    open_files: HashMap<u64, (String, u64)>,    // handle -> (path, read cursor)
    open_dirs:  HashMap<u64, (Vec<DirEntry>, usize)>,
}
```
- The **read cursor lives in `open_files`** (advanced by `read`, set by `seek`). `FileObject { handle, flags }` carries only the handle + flags; the cursor is keyed off the handle.
- **fstat note:** `rpcs3-lv2-fs::sys_fs_fstat` returns `ENOSYS` because `FileObject` has no path. To make fstat work (needed for cellFont), have `MemVfs` resolve the path via its `open_files[handle]` entry, OR store the source path in `FileObject`. Slice-1 fixture does not fstat, so this is deferred but flagged.

### Trait methods to impl (`rpcs3-lv2-fs/src/lib.rs:117-135`)
All 12 are copied from `MemFs`; only the first five are load-bearing for slice 1:
- `open(&mut self, path, flags) -> Result<FileObject, CellError>` — non-CREAT open requires the file to exist (else `ENOENT`); honors `O_CREAT/O_EXCL/O_TRUNC` for later slices.
- `read(&mut self, &mut FileObject, buf) -> Result<u64, CellError>` — `take = min(data.len() - cursor, buf.len())`; copy; advance cursor; return `take`.
- `close(&mut self, &mut FileObject) -> Result<(), CellError>` — drop the `open_files` entry.
- `seek(&mut self, &mut FileObject, offset: i64, whence: Whence) -> Result<u64, CellError>` — `Set => off`, `Cur => cursor + off`, `End => len + off`; reject negative → `EINVAL`; return new position.
- `stat(&self, path) -> Result<CellFsStat, CellError>` — `CellFsStat { mode: S_IFREG | 0o644, size: data.len() as u64, blksize: 512, ..Default::default() }` for files; `S_IFDIR | 0o777` for dirs.
- `write / mkdir / rmdir / unlink / opendir / closedir / readdir` — keep `MemFs` bodies for free; not exercised until later slices.

### EmuCore fields + fd table
Two new `pub` fields on `EmuCore` (struct at `rpcs3-emu-core/src/lib.rs:412`), mirroring the existing HLE-field pattern (`pub lv2_sync_state: Lv2SyncState` at :463, `pub sysmodule` at :476):
```rust
pub vfs: MemVfs,
pub fd_table: rpcs3_lv2_fs::FdTable,
```
Initialized in `EmuCore::new()` (~:505-531):
```rust
vfs: MemVfs::new(),
fd_table: rpcs3_lv2_fs::FdTable::new(),
```
The `FdTable` **must** be a persistent field (not per-syscall) so the fd from `#801` is valid in `#802`/`#804`. `FdTable.entries`/`next_fd` are private — emu-core drives them only through the `sys_fs_*` functions (fine for open→read→close).

### How the test pre-seeds files
Add a public helper mirroring the existing `core.sysutil_queue.push(...)` pre-seed pattern (`tests/hle_sysutil_callback.rs:48`):
```rust
pub fn vfs_add_file(&mut self, path: &str, data: Vec<u8>) {
    // auto-create ancestor dirs so non-CREAT open + stat succeed
    self.vfs.add_file(path, data);
}
```
Test usage (BEFORE `run_self`):
```rust
let mut core = EmuCore::new();
core.vfs_add_file("/dev_hdd0/test.bin", CONTENT.to_vec());
let report = core.run_self(&bytes)?;
```
The pre-seed key MUST be byte-identical to the NUL-terminated path the guest passes to `sysFsOpen` (no normalization, no trailing slash).

### Cargo dependency
`rpcs3-emu-core/Cargo.toml` does **not** yet depend on any fs crate. Add to `[dependencies]`:
```toml
rpcs3-lv2-fs = { path = "../rpcs3-lv2-fs" }
```
(`rpcs3-hle-cellfs`, `rpcs3-vfs-mount`, `rpcs3-vfs-paths` are NOT needed for slice 1.) All three fs crates are already workspace members per `rust/Cargo.toml`.

---

## 3. Numbered-syscall arms

### Syscall numbers (verified vs `rpcs3/Emu/Cell/lv2/lv2.cpp:782-799` BIND_SYSC table; PSL1GHT `sys/file.h` matches byte-for-byte)
| # (dec) | # (hex) | syscall | wire in slice |
|---|---|---|---|
| 801 | 0x321 | sys_fs_open | **slice 1** |
| 802 | 0x322 | sys_fs_read | **slice 1** |
| 803 | 0x323 | sys_fs_write | EXISTING TTY stub — leave / extend later |
| 804 | 0x324 | sys_fs_close | **slice 1** |
| 805 | 0x325 | sys_fs_opendir | slice 3 |
| 806 | 0x326 | sys_fs_readdir | slice 3 |
| 807 | 0x327 | sys_fs_closedir | slice 3 |
| 808 | 0x328 | sys_fs_stat | slice 2 |
| 809 | 0x329 | sys_fs_fstat | EXISTING S_IFCHR stub — extend in slice 2 |
| 811 | — | sys_fs_mkdir | later |
| 813 | — | sys_fs_rmdir | later |
| 814 | — | sys_fs_unlink | later |
| 818 | 0x332 | sys_fs_lseek | slice 2 |

**Wire first (slice 1): #801 open, #802 read, #804 close.** (#808 stat + #818 lseek follow in slice 2.)

### Exact insertion point
Inside `fn dispatch_syscall(&mut self)` at `rpcs3-emu-core/src/lib.rs:1186`, in the **numbered** `match number {` block that opens at **line 2411** and runs to ~3292. NOT the earlier import-stub / NID `match nid` block (lines 1201-2403) — PSL1GHT `sysFsOpen` issues a real `sc`, not a PLT thunk. Place the new arms next to the existing fs arms (#803 at :3104, #809 at :3134), i.e. after the mmapper band, ~line 3160.

`number = self.ppu.gpr[11]`. Args pre-read: `r3 = self.ppu.gpr[3]` (:2407), `r4 = self.ppu.gpr[4]` (:2408); read `r5..r8` inline via `self.ppu.gpr[5..=8]`.

### Critical wiring rules
- **CIA:** numbered-syscall arms must **NOT** touch `self.ppu.cia`. The interpreter's `sc` handler already advanced CIA before returning `StepOutcome::Syscall` (run loop :1038); the arm falls through to the shared `Ok(None)` at :3293. (Only NID-stub arms do `self.ppu.cia = lr & !0x3` because they are PLT trampoline returns. Do NOT copy that line.)
- **Errors → r3:** success `self.ppu.gpr[3] = 0` (CELL_OK); error `self.ppu.gpr[3] = e.0 as u64` (`CellError(pub u32)` at `rpcs3-emu-types/src/lib.rs:249`; lv2-fs returns `Result<_, CellError>`, so `e.0 as u64` is idiomatic).
- **OUT pointers:** all guest memory is **big-endian**. Write every OUT value via `self.mem.write(ptr, &val.to_be_bytes())?`.
- **Path arg:** read with the existing `self.read_guest_cstr(addr, 1024)` helper (:1171). It is `&self`; read the path into a **local String FIRST**, then mutably borrow `self.vfs` / `self.fd_table` / `self.mem` — never hold the borrows simultaneously (borrow-checker).
- **Existing #803/#809 are TAKEN** by TTY/stdio stubs. Adding a second `803 =>` / `809 =>` arm is a duplicate-match compile error. When slice 2/later needs real write/fstat, **EDIT** those arms to branch: `fd < 4` keeps TTY behavior, `fd >= 4` routes to the VFS.

### Per-syscall arg + OUT marshalling (PPC64: args r3.., return r3)

**#801 `sys_fs_open(path, flags, *fd_out, mode, *arg, size)` — 6-arg.**
`r3 = path ptr`, `r4 = flags (s32)`, `r5 = fd_out ptr`, `r6 = mode`, `r7 = arg`, `r8 = size`. The fd is written to `*r5` (BE u32), **not** returned in r3 (r3 carries only the error code).
```rust
801 => {
    let path_ptr = r3 as u32;
    let flags = r4 as u32;
    let fd_out = self.ppu.gpr[5] as u32;
    let path = self.read_guest_cstr(path_ptr, 1024); // borrow self.mem, then drop
    match rpcs3_lv2_fs::sys_fs_open(&mut self.vfs, &mut self.fd_table, &path, flags) {
        Ok(fd) => { self.mem.write(fd_out, &fd.to_be_bytes())?; self.ppu.gpr[3] = 0; }
        Err(e) => { self.ppu.gpr[3] = e.0 as u64; }
    }
}
```

**#802 `sys_fs_read(fd, *buf, nbytes, *nread)` — 4-arg.**
`r3 = fd`, `r4 = buf ptr`, `r5 = nbytes (u64)`, `r6 = nread ptr`. lv2 fills a host `&mut [u8]`; bytes are NOT auto-copied to guest mem — alloc a host `Vec`, then `mem.write` into the guest buffer.
```rust
802 => {
    let fd = r3 as u32;
    let buf_ptr = r4 as u32;
    let nbytes = self.ppu.gpr[5] as usize;
    let nread_ptr = self.ppu.gpr[6] as u32;
    let mut tmp = vec![0u8; nbytes];
    match rpcs3_lv2_fs::sys_fs_read(&mut self.vfs, &mut self.fd_table, fd, &mut tmp) {
        Ok(n) => {
            if n > 0 { self.mem.write(buf_ptr, &tmp[..n as usize])?; }
            self.mem.write(nread_ptr, &n.to_be_bytes())?; // BE u64
            self.ppu.gpr[3] = 0;
        }
        Err(e) => { self.ppu.gpr[3] = e.0 as u64; }
    }
}
```

**#804 `sys_fs_close(fd)` — 1-arg.** `r3 = fd`.
```rust
804 => {
    match rpcs3_lv2_fs::sys_fs_close(&mut self.vfs, &mut self.fd_table, r3 as u32) {
        Ok(()) => self.ppu.gpr[3] = 0,
        Err(e) => self.ppu.gpr[3] = e.0 as u64,
    }
}
```

**#808 `sys_fs_stat(path, *sb)` — 2-arg (slice 2).** `r3 = path ptr`, `r4 = stat ptr`. Serialize `CellFsStat` to a **52-byte BE** buffer at the exact offsets below, then `mem.write`.

**#818 `sys_fs_lseek(fd, offset, whence, *pos)` — 4-arg (slice 2).** `r3 = fd`, `r4 = offset` (PSL1GHT `sysLv2FsLSeek64` passes **u64**; read `r4 as u64` then cast `as i64`), `r5 = whence (s32)`, `r6 = pos ptr`. New position written to `*r6` (BE u64).

**#809 `sys_fs_fstat(fd, *sb)` — EDIT existing stub (slice 2).** `r3 = fd`, `r4 = stat ptr`. Branch `fd >= 4` → VFS (needs `MemVfs` to track fd→path; lv2 `sys_fs_fstat` returns ENOSYS otherwise).

### CellFsStat on-the-wire layout (52 bytes, 4-byte aligned, ALL big-endian)
RPCS3 `sys_fs.h` `CHECK_SIZE_ALIGN(CellFsStat, 52, 4)` — the `be_t<,4>` override packs 8-byte fields on 4-byte boundaries; **NO** trailing pad to 56.
| offset | field | type |
|---|---|---|
| 0  | mode    | u32/s32 |
| 4  | uid     | s32 |
| 8  | gid     | s32 |
| 12 | atime   | s64 |
| 20 | mtime   | s64 |
| 28 | ctime   | s64 |
| 36 | size    | u64 |
| 44 | blksize | u64 |

The Rust `CellFsStat` field order (lv2-fs `:67-77`) matches; hand-serialize the 8 fields to `[u8; 52]` (the existing #809 stub already uses a `[0u8; 52]` buffer, confirming the size). Deterministic file fill: `mode = S_IFREG | 0o666`, `uid = gid = 0`, times `= 0`, `size = content_len`, `blksize = 512`.

### CellFsDirent (slice 3) — 258 bytes
`d_type @0 (u8)`, `d_namlen @1 (u8)`, `d_name @2 (char[256])`. `sys_fs_readdir` writes the full 258 bytes to `*r4` and `sizeof(dirent)=258` (BE u64) to `*nread` on a hit, 0 at EOF.

---

## 4. First behavior-freeze fixture

**Fixture name:** `single_fs_read_v1` (CC0 PSL1GHT homebrew). Pattern: `sysFsOpen → sysFsRead → byte-sum → sysFsClose → return sum-derived constant`.

**Source** (`single_fs_read_v1.c`, uses `<lv2/sysfs.h>` so it issues the raw lv2 syscalls):
```c
#include <ppu-types.h>
#include <lv2/sysfs.h>
int main(void) {
    int fd = -1;
    u64 nread = 0;
    unsigned char buf[16];
    if (sysFsOpen("/dev_hdd0/test.bin", SYS_O_RDONLY, &fd, NULL, 0) != 0) return 0xBAD0;
    if (sysFsRead(fd, buf, sizeof(buf), &nread) != 0)               return 0xBAD1;
    sysFsClose(fd);
    if (nread != sizeof(buf))                                       return 0xBAD2;
    unsigned int sum = 0;
    for (unsigned i = 0; i < sizeof(buf); i++) sum += buf[i];
    /* content 0x01..0x10 -> sum = 136 = 0x88; gate to 0xC0DE for crisp pass */
    return (sum == 0x88) ? 0xC0DE : 0xBAD3;
}
```

**Pre-seed** (test `rust/rpcs3-emu-core/tests/run_self_fs_read_smoke.rs`): identical 16 bytes baked into the test, seeded before `run_self`:
```rust
const CONTENT: [u8; 16] = [1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16]; // sum = 0x88
let mut core = EmuCore::new();
core.step_budget = 5_000_000;
core.vfs_add_file("/dev_hdd0/test.bin", CONTENT.to_vec()); // BEFORE run_self
let report = core.run_self(&bytes)?;
assert_eq!(report.exit_status.status as u32, 0xC0DE);
```

**Expected exit codes:**
- **POST-wire (positive):** `0xC0DE` (open + read 16 bytes + sum == 0x88).
- **PRE-wire** (arms not yet added): `run_self` sets `permissive_unknown_syscalls = true` (:662), so unhandled #801 hits the catch-all (:3270), logs, returns CELL_OK(0) **without** opening; `*fd` stays 0/garbage, read fails → fixture returns `0xBAD0`/`0xBAD1` (NOT a panic — permissive mode silently no-ops). This makes the wiring observable as a `0xBAD0 → 0xC0DE` transition.
- **Negative control** (same binary, NO pre-seed): `sysFsOpen` → `ENOENT` → fixture returns `0xBAD0`. Mirrors the `hle_sysutil_callback` positive/negative pair convention.

Use the graceful skip-when-absent pattern from `tests/run_self_mutex_smoke.rs` for the `.self` path.

---

## 5. Risks / open questions

1. **Mount-point resolution — RESOLVED for in-memory: bypass it.** `MountTable::resolve` returns a host disk path (`$(EmulatorDir)dev_hdd0/...`) — wrong abstraction for an in-memory store. Key the `HashMap` directly by the raw guest path `/dev_hdd0/test.bin`. RISK: a future fixture passing a path that needs `/app_home` or relative resolution would require `vfs-paths`/`vfs-mount`. For slice 1, pre-seed key MUST === the exact NUL-terminated path string the guest passes.

2. **fd lifecycle / FdTable ownership.** `FdTable` must be a persistent `EmuCore` field so fds survive across open/read/close in one run. Its `entries`/`next_fd` are private — emu-core can only drive it through `sys_fs_*` (acceptable). fds start at 4 (0..3 stdio); keep allocations ≥4 so they don't collide with the TTY fds used by the GCM capture path. Open question: cross-`run_self` reset — `EmuCore::new()` gives a fresh table, fine for per-test isolation.

3. **stat struct layout.** 52 bytes, NOT 56 — `be_t<,4>` packs 8-byte fields on 4-byte boundaries. Writing `size`/`blksize` at natural 8-byte alignment would corrupt offsets. Use the exact offsets in §3. The Rust `CellFsStat` is host-rep only — no serialization helper exists; hand-serialize.

4. **fstat unimplemented (ENOSYS).** `rpcs3-lv2-fs::sys_fs_fstat` returns `ENOSYS` because `FileObject` has no path. cellFont's `OpenFontFile` opens-then-fstats for size, so slice 2 must make `MemVfs`/`FileObject` carry the source path (resolve via `open_files[handle]`). Slice-1 fixture does not fstat → deferred but blocking for font.

5. **cellFs HLE wrappers — needed or just lv2 syscalls?** For the numbered-syscall path: **just the lv2 syscalls.** PSL1GHT `sysFs*` issue raw `sc`, so emu-core calls `rpcs3_lv2_fs::sys_fs_*` directly. cellFs HLE (`rpcs3-hle-cellfs`) is only needed if a title calls `cellFsOpen` via NID dispatch (octal→lv2 flag translation) — a later slice. **Do not pull it in for slice 1.**

6. **Flag-encoding mismatch (latent bug in the crate).** Two open-flag spaces: real lv2/cellFs is **octal** (`O_CREAT=0o100=0x40`, `O_EXCL=0o200=0x80`, `O_TRUNC=0o1000=0x200`, `O_APPEND=0o2000=0x400`); but `rpcs3-lv2-fs` hardcodes POSIX-style bits (`O_CREAT=0x4`, `O_APPEND=0x8`, `O_TRUNC=0x10`, `O_EXCL=0x80`) and froze the WRONG values in `open_flag_constants_frozen`. `O_RDONLY/WRONLY/RDWR/ACCMODE (0/1/2/3)` DO match. **Slice-1 read-only open uses flags=0**, so this is dormant — but any CREAT/TRUNC fixture (savedata) must decode oflags with the real octal `CELL_FS_O_*` constants in the dispatch arm (or fix the crate). FLAG this before slice 4.

7. **DirEntry d_type inversion (latent).** `rpcs3-lv2-fs::FS_TYPE_REGULAR=1 / FS_TYPE_DIRECTORY=2` is INVERTED vs the real ABI (`CELL_FS_TYPE_DIRECTORY=1`, `CELL_FS_TYPE_REGULAR=2`). When marshalling readdir `d_type` to guest `CellFsDirent` (slice 3), use the real values, not the crate's swapped constants.

---

## 6. Build order

**Slice 1 (MVP — this doc's deliverable):**
- Add `rpcs3-lv2-fs` dep to `rpcs3-emu-core/Cargo.toml`.
- Create `src/vfs.rs` with `MemVfs` (lifted from lv2-fs test `MemFs`) + `add_file` + `vfs_add_file` helper.
- Add `pub vfs: MemVfs` + `pub fd_table: FdTable` fields to `EmuCore` (struct + `new()`).
- Wire arms **#801 open, #802 read, #804 close** into the numbered `match number` block (~:3160).
- Fixture `single_fs_read_v1` + test `run_self_fs_read_smoke.rs` (positive `0xC0DE`, negative `0xBAD0`).
- Gate: `cargo test --workspace --lib` green; record SHA in `docs/PROJECT_STATUS.md`.

**Slice 2 — stat / lseek / fstat:**
- Wire **#808 stat**, **#818 lseek**; serialize `CellFsStat` at the 52-byte/4-align offsets.
- Make `MemVfs`/`FileObject` track fd→path; **EDIT** existing **#809 fstat** stub to branch `fd >= 4` → VFS.
- New fixture exercising open→fstat-size→lseek-end→read.

**Slice 3 — directory enumeration:**
- Wire **#805 opendir / #806 readdir / #807 closedir**; marshal 258-byte `CellFsDirent` with **real** `d_type` (dir=1, regular=2) + BE-u64 nread.

**Slice 4 — write + cellFs HLE wrappers:**
- **EDIT #803 write** stub to branch `fd >= 4` → VFS; fix/guard the octal flag decode (`O_CREAT`/`O_TRUNC`) — risk #6.
- Wire #811 mkdir / #813 rmdir / #814 unlink as needed.
- Bring in `rpcs3-hle-cellfs` for NID-dispatched `cellFsOpen`/etc. (octal→lv2 translation).

**Slice 5 — savedata + cellFont:**
- savedata: PARAM.SFO read/write round-trip against `MemVfs` (needs write + stat from slices 2/4).
- cellFont: `OpenFontFile` open-then-fstat path (needs slice-2 fstat).

**Later (optional) — host-disk backend:**
- A real disk-backed `FileSystem` impl using `MountTable::resolve` (guest→host path). Separate backend; the trait abstraction already supports swapping it in without touching the dispatch arms.
