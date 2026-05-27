# R13 — cellGcm HLE (gcmInitDefault + draw + flip)

**Status:** SCOPING (2026-05-27). The separate sub-wave flagged at
R12 closure: get a real PSL1GHT homebrew using the FULL gcm path
(rsxInit/gcmInitDefault → surface → clear → draw → flip) to run
through EmuCore and capture its command buffer for `replay_gcm`.
R12.11b deliberately avoided this (manual context) precisely
because gcmInitDefault needs broad HLE.

## Empirical scoping (R13 probe, 2026-05-27)

Built `behavior-freeze/fixtures/rsx/sources/single_gcm_init_v1`
(CC0): a minimal `rsxInit(&ctx, 0x10000, 1MB, host_buffer)` then
return. Ran it through `EmuCore::run_self` (strict syscall mode).

**Finding:** rsxInit does **not** fault on a `sys_rsx_*` syscall
first — it faults earlier:

```
Interpreter(Memory(MissingFlags { addr: 0, required: PageFlags(0x82) }))
```

i.e. a **null call / null deref** (addr 0, exec/read required). This
means rsxInit branches through a **cellGcmSys PRX function table that
EmuCore never populated** — the PRX module isn't loaded, so the
indirect call target is 0. This happens before any sys_rsx syscall.

## What this means for scope

R13 is **not** "implement ~10 sys_rsx syscalls." It is "make
PRX-module-based cellGcm calls resolve in EmuCore," which is the
broad RSX/PRX setup:

1. **PRX cellGcmSys load + function-table resolution.** rsxInit
   calls into libgcm_sys functions that are PRX exports (resolved at
   load via the module's export table). EmuCore's R9 import-stub
   mechanism handled sysPrxForUser NIDs; cellGcmSys is a *separately
   loaded* PRX whose function table the homebrew calls through. Need
   to either (a) HLE the cellGcmSys module load so its exports point
   at handlers, or (b) intercept the specific cellGcm functions
   rsxInit calls.
2. **sys_rsx_* lv2 syscalls** (device_open 668, memory_allocate 669,
   context_allocate 672, context_iomap 673, context_attribute 674,
   device_map 676, attribute 677...) — once the PRX layer resolves,
   these are the actual kernel calls cellGcmInit makes. Documented in
   RPCS3 `sys_rsx.cpp`.
3. **RSX memory model** — the command buffer + IO mapping cellGcmInit
   sets up, so the homebrew's inline command writes land somewhere
   EmuCore can read via R12.11a's `capture_command_buffer`.

## Proposed slice decomposition (R9.1g-style iterative)

- **R13.1** — make the cellGcmSys PRX call resolve (fix the null
  call). Identify the exact function rsxInit invokes first (disasm
  around the faulting CIA / the import table), HLE it. Re-run, find
  next gap.
- **R13.2..n** — iterate: each re-run surfaces the next unmet
  PRX call or sys_rsx syscall; implement; repeat until rsxInit
  returns a valid context.
- **R13.x** — once rsxInit completes: clear + draw + flip fixture,
  capture the command buffer (R12.11a path) at flush, `replay_gcm`,
  assert the draw call + clear from a REAL full-gcm stream.

## Honest risk

This mirrors the R9.1g grind (PSL1GHT runtime init): an unknown
number of discovery iterations, each gated on the next missing
PRX/syscall. Bounded in principle (the gcm init path is finite) but
not a quick win — likely many slices. The payoff: a real
full-pipeline gcm frame (init→draw→flip) captured and replay-
validated, vs R12.11b's minimal manual-context clear.

## Out of scope (unchanged)

GPU backend — shader decompile, texture pixel-decode, Vulkan/GL,
actual rendering (Camadas C/D/E). R13 is still pure command-stream
(produce real bytes via the full gcm path; decode via the frozen
pipeline). No pixels.
