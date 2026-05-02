# single_spu_branch_loop_v1.notes.md

R5.11 — second replay-validated SPU trace fixture (oracle suite
expansion, post-R5 closure). Captured 2026-04-29 from RPCS3
against a CC0 PSL1GHT homebrew authored for this purpose, then
replayed end-to-end (parser → per-SPU transformer → SpuProgram
builder → replay × InterpreterExecutor + replay × RecompilerExecutor)
with byte-identical agreement on the final SpuStateSnapshot (PC,
GPRs, LS, channels, park_state).

## Origem do homebrew

Autoral. CC0 1.0 (public domain). Source committed at
`behavior-freeze/fixtures/spu/sources/single_spu_branch_loop_v1/`
with LICENSE.md.

Comportamento (uma linha): PPU pushes one 32-bit `cmd` (= 10);
SPU runs a Fibonacci recurrence for `cmd` iterations using pure
32-bit adds + comparison + back-edge branch (no multiplication, no
DMA, no extra channels), writes Fib(10) = 89 to OUT_MBOX, halts via
stop 0x101 (SYS_SPU_THREAD_STOP_GROUP_EXIT).

Same race-free single-round mailbox shape as `single_spu_mailbox_v1`
(R5.9e.7) — PPU pushes IN_MBOX, SPU computes + writes OUT_MBOX once,
PPU joins. The lv2 kernel reads OUT_MBOX as the group-exit status.

## Toolchain

Reuses the same from-source `ps3toolchain` setup as R5.9e.7
(Docker `debian:bookworm-slim` container `ps3-build`):

- binutils 2.43.1 (PPU + SPU)
- gcc 14.2.0 (PPU + SPU, freestanding)
- newlib 4.4.0
- PSL1GHT (latest commit at R5.9e.7 capture time)
- make_self / fself / sprxlinker / bin2s host tools

Build command (in container):

```
docker cp single_spu_branch_loop_v1 ps3-build:/tmp/
docker exec ps3-build bash -c \
  'cd /tmp/single_spu_branch_loop_v1 && \
   PS3DEV=/opt/ps3dev PSL1GHT=/opt/ps3dev/psl1ght \
   PATH=$PS3DEV/bin:$PS3DEV/ppu/bin:$PS3DEV/spu/bin:$PATH \
   make'
docker cp ps3-build:/tmp/single_spu_branch_loop_v1/single_spu_branch_loop_v1.self build/
```

SPU side compiled with `-O2 -Wall -nostartfiles -nostdlib
-Wl,--entry,main` (same rationale as `single_spu_mailbox_v1`:
skip crt0 / newlib that pulls in ROTQBY etc. outside the
iteration-1 SPU interpreter subset).

## Decoded SPU code (spu-objdump output)

```
00000000 <main>:
   0:  hbrr   .L_loop_top, .L_after_loop  ; branch hint (NOP in interp)
   4:  rdch   $7, $ch29                   ; r7 = cmd from IN_MBOX
   8:  il     $4, 1                       ; r4 = b initial (= 1)
   c:  brz    $7, .L_emit                 ; if cmd==0, skip loop
  10:  il     $3, 0                       ; r3 = i counter
  14:  il     $2, 1                       ; r2 = b
  18:  il     $5, 0                       ; r5 = a
  ; .L_loop_top:
  1c:  a      $4, $5, $2                  ; r4 = a + b (= t)
  20:  ai     $3, $3, 1                   ; i++
  24:  ori    $5, $2, 0                   ; a = b
  28:  ceq    $6, $7, $3                  ; r6 = (cmd == i) ? -1 : 0
  2c:  ori    $2, $4, 0                   ; b = t
  30:  brz    $6, .L_loop_top             ; if i != cmd, loop
  ; .L_emit:
  34:  wrch   $ch28, $4                   ; OUT_MBOX = b (= Fib(cmd))
  38:  nop
  3c:  stop   0x0101                      ; SYS_SPU_THREAD_STOP_GROUP_EXIT
  40:  il     $3, 0                       ; (unreachable epilogue)
  44:  bi     $0
```

All instructions are within the iteration-1 SPU interpreter subset
that R5.10a..p has cleared (`hbrr` is implemented as a NOP per
`hbrr_is_nop_for_interpreter` test in the interpreter crate;
`il` / `ai` / `a` / `ori` / `ceq` / `brz` / `rdch` / `wrch` /
`stop` / `nop` / `bi` are all covered).

## RPCS3 version + capture hooks

RPCS3 build: same R5.9c + R5.9e.3 trace writer used for
`single_spu_mailbox_v1` (`rpcs3/Emu/Cell/SPUTraceJsonl.{h,cpp}`).
C++ patches preserved unchanged at R5 closure — sha256
`d65aec91…ae1aba1c` (scaffolding) + `8f253d7d…66663a` (runtime hooks).

## Capture procedure

Same as `single_spu_mailbox_v1`'s capture (auto-exit config patched
once at R5.9e.7, persists). Either:

1. Double-click `enable_autoexit_and_capture.cmd` in Explorer, or
2. From bash:
   ```
   RPCS3_SPU_TRACE_JSONL=/tmp/single_spu_branch_loop_v1.jsonl \
     /r/bin/rpcs3.exe --headless \
     /path/to/build/single_spu_branch_loop_v1.self
   ```

Captured artifacts staged in this repo:

- `behavior-freeze/fixtures/spu/traces/single_spu_branch_loop_v1.jsonl`
  (5 events, 776 bytes)
- `behavior-freeze/fixtures/spu/images/f0531bb9…46fb1.spuimg` (262 KB,
  centralized layout per § F.4)

## Trace contents (5 events)

```
seq 0: ppu_push_inmbox     target_spu=256 value=10
seq 1: spu_image           sha=f0531bb9...46fb1 load=0x0 size=0x40000 entry_pc=0x0
seq 2: spu_wrch  ch28      target_spu=256 pc=52  value=89  (= Fib(10))
seq 3: spu_stop            target_spu=256 pc=60  stop_code=0x101
seq 4: final_state         target_spu=256
                           gpr={r1=262128, r2=89, r3=10, r4=89,
                                r5=55, r6=0xFFFFFFFF, r7=10}
                           channels={in_mbox=null, out_mbox=null,
                                     out_intr_mbox=null, snr1=0, snr2=0}
```

GPR snapshot makes physical sense:
- r1 = 0x3FFF0 (PS3 SPU SP convention).
- r2 = 89 (b after final iter — last computed Fib).
- r3 = 10 (i counter — equal to cmd).
- r4 = 89 (t = a + b in final iter).
- r5 = 55 (a after final iter — Fib(9), the prior b).
- r6 = 0xFFFFFFFF (CEQ result of i==cmd; SPU CEQ returns -1 on match).
- r7 = 10 (cmd, never overwritten).

## Acceptance criteria (per traces/README.md "Critérios de aceitação para NOVOS traces")

| # | Criterion | Status |
|---|-----------|--------|
| 1 | Origem CC0 / license-clean | ✅ author + LICENSE.md |
| 2 | Boota no RPCS3 instrumentado (R5.9c+R5.9e.3) | ✅ (same patches as R5.9e.7) |
| 3 | Cria SPU thread group | ✅ via PSL1GHT `sysSpuThreadGroupCreate` |
| 4 | Exerce mailbox (push + wrch) | ✅ ch29 read + ch28 write |
| 5 | Sem DMA (zero ch21) | ✅ verified by integration test |
| 6 | `.jsonl` real, nunca editado | ✅ |
| 7 | Companion `.notes.md` | ✅ this file |
| 8 | `.spuimg` no layout centralizado | ✅ `images/<sha>.spuimg` |
| 9 | Pipeline Rust passa com `diff_snapshots(...).is_identical()` | ✅ verified |
| 10 | Acceptance test commitado | ✅ `single_spu_branch_loop_v1_replay.rs` |

## Replay-validation

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

`total_steps` differs across backends (interp=71, jit=86) — same
expected divergence as `single_spu_mailbox_v1`: JIT counts dispatcher
iterations + JIT prefix steps, interpreter counts retired
instructions. Step count is internal accounting and is NOT part of
the byte-identical state contract.

## Engine-side fixes co-landed

**None.** This fixture rides entirely on the three engine-side
fixes that landed alongside `single_spu_mailbox_v1` at R5.9e.7
(transformer initial-state inference, lv2 stop-0x101 OUT_MBOX
drain, `SpuProgram.initial_gpr_overrides`). All three are general
(not single-fixture hacks), and `single_spu_branch_loop_v1` is the
first fixture that demonstrates this — passing on first attempt
without any new fixes.

## Stability

Once committed, this trace is a regression sentinel. Do NOT delete
or edit without recording the reason here. The captured `.spuimg`
hash `f0531bb93c432d93149f8648e7812cd02a3d54717df3733dbb82e55ed9846fb1`
is the canonical content-address; if the toolchain output drifts
(different `gcc` version, different optimizer flags), the hash
shifts and a new `vN+1` fixture is the right path — not editing
this one.
