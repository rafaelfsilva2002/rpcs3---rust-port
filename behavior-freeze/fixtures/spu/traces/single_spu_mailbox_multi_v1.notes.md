# single_spu_mailbox_multi_v1.notes.md

R6.4b-toolchain (capture) + R6.4b-replay (acceptance) — captured
2026-05-01 from RPCS3 against a CC0 PSL1GHT homebrew authored for
this purpose, then replayed end-to-end with byte-identical
agreement on the final SpuStateSnapshot (PC, GPRs, LS, channels,
park_state). **Status: REPLAY-VALIDATED.** Fifth oracle in the
suite, joining mailbox_v1 / branch_loop_v1 / signal_v1 /
loadstore_v1.

## Origem do homebrew

Autoral. CC0 1.0 (public domain). Source committed at
`behavior-freeze/fixtures/spu/sources/single_spu_mailbox_multi_v1/`
with `LICENSE.md`. Two `.c` files (PPU `main.c` + SPU
`spu/spu_mailbox_multi.c`) + `Makefile`. Targets PSL1GHT runtime.

Authoring intent: provide the first oracle fixture that exercises
the persistent-handle re-entry path of the C++↔Rust SPU bridge
(R6.4b). Single-shot fixtures (`mailbox_v1`, `branch_loop_v1`,
`loadstore_v1`, `signal_v1`) all run to `Stop` in one Rust executor
call; this fixture is the FIRST that REQUIRES two PPU-side inputs
separated by a guaranteed SPU stall.

Comportamento (uma linha): PPU pushes one 32-bit value
(`0x100`) to IN_MBOX; SPU reads, holds partial result
`0x100 + 0xA1 = 0x1A1` in r3; SPU blocks on SNR1; PPU sends
signal `slot=0, value=0x200` to SNR1; SPU resumes, computes
`reply = (0x200 + 0xB2) + partial = 0x453`, writes OUT_MBOX,
halts via stop `0x101`. lv2 reads OUT_MBOX = `0x453` as the
group-exit status; PPU sees `cause=0x1, status=0x453`.

The IN_MBOX-then-SNR1 mix sidesteps the PSL1GHT cooperative-thread
limitation that there is no public PPU API to read OUT_MBOX. Both
PPU writes go through PSL1GHT's exposed `sysSpuThreadWriteMb` and
`sysSpuThreadWriteSignal` syscalls.

## Toolchain

`ps3toolchain` built from source in a Docker `debian:bookworm`
container scaffolded at `.claude/ps3toolchain-docker/Dockerfile`
in this repo. Toolchain components at capture time:

- ps3toolchain commit `f8e8abc8f777362f061089d2c45acf716e013847`
- PSL1GHT — installed by `008-psl1ght.sh` as part of the
  ps3toolchain build (commit pulled from upstream at build time)
- powerpc64-ps3-elf-gcc (GCC) 7.2.0 (PPU)
- spu-gcc (GCC) 7.2.0 (SPU)
- bin2s, fself, sprxlinker host tools
- Skipped: ps3toolchain script `009-ps3libraries.sh` (libxml2
  upstream URL returned 404 at build time; not needed for this
  fixture which only links against lv2 / sysmodule / io / sysutil
  / rt).

Build command (in container):

```
cd behavior-freeze/fixtures/spu/sources/single_spu_mailbox_multi_v1
make V=1
```

Outputs (in fixture dir; the `.elf` and `.self` are then moved to
`build/` to match the convention of the other R5.11 fixtures):

```
build/single_spu_mailbox_multi_v1.elf  (937 KiB)
build/single_spu_mailbox_multi_v1.self (917 KiB; sha256 cd1ea38c…)
```

## Capture command

The trace was captured by running the just-built `.self` against
`R:\bin\rpcs3.exe` with `RPCS3_SPU_TRACE_JSONL` set:

```
$env:RPCS3_SPU_TRACE_JSONL = "$repo/behavior-freeze/fixtures/spu/traces/single_spu_mailbox_multi_v1.jsonl"
R:\bin\rpcs3.exe --headless "$repo/behavior-freeze/fixtures/spu/sources/single_spu_mailbox_multi_v1/build/single_spu_mailbox_multi_v1.self"
```

`RPCS3_SPU_RUST_BRIDGE` was UNSET for the capture, so the trace is
the canonical C++ executor output (the writer fires from the C++
hooks; the bridge ON path bypasses execution and would not
generate per-instruction events).

TTY: `[mbmulti_v1] OK cause=0x1 status=0x453`.

## Trace contents

10 events:

| seq | side | kind | summary |
|---|---|---|---|
| 0 | PPU | `ppu_push_inmbox` | target=256 (= lv2_id 0x100), value=256 (= 0x100) |
| 1 | SPU | `spu_image` | sha256 = `eb316a98…`, size=262144, entry_pc=0 |
| 2 | SPU | `spu_rdch` | pc=12, channel=3 (SNR1), value=null, would_stall=true |
| 3 | SPU | `spu_park` | pc=12, reason=channel_read, channel=3 |
| 4 | PPU | `ppu_signal` | target=256, slot=0 (SNR1), value=512 (= 0x200) |
| 5 | SPU | `spu_wake` | pc=12 |
| 6 | SPU | `spu_rdch` | pc=12, channel=3, value=512 (= 0x200), would_stall=false |
| 7 | SPU | `spu_wrch` | pc=24, channel=28 (OUT_MBOX), value=1107 (= 0x453), would_stall=false |
| 8 | SPU | `spu_stop` | pc=28, stop_code=257 (= 0x101) |
| 9 | SPU | `final_state` | r3=256, r4=256, r5=768 (= 0x300), r6=1107 (= 0x453); channels: snr1=0, snr2=0, in_mbox=null, out_mbox=null |

**Park/wake observed.** The PPU sleeps 100ms (via `sysUsleep`)
between `sysSpuThreadWriteMb` and `sysSpuThreadWriteSignal`, which
gives the SPU thread time to consume IN_MBOX, advance to the
`rdch ch3` instruction at pc=12, and park (SNR1 still empty). Then
the PPU's signal arrives, the SPU wakes, reads the value, and
finishes the program. This is the **load-bearing stall pattern**
the C++↔Rust SPU bridge's persistent-handle path (R6.4b) exists
to handle.

(An earlier capture without the sleep produced a "no-stall" trace
where both PPU writes arrived before SPU dispatch — the replay
engine rejected that with `WakeKindMismatch`, since the per-SPU
transformer's wake events expect the SPU to actually be parked
when a PPU input arrives. The 100ms sleep is the minimal
synchronization that makes the trace shape match the model.)

## Side-file

`behavior-freeze/fixtures/spu/images/eb316a9875a21c7d05013c185fc54293298855c08e177b2b9dda820ae38c7e07.spuimg`

— 256 KiB content-addressed SPU LS dump produced at thread entry
by the trace writer's `record_spu_image` hook. The SHA-256 matches
the `image_sha256` field in the `spu_image` event at seq 1. Lives
in the canonical centralized R5.9e.7+ images directory (the writer
initially landed it in `single_spu_mailbox_multi_v1.jsonl.images/`
adjacent to the JSONL; R6.4b-replay moved it to the canonical
location and that empty per-trace directory was removed).

## Replay acceptance status

**REPLAY-VALIDATED.** All six replay gate criteria from
`behavior-freeze/fixtures/spu/traces/README.md` § "Critérios de
aceitação para NOVOS traces replay-validated" pass:

1. `parse_jsonl_trace()` → Ok (10 events)
2. `captured_events_to_traces_per_spu()` → Ok (1 group, target_spu=256)
3. `build_spu_program_from_captured_image()` → Ok
4. `replay_per_spu_traces::<InterpreterExecutor>(...)` →
   `Finished{stop_code=0x101}` after 10 steps
5. `replay_per_spu_traces_with(..., RecompilerExecutor)` →
   `Finished{stop_code=0x101}` after 17 steps (JIT counts dispatch
   iterations + JIT prefix steps; per the documented exclusion in
   `R5.9e.7` test, `total_steps` is per-backend tally, not part of
   the byte-identical state contract)
6. `diff_snapshots(interp.final_snapshot, jit.final_snapshot)
   .is_identical() == true`

Acceptance gate test:
`rust/rpcs3-spu-recompiler/tests/single_spu_mailbox_multi_v1_replay.rs`
— mirrors the four prior tests, plus three additional asserts
specific to the multi-PPU-input shape:
- `≥1 ppu_push_inmbox` (round 1 input)
- `≥1 ppu_signal` (round 2 input — the load-bearing distinguishing
  feature)
- OUT_MBOX value must equal `0x453` (= `0x1A1 + 0x2B2`, the
  canonical sum of both rounds; any deviation means the SPU did
  not consume both inputs in order)

The fifth test in the suite. After landing it, the
`REPLAY_VALIDATED_TRACE_EXISTS` flag in `check_trace_fixtures.py`
remains True (was already True from the four prior fixtures); the
Python harness's `files:` list now includes the new `.jsonl` and
`.notes.md` for this fixture.

## Real-binary bridge acceptance (R6.5)

R6.5 closure 2026-05-01: this fixture's `.self` was run against
`R:\bin\rpcs3.exe` (the R6.4b binary, build 11:00) with
`RPCS3_SPU_RUST_BRIDGE=1`. The bridge's persistent-handle
multi-round loop was exercised end-to-end:

```
TTY:    [mbmulti_v1] OK cause=0x1 status=0x453

RustSPU SUCCESS log:
  Stop code=0x101 total_steps=9 in_mbox_consumed=1
  signal_forwarded=1 stall_iters=1 final_pc=0x1c
```

`stall_iters=1` is the load-bearing evidence: the multi-round loop
in `try_delegate_execution()` iterated once. Sequence inferred
from the implementation + this single counter:

1. Phase 1 peek: `ch_in_mbox` had `0x100` (PPU's `sysSpuThreadWriteMb`
   was processed before the bridge entered) → pushed to Rust.
2. Phase 1b peek: `ch_snr1` was empty (PPU still in
   `sysUsleep(100ms)` before `sysSpuThreadWriteSignal`) → no
   forward.
3. First `rust_spu_run_until_event`: SPU consumed IN_MBOX,
   advanced to `rdch ch3` at pc=12, returned `StallRead code=3`
   after `out_steps` instructions.
4. Multi-round branch matched (channel=3 is supported): bridge
   called `spu.ch_snr1.pop_wait(spu)` — this BLOCKED the cpu
   thread on RPCS3's native channel-wait machinery, exactly the
   path `spu_thread::get_ch_value` takes for `SPU_RdSigNotify1`.
5. PPU's `sysUsleep` finished, then `sysSpuThreadWriteSignal(0,
   0x200)` populated `ch_snr1` and woke the SPU thread.
6. `pop_wait` returned `0x200`; bridge forwarded via
   `rust_spu_signal(slot=0, 0x200)`; `stall_iterations++` (now 1).
7. Second `rust_spu_run_until_event` on the SAME `rust_spu_t*`:
   SPU read SNR1 → `0x200`, computed `reply = 0x453`, wrote
   OUT_MBOX, executed `stop 0x101`, returned `Stop`.
8. Phase 3 commit: drained the peeked IN_MBOX from RPCS3
   (`in_mbox_consumed += 1` → total 1 for this fixture's `in_count
   = 1` from initial peek), drained Rust OUT_MBOX → set RPCS3's
   `ch_out_mbox`, synced PC + GPRs, called
   `stop_and_signal(0x101)`. `signal_forwarded` = 1 (from the
   loop branch above).
9. RPCS3 group join saw `cause=0x1, status=0x453`.

`total_steps=9` is +1 vs the no-stall path (`single_spu_signal_v1`
hits Stop in 8 steps): the extra step is the SPU-side `rdch`
that returned `would_stall=true` and parked. After the wake,
the same `rdch` re-executes and consumes the value — but the
Rust executor's step counter advances on the second attempt only
once.

This proves R6.4b persistent-handle infrastructure works on a
real RPCS3 binary, not just synthetic FFI tests. R6.5 is the
load-bearing acceptance that flips the bridge's "verified
workloads" set from "single-shot oracles only" to "single-shot +
multi-round-stall oracles".
