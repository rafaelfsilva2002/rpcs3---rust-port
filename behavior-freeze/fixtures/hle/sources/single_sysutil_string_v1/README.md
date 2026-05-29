# single_sysutil_string_v1 (HLE wave — cellSysutil string param)

Extends the cellSysutil integration (R13.6) to the **string** param path,
reusing the same `rpcs3-hle-cellsysutil` dep + `EmuSysutilConfig` provider.

## Behaviour

A PPU-only homebrew that calls
`cellSysutilGetSystemParamString(ID_NICKNAME, buf, sizeof buf)` (via PSL1GHT's
`sysUtilGetSystemParamString`) into a zero-initialised buffer, then returns a
byte-sum of the result:

- `ret != 0` → returns `0x0BAD`
- `ret == 0` → returns `sum(buf bytes)` (`0` pre-wire — buffer untouched;
  `363` = byte-sum of `"RPCS3"` once wired)

No printf / SPU / RSX — a clean PPU + cellSysutil string call.

## How it wires

1. The guest call fires the cellSysutil string-param NID (captured at runtime
   from the `[R9.1g.7] unimplemented import` log; r3=0x113=ID_NICKNAME, r4=buf,
   r5=bufsize).
2. The dispatcher routes it to
   `rpcs3_hle_cellsysutil::cell_sysutil_get_system_param_string(&EmuSysutilConfig,
   id, bufsize)`, then copies the returned string into the guest buffer with
   truncation + NUL termination.
3. `EmuSysutilConfig::get_param_string` now returns a default nickname
   (`"RPCS3"`) for `Nickname` / `CurrentUsername`.

## Result

`EmuCore::run_self` exit status = **363** (byte-sum of `"RPCS3"`), vs `0`
pre-wire (the stub never wrote the buffer) — proving the string path runs
end-to-end.

## Consumed by

`rust/rpcs3-emu-core/tests/hle_sysutil_string.rs`. The `.self`/`.elf` are built
locally via the PSL1GHT Docker toolchain and gitignored.

CC0 1.0 (public domain) — see LICENSE.md.
