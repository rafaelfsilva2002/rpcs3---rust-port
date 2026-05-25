# R9.1k handoff — __syscalls table state probe

**Status:** R9 wave **truly closed** at "constructor didn't run;
TTY emit requires constructor mechanism investigation".

## R9.1k deliverable: empirical __syscalls scan

Added a post-run scan in `tests/run_self_smoke.rs` that walks
PHDR[3] (.data segment at 0x10010000, 0x14D8 bytes) looking
for a contiguous run of u32 BE values that look like .text
function pointers (within 0x10000..0x2C000).

**Empirical result:**

```
[R9.1k scan] longest text-ptr run in .data: start=0x10010fac len=1 u32s
[R9.1k scan]   [00] +0x0000: 0x00020000
```

A single isolated `0x00020000` value (likely a .data
initializer for some other variable, not a function pointer).

## What this proves

The `__syscalls_init` constructor at priority 104 **did not
write a single function pointer to .data**. If it had run,
~41 sequential u32 BE values would appear (the populated
`__syscalls` table).

Therefore one of:

1. PSL1GHT's `_init()` did NOT walk `.init_array` /
   constructor chain at all. Our PPU's interpreter executes
   the prologue (mflr/std/stdu) + epilogue (addi/ld/mtlr/blr)
   correctly per lv2-crti.S / lv2-crtn.S, but the
   constructor-bl chain GCC inserts between them may not be
   present at the expected offsets in our binary.

2. The constructor chain is present but our PPU jumps over
   it (e.g., interprets a `bl` to an invalid CIA that hits
   our error path silently).

Either way, the symptom is identical: `__syscalls.write_r`
remains NULL, newlib's `_write_r` returns -1 ENOSYS, printf
silently fails.

## What R9.1k did NOT achieve

The investigation to pre-populate `__syscalls.write_r` at
run_self init time was blocked by:

1. The struct `_syscalls` layout (which field is at which
   byte offset within `__syscalls`) is in PSL1GHT's
   `<sys/reent.h>` — that header is NOT in the GitHub
   tree (likely lives inside newlib's installed headers
   only).

2. The address of `__syscalls` in `.data` cannot be inferred
   from the binary without the constructor populating it
   (the scan above found nothing matching).

3. The address of `__librt_write_r` in `.text` cannot be
   reliably extracted without symbol-table content (stripped
   per R9.1g.1 audit).

All three are tractable but require **PSL1GHT-specific
binary archaeology** that's significantly more invasive
than the syscall/import interception R9.1g-j has built up.

## R9 wave: final closure

What R9 (commits R9.1.a → R9.1.k) delivered:

1. **Loader / process bootstrap**:
   - SCE + SELF + PT_SCE custom-segments parsing
     (R9.1g.2 sys_process_param, R9.1g.3 sys_proc_prx_param).
   - PPC64 ELFv1 FD-deref in load_elf (R9.1b).
   - TLS init with Linux ELFv1 r13+0x7000 bias (R9.1g.4).
   - User-mode stack init from sys_process_param (R9.1c).
   - libstub PpuPrxModuleInfo parser + 119 import-stub
     trampoline installer (R9.1g.5, R9.1g.6).

2. **PPC interpreter coverage**:
   - 12 new opcodes (R9.1c STDU, R9.1d sradi, R9.1e batch of
     10 P31 XOs, R9.1g.9 with-update D-form + nand/eqv/orc/
     subfc/adde/subfe/subfze/addme/subfme).

3. **Dispatcher architecture**:
   - NID-aware dispatcher with 20 specific NID handlers
     (R9.1g.7, R9.1h, R9.1i).
   - 7 lv2 syscall arms (R9.1g.9, R9.1g.10, R9.1i) + a
     permissive catch-all.
   - sys_process_exit terminates run cleanly (R9.1g.11).

4. **SPU lifecycle wiring**:
   - sys_spu_image_import → SpuImage capture (R9.1g.10).
   - sys_spu_thread_group_start → spu-interpreter run
     (R9.1g.10).
   - sys_spu_thread_group_join → OUT_MBOX-back-to-PPU
     (R9.1g.10).

5. **stdio handler matrix** (R9.1i, R9.1j):
   - sys_fs_fstat with S_IFCHR for stdout/stderr.
   - sys_fs_write routing fd=1/2 to TTY ch3.
   - 8 NID handlers for _sys_printf / _sys_vprintf /
     _sys_sprintf / _sys_snprintf / _sys_vsnprintf /
     _sys_vsprintf / console_putc / console_write.
   - 3 newlib NID handlers (write, puts, printf).
   - mini_printf format resolver with %d/%u/%x/%X/%s/%c/%p/%%
     + width prefixes.

What R9 did NOT achieve:

- **TTY emit from PSL1GHT main()'s printf call.**
  Blocked by uninitialized `__syscalls` table in
  statically-linked newlib; investigation requires
  constructor-chain disassembly + binary-specific reverse
  engineering beyond what's possible via syscall/import
  interception.

## Recommended outcomes

- **(a) Accept R9 as architecturally complete.** The SPU stack
  + LV2/PPU layer are integrated end-to-end. PSL1GHT main()
  runs through every SPU lifecycle syscall. Move on to other
  subsystems (RSX/audio/filesystem or further SPU work).

- **(b) Schedule a dedicated session for constructor
  archaeology**: locate `__syscalls_init` in the binary via
  byte-pattern matching, decode its body to find both the
  `__syscalls` global address and the `__librt_*` function
  addresses, then pre-populate at run_self init. Bounded
  work (~1 session) if the patterns are stable across PSL1GHT
  builds.

- **(c) Path B from R9.1f**: skip PSL1GHT crt0 entirely by
  jumping CIA directly to `main()`. Requires symbol-table
  presence (stripped in our `.self`) or alternative ways to
  locate `main()`. May not unblock TTY emit if the issue is
  newlib stdout setup separately from constructor chain.

## Validation status (R9 wave end-to-end)

- cargo test --workspace --tests --release: 264 result blocks,
  0 failures throughout 17 R9 commits.
- 20 SPU oracle replay tests still green.
- behavior-freeze contract preserved.
- Zero workspace regression.

## R9 commits (17)

```
a73c17b4d  r9.1j: post-fstat disassembly proves static-newlib blocker
c4bbb312b  r9.1i: 8 stdio NID handlers + sys_fs_fstat/write + full NID map
a854e431e  r9.1h slice 2-4: PSL1GHT sysPrxForUser import handlers + import dump
b2558fe5f  r9.1h: handoff doc — autonomous loop paused
b4b764906  r9.1h: NID-specific stubs for sys_spinlock + sys_mmapper imports
9794837ff  r9.1g.10: wire SPU execution into sys_spu_thread_group_start
... (11 earlier R9.1g.* slices)
```

R9.1k closes the wave honestly: full architectural
integration achieved, end-user TTY output blocked by
static-newlib constructor mechanism that requires offline
binary archaeology to unblock.
