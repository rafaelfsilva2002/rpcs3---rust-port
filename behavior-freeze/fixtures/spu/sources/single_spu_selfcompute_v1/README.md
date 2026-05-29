# single_spu_selfcompute_v1

A **self-contained** SPU homebrew fixture purpose-built to validate the SPU
JIT backend (`RecompilerExecutor`) **end-to-end through `EmuCore::run_self`**.

## Why this fixture exists

`EmuCore::run_self` runs an SPU thread group **synchronously** at
`sys_spu_thread_group_start` — the SPU runs to completion before the PPU
continues. Consequently the existing SPU oracles do **not** boot cleanly
through it:

- **Mailbox / signal / branch-loop oracles** read `IN_MBOX` and block waiting
  for a PPU push that, in real hardware, happens concurrently *after*
  `group_start`. In the synchronous model that push never arrives mid-run →
  the SPU stalls (`ChannelStall`) → `run_self` errors.
- **DMA oracles** need live EA↔LS transfer that the standalone SPU
  interpreter (and the `SpuProgram`-based `RecompilerExecutor`) does not
  perform without the C++ runtime bridge.

This fixture sidesteps both: the SPU takes **no input** (no `IN_MBOX`, no DMA,
no thread args), runs a fixed compute loop, writes the result to `OUT_MBOX`,
and halts. That is the one shape that boots cleanly through `run_self`.

## Behaviour

1. PPU: `sysSpuInitialize` → `sysSpuImageImport` → `sysSpuThreadGroupCreate`
   → `sysSpuThreadInitialize` → `sysSpuThreadGroupStart`.
2. SPU: computes `sum(1..=1000) = 500500 = 0x0007A314` (a `volatile`-bounded
   loop so it runs at runtime, not folded), writes it to `OUT_MBOX`, halts
   with `stop 0x101`.
3. PPU: `sysSpuThreadGroupJoin` reads `OUT_MBOX` as the exit status, then
   returns `0xC0DE` iff `status == 0x0007A314` (else the raw status).

Expected SPU `OUT_MBOX` (captured as `EmuCore::spu_exit_status`):
**`0x0007A314`**, identical under both the interpreter and the Cranelift JIT
backend (`EmuCore::spu_backend`).

> NOTE: the PPU *process* exit code is **not** reliable through `run_self`
> here — PSL1GHT's `sysSpuImageClose` / newlib teardown reaches an
> unimplemented `sysPrxForUser 0xe0da8efd` import that perturbs the exit
> code (it lands as `1`, not the `main()`-returned `0xC0DE`). So the test
> asserts on the SPU's OUT_MBOX, which is exactly the value each backend
> produced — the thing this fixture exists to compare. Making the PPU exit
> code faithful is a separate emu-core follow-up (implement
> `sysSpuImageClose` + the newlib exit import).

## Build

Via the PSL1GHT Docker toolchain (`rpcs3-ps3dev-toolchain:local`), from this
directory:

```
MSYS_NO_PATHCONV=1 docker run --rm \
  -v "<literal-windows-path-to-this-dir>:/work" \
  -w /work rpcs3-ps3dev-toolchain:local bash -lc 'make'
```

Produces `single_spu_selfcompute_v1.self`. The `.self`/`.elf`/`.bin`
artifacts are gitignored (path-lock hook) and built locally.

## Consumed by

`rust/rpcs3-emu-core/tests/spu_selfcompute_jit.rs` — boots this `.self` via
`EmuCore::run_self` under both `SpuBackend::Interpreter` and (feature
`spu-recompiler`) `SpuBackend::Recompiler`, asserting both reach `0xC0DE`.

CC0 1.0 (public domain) — see LICENSE.md.
