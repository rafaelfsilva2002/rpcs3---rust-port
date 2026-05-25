# R9.1i handoff — printf path identification

**Status:** PAUSED after fstat + sys_fs_write + console_write
+ _sys_printf + _sys_vprintf + _sys_snprintf + _sys_sprintf
handlers all added — none fired in mailbox_v1's printf path.

## What R9.1i delivered

### Full NID identification

Extracted the 118 unique NIDs from mailbox_v1.self's import
table and cross-referenced every one against
`rpcs3-upstream-clean/rpcs3/Emu/Cell/PPUFunction.cpp`. All 118
mapped to known function names. The full breakdown by family:

- **Memory mgmt** (16): _sys_heap_*, _sys_malloc, _sys_memalign,
  _sys_memchr/cmp/cpy/move/set, sys_mempool_*, sys_mmapper_*
- **String / libc** (12): _sys_strcat/chr/cmp/cpy/len/ncasecmp/
  ncat/ncmp/ncpy/rchr, _sys_tolower, _sys_toupper, _sys_qsort
- **Printf family** (7): _sys_printf, _sys_vprintf, _sys_sprintf,
  _sys_snprintf, _sys_vsnprintf, _sys_vsprintf, console_putc,
  console_write
- **Sync primitives** (10): sys_lwcond_*, sys_lwmutex_*,
  sys_spinlock_*
- **Process/thread** (13): _sys_process_at*, sys_ppu_thread_*,
  sys_process_exit, sys_process_is_stack, sys_initialize_tls,
  sys_interrupt_thread_disestablish
- **PRX modules** (13): sys_prx_*
- **SPU** (12): sys_spu_*, sys_raw_spu_*, _sys_spu_printf_*
- **Game system** (8): sys_game_*
- **Other** (27): sys_time_get_system_time, sys_get_random_number,
  etc.

### Handler implementations (R9.1g + R9.1h + R9.1i)

20 NID-specific handlers + 6 syscall-specific arms added:

NID-keyed (in `dispatch_syscall`'s stub-region branch):
- `0xe6f2c1e7` sys_process_exit → terminates run
- `0x8c2bb498` sys_spinlock_initialize → zeros *lock
- `0xa285139d` sys_spinlock_lock → no-op
- `0x5267cb35` sys_spinlock_unlock → no-op
- `0x4643ba6e` sys_mmapper_unmap_memory → success
- `0x409ad939` sys_mmapper_free_memory → success
- `0x2f85c0ef` sys_lwmutex_create → zeros *lock
- `0x1573dc3f` sys_lwmutex_lock → no-op
- `0xb257540b` sys_mmapper_allocate_memory → writes unique id
- `0xebe5f72f` sys_spu_image_import → parses SPU ELF + captures
- `0xe66bac36` console_putc → TTY ch3
- `0xf57e1d6f` console_write → TTY ch3
- `0xfa7f693d` _sys_vprintf → mini_printf → TTY ch3
- `0x06574237` _sys_snprintf → buffer
- `0xa1f9eafe` _sys_sprintf → buffer
- `0x9f04f7af` _sys_printf → mini_printf → TTY ch3
- `0x791b9219` _sys_vsprintf → buffer
- `0x526a496a` write (newlib) → TTY ch3
- `0xe3cc73f3` puts → TTY ch3 + newline
- `0xc01d9f97` printf (newlib) → mini_printf → TTY ch3

Syscall-keyed:
- `#172` sys_spu_thread_initialize: captures args
- `#173` sys_spu_thread_group_start: runs SPU via spu-interpreter
- `#178` sys_spu_thread_group_join: writes cause + captured status
- `#330` sys_mmapper_allocate_address: writes valid base
- `#403` sys_tty_write: writes payload to TTY buffer
- `#803` sys_fs_write: routes fd=1/2 to TTY ch3
- `#809` sys_fs_fstat: returns S_IFCHR mode for fd 1/2

### Empirical R9.1i smoke trace

Comprehensive smoke trace (post-R9.1i):

1. sys_spinlock_initialize × 2 (zero 4 bytes at each *lock)
2. sys_spinlock_lock/unlock (lwmutex + spinlock pairs)
3. sys_mmapper_allocate_memory (heap setup) + #337 map
4. sys_spu_image_import (captures SPU image)
5. sys_spu_thread_initialize (args = all zero — PSL1GHT passes
   args via struct in r6 not directly)
6. sys_spu_thread_group_start (no SPU image captured properly
   because struct decoding incomplete)
7. sys_spu_thread_group_join: cause=1, status=0
8. **sys_fs_fstat(fd=1)** at CIA 0x1129c — PSL1GHT's printf
   path probes stdout
9. **(NO write/printf/console NID fires)** — printf doesn't
   route through any external syscall/import
10. atexit cleanup (mmapper_unmap + free, spinlock pairs)
11. sys_process_exit → terminates run

TTY remains empty: `tty_ch3=Some("")`.

## Honest blocker

PSL1GHT's printf in mailbox_v1.self routes through
**statically-linked newlib code** in the binary itself,
NOT through any external SPRX import we can intercept.

After fstat(fd=1) returns S_IFCHR mode, newlib's stdio decides
the buffering policy and calls `__syscalls.write(fd, buf, len)`.
The `__syscalls` table is populated by `__syscalls_init`
(a `__constructor__(104)` function in PSL1GHT crt1.c) during
`_init()` → `.init_array` walk.

Two failure modes possible:

1. **`_init()` didn't actually walk `.init_array`**, so
   `__syscalls_init` never ran, and `__syscalls.write` is NULL.
   Newlib's stdio sees NULL function pointer and silently
   bails. Verifying requires tracing PSL1GHT's `_init()`
   prologue + checking whether the .init_array offset
   resolution works under our R9.1b FD-deref scheme.

2. **`_init()` ran but newlib's stdout FILE struct didn't
   initialize**. Newlib's `__sinit()` is called lazily on first
   stdio op, and it depends on lv2 syscalls returning sane
   values. If any return value is wrong, `__sinit()` may set
   stdout to NULL or invalid state.

Both failure modes require **PSL1GHT static newlib trace**
which is beyond syscall/import interception.

## Recommended next investigation (offline)

1. Disassemble PSL1GHT main() at vaddr 0x10848-ish in the binary
   and identify the EXACT post-fstat instruction sequence —
   that tells us what printf actually does after the fstat call.

2. Inspect `.init_array` section of the binary (offset can be
   computed from the .opd/PHDRs) and verify constructors are
   correctly listed.

3. Possibly trace `_init()`'s prologue — what does it actually
   do? If it bails before walking constructors, that's the
   root cause.

## Validation status (preserved across all R9.1.x slices)

- cargo test --workspace --tests --release: 264 result blocks,
  0 failures. ZERO regression maintained.
- 20 SPU oracle replay tests still green.
- Behavior-freeze contract preserved.

## Commits in R9 wave

```
a854e431e  r9.1h slice 2-4: PSL1GHT sysPrxForUser import handlers + import dump
b2558fe5f  r9.1h: handoff doc — autonomous loop paused (R9.1h slice 1)
b4b764906  r9.1h: NID-specific stubs for sys_spinlock + sys_mmapper imports
9794837ff  r9.1g.10: wire SPU execution into sys_spu_thread_group_start
... (earlier R9.1g.* slices)
```

R9.1i ships:
- Comprehensive 7-NID stdio family handlers
- Cross-referenced full 118-NID import table (this doc)
- sys_fs_fstat, sys_fs_write syscall arms
- Honest scope assessment: deeper investigation needs
  static-newlib trace inside the binary itself

R9 wave: SPU stack architecturally integrated, PPU executes
PSL1GHT main() end-to-end through SPU lifecycle, but printf
output remains buffered in statically-linked newlib state
that's never flushed to TTY.
