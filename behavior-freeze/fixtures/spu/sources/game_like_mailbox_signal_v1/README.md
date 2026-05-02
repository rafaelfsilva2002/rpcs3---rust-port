# game_like_mailbox_signal_v1 — R6.6 game-like oracle fixture

**Status (R6.6):** sources authored + .self built + trace captured + replay test landed. Promoted to **6th replay-validated fixture**.

## Why this fixture exists

The five R5.11/R5.11b/R6.4b oracles each exercise a single
representative bridge code path: IN_MBOX (mailbox_v1), branch+loop
ISA (branch_loop_v1), LS load/store (loadstore_v1), SNR1 (signal_v1),
and IN_MBOX→SNR1 multi-round (mailbox_multi_v1). Real games use
**multiple paths simultaneously** in the same SPU program. This
fixture is the first oracle designed to exercise FIVE bridge paths
in one program, so any cross-path interaction bug surfaces here
(byte-identical interpreter+recompiler agreement on this trace
implies the bridge handles all five simultaneously).

Paths exercised:
1. `rdch ch29` (IN_MBOX, R5.9e.7 path)
2. `stqd` / `lqd` via `volatile uint32_t buf[16]` (R5.11b loadstore)
3. branch + loop ISA (R5.11 branch_loop)
4. `rdch ch3` (SNR1, R5.11/R6.3c signal)
5. **StallRead between IN_MBOX consumption and SNR1 read** (R6.4b
   multi-round persistent-handle path, surfaces when PPU sleeps
   between WriteMb and WriteSignal)

All five run in one execution; the canonical OUT_MBOX value
(`0x051A03C9`) bit-encodes whether the SPU consumed BOTH PPU inputs
in order AND ran both mix loops correctly.

## Behaviour

```
PPU                                    SPU (Rust executor)
─────────────────────────────────────  ──────────────────────────────────
sysSpuThreadWriteMb(0x21)
  → ch_in_mbox = [0x21]
                                       seed = rdch ch29 → 0x21
                                       buf[i] = (seed << 4) ^ i, i in 0..16
                                          (16 stqd writes to LS)
                                       cs = seed
                                       loop 1 (16 iters):
                                         v = lqd(buf + i*4)
                                         cs = cs + v
                                         cs = cs ^ (cs << 1)
                                       sig = rdch ch3 → STALL
sysUsleep(100ms)
sysSpuThreadWriteSignal(slot=0, 0x07)
  → ch_snr1 = 0x07
                                       (resume) sig = rdch ch3 → 0x07
                                       loop 2 (8 iters):
                                         cs = cs + sig
                                         cs = cs ^ buf[i]
                                       wrch ch28, cs (= 0x051A03C9)
                                       stop 0x101
sysSpuThreadGroupJoin()
  → cause=0x1, status=0x051A03C9
```

## Canonical output

For inputs `seed = 0x21`, `sig = 0x07`:

```
status = 0x051A03C9
```

Reference Python computation (matches the SPU exactly because every
operation is well-defined `u32` arithmetic):

```python
seed = 0x21
sig  = 0x07
mask = 0xFFFFFFFF

buf = [(seed << 4) ^ i for i in range(16)]
# buf[0]=0x210, buf[1]=0x211, ..., buf[15]=0x21F

cs = seed
for i in range(16):
    cs = (cs + buf[i]) & mask
    cs = cs ^ ((cs << 1) & mask)
# after loop 1: cs = 0x051A0379

for i in range(8):
    cs = (cs + sig) & mask
    cs = cs ^ buf[i]
# final: cs = 0x051A03C9
```

## Build

The PSL1GHT toolchain image scaffolded at
`.claude/ps3toolchain-docker/Dockerfile` (R6.4b-toolchain) builds
this fixture cleanly:

```bash
cd <repo root>
MSYS_NO_PATHCONV=1 docker run --rm -v "$PWD":/work \
  -w /work/behavior-freeze/fixtures/spu/sources/game_like_mailbox_signal_v1 \
  rpcs3-ps3dev-toolchain:local \
  bash -lc 'make clean; make V=1'
# Move .self/.elf to build/ for canonical layout
mv game_like_mailbox_signal_v1.{self,elf} build/
```

## Capture flow

```bash
$env:RPCS3_SPU_TRACE_JSONL = "<repo>\behavior-freeze\fixtures\spu\traces\game_like_mailbox_signal_v1.jsonl"
R:\bin\rpcs3.exe --headless "<repo>\behavior-freeze\fixtures\spu\sources\game_like_mailbox_signal_v1\build\game_like_mailbox_signal_v1.self"
```

The trace writer auto-creates `<jsonl>.images/<sha>.spuimg` adjacent
to the JSONL; per project convention, move it to the canonical
`behavior-freeze/fixtures/spu/images/<sha>.spuimg` layout.

## Acceptance gate

`rust/rpcs3-spu-recompiler/tests/game_like_mailbox_signal_v1_replay.rs`
mirrors the existing fixture-tests + asserts:
- `status = 0x051A03C9` exactly
- ≥1 ppu_push_inmbox event
- ≥1 ppu_signal event
- ≥1 spu_park event (proves the StallRead actually fired)
- 0 spu_wrch ch21 events (NO DMA)
