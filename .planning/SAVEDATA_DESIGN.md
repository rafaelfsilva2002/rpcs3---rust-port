# cellSaveData Bridge Implementation Blueprint (AutoSave2 / AutoLoad2)

Status: DESIGN (pre-wire). Target: drive a real PSL1GHT homebrew save->load
round-trip through `EmuCore::run_self`, end-to-end, returning `0xC0DE`.

This blueprint covers the **minimal** AutoSave/AutoLoad path only. List*/Fixed*
APIs and PARAM.SFO validation are explicitly out of scope (see §1, §7).

Grounding (verified against the tree, not just the findings JSON):
- `EmuCore::call_guest_function` exists and is reusable as-is
  (`rust/rpcs3-emu-core/src/lib.rs:1118`): reads compact 4-byte FD or full OPD
  `{code@0, toc@4}`, sets `r2 = toc`, seeds `r3..=r10`, runs a nested loop that
  stops when `cia == GUEST_CALLBACK_SENTINEL (0xD0FF_0000)` (`lib.rs:289`,
  `:1153-1158`, `:1173`), and **restores the full register frame** afterward
  (`PpuRegSnapshot`, `:1137-1149`) while **deliberately keeping guest memory
  writes** (`:1136`, `:295`). This is exactly the re-entrancy contract the
  fileCb loop needs.
- There are **no** `read_be_u32 / write_be_u32 / write_struct` helpers in
  emu-core yet — the bridge does raw `self.mem.read/write(addr, &bytes)` with
  `u32::from_be_bytes` / `.to_be_bytes()` (precedent at `lib.rs:600,686,1013-1023`).
  Adding two tiny private helpers (`read_be_u32`, `write_be_u32`) is part of
  slice A.
- `read_guest_cstr(addr, max)` already exists (`lib.rs:1226`) — used to read
  `fileSet->fileName`.
- `MemVfs.add_file(path, data)` auto-creates ancestor dirs (`vfs.rs:42-56`) and
  files live in a path-keyed map, so the bridge writes/reads save files directly
  without needing fd/open plumbing, and the Rust test can read back via the same
  map key.

---

## 1. Goal, scope, and the honest verdict

### Goal
Add `cellSaveDataAutoSave2` and `cellSaveDataAutoLoad2` NID dispatch arms that
emulate the PS3 HLE callback protocol: invoke the game-supplied `statCb` once,
then drive the `fileCb` loop, performing real VFS file I/O on the game's own
buffers, and return `CELL_OK`. The deliverable is a self-checking homebrew that
saves a known 8-byte payload then loads it back and returns `0xC0DE` on match.

### Scope verdict — this is TWO slices (A + B), not one. C is a separate wave.

- **Slice A — `cellSaveDataAutoSave2` (write path).** The real work:
  - new `SAVEDATA_SCRATCH_VADDR` reserved page + sub-offset layout for the
    5 callback structs;
  - marshal the structs (CBResult 20B, StatGet minimal, StatSet 12B, FileGet 68B,
    FileSet 48B); SetBuf 40B is read-only;
  - the statCb-once + fileCb-loop CBRESULT driver on top of the existing
    `call_guest_function`;
  - the WRITE op via `EmuCore.vfs`;
  - the NID arm;
  - **building the `single_savedata_autosave_v1` homebrew + confirming the NID
    imports** (the risky part).
  - Estimate: 1 focused slice, ~1–2 days, dominated by (a) the homebrew
    build + NID confirmation and (b) zeroing StatGet's nested layout correctly.

- **Slice B — `cellSaveDataAutoLoad2` (read-back).** Mostly reuses slice A's
  driver: add the AutoLoad2 NID arm (READ fileCb path, copy VFS bytes into
  `fileSet->fileBuf`, set `excSize`), extend the SAME homebrew to do the
  round-trip and return `0xC0DE`. Small once A lands (~0.5 day).
  The `0xC0DE` oracle needs BOTH A and B, so ship A then B back-to-back (or one
  combined PR, with the write asserted from Rust at the A boundary).

- **Slice C — List*/Fixed* (OUT OF SCOPE).** Adds funcList/funcFixed,
  dir-enumeration into `setBuf->buf`, focusPosition/newData handling
  (cpp:1135-1320), the save-dialog selection model. Its own wave. Defer entirely.

### The single biggest blocker
**The PSL1GHT homebrew SOURCE side, not the Rust side.** PSL1GHT ships **no**
cellSaveData header. The homebrew must hand-declare the structs (byte-identical
to `cellSaveData.h`) and `extern` `cellSaveDataAutoSave2` / `AutoLoad2` so the
PPU GCC toolchain emits libstub NID-import entries. If the linker emits no
import (wrong module name / missing stub lib), the `.self` never references the
NID and the bridge never fires. The Rust side is mechanical struct marshalling
on top of an already-working `call_guest_function`. **Everything else is known;
the import emission + runtime NID confirmation is the only unknown-unknown.**

Mitigation: model the `extern` decl on an existing cell* fixture that lacks a
PSL1GHT wrapper (e.g. `single_videoout_*`, `single_sysutil_param_v1`), and
confirm the libstub NIDs at runtime (log every NID hit) BEFORE locking the arm.

The scratch allocator is **NOT** a blocker (fixed reserved region suffices,
proven by upstream's `g_savedata_context` static vm::gvar; see §3).

---

## 2. The callback protocol to emulate

Source of truth: `cellSaveData.cpp` `savedata_op` core (`:699`), funcStat call
(`:1622-1636`), funcFile while-loop (`:1837-1871`), read/write op
(`:1969-2089`). Header struct layouts: `cellSaveData.h:164-335`.

### Callback ABI (marshalled through `call_guest_function`)
Both callbacks are `void`, three pointer args, all pointing into the host
SAVEDATA scratch region (§3):

```
statCb(CellSaveDataCBResult* result, CellSaveDataStatGet* get, CellSaveDataStatSet* set)
  -> r3 = result_ptr, r4 = statGet_ptr, r5 = statSet_ptr
fileCb(CellSaveDataCBResult* result, CellSaveDataFileGet* get, CellSaveDataFileSet* set)
  -> r3 = result_ptr, r4 = fileGet_ptr, r5 = fileSet_ptr
```

`call_guest_function` reads the FD's OPD (`code@0, toc@4`), sets `r2 = toc`,
seeds `r3/r4/r5`, runs to the sentinel, restores the frame. Callbacks should be
**leaf functions** that write fixed bytes + set `result->result` (avoid heavy
TOC-relative data; the OPD-supplied `r2` covers simple global/literal access).

### Exact step order

1. Zero `CBResult` (20B). Build a minimal `StatGet` (mostly zeroed — see §2 field
   table; `isNewData=1`, free space large, `fileListNum=0`, dir/system params
   zeroed). Build `StatSet` (12B, all zero so `setParam == null`).
2. `call_guest_function(funcStat_fd, [result_ptr, statGet_ptr, statSet_ptr])`.
3. Read `result->result` (s32 BE @ `CBResult+0`).
   - `OK_NEXT (0)` -> proceed to the fileCb loop.
   - `OK_LAST (1)` -> skip all file ops, return `CELL_OK`.
   - anything else (incl. `OK_LAST_NOCONFIRM` which is invalid for statCb)
     -> return `CELL_SAVEDATA_ERROR_CBRESULT`.
   - **`statSet->setParam` MUST stay null on the minimal path** — that guards
     all ~8 PARAM.SFO validation branches (cpp:1638-1697). Do not populate it.
4. fileCb loop (`while (funcFile)`), each iteration:
   a. `memset` `fileSet` (48B), `fileGet.reserved`, and `result` (20B)
      (cpp:1841-1843).
   b. Write `fileGet->excSize` (u32 BE @ FileGet+0) = bytes transferred by the
      previous iteration (0 on the first iteration).
   c. `call_guest_function(funcFile_fd, [result_ptr, fileGet_ptr, fileSet_ptr])`.
   d. Read `result->result`.
   e. If `result->result` is a *continue/stop* code, read the FileSet fields and
      perform the op (read/write/delete) on `fileSet->fileBuf` (§4), recording
      the transferred byte count as the next `excSize`.
   f. Loop control (see CBRESULT table): `OK_NEXT` -> do op, continue;
      `OK_LAST` / `OK_LAST_NOCONFIRM` -> do op, then **break**; negative ->
      break with `CELL_SAVEDATA_ERROR_CBRESULT`.
5. Return `CELL_OK` (set `r3 = 0`, `cia = lr & !0x3`, `Ok(None)`).

> **The fileCb is a LOOP, not one call.** A minimal fixture returns `OK_LAST`
> on the first call so the loop runs exactly once — but the bridge MUST
> implement the loop, or a 2-file game silently drops files. Do not hardcode
> "one call".

### Minimal-path struct field offsets (all fields BIG-endian; ptrs = 4B)

`be_t<u128,1>` is a 16-byte, 1-aligned packed field (no padding before it).

`CellSaveDataCBResult` (20B) — bridge READS `result`; game WRITES it:
| field        | type | off |
|--------------|------|-----|
| result       | s32  | 0   |
| progressBarInc | u32 | 4  |
| errNeedSizeKB | s32 | 8   |
| invalidMsg   | ptr  | 12  |
| userdata     | ptr  | 16  |

`CellSaveDataFileSet` (48B) — bridge READS all of these AFTER fileCb returns:
| field         | type  | off |
|---------------|-------|-----|
| fileOperation | u32   | 0   |
| reserved      | ptr   | 4   |
| fileType      | u32   | 8   |
| secureFileId  | u128  | 12  |
| fileName      | ptr   | 28  |
| fileOffset    | u32   | 32  |
| fileSize      | u32   | 36  |
| fileBufSize   | u32   | 40  |
| fileBuf       | ptr   | 44  |

`CellSaveDataFileGet` (68B) — bridge WRITES `excSize` before each fileCb:
| field    | type      | off |
|----------|-----------|-----|
| excSize  | u32       | 0   |
| reserved | char[64]  | 4   |

`CellSaveDataSetBuf` (40B) — read-only; supplied by the homebrew:
| field       | type     | off |
|-------------|----------|-----|
| dirListMax  | u32      | 0   |
| fileListMax | u32      | 4   |
| reserved    | u32[6]   | 8   |
| bufSize     | u32      | 32  |
| buf         | ptr      | 36  |

`CellSaveDataStatSet` (minimal, 12B) — leave fully zeroed so `setParam == null`.
`CellSaveDataStatGet` — large (nested `CellSaveDataDirStat` +
`CellSaveDataSystemFileParam`, ~1.5 KB), but for the minimal path nearly all of
it is zeroed; set only `isNewData=1`, a generous free-space field, `fileListNum=0`,
and `dir.dirName` = the autosave dir. Times (`atime/mtime/ctime`) may be zeroed —
the fixture never reads them (determinism, §6).

### CBRESULT loop-control values (`lib.rs:74-81`, byte-identical to header)
| code                  | val | statCb            | fileCb                          |
|-----------------------|-----|-------------------|---------------------------------|
| `OK_NEXT`             |  0  | proceed to fileCb | do this op, loop again          |
| `OK_LAST`             |  1  | skip ops, ret OK  | do this op, then STOP (CELL_OK) |
| `OK_LAST_NOCONFIRM`   |  2  | INVALID (error)   | same as OK_LAST                 |
| `ERR_NOSPACE`         | -1  | abort             | abort                           |
| `ERR_FAILURE`         | -2  | abort             | abort                           |
| `ERR_BROKEN`          | -3  | abort             | abort                           |
| `ERR_NODATA`          | -4  | abort             | abort                           |
| `ERR_INVALID`         | -5  | abort             | abort                           |

Negatives -> `CELL_SAVEDATA_ERROR_CBRESULT` / `..._PARAM`.

### fileOperation codes (`lib.rs:85-88`)
| code              | val |
|-------------------|-----|
| `FILEOP_READ`     | 0   |
| `FILEOP_WRITE`    | 1   |
| `FILEOP_DELETE`   | 2   |
| `FILEOP_WRITE_NOTRUNC` | 3 |

fileType (`lib.rs:92-97`): minimal path uses `FILETYPE_NORMALFILE (1)`.

---

## 3. Guest-scratch allocation

### Mechanism found (no allocator needed)
Upstream proves the scratch is **one fixed reserved region**, not a malloc.
`cellSaveData.cpp:109-123` defines a `savedata_context` struct holding all 8
callback structs; `:761-771` allocates exactly one static `vm::gvar`
`g_savedata_context` and hands the callbacks sub-offset pointers
(`result = ptr(&::result)`, `statGet = ptr(&::statGet)`, ...). No per-call
allocation. The game-owned buffers (`setBuf->buf`, `fileSet->fileBuf`) come from
the homebrew's own `.bss`/stack — the HLE never allocates them
(cpp:645 requires `setBuf->buf` non-null; cpp:1522 sets the fileList over it).

### What to add to emu-core (slice A)
Mirror it with ONE reserved page, analogous to the existing reserved windows
(`GUEST_CALLBACK_SENTINEL = 0xD0FF_0000` @ `lib.rs:289`,
`USER_IMPORT_STUB_VADDR = 0xD0010000` @ `lib.rs:390-391`):

```rust
/// Fixed host-reserved page for the cellSaveData callback structs.
/// Mirrors upstream's single static g_savedata_context (cellSaveData.cpp:761).
/// Must not overlap any mapped code/stack page or the other 0xD0xx windows.
pub const SAVEDATA_SCRATCH_VADDR: u32 = 0xD002_0000;
```

Lay the structs at 16-aligned sub-offsets inside that page (example):
```
result_ptr   = SAVEDATA_SCRATCH_VADDR + 0x000   // CBResult  (20B)
statSet_ptr  = SAVEDATA_SCRATCH_VADDR + 0x020   // StatSet   (12B, zeroed)
fileGet_ptr  = SAVEDATA_SCRATCH_VADDR + 0x040   // FileGet   (68B)
fileSet_ptr  = SAVEDATA_SCRATCH_VADDR + 0x0C0   // FileSet   (48B)
statGet_ptr  = SAVEDATA_SCRATCH_VADDR + 0x100   // StatGet   (~1.5KB, mostly zero)
```
Write each struct with `to_be_bytes`, pass the addresses in r3/r4/r5. No scratch
allocator is required for slices A/B.

> The page must be backed in `self.mem` (writable). If the existing memory map
> does not already cover `0xD0020000`, reserve/back it the same way the
> import-stub window is set up at boot — verify during slice A and add a small
> reservation if absent.

---

## 4. The bridge algorithm (cellSaveDataAutoSave2 NID arm)

### Entry signature (cpp:2374; AutoLoad2 @ :2383 is identical)
NID-dispatched cell* import — the arm goes in the `match nid` block, **NOT** the
numbered-syscall block. Guest call args:
```
cellSaveDataAutoSave2(version u32, dirName char*, errDialog u32,
                      setBuf*, funcStat*, funcFile*, container u32, userdata*)
  r3 = version, r4 = dirName_ptr, r5 = errDialog, r6 = setBuf_ptr,
  r7 = funcStat_fd, r8 = funcFile_fd, r9 = container, r10 = userdata_ptr
```
AutoSave2 takes `setBuf` only (NO setList — cpp:2374-2380).

### Re-entrancy / borrow rule
Extract ALL scalar args (`dirName`, `funcStat_fd`, `funcFile_fd`, `setBuf_ptr`,
`userdata_ptr`) into locals FIRST, dropping any `import_plan` borrow, BEFORE
calling `call_guest_function` (which mutably re-enters the dispatcher). Same rule
the guest-callback design enforces.

### Skeleton (write path)
```rust
// --- preamble: extract scalars, drop borrows ---
let dir_name = self.read_guest_cstr(dirname_ptr, DIRNAME_SIZE);
let func_stat = self.read_gpr_arg(7) as u32;   // funcStat FD ptr
let func_file = self.read_gpr_arg(8) as u32;   // funcFile FD ptr
let save_dir  = format!("/dev_hdd0/home/00000001/savedata/{dir_name}"); // §5

// --- 1. statCb once ---
self.mem.write(result_ptr, &[0u8; 20])?;        // CBResult zeroed
self.build_statget_minimal(statget_ptr, &dir_name)?; // isNewData=1, free space, fileListNum=0
self.mem.write(statset_ptr, &[0u8; 12])?;        // setParam stays null
self.call_guest_function(func_stat, &[result_ptr as u64, statget_ptr as u64, statset_ptr as u64])?;
let res = self.read_be_s32(result_ptr)?;         // CBResult::result @0
match res {
    0 => {}                                       // OK_NEXT -> proceed
    1 => { self.ppu.gpr[3] = 0; return self.return_to_lr(); } // OK_LAST: skip ops, CELL_OK
    _ => { self.ppu.gpr[3] = CELL_SAVEDATA_ERROR_CBRESULT; return self.return_to_lr(); }
}

// --- 2. fileCb loop ---
let mut prev_exc: u32 = 0;
loop {
    self.mem.write(fileset_ptr, &[0u8; 48])?;     // memset fileSet (cpp:1841)
    self.write_be_u32(fileget_ptr, prev_exc)?;    // FileGet::excSize @0
    self.mem.write(result_ptr, &[0u8; 20])?;
    self.call_guest_function(func_file, &[result_ptr as u64, fileget_ptr as u64, fileset_ptr as u64])?;
    let r = self.read_be_s32(result_ptr)?;
    if r < 0 { self.ppu.gpr[3] = CELL_SAVEDATA_ERROR_CBRESULT; return self.return_to_lr(); }

    // read FileSet fields @0/8/28/32/36/40/44
    let op        = self.read_be_u32(fileset_ptr + 0)?;
    let _ftype    = self.read_be_u32(fileset_ptr + 8)?;
    let name_ptr  = self.read_be_u32(fileset_ptr + 28)?;
    let offset    = self.read_be_u32(fileset_ptr + 32)?;
    let size      = self.read_be_u32(fileset_ptr + 36)?;
    let buf_size  = self.read_be_u32(fileset_ptr + 40)?;
    let buf_ptr   = self.read_be_u32(fileset_ptr + 44)?;
    let fname     = self.read_guest_cstr(name_ptr, 16); // <=12 chars (§ gotcha)
    let path      = format!("{save_dir}/{fname}");

    // validate (cpp:1981/1988): fileBuf non-null && fileBufSize >= fileSize
    if buf_ptr == 0 || buf_size < size {
        self.ppu.gpr[3] = CELL_SAVEDATA_ERROR_PARAM; return self.return_to_lr();
    }

    prev_exc = match op {
        1 | 3 => { // WRITE / WRITE_NOTRUNC: read `size` bytes FROM guest fileBuf
            let mut payload = vec![0u8; size as usize];
            self.mem.read(buf_ptr, &mut payload)?;   // raw byte copy (payload is NOT BE-swapped)
            // offset honored for NOTRUNC; minimal path uses offset==0
            self.vfs.add_file(&path, payload);        // auto-creates ancestor dirs (§5)
            size
        }
        0 => { // READ (AutoLoad2 path): copy excSize bytes TO guest fileBuf
            let data = self.vfs.files.get(&path).cloned().unwrap_or_default();
            let n = core::cmp::min(size as usize, core::cmp::min(buf_size as usize, data.len()));
            self.mem.write(buf_ptr, &data[..n])?;
            n as u32
        }
        2 => { self.vfs.files.remove(&path); 0 } // DELETE
        _ => 0,
    };

    match r {
        1 | 2 => break,   // OK_LAST / OK_LAST_NOCONFIRM: op done, stop
        0     => continue,// OK_NEXT: op done, loop again
        _     => break,   // (negatives handled above)
    }
}

// --- 3. return CELL_OK ---
self.ppu.gpr[3] = 0;
self.return_to_lr()   // cia = lr & !0x3; Ok(None)
```

### Crate vs direct VFS I/O — do VFS I/O DIRECTLY
The existing `rpcs3-hle-cellsavedata` crate (`lib.rs:175-189`) collapses the
callback stack into a pre-built `ops` list (`SaveDataState` trait) — it does
**not** model the guest-callback protocol the bridge must drive, and it diverges
on the filename limit (validates 63 vs the ABI's 13, `lib.rs:216`). The bridge's
job *is* the callback loop; once it has the resolved `(op, path, bytes)` it does
the file I/O straight against `EmuCore.vfs` (`add_file` / `files` map). Reuse the
crate's **constants only** (`CBRESULT_*`, `FILEOP_*`, `FILETYPE_*`,
`DIRNAME_SIZE` @ `lib.rs:70-97`) so values stay single-sourced. Flag the
13-vs-63 filename-limit divergence for a later decision; the fixture sidesteps it
with `DATA.BIN` (<=12 chars).

---

## 5. VFS save-dir path convention

```
/dev_hdd0/home/<userId8>/savedata/<dirName>/<fileName>
```
- `<userId8>` = userId formatted as 8 decimal digits; userId 0 => default
  `00000001` (cpp:775). Minimal path uses `00000001`.
- `<dirName>` = the AutoSave2 `dirName` arg, e.g. `SLOTAUTO00`.
- `<fileName>` = `fileSet->fileName`, e.g. `DATA.BIN`.

Minimal-path concrete path:
```
/dev_hdd0/home/00000001/savedata/SLOTAUTO00/DATA.BIN
```
`MemVfs.add_file` auto-creates the ancestor dirs (`vfs.rs:42-56`), so the bridge
just calls `add_file(path, payload)` on WRITE; no explicit dir-create needed.
The READ path keys the same string into `self.vfs.files`. The path prefix MUST
be byte-identical between save and load or the round-trip breaks.

PARAM.SFO is skipped on the minimal path (statSet->setParam null), so the bridge
writes only `DATA.BIN`; it does not create a `PARAM.SFO`. Flag for slice C.

---

## 6. First fixture + expected exit

### Fixture: `single_savedata_autosave_v1` (CC0 PSL1GHT homebrew)
Mirror the exact dir layout of existing HLE fixtures
(`single_fs_write_v1`, `single_msgdialog_callback_v1`):
```
behavior-freeze/fixtures/hle/sources/single_savedata_autosave_v1/
  main.c  Makefile  LICENSE.md  README.md  .gitignore  *.elf  *.self
```
`.self` is gitignored, built via the PSL1GHT Docker toolchain
(`.claude/ps3toolchain-docker/`, `subst R:` trick, `MSYS_NO_PATHCONV=1`).
Test file: `rust/rpcs3-emu-core/tests/hle_savedata_autosave.rs` — copy the
skip-when-absent + positive/negative convention verbatim from
`hle_fs_write.rs` / `hle_sysutil_callback.rs`.

### Homebrew main() (self-checking round-trip oracle)
- Hand-declare the structs (byte-identical to `cellSaveData.h`) and
  `extern cellSaveDataAutoSave2/AutoLoad2` (model on a wrapper-less cell*
  fixture). Keep callbacks as leaf functions.
- `statCb`: `result->result = OK_NEXT`; leave `statSet->setParam` null.
- `fileWriteCb`: fill a WRITE FileSet (`DATA.BIN`, fileBuf=g_filebuf,
  fileBufSize=16, fileSize=8, offset=0), `memcpy` the 8-byte payload into
  g_filebuf, `result->result = OK_LAST`.
- `fileReadCb`: READ FileSet (same names/sizes), `result->result = OK_LAST`.
- `main`: AutoSave2 -> memset g_filebuf -> AutoLoad2 -> compare bytes.

### Exit-code ladder
| code   | meaning                                  |
|--------|------------------------------------------|
| 0xBAD0 | AutoSave2 returned nonzero               |
| 0xBAD1 | AutoLoad2 returned nonzero               |
| 0xBAD2 | read-back excSize != 8 (or bytes absent) |
| 0xBAD3 | bytes differ                             |
| 0xC0DE | full save->load round-trip matched       |

### Expected exit (the wiring-progress signal)
- **PRE-wire** (arms absent; `run_self` permissive => unknown NID returns
  `CELL_OK(0)` WITHOUT firing callbacks, `lib.rs:~607`): AutoSave2 "succeeds"
  writing nothing, AutoLoad2 "succeeds" reading nothing, `excSize` stays 0 =>
  **`0xBAD2`**.
- **POST slice A only** (AutoSave2 wired, AutoLoad2 still permissive): the Rust
  test asserts `core.vfs.files` contains the 8 bytes at the SLOTAUTO00 path
  (belt-and-suspenders boundary check). Homebrew still returns `0xBAD2` until B.
- **POST slice A+B**: full round-trip => **`0xC0DE`** (`report.exit_status.status
  == 0xC0DE`).
- **Negative control**: stub AutoSave2 as a permissive no-op => fileCb never
  fires => load finds nothing => non-`0xC0DE` (e.g. `0xBAD2`).

> Because pre-wire (permissive) and a WRONG NID both look like a silent
> `0xBAD2`, **log every NID hit and confirm the AutoSave2/AutoLoad2 NIDs are
> actually dispatched** before trusting any green/red. The transition
> `0xBAD2 -> 0xC0DE` is the observable wiring proof.

---

## 7. Build order (slices) + risks

### Build order
0. **NID prep**: compute `cellSaveDataAutoSave2` / `AutoLoad2` NIDs via
   `ppu_generate_id` (SHA1(name+suffix)[..4] LE), per
   `.planning/GUEST_CALLBACK_DESIGN.md` §4. Treat as provisional until step 1c.
1. **Slice A (write)**:
   - 1a. Add `SAVEDATA_SCRATCH_VADDR` + back the page; add `read_be_u32/s32`,
     `write_be_u32` private helpers.
   - 1b. Build `single_savedata_autosave_v1` homebrew (write+read cbs, round-trip
     main); build `.self` via Docker; commit CC0 sources.
   - 1c. **Confirm libstub NIDs at runtime** (dump/log) against the fixture —
     lock the arm only after a confirmed hit.
   - 1d. Implement the `cellSaveDataAutoSave2` arm (statCb-once + fileCb-loop +
     VFS write). Test asserts `core.vfs` has the 8 bytes at the SLOTAUTO00 path.
2. **Slice B (read-back)**: add `cellSaveDataAutoLoad2` arm (READ fileCb path,
   copy VFS bytes into fileBuf + set excSize); homebrew already does the
   round-trip => assert `0xC0DE`. Ship A then B (or one combined PR).
3. **Gate**: `cargo test --workspace --lib` green; new test
   `hle_savedata_autosave.rs` green (skip-when-absent + positive/negative).
4. **Slice C (List*/Fixed*)** — deferred wave, not in this effort.

### Top risks
1. **Homebrew NID-import emission (HIGHEST).** No PSL1GHT cellSaveData header;
   if the linker emits no NID import the bridge never fires and it looks like a
   silent `0xBAD2` pass-through (indistinguishable from "arm not wired").
   Mitigate: copy a wrapper-less cell* fixture's extern/stub idiom; log+confirm
   the NID hit at runtime before locking the arm.
2. **fileCb loop correctness.** Must memset fileSet+fileGet.reserved+result
   before EACH iteration, read `result->result` each time, only break on
   OK_LAST / OK_LAST_NOCONFIRM / negative. Hardcoding "one call" passes the
   minimal fixture but silently drops files for any real 2-file game.
3. **StatGet nested layout / setParam guard.** StatGet is ~1.5 KB of nested
   structs; getting an offset wrong (or accidentally making `statSet->setParam`
   non-null) drags in ~8 PARAM.SFO validation branches and breaks the minimal
   path. Zero everything except `isNewData`, free-space, `fileListNum=0`,
   `dir.dirName`; keep StatSet all-zero.

### Secondary risks (watch, not blockers)
- `0xD0020000` page may need explicit backing in the memory map (verify in 1a).
- fileBuf payload is a **raw byte copy** (NOT BE-swapped); only the struct scalar
  fields are BE. Replicate cpp:1981/1988 validation (`fileBuf != null`,
  `fileBufSize >= fileSize`) so the negative control fails for the right reason.
- Filename-limit divergence (ABI 13 vs crate 63): fixture uses `DATA.BIN`
  (<=12) to dodge both; flag for a later decision.
- TOC: keep callbacks leaf (the OPD `r2` covers simple globals; avoid heavy
  TOC-relative data).
