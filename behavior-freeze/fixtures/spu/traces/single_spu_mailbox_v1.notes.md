# single_spu_mailbox_v1.notes.md

R5.9e.7 — first replay-validated SPU trace fixture. Captured
2026-04-29 from RPCS3 against a CC0 PSL1GHT homebrew authored for
this purpose, then replayed end-to-end (parser → per-SPU
transformer → SpuProgram builder → replay × InterpreterExecutor +
replay × RecompilerExecutor) with byte-identical agreement on the
final SpuStateSnapshot (PC, GPRs, LS, channels, park_state).

## Origem do homebrew

Autoral. CC0 1.0 (public domain). Source committed at
`behavior-freeze/fixtures/spu/sources/single_spu_mailbox_v1/` with
LICENSE.md. Two .c files (PPU `main.c` + SPU `spu/spu_mailbox.c`)
+ Makefile. Targets PSL1GHT runtime.

Comportamento (uma linha): PPU pushes one 32-bit command (0x100)
to SPU IN_MBOX; SPU adds 0x29 to produce reply (0x129); writes
reply to OUT_MBOX; halts via stop 0x101 (SYS_SPU_THREAD_STOP_GROUP_EXIT).

Race-free single-round (no DMA, no PPU spin-loop on OUT_MBOX) —
PSL1GHT cooperative-thread MMIO complications avoided by
deliberately not draining the mailbox from the PPU side.

## Toolchain

`ps3toolchain` built from source in a Docker `debian:bookworm-slim`
container (auth ai run-id `ps3-build`). Toolchain components and
versions per the upstream `ps3toolchain` ToT at capture time:

- binutils 2.43.1 (PPU + SPU)
- gcc 14.2.0 (PPU + SPU, freestanding)
- newlib 4.4.0
- PSL1GHT (latest commit at capture time)
- make_self / fself / sprxlinker / bin2s host tools

Build command (in container):

```
cd behavior-freeze/fixtures/spu/sources/single_spu_mailbox_v1
PS3DEV=/usr/local/ps3dev PSL1GHT=/usr/local/ps3dev/psl1ght make
```

SPU side compiled with `-O2 -Wall -nostartfiles -nostdlib
-Wl,--entry,main`. The `-nostartfiles -nostdlib` is load-bearing:
PSL1GHT's `crt0` + the `spu_thread_group_exit` library function
both pull in the SPU C runtime which contains ROTQBY / other
opcodes outside the iteration-1 SPU interpreter subset. Inlining
the exit (`spu_writech(SPU_WrOutMbox, reply); spu_stop(0x101);`)
keeps the produced binary entirely within the supported subset
(6 instructions, see decoded SPU code below).

## RPCS3 version + capture hooks

RPCS3 build: ToT from this repository at capture time, with the
R5.9c + R5.9e.3 SPU trace writer
(`rpcs3/Emu/Cell/SPUTraceJsonl.{h,cpp}` + integration hooks in
`SPUThread.cpp`, `SPUInterpreter.cpp`, `lv2/sys_spu.cpp`). No
custom patch applied at capture time beyond what was already
committed.

## Capture procedure

Driven by
`behavior-freeze/fixtures/spu/sources/single_spu_mailbox_v1/enable_autoexit_and_capture.cmd`
which:

1. Patches `R:\bin\config\config.yml` to set
   `Exit RPCS3 when process finishes: true` (without this the
   trace writer's destructor never runs and the JSONL stays
   0 bytes).
2. Cleans prior `%TEMP%\single_spu_mailbox_v1.jsonl` + side-file
   directories.
3. Sets `RPCS3_SPU_TRACE_JSONL=%TEMP%\single_spu_mailbox_v1.jsonl`.
4. Launches `rpcs3.exe --headless build\single_spu_mailbox_v1.self`.
5. Verifies trace JSONL + `<trace>.jsonl.images/<sha>.spuimg`
   side-file presence on exit.

Captured artifacts then staged in this repo:

- `behavior-freeze/fixtures/spu/traces/single_spu_mailbox_v1.jsonl`
- `behavior-freeze/fixtures/spu/images/68cf203b...abac43.spuimg`

(Centralized image dir per § F.4 of the F-series plan.)

## Trace contents (5 events)

```
seq 0: ppu_push_inmbox     target_spu=256 value=0x100
seq 1: spu_image           sha=68cf203b... load=0x0 size=0x40000 entry_pc=0x0
seq 2: spu_wrch  ch28=0x129 pc=8 target_spu=256
seq 3: spu_stop            stop_code=0x101 pc=12 target_spu=256
seq 4: final_state         gpr={r1=262128, r2=256, r3=297}
                           channels={in_mbox=null, out_mbox=null,
                                     out_intr_mbox=null, snr1=0, snr2=0}
```

Acceptance criteria (R5.9e.7 contract):

- ≥ 1 spu_wrch ch28 event       ✓ (1)
- 0 spu_wrch ch21 (MFC_Cmd)     ✓ (DMA-free)
- exactly 1 spu_image event     ✓
- exactly 1 target_spu          ✓ (256)

## Replay-validation

Drives the full pipeline from
`rust/rpcs3-spu-recompiler/tests/single_spu_mailbox_v1_replay.rs`:

```
parse_jsonl_trace
  -> captured_events_to_traces_per_spu
  -> build_spu_program_from_captured_image
  -> replay_per_spu_traces::<InterpreterExecutor>
  -> replay_per_spu_traces_with(|_| RecompilerExecutor::new())
  -> diff_snapshots(interp, jit).is_identical()
```

Status: ✅ parser ok / transformer ok / interp replay ok / JIT
replay ok / cross-backend snapshot diff identical.

`total_steps` differs across backends (interp=5, jit=9) — this is
expected: the JIT counts dispatcher iterations + JIT prefix
steps, the interpreter counts raw retired instructions. Step
count is internal accounting and is NOT part of the byte-identical
state contract (`diff_snapshots` excludes it; the test asserts
only that both report > 0).

## Engine-side fixes landed for this fixture

Three small replay-engine fixes were required to bridge the gap
between RPCS3-captured behavior and the deterministic Rust replay
model. All three are general — not single-fixture hacks — and stay
landed:

1. **Initial-state inference for the per-SPU transformer.**
   `infer_initial_state` in `trace_fmt.rs`: when the trace's first
   non-image event is a PPU action with no preceding `spu_park`,
   the transformer infers the matching `Parked{ChannelRead/Write}`
   state instead of defaulting to `Running`. RPCS3's writer omits
   the implicit initial park when the PPU writes a mailbox before
   the SPU has had a chance to run (race-free single-round case);
   the replay driver always step_spu's first and parks on the
   blocking channel op, so the transformer needs to expect `Ready`
   wake, not `NotParked`.

2. **lv2 stop-0x101/0x102 OUT_MBOX drain in the transformer.**
   `transform_single_spu_subset` in `trace_fmt.rs`: when an
   `SpuStop` event has `stop_code` = 0x101
   (SYS_SPU_THREAD_STOP_GROUP_EXIT) or 0x102
   (SYS_SPU_THREAD_STOP_THREAD_EXIT), inject a synthetic
   `PpuPopOutMbox{expect: None, expect_wake: None}` event after
   `ExpectSpuFinished`. This mirrors the architectural
   side-effect: the lv2 kernel reads OUT_MBOX as the group/thread
   exit status, which the captured `final_state` reflects but the
   raw SPU interpreter does not.

3. **Initial GPR overrides on `SpuProgram`.** New optional
   `initial_gpr_overrides: Vec<(u8, u128)>` field with builder
   `with_initial_gpr`. `InterpreterExecutor::execute` and
   `RecompilerExecutor::try_jit_run` apply overrides after segment
   load, before pc-set + first step. `build_spu_program_from_captured_image`
   sets gpr[1] preferred-slot = 0x3FFF0 to match the lv2 kernel's
   `cpu_init` semantics (`spu_thread::cpu_init` in
   `rpcs3/Emu/Cell/SPUThread.cpp:1342`). r3..r6 left zero
   (sysSpuThreadArgumentInitialize zeros the args).

## Stability

Once committed, this trace is a regression sentinel. Do NOT delete
or edit without recording the reason here (e.g. RPCS3 trace writer
schema change → recapture; SPU C source change → bump to
`single_spu_mailbox_v2`).
