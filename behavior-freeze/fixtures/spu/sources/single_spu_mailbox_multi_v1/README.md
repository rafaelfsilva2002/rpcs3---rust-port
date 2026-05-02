# single_spu_mailbox_multi_v1 — R6.4b stall-bound oracle fixture

**Status (R6.4b-pre):** sources authored; `.self` not yet built
because the PSL1GHT toolchain is unavailable in the current
development environment. Equivalent acceptance gate landed as a
synthetic FFI test (`rust_spu_mailbox_multi_round_via_ffi` in
`rust/rpcs3-spu-ffi/src/tests.rs`). See "Build status" below.

## Why this fixture exists

The four R5.11 / R5.11b oracle fixtures
(`single_spu_mailbox_v1`, `single_spu_branch_loop_v1`,
`single_spu_loadstore_v1`, `single_spu_signal_v1`) all reach
`Stop` in a single Rust executor call without parking. They
proved the R6.2 / R6.3a-c bridge can delegate single-shot SPU
workloads, but they do **not** exercise the persistent-handle
re-entry path that R6.4b will add.

This fixture is the first oracle designed to **force** a
`StallRead` outcome between the SPU's first and second
`rdch ch29` reads, with a guaranteed PPU-side write between them.
The execution sequence cannot be performed by a stateless bridge
without either:

1. Falling back to the C++ executor entirely (which means the
   delegation never happens — defeats the bridge), or
2. Replaying the first OUT_MBOX write twice (which corrupts the
   PPU-visible group exit status).

The honest answer is a **persistent `rust_spu_t*` handle** that
survives across `cpu_task` re-entries. R6.4b will implement this.

## Behaviour (post-R6.4b-toolchain redesign)

PSL1GHT exposes `sysSpuThreadWriteMb` (PPU→IN_MBOX) and
`sysSpuThreadWriteSignal` (PPU→SNR1/SNR2) but **not** a public
syscall to read a cooperative SPU's OUT_MBOX. So the original
"OUT_MBOX drain between rounds" design is not expressible
end-to-end via PSL1GHT. The fixture now uses **IN_MBOX for round
1 and SNR1 for round 2**, which produces the same load-bearing
behaviour (a guaranteed SPU stall between two PPU-pushed values)
within the supported syscall surface.

```
PPU                                    SPU (Rust executor)
─────────────────────────────────────  ──────────────────────────────────
sysSpuThreadWriteMb(0x100)
  → ch_in_mbox = [0x100]
                                       cmd1 = rdch ch29 → 0x100
                                       partial = 0x100 + 0xA1 = 0x1A1 (in r3)
                                       cmd2 = rdch ch3 → STALL (SNR1 empty)
sysSpuThreadWriteSignal(slot=0, 0x200)
  → ch_snr1 = 0x200
                                       (resume)  cmd2 = rdch ch3 → 0x200
                                       reply = (cmd2 + 0xB2) + partial
                                             = 0x2B2 + 0x1A1 = 0x453
                                       wrch ch28, 0x453
                                       stop 0x101
sysSpuThreadGroupJoin()
  → cause=0x1, status=0x453
```

`status = 0x453` is observable IFF the SPU consumed BOTH inputs
in order. A buggy stateless bridge that drops the handle on the
first StallRead and replays the program from PC=0 after refilling
SNR1 would compute `reply = 0x2B2` (with `partial = 0` because
round 1 never ran) — a detectable corruption.

## Expected outputs (when `.self` is built and run under RPCS3)

| Mode | TTY |
|---|---|
| Bridge OFF (C++ executor) | `[mbmulti_v1] OK cause=0x1 status=0x453` |
| Bridge ON, R6.4a (stateless) | Falls back to C++ on first stall; TTY same as OFF; bridge log shows one `StallRead on SNR1 (ch3)` warning |
| Bridge ON, R6.4b (persistent handle) | Delegates fully via the multi-round loop; one `DELEGATED EXECUTION OK` line with `stall_iters=1`; TTY same as OFF |

## Build status

The PSL1GHT toolchain (`spu-gcc`, `powerpc64-ps3-elf-gcc`,
`bin2s`, `fself`, `sprxlinker`) is **not installed** on the
current development host (Windows 11 + Git Bash; checked
`$PS3DEV`, `$PSL1GHT`, `which spu-gcc`, etc.). The sources are
checked in and the Makefile mirrors the other R5.11 fixtures
exactly, so building is a one-line invocation on a properly-
provisioned host:

```bash
cd behavior-freeze/fixtures/spu/sources/single_spu_mailbox_multi_v1
make
# Produces build/single_spu_mailbox_multi_v1.self
```

When that environment is available, the standard capture flow
applies:

```bash
RPCS3_SPU_TRACE_FILE=behavior-freeze/fixtures/spu/traces/single_spu_mailbox_multi_v1.jsonl \
  rpcs3 --headless build/single_spu_mailbox_multi_v1.self
```

The captured trace can then drive a new replay test in
`rust/rpcs3-spu-recompiler/tests/single_spu_mailbox_multi_v1_replay.rs`
following the R5.9e.7 pattern.

## R6.4b acceptance gate (currently active)

Until the `.self` is built, the **executable** acceptance gate
for R6.4b lives in
`rust/rpcs3-spu-ffi/src/tests.rs::rust_spu_mailbox_multi_round_via_ffi`.
That test loads a hand-encoded SPU program with the same
behavioural shape (rdch / wrch / rdch / wrch / stop), pushes the
first IN_MBOX value, runs to `StallRead`, drains OUT_MBOX, pushes
the second IN_MBOX value, and resumes the SAME `rust_spu_t*`
handle to completion. Any R6.4b implementation that handles
real-PPU-side stalls correctly should also pass this test
deterministically; the test is the FFI-level dual of what the
fixture exercises end-to-end through the RPCS3 binary.
