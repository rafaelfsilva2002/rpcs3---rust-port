# R9 — LV2 / PPU integration: drive existing SPU oracles end-to-end from Rust

**Status:** PLAN (drafted 2026-05-23 post R8.5e wave closure).
**Authority:** This document is the planning artifact for the
strategic pivot recommended in
[`docs/PROJECT_STATUS.md` § 9.4](../docs/PROJECT_STATUS.md) and
[`docs/SPU_DMA_MFC_R6_7_DESIGN.md` § 20.8](../docs/SPU_DMA_MFC_R6_7_DESIGN.md).
**Not yet approved.** Treat as a proposal until the user
acknowledges direction.

## 1. Why R9 now

The R8.5 wave closed with 20 oracles. R8.5e's empirical
result — PUTL stall (20th oracle) passed first-run with
**zero new architectural code** — confirmed that further
depth-only work on the SPU MFC stack has saturated. The
20 existing oracles' SPU images are byte-exact
reproductions of CC0 homebrews, but the project today still
needs **real RPCS3 (C++)** to:

1. Load the PPU `.self` binary.
2. Run the PPU `main()` that calls `sysSpuInitialize`,
   `sysSpuImageImport`, `sysSpuThreadGroupCreate`,
   `sysSpuThreadInitialize`, `sysSpuThreadGroupStart`,
   `sysSpuThreadGroupJoin`.
3. Marshal arguments into SPU registers and start execution.
4. Read OUT_MBOX from the joined group, print canonical TTY.

The Rust port covers SPU execution end-to-end (interpreter +
recompiler + bridge runtime + replay). The R9 wave's goal is
to push the wave-front higher up the stack so the **20 existing
fixtures become end-to-end integration tests in Rust without
RPCS3**.

## 2. Current state inventory (audited 2026-05-23)

### 2.1 What's already implemented

| Crate | LOC | Tests | Status |
|-------|-----|-------|--------|
| `rpcs3-ppu-interpreter` | 3,862 | 136 | Substantially implemented (instruction decode + execute) |
| `rpcs3-ppu-thread` | 406 | 19 | Thread state + PC/CR/LR/XER |
| `rpcs3-ppu-opcodes` | 699 | 25 | Opcode enums + bit-extraction helpers |
| `rpcs3-spu-interpreter` (R5+) | ~5,200 | ~150 | Full SPU ISA executor used by all 20 oracles |
| `rpcs3-spu-recompiler` (R6+) | ~1,000 | 22 (incl. 20 oracle tests) | LLVM-equivalent JIT scaffolding |
| `rpcs3-spu-thread` | 2,080 | 49 | SPU thread + channels (MFC + mailbox + signal + ch25/26 stall) |
| `rpcs3-spu-differential` (R5.6+) | ~3,500 | ~100 | Trace parser + replay state machine + dma chunk + listdesc resolver |
| `rpcs3-lv2-process` | 363 | 14 | `sys_process_*` syscalls |
| `rpcs3-lv2-spu-group` | 572 | 21 | `sys_spu_thread_group_*` syscalls |
| `rpcs3-loader-elf-self` | 570 | 14 | `.self`/`.elf` parsing (SCE header strip + ELF load) |
| `rpcs3-vfs-mount` | 271 | 13 | VFS mount table |
| `rpcs3-vfs-paths` | 263 | 17 | Path resolution (host ↔ PS3 paths) |
| `rpcs3-emu-core` | 757 | 14 | Integrates LV2 + PPU thread + SPU group registry |

Plus ~20 other LV2 syscall crates (mutex, sema, cond, event,
event-flag, rwlock, fs, gpio, dbg, …) — each independently
ported and tested.

### 2.2 Gaps blocking end-to-end execution

1. **`rpcs3-ppu-decoder`** — directory exists in cargo workspace
   but `src/` is missing. Decoder is needed to drive
   `ppu-interpreter`. If `ppu-interpreter` already inlines decode,
   the decoder crate may be vestigial (verify).
2. **PPU entry → `main()` invocation** — `rpcs3-ppu-thread` has
   thread state; `rpcs3-ppu-interpreter` has the executor.
   The PSL1GHT lv2 process loader's behavior (load ELF, set up
   `sys_initialize_tls` / `_init_libsysmodule` / `_init_libsysutil`
   stubs, jump to `_start`) is not yet wired. Need a
   `rpcs3-lv2-loader` integration step.
3. **PPU syscall dispatch** — `rpcs3-emu-core` shows the `match number`
   dispatcher with arms for `sys_process_getpid`/sdk_version/etc.
   Syscall coverage for the SPU thread group lifecycle path
   (`sysSpuInitialize` → `sysSpuImageImport` → `sysSpuThreadGroupCreate`
   → `sysSpuThreadInitialize` → `sysSpuThreadGroupStart`
   → `sysSpuThreadGroupJoin`) needs end-to-end verification —
   each call individually has unit tests but the **chained
   lifecycle** has not been exercised against a real PPU binary.
4. **Memory / VM** — PPU code reads from EA addresses; SPU's
   `vm::_ptr<u8>(ea)` needs an equivalent Rust-side memory map.
   `rpcs3-vfs-mount` covers FS paths; **EA → host pointer** mapping
   (the 0x10000200..0x10000300 range our fixtures use) needs
   audit / scaffolding if not already there.
5. **PPU `printf` / stdout** — fixtures end with
   `printf("[fixture] OK ...")`. The integration test needs to
   capture this output to verify canonical TTY. This likely
   means stub-ing `sys_tty_write` and similar.
6. **SPU thread args marshaling** — when PPU calls
   `sysSpuThreadInitialize` with `thread_args.arg0/arg1/arg2/arg3`,
   the lv2 path stores them into `SpuThread` initial r3/r4/r5/r6.
   `rpcs3-lv2-spu-group` likely handles this; needs verification
   against a real run.
7. **Bridge / Interpreter mode selection** — for integration
   tests we want **Interpreter (static)** SPU execution path
   (matches our 20 oracles' replay-validated semantics). The
   bridge default-OFF policy stays; SPU runs via
   `rpcs3-spu-interpreter` directly.

### 2.3 Out of scope for R9

- Recompiler (LLVM-JIT) path for PPU. Use interpreter only.
- Graphics (RSX). The 20 oracles are headless — no RSX touches.
- Audio. Same — no audio touches.
- Multi-PPU-thread coordination. The 20 fixtures use a single
  PPU thread + an SPU thread group with 1 SPU. Multi-PPU defers
  to a future phase.
- Recovery of commercial titles. CC0 fixtures only.
- SPURS / v4 / atomics / sync cmds / PUTRL / multi-SPU. Already
  deferred in § 9.3.

## 3. R9 wave decomposition

**Total estimated scope: 4-6 slices**, each shippable as a
single commit and validated against existing fixtures.

### R9.1 — Audit + first slice: single mailbox oracle

**Goal:** drive `single_spu_mailbox_v1.self` (the 1st oracle,
simplest fixture — IN_MBOX/OUT_MBOX only, no MFC DMA) end-to-end
through the Rust stack. Capture the printf output and verify
it matches the canonical TTY string.

**Pre-work (audit phase, no code):**
- Verify `rpcs3-ppu-decoder` status (vestigial or real gap?).
- Trace the PSL1GHT entry sequence: from `_start` to `main()`,
  what TLS / module init does the lv2 loader stub?
- Identify the minimal set of syscalls the mailbox_v1 fixture
  actually invokes (`sys_tty_write`, `sysSpuInitialize`,
  `sysSpuImageImport`, …) and verify each has a Rust counterpart.
- Identify EA → host memory mapping (where does `static u8
  ea_buf1[128]` get its address from? What mapping must Rust
  set up?).

**Implementation:**
- New crate `rpcs3-app-runner` (or integrate into `rpcs3-emu-core`)
  with a `run_self(self_path: &Path) -> Result<RunReport>` entry.
- The runner: load `.self` via `rpcs3-loader-elf-self`, set up
  PPU thread + memory, dispatch lv2 syscalls, run PPU interpreter
  to completion, return captured stdout.
- Integration test: load `single_spu_mailbox_v1.self`, assert
  output matches `[mailbox_v1] OK status=0x453` (or whatever the
  canonical TTY is).

**Acceptance:**
- `cargo test -p rpcs3-app-runner --test mailbox_v1_end_to_end` green.
- The test exercises: ELF parse, PPU interp, lv2 syscall dispatch,
  SPU thread group lifecycle, SPU interpreter, OUT_MBOX read,
  printf capture.
- All 20 existing replay oracles still green (R9.1 must not
  regress anything).

**Risk:**
- High. This is the first time many subsystems are exercised
  together. Expect "PPU at PC 0x... reads from unmapped memory"-
  class issues. Slice scope is intentionally minimal (1 fixture,
  no DMA) to surface integration bugs cheaply.

### R9.2 — Extend to DMA fixtures (GET / PUT)

**Goal:** drive `single_spu_dma_get_v1.self` + `single_spu_dma_put_v1.self`
end-to-end. These add the EA buffer setup + post-join EA reads
(for PUT) to the mailbox path.

**Key new requirement:** RPCS3 EA → host memory mapping must
work in both directions:
- PPU sets up `static u8 ea_buf[128]` at some PPU-VM address.
- PPU passes the address to SPU.
- SPU dispatches a DMA against that EA.
- Bridge / `vm::_ptr<u8>(ea)` resolves it back to host pointer.
- Both ends agree on the bytes.

**Implementation:** wire `rpcs3-vm` (if exists) / equivalent
into the runner; verify the existing SPU bridge's
`vm::_ptr<u8>` calls work transparently.

**Acceptance:** GET + PUT fixtures pass end-to-end.

### R9.3 — List-DMA family (GETL / PUTL / variants)

**Goal:** drive the 6-code list-DMA oracles end-to-end.
Likely just exercise + verify; no new code beyond what R9.2
gives.

### R9.4 — Stall fixtures (GETL stall + PUTL stall)

**Goal:** drive `getl_stall_v1` + `putl_stall_v1` end-to-end.
Exercise the ch25/ch26 handshake path through the full
Rust stack.

### R9.5+ — Optional: triple-symmetric promotion of stall oracles

Once R9.4 lands, the stall fixtures gain bridge-ON triple
symmetry. Promotes them to the same triple-symmetric standard
as the 6-code list-DMA family.

## 4. R9 hard rules

- **The 20 existing oracle tests stay green at every phase
  boundary.** Replay-only validation remains the source of truth
  for the SPU stack's correctness. R9 adds new end-to-end tests
  on TOP — it does not modify any existing oracle test.
- **No fake stdout.** The integration test must capture the
  PPU's actual `printf` output via stubbed `sys_tty_write` (or
  equivalent), not a hand-written expected string.
- **No fake EA mapping.** Memory addresses come from a real
  `rpcs3-vm` allocator that mirrors lv2's behavior.
- **No fake syscall dispatch.** When the PPU calls
  `sysSpuThreadGroupCreate`, it goes through the actual
  `rpcs3-lv2-spu-group::sys_spu_thread_group_create`.
- **Interpreter path only** for SPU during R9 integration tests
  (recompiler / bridge work stays optional). This matches the
  capture decoder mode used in R8.x.
- **CC0 only.** The 20 existing CC0 binaries are the test
  corpus. No commercial code enters R9 scope.
- **Behavior-freeze contract remains active.** R9 produces no
  new `.dmachunk` / `.dmalistdesc` / `.spuimg` / `.self` — it
  only adds test code and possibly new lv2 syscall implementations.

## 5. Open questions (block until answered)

1. **Is `rpcs3-ppu-decoder` a real gap?** Or is `ppu-interpreter`
   self-contained? Verify before R9.1 audit phase.
2. **EA / VM model.** Does a `rpcs3-vm` crate exist? If not,
   what's the current pattern for PPU-side EA allocation in tests?
3. **PPU entry sequence.** What does PSL1GHT's `_start` actually
   do before calling `main()`? Need to inventory the TLS / module
   init stubs.
4. **printf path.** Is `sys_tty_write` (or equivalent) ported?
   If yes, where is the output captured for tests today?
5. **Scope of `rpcs3-emu-core`.** It already integrates many
   pieces — can the runner live there, or does it need a new
   `rpcs3-app-runner` crate?

## 6. Next action

**Recommended:** answer the open questions in § 5 via
read-only audit, then ship R9.1 audit doc as a separate
commit before any code lands. R9.1's "audit phase" deliverable
is a follow-up planning doc with the gaps mapped concretely.

**Alternative:** if user wants to ship R9 incrementally with
verification at each step, start R9.1 audit phase as the
first concrete action.

This plan itself is the **first deliverable of the R9
strategic pivot.** No code, no fixture changes, no SHA bumps.
