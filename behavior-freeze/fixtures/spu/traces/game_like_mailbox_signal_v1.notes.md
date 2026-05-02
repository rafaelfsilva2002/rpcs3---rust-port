# game_like_mailbox_signal_v1.notes.md

R6.6 — captured 2026-05-01 from RPCS3 against a CC0 PSL1GHT
homebrew authored for this purpose, then replayed end-to-end with
byte-identical agreement on the final SpuStateSnapshot. **Status:
REPLAY-VALIDATED.** Sixth oracle in the suite, joining mailbox_v1 /
branch_loop_v1 / signal_v1 / loadstore_v1 / mailbox_multi_v1.

This is the first **game-like** fixture: combines five previously-
isolated bridge code paths in a single SPU program. Byte-identical
interpreter+recompiler agreement here implies the bridge handles
all five simultaneously without cross-path interaction bugs.

## Origem do homebrew

Autoral. CC0 1.0 (public domain). Source committed at
`behavior-freeze/fixtures/spu/sources/game_like_mailbox_signal_v1/`
with `LICENSE.md`. Two `.c` files (PPU `main.c` + SPU
`spu/spu_game_like.c`) + `Makefile`. Targets PSL1GHT runtime.

Behavioral spec (one line): SPU reads seed from IN_MBOX (ch29),
initializes a 16-word volatile LS buffer with `(seed << 4) ^ i`,
runs a 16-iter mix loop accumulating `cs = cs ^ (cs << 1)` after
each `cs = cs + buf[i]`, blocks on SNR1 (ch3) for second input
(PPU-side `sysUsleep(100ms)` forces real `spu_park` event),
resumes with second input, runs 8-iter final mix combining sig +
buf, writes final `cs` to OUT_MBOX, halts via stop `0x101`.

Canonical inputs `seed = 0x21`, `sig = 0x07` produce OUT_MBOX =
`0x051A03C9`. Verified by reference Python implementation in
`README.md` and observed under both bridge OFF (C++ executor) and
bridge ON (Rust executor) on the real RPCS3 binary.

## Toolchain

Same `rpcs3-ps3dev-toolchain:local` Docker image scaffolded at
`.claude/ps3toolchain-docker/Dockerfile` (R6.4b-toolchain).
Toolchain provenance:

- ps3toolchain commit `f8e8abc8f777362f061089d2c45acf716e013847`
- powerpc64-ps3-elf-gcc (GCC) 7.2.0 (PPU)
- spu-gcc (GCC) 7.2.0 (SPU)
- PSL1GHT installed by ps3toolchain script `008-psl1ght.sh`
- bin2s, fself, sprxlinker host tools

Build command (in container):

```
cd behavior-freeze/fixtures/spu/sources/game_like_mailbox_signal_v1
make V=1
```

Outputs (moved to `build/` post-make):
- `build/game_like_mailbox_signal_v1.elf`  (937 KiB)
- `build/game_like_mailbox_signal_v1.self` (917 KiB; sha256 `21f30b36…`)

## Capture command

```bash
$env:RPCS3_SPU_TRACE_JSONL = "<repo>\behavior-freeze\fixtures\spu\traces\game_like_mailbox_signal_v1.jsonl"
R:\bin\rpcs3.exe --headless "<repo>\behavior-freeze\fixtures\spu\sources\game_like_mailbox_signal_v1\build\game_like_mailbox_signal_v1.self"
```

`RPCS3_SPU_RUST_BRIDGE` UNSET for capture — the JSONL is the
canonical C++ executor output; the bridge ON path bypasses
execution and would not generate per-instruction events.

TTY: `[game_like_v1] OK cause=0x1 status=0x51a03c9`.

## Trace contents

10 events:

| seq | side | kind | summary |
|---|---|---|---|
| 0 | PPU | `ppu_push_inmbox` | target=256, value=0x21 |
| 1 | SPU | `spu_image` | sha256 = `4054960a…`, size=262144, entry_pc=0 |
| 2 | SPU | `spu_rdch` | pc=12, ch=3 (SNR1), value=null, would_stall=true |
| 3 | SPU | `spu_park` | pc=12, reason=channel_read, channel=3 |
| 4 | PPU | `ppu_signal` | target=256, slot=0, value=0x07 |
| 5 | SPU | `spu_wake` | pc=12 |
| 6 | SPU | `spu_rdch` | pc=12, ch=3, value=0x07, would_stall=false |
| 7 | SPU | `spu_wrch` | pc=?, ch=28 (OUT_MBOX), value=0x051A03C9 |
| 8 | SPU | `spu_stop` | pc=?, stop_code=0x101 |
| 9 | SPU | `final_state` | r3=0x051A03C9, channels: snr1=0, snr2=0, all mbox null |

(Note: only the SECOND `rdch` is captured — the first `rdch` for
IN_MBOX completed without stalling because the PPU's WriteMb
arrived before SPU dispatch, so the writer's `would_stall=false`
path didn't emit a balanced rdch event for it.)

`spu_park` + `spu_wake` confirms the real stall pattern. Zero
`spu_wrch ch21` events.

## Side-file

`behavior-freeze/fixtures/spu/images/4054960a038202949463876bc7ed9833de2021203df9d0eff566653e8ad225d6.spuimg`

— 256 KiB content-addressed SPU LS dump. SHA-256 matches the
`image_sha256` field in the `spu_image` event at seq 1. Lives in
the canonical R5.9e.7+ centralized layout.

## Replay acceptance gate

`rust/rpcs3-spu-recompiler/tests/game_like_mailbox_signal_v1_replay.rs`
mirrors the existing 5 fixture tests + asserts:
- ≥1 `ppu_push_inmbox`
- ≥1 `ppu_signal`
- ≥1 `spu_park` (proves real stall)
- exactly 1 `spu_wrch ch28`
- OUT_MBOX value = `0x051A03C9` (canonical)
- exactly 1 `spu_stop` with `stop_code=0x101`
- 0 `spu_wrch ch21` (NO DMA)
- Interpreter `Finished{0x101}`
- Recompiler `Finished{0x101}`
- `diff_snapshots(interp, jit).is_identical() == true`

Test PASSED byte-identical.

## Real-binary bridge acceptance

Verified on `R:\bin\rpcs3.exe` (R6.5b binary, build 21:49):

- **Bridge OFF:** `[game_like_v1] OK cause=0x1 status=0x51a03c9` ✓
- **Bridge ON:** `[game_like_v1] OK cause=0x1 status=0x51a03c9` ✓
  Bridge log: `Stop code=0x101 total_steps=488 in_mbox_consumed=1
  signal_forwarded=1 stall_iters=1 stall_write_iters=0 final_pc=0xc4`

`stall_iters=1` ✓ — the persistent-handle re-entry path (R6.4b)
was exercised. `total_steps=488` reflects the two mix loops
running real instructions across the LS round-trips.
