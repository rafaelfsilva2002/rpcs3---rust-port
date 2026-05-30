# single_savedata_autosave_v1 (R16 — cellSaveData callback bridge)

The first **callback-driven** HLE family: `cellSaveDataAutoSave2` /
`cellSaveDataAutoLoad2`. Unlike every prior HLE fixture (where the syscall
returns a value), savedata works by the *system* calling back into guest code —
the game registers a status callback + a file callback, and the emu-core bridge
invokes them via `EmuCore::call_guest_function` (the R14 guest-PPU re-entry
infra), marshalling the `sysSave*` structs through a guest scratch page and
performing the actual file I/O against the in-memory VFS (R15).

## Behaviour

```c
sysSaveAutoSave2(version, "SLOTAUTO00", NONE, &buf, status_cb, file_write_cb, 0, 0);
// status_cb sets result=CONTINUE; file_write_cb requests WRITE of 8 bytes -> DONE
memset(g_filebuf, 0, ...);          // wipe local buffer
sysSaveAutoLoad2(version, "SLOTAUTO00", NONE, &buf, status_cb, file_read_cb, 0, 0);
// file_read_cb requests READ of 8 bytes back into g_filebuf -> DONE
return (g_filebuf == PAYLOAD) ? 0xC0DE : 0xBAD3;
```

No printf / SPU / RSX — pure cellSaveData HLE + guest callbacks + VFS.

## How it wires

- `cellSaveDataAutoSave2` (NID captured at runtime) → emu-core bridge:
  1. Allocate a guest scratch page (`SAVEDATA_SCRATCH_VADDR`), zero the
     `sysSaveStatusIn`/`Out`, `sysSaveCallbackResult`, `sysSaveFileIn`/`Out`.
  2. Invoke the **status callback** once via `call_guest_function(statCb, [result, statIn, statOut])`.
  3. Loop the **file callback** via `call_guest_function(fileCb, [result, fileIn, fileOut])`;
     read `sysSaveFileOut` {fileOperation, filename, offset, size, buffer}; on
     `WRITE` copy `size` bytes from the guest `buffer` into the VFS at
     `/dev_hdd0/home/00000001/savedata/<dirName>/<fileName>`; stop when the
     callback returns `DONE` (OK_LAST).
- `cellSaveDataAutoLoad2` mirrors it: `READ` copies VFS bytes back into the
  guest `buffer`.

## Result

`EmuCore::run_self` exit status = **0xC0DE** (round-trip match), vs **0xBAD3**
pre-wire (callbacks never fire → load reads nothing) / **0xBAD0** if AutoSave2
returns non-zero / **0xBAD1** if AutoLoad2 returns non-zero.

## Consumed by

`rust/rpcs3-emu-core/tests/hle_savedata_autosave.rs`. The `.self`/`.elf` are
built locally via the PSL1GHT Docker toolchain and gitignored.

CC0 1.0 (public domain) — see LICENSE.md.
