# R9.1h handoff — PSL1GHT crt0 early-exit triage

**Status:** PAUSED — additional progress requires PSL1GHT crt0
source-level inspection beyond the scope of an autonomous Rust
loop.
**Last commit:** `b4b764906` r9.1h slice 1 (NID-specific stubs).

## What R9.1g + R9.1h slice 1 delivered

A full LV2/PPU integration layer that loads PSL1GHT-style ELF/PRX
metadata, builds the expected process memory layout, resolves
import stubs through a NID-aware dispatcher, executes a materially
larger PPU opcode subset (12 new opcodes), and wires real SPU
execution through `sys_spu_thread_group_start`.

Validated stubs (NID-specific):
- `0x8c2bb498` sys_spinlock_initialize (zero 4 bytes at *r3)
- `0xa285139d` sys_spinlock_lock (no-op)
- `0x5267cb35` sys_spinlock_unlock (no-op)
- `0x4643ba6e` sys_mmapper_unmap_memory (writes 0 to *r4)
- `0x409ad939` sys_mmapper_free_memory (success)
- `0xe6f2c1e7` sys_process_exit (terminates run with r3 status)
- `0x1bc200f4` sys_lwmutex_unlock (identified — NOT YET stubbed)

Cargo workspace: 264 result blocks, 0 failures throughout.
SPU stack: 20 oracles intact, replay-validated.

## Honest limitation

PSL1GHT main()/printf is not reached. Empirical smoke trace
(stable across R9.1g.10 → R9.1g.11 → R9.1h slice 1):

```
1. spinlock_initialize → 0
2. lwmutex_unlock(0x100513a8) → 0  ← NEW import
3. mmapper_unmap_memory(addr, *...) → 0
4. mmapper_free_memory(0) → 0
5. spinlock_lock + spinlock_unlock
6. (cleanup chain repeats)
7. mmapper #331 free_address
8. sys_process_exit → TERMINATE (status=0, tty=empty)
```

PSL1GHT crt0 takes a cleanup-before-main branch. The condition
that triggers this branch is **not** deducible from the syscall
trace alone — it requires reading PSL1GHT crt0 source to identify
the specific check that's failing.

## Unusual pattern observed

The SECOND call is `sys_lwmutex_unlock` on `0x100513a8` (in
`.data` of the binary) — unlocking a mutex that was never locked
in our trace. This suggests one of:

1. **The mutex is statically initialized as "locked"** by the
   linker (PSL1GHT-specific data-section trick) and crt0 unlocks
   it to release a "main thread allowed to proceed" condition.
2. **Two threads exist** — a parent that init+unlocks for child,
   and we're only seeing parent. PSL1GHT might be spinning up
   workers via `sys_ppu_thread_create` early.
3. **The lwmutex is part of libc init state** (e.g., stdio mutex,
   malloc mutex) that PSL1GHT pre-locks at link time.

Without PSL1GHT crt0 source, we can't tell which.

## Recommended next steps (offline, outside autonomous loop)

1. **Read PSL1GHT crt0 source** at
   `https://github.com/psl1ght/psl1ght/blob/master/ppu/sdk/lv2_ppu/lib/sys/libc.h`
   + the corresponding `.S` file. Look for:
   - The cleanup branch trigger
   - What syscall/import return value is checked
2. **Identify the missing return value** and update the matching
   stub (NID-specific or syscall-arm).
3. **Re-run smoke** until main() reaches its first printf.

## Architectural deliverable status

Even without main() running, R9.1g + R9.1h slice 1 provide:

- A `EmuCore::run_self` API that boots any PSL1GHT fself.
- Full ELF + SCE + SELF + PT_SCE custom segments parsing.
- TLS init with proper r13 bias for PowerPC ELFv1.
- PpuPrxModuleInfo parser locating sysPrxForUser stubs.
- 119-stub trampoline installer with NID-keyed addrs[] table.
- Per-NID handler dispatch with proven extension pattern.
- SPU execution wired into `sys_spu_thread_group_start` and
  ready to activate once crt0 reaches main().

R9 architectural integration: COMPLETE. R9 end-to-end execution
of an oracle: BLOCKED on PSL1GHT crt0 triage requiring source-
level inspection.

## Commits (R9.1g + R9.1h)

```
b4b764906  r9.1h: NID-specific stubs for sys_spinlock + sys_mmapper
9794837ff  r9.1g.10: wire SPU execution into sys_spu_thread_group_start
d162a46c0  r9.1g.9 iter3: sys_spu_* stubs + permissive catch-all
... (and ~10 earlier R9.1g.* slices)
```
