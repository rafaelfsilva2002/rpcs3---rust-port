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

## R13.1 — first concrete target (located 2026-05-27)

Disassembly of the fault: at CIA `0x12784` rsxInit executes
`std r8, 0(r9)` with **r9 = 0** — a null store. r9 should hold a
pointer that an earlier `cellGcmSys` import was supposed to produce.

The probe's import-stub map shows rsxInit called two cellGcmSys PRX
imports, both returning 0 via R9's unimplemented-import fast path
(this path ignores `permissive_unknown_syscalls`):

| NID | trampoline | args at call | likely fn |
|---|---|---|---|
| `0x15bae46b` | 0xd0010000 | r3=0x10300000, r4=0x10000 | **cellGcmInitBody** (context, cmdSize, …) |
| `0xe315a0b2` | 0xd0010030 | r3=0x10300014, r4=0x10000 | second init call (config/default-cmd-buffer) |

So R13.1 = implement these two NID handlers in `EmuCore`:
`cellGcmInitBody` must set up the `gcmContextData` (begin/end/
current/callback — layout known from R12.11b), allocate the command
buffer + IO region in guest memory, and populate the config +
control-register structs so rsxInit's subsequent stores land on a
valid pointer (r9 != 0). Then re-run → next gap.

Note: the cellGcmSys functions ARE PRX NID imports (resolved via R9's
import-stub mechanism), not static — the `rsx*` wrappers are static
but they call through to these `cellGcm*` PRX exports.

Risk: faithfully reproducing the exact struct layouts + memory
addresses PSL1GHT expects is iterative — a wrong field offset →
another downstream fault. Each re-run pins the next.

## R13.1 status — RESOLVED 2026-05-27 (init unblocked; uncommitted)

Both walls fell: the clean RPCS3 source IS in the tree at
`rpcs3/Emu/Cell/Modules/cellGcmSys.cpp`, and a host-side Python
PPC64-BE decoder (reads the `.elf` directly) replaced the broken
Docker→objdump capture for the RE.

**Root cause (RE-confirmed, no guessing):**
1. The second cellGcm NID `0xe315a0b2` = **`cellGcmGetConfiguration`**
   — proved by reimplementing RPCS3's NID hash
   (`SHA1(name + suffix)[..4]` little-endian, `PPUModule.cpp:55`) and
   matching it across all 100 cellGcmSys names (`_cellGcmInitBody`
   → `0x15bae46b` was the sanity check). NOT GetControlRegister (the
   earlier guess, correctly reverted).
2. The faulting routine at `0x126c0` (called from rsxInit at
   `0x10858`) is PSL1GHT's **local-memory pool allocator**, not a
   cellGcm consumer. The caller (`0x10810`-`0x10858`) does:
   `bl 0x26650` (= cellGcmGetConfiguration) → `lwz r4,0(r30)`
   (config.localAddress) + `lwz r5,8(r30)` (config.localSize) →
   `bl 0x126c0`. The allocator writes a free-block header at the
   base (`std r8,0(r9)`, r9 = localAddress) AND a boundary tag near
   the end (`stdx r31,r31,r9` ≈ localAddress + localSize − 16). With
   the config left zero, localAddress = 0 → null store at `0x12784`.

**Fix (uncommitted, `rust/rpcs3-emu-core/src/lib.rs`), all values
from the RPCS3 reference:**
- `cellGcmGetConfiguration` (0xe315a0b2): writes the 24-byte
  `CellGcmConfig` (GCM.h:12) to `*config` — localAddress 0xC0000000,
  ioAddress, localSize 0xf900000, ioSize, memoryFreq 650 MHz,
  coreFreq 500 MHz (cellGcmSys.cpp:436-441).
- `_cellGcmInitBody` (0x15bae46b): additionally backs the local
  video-memory region `[0xC0000000, +0xf900000)` (cellGcmSys.cpp:404-
  406 `vm::falloc(local_mem_base, local_size, vm::video)`) so the
  pool allocator's base + end-boundary writes land on real pages.

**Validated:** `rsx_init_probe` + new `rsx_gcm_init` test — fixture
runs rsxInit to completion, `return 0xC0DE` (exit 49374), NO null
store at 0x12784. Context (begin=0x10201000, end=0x10207ffc,
current=begin) + control (ref=0xffffffff) match cellGcmSys.cpp
exactly. Gate: `cargo test --workspace --tests --release` = 276
blocks, 0 fail; 20 SPU oracles intact.

**Capture status:** single_gcm_init_v1 emits NO commands (rsxInit
then return), so `[GET..PUT)` is empty by design — the test asserts
init + capture-wiring against the real context. A NON-EMPTY real-
libgcm capture is R13.2 below (Docker-gated; Docker unresponsive
this session). The decode/replay pipeline is already proven on real
PSL1GHT bytes by R12.11b.

## R13.2 — next: emitting fixture for non-empty real capture

`single_gcm_emit_v1` (source prepared, pending Docker build):
rsxInit (now works) → `rsxSetClearColor`/`rsxClearSurface` through
the **real** cellGcm-init'd context (inline writes into the io
buffer at begin=ioAddress+4096) → return 0xC0DE. Then a test reads
`[begin .. current)` from EmuCore memory (current advances as
libgcm emits) and `replay_gcm` → asserts `ClearSurface` from a REAL
full-gcm-path stream (vs R12.11b's manual context). Avoids
cellGcmFlush/SetFlip (new NIDs) by capturing from the context's
current pointer, not a flush. Build needs the PSL1GHT Docker
toolchain.

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
