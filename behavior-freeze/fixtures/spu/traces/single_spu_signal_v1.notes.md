# single_spu_signal_v1.notes.md

R5.11 — third replay-validated SPU trace fixture (oracle suite
expansion, post-R5 closure). Captured 2026-04-29 from RPCS3
against a CC0 PSL1GHT homebrew authored for this purpose, then
replayed end-to-end with byte-identical agreement on the final
SpuStateSnapshot across InterpreterExecutor and RecompilerExecutor.

This fixture is the **first replay-validated trace exercising the
signal-notification path** (PPU `sysSpuThreadWriteSignal` →
`ppu_signal { slot, value }` event → SPU `rdch ch3 (SPU_RdSigNotify1)`),
distinct from the IN_MBOX path covered by `single_spu_mailbox_v1`
and `single_spu_branch_loop_v1`.

## Origem do homebrew

Autoral. CC0 1.0 (public domain). Source committed at
`behavior-freeze/fixtures/spu/sources/single_spu_signal_v1/`
with LICENSE.md.

Comportamento (uma linha): PPU writes 0x1234 to SNR1 via
`sysSpuThreadWriteSignal(thread, 0, 0x1234)` (lv2 syscall 184); SPU
reads via `rdch ch3`, computes reply = 0x1234 + 0xFEED = 0x11121,
writes reply to OUT_MBOX (ch28), halts via stop 0x101.

Same race-free single-round shape as `single_spu_mailbox_v1` /
`single_spu_branch_loop_v1` — PPU acts once, SPU computes, SPU
writes OUT_MBOX once, lv2 reads it as the group-exit status, PPU
joins.

## Toolchain

Reuses the same from-source `ps3toolchain` Docker container
`ps3-build` setup as R5.9e.7 / R5.11 fixture #1.

Build command (in container):

```
docker cp single_spu_signal_v1 ps3-build:/tmp/
docker exec ps3-build bash -c \
  'cd /tmp/single_spu_signal_v1 && \
   PS3DEV=/opt/ps3dev PSL1GHT=/opt/ps3dev/psl1ght \
   PATH=$PS3DEV/bin:$PS3DEV/ppu/bin:$PS3DEV/spu/bin:$PATH \
   make'
docker cp ps3-build:/tmp/single_spu_signal_v1/single_spu_signal_v1.self build/
```

SPU side compiled with `-O2 -Wall -nostartfiles -nostdlib
-Wl,--entry,main` (same rationale as sibling fixtures).

## Decoded SPU code (spu-objdump output)

```
00000000 <main>:
   0:  hbr    24 <after>, $0       ; branch hint (NOP in interp)
   4:  nop    $127
   8:  nop    $127
   c:  rdch   $2, $ch3             ; r2 = signal from SNR1
  10:  ila    $3, 0xFEED           ; r3 = 0xFEED (RI18 immediate)
  14:  a      $4, $2, $3           ; r4 = r2 + r3 = 0x11121
  18:  wrch   $ch28, $4            ; OUT_MBOX = r4
  1c:  stop   0x0101               ; SYS_SPU_THREAD_STOP_GROUP_EXIT
  20:  il     $3, 0                ; (unreachable epilogue)
  24:  bi     $0
```

All instructions are within the iteration-1 SPU interpreter subset
(`hbr` is NOP per `hbrr_is_nop_for_interpreter`; `nop` / `rdch` /
`ila` / `a` / `wrch` / `stop` / `il` / `bi` are all covered).

## RPCS3 version + capture hooks

RPCS3 build: same R5.9c + R5.9e.3 trace writer used for the
prior two fixtures. C++ patches preserved unchanged at R5
closure — sha256 `d65aec91…ae1aba1c` (scaffolding) +
`8f253d7d…66663a` (runtime hooks). The writer's `ppu_signal`
event emission was already in place (no patch changes for this
fixture).

## Capture procedure

Same as `single_spu_mailbox_v1` / `single_spu_branch_loop_v1`. From
bash:

```
RPCS3_SPU_TRACE_JSONL=/tmp/single_spu_signal_v1.jsonl \
  /r/bin/rpcs3.exe --headless \
  /path/to/build/single_spu_signal_v1.self
```

Captured artifacts staged in this repo:

- `behavior-freeze/fixtures/spu/traces/single_spu_signal_v1.jsonl`
  (6 events, 834 bytes)
- `behavior-freeze/fixtures/spu/images/b998ab08…ba9de.spuimg` (262 KB,
  centralized layout)

## Trace contents (6 events)

```
seq 0: ppu_signal          target_spu=256 slot=0 value=4660 (= 0x1234)
seq 1: spu_image           sha=b998ab08...ba9de load=0x0 size=0x40000 entry_pc=0x0
seq 2: spu_rdch  ch3       target_spu=256 pc=12 value=4660 would_stall=false
seq 3: spu_wrch  ch28      target_spu=256 pc=24 value=69921 (= 0x11121)
seq 4: spu_stop            target_spu=256 pc=28 stop_code=0x101
seq 5: final_state         target_spu=256
                           gpr={r1=262128, r2=4660, r3=65261, r4=69921}
                           channels={in_mbox=null, out_mbox=null,
                                     out_intr_mbox=null, snr1=0, snr2=0}
```

Note `would_stall=false` on the SPU's `rdch ch3` at seq=2 — by the
time the SPU reached the read in real RPCS3, the signal had
already arrived (PPU's syscall 184 fired before the SPU's first
instruction got past the 3-cycle prologue of `hbr` + 2 `nop`s).
The replay model still parks the SPU at `rdch ch3` because in
the deterministic replay the PPU action hasn't fired yet — see
"Engine-side fix" below.

## Acceptance criteria (per traces/README.md)

| # | Criterion | Status |
|---|-----------|--------|
| 1 | Origem CC0 / license-clean | ✅ |
| 2 | Boota no RPCS3 instrumentado (R5.9c+R5.9e.3) | ✅ (same patches) |
| 3 | Cria SPU thread group | ✅ |
| 4 | Exerce signal (ppu_signal + spu_rdch ch3) | ✅ first such fixture |
| 5 | Sem DMA (zero ch21) | ✅ |
| 6 | `.jsonl` real, nunca editado | ✅ |
| 7 | Companion `.notes.md` | ✅ this file |
| 8 | `.spuimg` no layout centralizado | ✅ |
| 9 | Pipeline Rust passa com `diff_snapshots(...).is_identical()` | ✅ verified |
| 10 | Acceptance test commitado | ✅ `single_spu_signal_v1_replay.rs` |

## Replay-validation

Status: ✅ parser ok / transformer ok / interp replay ok / JIT
replay ok / cross-backend snapshot diff identical.

`total_steps`: interp=9, jit=14 (legitimately differs across
backends; not part of the byte-identical contract).

## Engine-side fix co-landed (general, not single-fixture)

**Cell BE SPU SNR-channel blocking semantics.** Prior to R5.11,
`SpuChannels::read()` for `SPU_RDSIGNOTIFY1` / `SPU_RDSIGNOTIFY2`
returned `Ok(snr[i])` unconditionally — even when `snr[i] == 0`
— never returning `WouldStall`. This was a documented
simplification (see comment at `rpcs3-spu-differential/src/lib.rs`
PpuAction::Signal pre-R5.11) but did NOT match real Cell BE
hardware, where rdch on SNR channels stalls when count == 0.

The fix (`rust/rpcs3-spu-thread/src/lib.rs` SpuChannels::read):

```rust
SPU_RDSIGNOTIFY1 => {
    if self.snr[0] == 0 {
        return Err(ChannelStatus::WouldStall);
    }
    let v = self.snr[0];
    self.snr[0] = 0;
    self.event_stat &= !0x00000001;
    Ok(v)
}
// (same shape for SPU_RDSIGNOTIFY2)
```

Companion test rewrites:
- `rpcs3-spu-interpreter::tests::rdch_signal1_clears_after_read`
  — second consecutive rdch on a drained SNR now would park, so
  the test now does a single read + asserts both r3 value and
  `channels.snr[0] == 0` post-clear.
- `rpcs3-spu-differential::tests::lockstep_signotify_does_not_naturally_park`
  → renamed to `lockstep_signotify_parks_naturally_and_signal_wakes`,
  the test now validates the natural park → Signal-wake → Finished
  path end-to-end through `SpuPpuLockstepDriver` (same shape this
  fixture exercises against captured behavior).

This fix is GENERAL — not single-fixture. It corrects the
SPU executor to match Cell BE spec, makes the Signal action
naturally reachable through `run_until_event`, and stays landed
beyond this fixture. Future signal-bearing fixtures and any R6
live-bridge work inherit the fix.

## Stability

Once committed, this trace is a regression sentinel. Do NOT delete
or edit without recording the reason here. Captured `.spuimg`
hash `b998ab088a633a9096298b4d5fd7b734ffb6790f6c666c164ba92411ad7ba9de`
is the canonical content-address.
