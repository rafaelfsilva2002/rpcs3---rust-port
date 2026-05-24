# single_spu_dma_putl_stall_v1.notes.md

R8.5e E.5 — first MFC PUTL **stall-and-notify** (`sb & 0x80`)
capture. **NOT YET PROMOTED to a replay-validated oracle** —
E.6 will land the replay test + triple-symmetry validation.
Symmetric inverse of R8.5d D.5 (`single_spu_dma_getl_stall_v1`):
LS → EA direction with the same 3-element + element-1-sb=0x80
layout.

Update history:
- R8.5e E.3 (`c171c09b0`, 2026-05-24): CC0 source authored.
- R8.5e E.4 (uncommitted artifacts, 2026-05-24): Docker build
  via `rpcs3-ps3dev-toolchain:local` produced
  `single_spu_dma_putl_stall_v1.self` (940 KB,
  sha `c670b59a36a89bb3a5301361e0e338168242d49da8ac257595f006f6e26a326d`).
- R8.5e E.5 (this capture, 2026-05-24): real JSONL capture via
  rpcs3.exe `--headless` + R8.5b writer extension active +
  R8.5d D.2 bridge runtime (Interpreter (static) decoders
  during capture; bridge code not on the path).

Captured 2026-05-24 from RPCS3 against the CC0 PSL1GHT
homebrew authored at E.3.

## Origem do homebrew

Autoral. CC0 1.0 (public domain). Source committed at
`behavior-freeze/fixtures/spu/sources/single_spu_dma_putl_stall_v1/`
with LICENSE.md.

Behaviour: PPU pre-fills 3 EA destination buffers with 0xAA
sentinel, packs all three EAs into `thread_args.arg0` (high=EA1,
low=EA2) + `arg1` (high=EA3). SPU fills LS source regions with
counting / 0x42 / 0x11 patterns (matching `getl_stall_v1` for
chunk dedup), builds 3-element list_element[] in LS with element
1 sb=0x80, dispatches MFC PUTL (cmd=0x24, size=24, tag=3). After
elements 0 + 1 transfer (Cell BE Sec. 12.5 transfer-then-stall —
element 1 IS in EA before the stall raises), the MFC sets the
stall bit `1 << 3 = 0x08` on the tag's MFC_RdListStallStat
(ch25). The SPU reads ch25 (destructive, returns 0x08),
acknowledges via ch26 ← 3 (tag id, NOT mask), the MFC clears
the stall bit and resumes the descriptor walk (element 2
LS[0x100C0..0x10120] → ea_dst3). SPU waits via
ch22/ch23/ch24 (mask=0x08, ALL) → tag_stat=0x08. SPU emits the
FIXED sentinel `0xC0FFEEC3` to OUT_MBOX, halts via stop 0x101.

**Captured OUT_MBOX value confirms: 0xC0FFEEC3 (= 3237998275).**

PPU then sums the three EA buffers and computes
`ea_status = ((sum_ea1 << 16) | ((sum_ea2 + sum_ea3) & 0xFFFF))
^ 0xBEEFCAFE = 0xA12F_DC1E`. The ea_status is not in the JSONL
(it's PPU-side post-join computation); the replay test
verifies the SPU-side sentinel + handshake events.

## Toolchain

Same `rpcs3-ps3dev-toolchain:local` Docker image
(sha `ed2167a9ac59…`).

- `.self`: 940 KB, sha
  `c670b59a36a89bb3a5301361e0e338168242d49da8ac257595f006f6e26a326d`
- SPU `.elf`: 1.5 KB, sha
  `605f4647b37b6d9f19cb742e7f4fe2e0be89e2d3e2db6ea1edf8d21d39f87af7`

## RPCS3 version + capture hooks

R8.5b writer extension active. Patches:

- scaffolding: `d0760a2ca0c2425eb7c4bc4854923d95b8bf835ed676cadf9f4e8c053f48456c`
  (R8.5b BUMP)
- runtime hooks: `ae4eaedcd7734a9a4bd6c40db0cceb8a2ce65edb6173995ad42601a66bcd7ab2`
  (R8.5b BUMP)
- rust bridge: `19a81b5452c44e720958805d034b7d19739a3ab256f9443503ee1dfcc4e89762`
  (R8.5d D.2 — `ListPartialState{is_put}` symmetric bridge).
  The capture itself ran on Interpreter (static) decoders so
  bridge code is not on the path; the bridge sha applies to
  runtime delegation only (E.6 will validate bridge ON
  delegation via triple-symmetry).

`bin/rpcs3.exe`:
- size 63,966,720 bytes
- sha `57D3746AFF74B6630B3EC559CD041FF5770372ECB7B1185D4CACCDCCE002E243`
- (R7.1 + R7.2 + R8.1 + R8.3a + R8.3b + R8.4b + R8.5b surface;
  same binary used for R8.5d D.5 capture — no rpcs3 rebuild
  required for this slice)

## Capture procedure

Same as prior R8.x captures: `Core: SPU/PPU Decoder:
Interpreter (static)` both decoders during capture; restored
to `Recompiler (LLVM)` after.

```cmd
set RPCS3_SPU_TRACE_JSONL=behavior-freeze\fixtures\spu\traces\single_spu_dma_putl_stall_v1.jsonl
rpcs3-upstream-clean\bin\rpcs3.exe --headless ^
   behavior-freeze\fixtures\spu\sources\single_spu_dma_putl_stall_v1\build\single_spu_dma_putl_stall_v1.self
```

Capture: 17 JSONL events, 3816 bytes. Run time: ~3 seconds
wall-clock for the SPU section; rpcs3.exe was kept alive ~60
seconds before external `Stop-Process` cleanup (JSONL is
flushed on emulator-stop, before the kill).

Captured artifacts:

- `behavior-freeze/fixtures/spu/traces/single_spu_dma_putl_stall_v1.jsonl`
  (17 events, 3816 bytes)
- `behavior-freeze/fixtures/spu/images/b9997f9b…spuimg`
  (256 KiB LS image — NEW, no dedup with existing pool)
- `behavior-freeze/fixtures/spu/dma/471fb943…dmachunk` —
  DEDUPS with R8.5d D.5 pool
- `behavior-freeze/fixtures/spu/dma/c422e707…dmachunk` —
  DEDUPS
- `behavior-freeze/fixtures/spu/dma/683d9fa5…dmachunk` —
  DEDUPS
- `behavior-freeze/fixtures/spu/dma/f72728d1…dmalistdesc` —
  DEDUPS (identical 24-byte descriptor bytes as
  getl_stall_v1; only the cmd differs, and cmd is not in the
  descriptor)

Trace-local sibling dirs `.jsonl.dma/` + `.jsonl.images/`
consolidated into canonical pools post-capture; consumed and
removed. Only the spuimg was new — all 4 dma side-files
deduped against the R8.5d D.5 pool (perfect deduplication).

## Captured event sequence (summary)

| seq | kind                | notable fields                                 |
|-----|---------------------|------------------------------------------------|
| 0   | spu_image           | sha `b9997f9b…` size=262144 entry_pc=0         |
| 1   | spu_wrch (ch16)     | LSA=0x10000                                    |
| 2   | spu_wrch (ch17)     | EAH=0                                          |
| 3   | spu_wrch (ch18)     | EAL=384 (LS offset of `list_descriptors[0]`)   |
| 4   | spu_wrch (ch19)     | Size=24 (3 × 8)                                |
| 5   | spu_wrch (ch20)     | TagID=3                                        |
| 6   | spu_wrch (ch21)     | Cmd=0x24 (PUTL)                                |
| 7   | spu_mfc_cmd         | cmd=36 (0x24), tag=3, size=24, lsa=65536, eal=384, |
|     |                     | descriptor sha + 3 element chunks + sizes + eals|
| 8   | **spu_rdch (ch25)** | **value=8** (stall mask for tag 3 — destructive read) |
| 9   | **spu_wrch (ch26)** | **value=3** (ack tag 3 — NOT bitmask)          |
| 10  | mfc_dma_complete    | tag=3 transferred_bytes=288 (128+64+96)        |
| 11  | spu_wrch (ch22)     | WrTagMask=0x08                                 |
| 12  | spu_wrch (ch23)     | WrTagUpdate=ALL                                |
| 13  | spu_rdch (ch24)     | RdTagStat=0x08                                 |
| 14  | spu_wrch (ch28)     | OUT_MBOX=3237998275 (= **0xC0FFEEC3**)         |
| 15  | spu_stop            | stop_code=257 (= **0x101**)                    |
| 16  | final_state         | full GPR + channels snapshot                   |

The pair of events 8 + 9 is the stall-and-notify handshake.
The mfc_dma_complete `transferred_bytes = 288` confirms Cell
BE Sec. 12.5 (= 128 element-0 + 64 stalled-element-1 + 96
post-ack-element-2). The SPU sentinel `0xC0FFEEC3` is FIXED
(not computed from the EA bytes — for PUTL the SPU has no
post-DMA reason to read back from LS, so the post-PUTL status
check is split: SPU emits a fixed sentinel; PPU computes
ea_status from the EA destinations).

## Hard rules

- No fake descriptor — the `.dmalistdesc` content MUST be the
  actual 24 bytes the SPU wrote to LS at the dispatch moment.
- No fake element chunk — each `.dmachunk` MUST be the actual
  LS bytes per element at the dispatch moment (= the bytes
  written to EA by RPCS3 during PUTL).
- No manual JSONL editing.
- No PPU-side stall ack — the SPU↔MFC handshake is
  self-contained; the PPU only joins post-list to read group
  exit status + sum EA buffers for ea_status TTY.
- v4/SPURS diagnostic-only forever.
