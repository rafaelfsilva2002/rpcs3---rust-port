# single_spu_dma_getl_stall_v1.notes.md

R8.5d D.5 — first MFC GETL **stall-and-notify** (`sb & 0x80`)
capture. **NOT YET PROMOTED to a replay-validated oracle** —
D.6 will land the replay test + triple-symmetry validation.

Update history:
- R8.5b (`1f5450b56`, 2026-05-22): writer unlocked ch25/ch26
  capture surface (Schema A — reuse `spu_rdch` / `spu_wrch`).
- R8.5c (`a4d0d58f7`, 2026-05-23): Rust replay state machine
  for the stall-and-notify handshake (`process_spu_rdch_list_stall_stat`,
  `process_spu_wrch_list_stall_ack`, `ListDmaPartialProgress`)
  with Cell BE Sec. 12.5 transfer-then-stall invariant. 7
  synthetic round-trip tests in `mfc_replay.rs`.
- R8.5d D.1 (`b6c717b55`, 2026-05-23): Rust FFI scaffolding
  (`rust_spu_set_dma_list_stall_ack_callback`,
  `RUST_SPU_DMA_LIST_STALL_PENDING = -2`,
  `mfc_list_stall_mask`) + C++ bridge runtime (`GetlPartialState`,
  `bridge_dma_list_stall_ack_callback`) — GETL-only.
- R8.5d D.2 (`8e4039677`, 2026-05-23): unify
  `GetlPartialState → ListPartialState{is_put}` and extend the
  bridge runtime to PUTL family (symmetric mem←LS resume).
- R8.5d D.3 (`35d637a3a`, 2026-05-23): CC0 source authored for
  this fixture (3-element list with element 1 sb=0x80).
- R8.5d D.4 (uncommitted artifacts, 2026-05-23): Docker build
  via `rpcs3-ps3dev-toolchain:local` produced
  `single_spu_dma_getl_stall_v1.self` (940 KB,
  sha `2b3bf7cfe3a07132da24368e4a3e5b74b7fad5b7e0afa74447ab319b366aba1a`).
- R8.5d D.5 (this capture, 2026-05-23): real JSONL capture
  via rpcs3.exe `--headless` + R8.5b writer extension active.

Captured 2026-05-23 from RPCS3 against the CC0 PSL1GHT
homebrew authored at D.3.

## Origem do homebrew

Autoral. CC0 1.0 (public domain). Source committed at
`behavior-freeze/fixtures/spu/sources/single_spu_dma_getl_stall_v1/`
with LICENSE.md.

Behaviour: PPU prepares 3 EA buffers (128 B counting 0..127,
64 B constant 0x42, 96 B constant 0x11), packs all three EAs
into thread_args.arg0 (high=EA1, low=EA2) + arg1 (high=EA3).
SPU builds a 3-element list_element[] in LS with element 1
carrying sb=0x80, dispatches MFC GETL (cmd=0x44, size=24, tag=3).
After elements 0 + 1 transfer (Cell BE Sec. 12.5 transfer-then-
stall — element 1 IS in LS before the stall raises), the MFC
sets the stall bit `1 << 3 = 0x08` on the tag's
MFC_RdListStallStat (ch25). The SPU reads ch25 (destructive,
returns 0x08), acknowledges via ch26 ← 3 (tag id, NOT mask),
the MFC clears the stall bit and resumes the descriptor walk
(element 2 lands at LS[0x100C0..0x10120]). SPU waits via
ch22/ch23/ch24 (mask=0x08, ALL) → tag_stat=0x08. SPU sums all
three LS regions:

- sum1 = 0x1FC0 (counting 0..127)
- sum2 = 0x1080 (64 × 0x42)
- sum3 = 0x0660 (96 × 0x11)
- combined = (sum1 << 16) | ((sum2 + sum3) & 0xFFFF) = 0x1FC0_16E0
- status = combined ^ 0xC0DEFADA = **0xDF1E_EC3A**

SPU writes `status` to OUT_MBOX, halts via stop 0x101.
**Captured OUT_MBOX value confirms: 0xDF1EEC3A (= 3743345722).**

## Toolchain

Same `rpcs3-ps3dev-toolchain:local` Docker image
(sha `ed2167a9ac59…`).

- `.self`: 940 KB, sha
  `2b3bf7cfe3a07132da24368e4a3e5b74b7fad5b7e0afa74447ab319b366aba1a`
- SPU `.elf`: 1.4 KB, sha
  `de3bbc768c88beaadae8c83d72c2de1c0a3fe26c595b2d434e36e2b75f7594d2`

## RPCS3 version + capture hooks

R8.5b writer extension active. Patches:

- scaffolding: `d0760a2ca0c2425eb7c4bc4854923d95b8bf835ed676cadf9f4e8c053f48456c`
  (R8.5b BUMP)
- runtime hooks: `ae4eaedcd7734a9a4bd6c40db0cceb8a2ce65edb6173995ad42601a66bcd7ab2`
  (R8.5b BUMP: SPUThread.cpp ch21 dispatcher relaxed to accept
  `sb & 0x80` descriptors; ch25 destructive read +
  `MFC_WrListStallAck` ch26 write captured as
  `spu_rdch ch=25` + `spu_wrch ch=26` per Schema A)
- rust bridge: `19a81b5452c44e720958805d034b7d19739a3ab256f9443503ee1dfcc4e89762`
  (R8.5d D.2 — `bridge_dma_list_stall_ack_callback` +
  `ListPartialState{is_put}`). The capture itself was done
  with Interpreter (static) decoders so bridge code is not
  on the path; the new bridge sha applies to runtime
  delegation only (D.6 will validate bridge ON delegation).

`bin/rpcs3.exe`:
- size 63,966,720 bytes
- sha `57D3746AFF74B6630B3EC559CD041FF5770372ECB7B1185D4CACCDCCE002E243`
- (R7.1 + R7.2 + R8.1 + R8.3a + R8.3b + R8.4b + R8.5b surface;
  built 2026-05-23 with the GETL-stall stall-ack callback wired
  per R8.5d D.1.b + D.2)

## Capture procedure

Same as prior R8.x captures: `Core: SPU/PPU Decoder:
Interpreter (static)` both decoders during capture; restored
to `Recompiler (LLVM)` after.

```cmd
set RPCS3_SPU_TRACE_JSONL=behavior-freeze\fixtures\spu\traces\single_spu_dma_getl_stall_v1.jsonl
rpcs3-upstream-clean\bin\rpcs3.exe --headless ^
   behavior-freeze\fixtures\spu\sources\single_spu_dma_getl_stall_v1\build\single_spu_dma_getl_stall_v1.self
```

Capture: 17 JSONL events. Run time: ~3 seconds wall-clock for
the SPU section; rpcs3.exe was kept alive ~90 seconds before
external `Stop-Process` cleanup (PPU process exit doesn't
auto-quit `--headless` rpcs3.exe — JSONL is flushed on
emulator-stop, before the kill).

Captured artifacts:

- `behavior-freeze/fixtures/spu/traces/single_spu_dma_getl_stall_v1.jsonl`
  (17 events, 4057 bytes)
- `behavior-freeze/fixtures/spu/images/2b2b8d86…spuimg`
  (256 KiB LS image — NEW, no dedup with existing pool)
- `behavior-freeze/fixtures/spu/dma/471fb943…dmachunk`
  (128 B counting — DEDUPS with GETL_v1 pool)
- `behavior-freeze/fixtures/spu/dma/c422e707…dmachunk`
  (64 B 0x42 — DEDUPS with GETL_v1 pool)
- `behavior-freeze/fixtures/spu/dma/683d9fa5…dmachunk`
  (96 B 0x11 — NEW, no dedup)
- `behavior-freeze/fixtures/spu/dma/f72728d1…dmalistdesc`
  (24 B, 3-element descriptor — NEW, distinct from GETL_v1's
  16 B 2-element descriptor)

Trace-local sibling dirs `.jsonl.dma/` + `.jsonl.images/`
(written by RPCS3 as a provisional staging area) consolidated
into canonical pools post-capture; consumed and removed.

## Captured event sequence (summary)

| seq | kind                | notable fields                                 |
|-----|---------------------|------------------------------------------------|
| 0   | spu_image           | sha `2b2b8d86…` size=262144 entry_pc=0         |
| 1   | spu_wrch (ch16)     | LSA=0x10000                                    |
| 2   | spu_wrch (ch17)     | EAH=0                                          |
| 3   | spu_wrch (ch18)     | EAL=512 (LS offset of `list_descriptors[0]`)   |
| 4   | spu_wrch (ch19)     | Size=24 (3 × 8)                                |
| 5   | spu_wrch (ch20)     | TagID=3                                        |
| 6   | spu_wrch (ch21)     | Cmd=0x44 (GETL)                                |
| 7   | spu_mfc_cmd         | cmd=68, tag=3, size=24, lsa=65536, eal=512,    |
|     |                     | descriptor sha + 3 element chunks + sizes + eals|
| 8   | **spu_rdch (ch25)** | **value=8** (stall mask for tag 3 — destructive read) |
| 9   | **spu_wrch (ch26)** | **value=3** (ack tag 3 — NOT bitmask)          |
| 10  | mfc_dma_complete    | tag=3 transferred_bytes=288 (128+64+96)        |
| 11  | spu_wrch (ch22)     | WrTagMask=0x08                                 |
| 12  | spu_wrch (ch23)     | WrTagUpdate=ALL                                |
| 13  | spu_rdch (ch24)     | RdTagStat=0x08                                 |
| 14  | spu_wrch (ch28)     | OUT_MBOX=3743345722 (= **0xDF1EEC3A**)         |
| 15  | spu_stop            | stop_code=257 (= **0x101**)                    |
| 16  | final_state         | full GPR + channels snapshot                   |

The pair of events 8 + 9 is the stall-and-notify handshake;
their presence validates the R8.5b writer extension (ch25 +
ch26 events are emitted), the homebrew correctness (stall_mask
matches expected `1 << tag = 0x08`, ack writes the tag id `3`),
and the Cell BE Sec. 12.5 invariant (the mfc_dma_complete event
reports the FULL `transferred_bytes = 288` after resume, not
just the 192 bytes that landed before the stall).

## Hard rules

- No fake descriptor — the `.dmalistdesc` content MUST be the
  actual 24 bytes the SPU wrote to LS at the dispatch moment.
- No fake element chunk — each `.dmachunk` MUST be the actual
  EA bytes per element at the dispatch moment.
- No manual JSONL editing.
- No PPU-side stall ack — the SPU↔MFC handshake is
  self-contained; the PPU only joins post-list to read the
  group-exit status (identical to other list-DMA oracles).
- No PUTL stall in this fixture — bridge already supports
  both directions via D.2, but a PUTL stall oracle (if/when
  authored) is a separate fixture.
