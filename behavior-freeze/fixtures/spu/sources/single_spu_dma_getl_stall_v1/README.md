# single_spu_dma_getl_stall_v1 — R8.5d D.3 fixture (source-only)

**R8.5d D.3 first MFC GETL stall-and-notify capture (19th oracle target).**
CC0 1.0 public domain.

## Status

- **D.3 (this slice)**: source authoring only. `.self` not yet
  built; no JSONL capture; no replay test; no oracle promotion.
- **D.4**: build via `rpcs3-ps3dev-toolchain:local` Docker image.
- **D.5**: capture JSONL on RPCS3 Interpreter (static) with the
  R8.5b writer extension active (already landed at
  `1f5450b56`).
- **D.6**: promote to 19th oracle (replay test + triple-symmetry
  validation).

## Behaviour

1. PPU fills three EA buffers:
   - `ea_buf1` = 128 B counting pattern 0..127 (matches GETL_v1
     fixture; `.dmachunk` deduplicates with canonical pool)
   - `ea_buf2` = 64 B constant 0x42 (matches GETL_v1; dedups)
   - `ea_buf3` = 96 B constant 0x11 (NEW; one new side-file at
     capture time)
2. PPU packs all three EAs into `thread_args.arg0` (high 32 = EA1,
   low 32 = EA2) and `thread_args.arg1` (high 32 = EA3).
3. SPU unpacks (PSL1GHT convention: arg0 → r3, arg1 → r4):
   ```
   ea1 = r3 >> 32
   ea2 = r3 & 0xFFFFFFFF
   ea3 = r4 >> 32
   ```
4. SPU builds a 3-element list_element[] in LS at a static offset:
   ```
   list[0] = { sb=0,    pad=0, ts=128, ea=EA1 }
   list[1] = { sb=0x80, pad=0, ts= 64, ea=EA2 }   // STALL
   list[2] = { sb=0,    pad=0, ts= 96, ea=EA3 }
   ```
5. SPU dispatches MFC GETL (cmd=0x44) with ch16-21:
   - LSA      = 0x10000 (destination base)
   - EAH      = 0
   - EAL      = LS offset of the descriptor list (NOT data EA)
   - Size     = 24 (= 3 elements × 8 bytes)
   - TagID    = 3
   - Cmd      = 0x44 GETL
6. RPCS3 walks the descriptors:
   - element 0: LS[0x10000..0x10080] ← ea_buf1 (counting)
   - element 1: LS[0x10080..0x100C0] ← ea_buf2 (0x42)
     - **Cell BE Sec. 12.5**: this transfer completes BEFORE
       the stall bit is raised. After the memcpy lands in LS,
       the MFC sets bit `1 << tag = 0x08` in the per-tag
       MFC_RdListStallStat register and pauses dispatch.
7. SPU reads ch25 (`MFC_RdListStallStat`) → `stall_mask = 0x08`.
   (Destructive read: returns mask, then clears.)
8. SPU writes ch26 (`MFC_WrListStallAck`) ← `3` (tag id, NOT
   bitmask). The MFC clears the per-tag stall state and resumes
   the descriptor walk.
9. RPCS3 resumes:
   - element 2: LS[0x100C0..0x10120] ← ea_buf3 (0x11)
10. MFC raises the tag-stat bit normally; SPU waits via
    ch22/ch23/ch24 (mask=0x08, ALL) → `tag_stat = 0x08`.
11. SPU sums all three regions:
    ```
    sum1 = sum(LS[0x10000..0x10080]) = 0x1FC0   (= 8128 dec)
    sum2 = sum(LS[0x10080..0x100C0]) = 0x1080   (= 4224 dec)
    sum3 = sum(LS[0x100C0..0x10120]) = 0x0660   (= 1632 dec)
    combined = (sum1 << 16) | ((sum2 + sum3) & 0xFFFF)
             = (0x1FC0 << 16) | 0x16E0
             = 0x1FC0_16E0
    status   = combined ^ 0xC0DEFADA = 0xDF1E_EC3A
    ```
12. SPU writes `status` to OUT_MBOX, halts via stop 0x101.

## Canonical TTY

```
[getl_stall_v1] OK cause=0x1 status=0xdf1eec3a
```

## Differs from single_spu_dma_getl_v1

| Aspect | getl_v1 (13th oracle) | getl_stall_v1 (19th oracle target) |
|--------|----------------------|------------------------------------|
| Elements | 2 | 3 |
| Stall bit | none | element 1 sets sb=0x80 |
| ch25 read | not used | reads stall mask (expected 0x08) |
| ch26 write | not used | acks tag 3 |
| Buffers | ea_buf1 (counting) + ea_buf2 (0x42) | + ea_buf3 (96 B of 0x11) |
| sum1 | 0x1FC0 | 0x1FC0 (same) |
| sum2 | 0x1080 | 0x1080 (same) |
| sum3 | — | 0x0660 |
| Status | 0xDF1E_EA5A | 0xDF1E_EC3A |
| Total transferred | 192 B | 288 B |

The first two element buffers and their byte patterns match
GETL_v1 deliberately so the corresponding `.dmachunk` side-files
deduplicate with the canonical pool (zero new chunk side-files
for elements 0 + 1; only element 2's 96-byte chunk is new).

## Build

```bash
docker run --rm -v "$PWD":/work \
  -e PS3DEV=/opt/ps3dev -e PSL1GHT=/opt/ps3dev/psl1ght \
  -w /work/behavior-freeze/fixtures/spu/sources/single_spu_dma_getl_stall_v1 \
  rpcs3-ps3dev-toolchain:local \
  bash -lc 'make clean && make V=1'
```

Output: `single_spu_dma_getl_stall_v1.self`. Move to `build/`
post-D.4.

## Capture (D.5)

After D.4 produces the `.self`, capture with rpcs3.exe built
with the R8.5b writer extension (already landed at
`1f5450b56`) using Interpreter (static) decoders:

```cmd
set RPCS3_SPU_TRACE_JSONL=behavior-freeze\fixtures\spu\traces\single_spu_dma_getl_stall_v1.jsonl
R:\bin\rpcs3.exe --no-gui ^
   behavior-freeze\fixtures\spu\sources\single_spu_dma_getl_stall_v1\build\single_spu_dma_getl_stall_v1.self
```

The capture produces the JSONL trace + 1 new `.dmalistdesc`
(24 bytes) + 1 new `.dmachunk` (96 B of 0x11); elements 0 + 1
chunks dedup with the existing canonical pool.

## Hard rules

- No fake descriptor — the `.dmalistdesc` content MUST be the
  actual bytes the SPU wrote to LS at the dispatch moment.
- No fake element chunk — each `.dmachunk` MUST be the actual
  EA bytes per element at the dispatch moment.
- No manual JSONL editing.
- No PPU-side stall ack — the SPU↔MFC handshake is
  self-contained (PPU only joins post-list to read group exit
  status, identical to other list-DMA oracles).
- No PUTL stall in this fixture — PUTL stall fixture is a
  separate slice (the bridge runtime already supports both
  directions via [[r8-5d-d2-putl-stall-bridge]], but the
  source fixture has its own oracle path).
