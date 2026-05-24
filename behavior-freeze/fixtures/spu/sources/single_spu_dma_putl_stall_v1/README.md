# single_spu_dma_putl_stall_v1 — R8.5e E.3 fixture (source-only)

**R8.5e E.3 first MFC PUTL stall-and-notify capture (20th oracle target).**
CC0 1.0 public domain. Symmetric inverse of
`single_spu_dma_getl_stall_v1` — LS → EA direction with the
same 3-element + element-1-sb=0x80 layout.

## Status

- **E.3 (this slice)**: source authoring only. `.self` not yet
  built; no JSONL capture; no replay test; no oracle promotion.
- **E.4**: build via `rpcs3-ps3dev-toolchain:local` Docker image.
- **E.5**: capture JSONL on RPCS3 Interpreter (static) with the
  R8.5b writer extension active (already landed at
  `1f5450b56`).
- **E.6**: promote to 20th oracle (replay test + triple-symmetry
  validation).

## Behaviour

1. PPU fills three EA destination buffers pre-set to a 0xAA
   sentinel (so a dropped/silent PUTL surfaces as a distinct
   wrong ea_status rather than coincidentally matching):
   - `ea_dst1` (128 B)
   - `ea_dst2` (64 B)
   - `ea_dst3` (96 B)
2. PPU packs all three EAs into `thread_args.arg0` (high 32 = EA1,
   low 32 = EA2) and `thread_args.arg1` (high 32 = EA3).
3. SPU unpacks (PSL1GHT convention: arg0 → r3, arg1 → r4):
   ```
   ea1 = r3 >> 32
   ea2 = r3 & 0xFFFFFFFF
   ea3 = r4 >> 32
   ```
4. SPU fills LS source regions with R8.2-shared patterns
   (perfect dedup with canonical pool):
   ```
   LS[0x10000..0x10080] = i & 0xFF (counting, sum=0x1FC0)
   LS[0x10080..0x100C0] = 0x42      (constant, sum=0x1080)
   LS[0x100C0..0x10120] = 0x11      (constant, sum=0x0660)
   ```
5. SPU builds a 3-element list_element[] in LS:
   ```
   list[0] = { sb=0,    pad=0, ts=128, ea=EA1 }
   list[1] = { sb=0x80, pad=0, ts= 64, ea=EA2 }   // STALL
   list[2] = { sb=0,    pad=0, ts= 96, ea=EA3 }
   ```
6. SPU dispatches MFC PUTL (cmd=0x24) with ch16-21:
   - LSA      = 0x10000 (source base in LS)
   - EAH      = 0
   - EAL      = LS offset of the descriptor list
   - Size     = 24 (= 3 elements × 8 bytes)
   - TagID    = 3
   - Cmd      = 0x24 PUTL
7. RPCS3 walks the descriptors:
   - element 0: LS[0x10000..0x10080] → ea_dst1 (counting)
   - element 1: LS[0x10080..0x100C0] → ea_dst2 (0x42)
     - **Cell BE Sec. 12.5**: this transfer completes BEFORE
       the stall bit is raised. After the memcpy lands in EA,
       the MFC sets bit `1 << tag = 0x08` on
       MFC_RdListStallStat and pauses dispatch.
8. SPU reads ch25 (`MFC_RdListStallStat`) → `stall_mask = 0x08`.
9. SPU writes ch26 (`MFC_WrListStallAck`) ← `3` (tag id, NOT
   bitmask). The MFC clears the per-tag stall state and resumes
   the descriptor walk.
10. RPCS3 resumes:
    - element 2: LS[0x100C0..0x10120] → ea_dst3 (0x11)
11. MFC raises the tag-stat bit normally; SPU waits via
    ch22/ch23/ch24 (mask=0x08, ALL) → `tag_stat = 0x08`.
12. SPU writes the FIXED sentinel `0xC0FFEEC3` to OUT_MBOX,
    halts via stop 0x101.
13. PPU joins; lv2 reads OUT_MBOX as `spu_sentinel`; then PPU
    sums all three EA buffers:
    ```
    sum_ea1 = sum(ea_dst1[0..128]) = 0x1FC0
    sum_ea2 = sum(ea_dst2[0..64])  = 0x1080
    sum_ea3 = sum(ea_dst3[0..96])  = 0x0660
    combined = (sum_ea1 << 16) | ((sum_ea2 + sum_ea3) & 0xFFFF)
             = (0x1FC0 << 16) | 0x16E0
             = 0x1FC0_16E0
    ea_status = combined ^ 0xBEEFCAFE = 0xA12F_DC1E
    ```

## Canonical TTY

```
[putl_stall_v1] OK cause=0x1 spu=0xc0ffeec3 ea_status=0xa12fdc1e
```

## Differs from sister fixtures

| Aspect | putl_v1 (14th oracle) | getl_stall_v1 (19th oracle) | putl_stall_v1 (20th oracle target) |
|--------|-----------------------|-----------------------------|------------------------------------|
| Direction | PUTL (LS → EA) | GETL (EA → LS) | PUTL (LS → EA) |
| Elements | 2 | 3 | 3 |
| Stall bit | none | element 1 sb=0x80 | element 1 sb=0x80 |
| ch25 read | not used | reads mask=0x08 | reads mask=0x08 |
| ch26 write | not used | acks tag=3 | acks tag=3 |
| SPU sentinel | 0xC0FFEEBA | (computed status) | 0xC0FFEEC3 |
| ea_status / status | 0xA12FDA7E (ea_status) | 0xDF1EEC3A (status) | 0xA12FDC1E (ea_status) |
| Total transferred | 192 B | 288 B | 288 B |

The chunk patterns match `getl_stall_v1` exactly so all three
elements' `.dmachunk` side-files dedup with the canonical
pool: 471fb943… (counting 128 B), c422e707… (64 B of 0x42),
683d9fa5… (96 B of 0x11). The descriptor is a NEW
`.dmalistdesc` because cmd=0x24 + ea offsets differ from the
GETL stall descriptor.

## Build

```bash
docker run --rm -v "$PWD":/work \
  -e PS3DEV=/opt/ps3dev -e PSL1GHT=/opt/ps3dev/psl1ght \
  -w /work/behavior-freeze/fixtures/spu/sources/single_spu_dma_putl_stall_v1 \
  rpcs3-ps3dev-toolchain:local \
  bash -lc 'make clean && make V=1'
```

Output: `single_spu_dma_putl_stall_v1.self`. Move to `build/`
post-E.4.

## Capture (E.5)

After E.4 produces the `.self`, capture with rpcs3.exe built
with the R8.5b writer extension (already landed at
`1f5450b56`) + R8.5d D.2 bridge runtime (already landed at
`8e4039677`). Use Interpreter (static) decoders:

```cmd
set RPCS3_SPU_TRACE_JSONL=behavior-freeze\fixtures\spu\traces\single_spu_dma_putl_stall_v1.jsonl
rpcs3-upstream-clean\bin\rpcs3.exe --headless ^
   behavior-freeze\fixtures\spu\sources\single_spu_dma_putl_stall_v1\build\single_spu_dma_putl_stall_v1.self
```

The capture produces the JSONL trace + 1 new `.dmalistdesc`
(24 bytes, PUTL variant). Element chunks dedup against the
existing pool (the three 471fb943/c422e707/683d9fa5 chunks
landed in R8.5d D.5).

## Hard rules

- No fake descriptor — the `.dmalistdesc` content MUST be the
  actual 24 bytes the SPU wrote to LS at the dispatch moment.
- No fake element chunk — each `.dmachunk` MUST be the actual
  LS bytes per element at the dispatch moment (which equal
  the captured EA destination bytes post-PUTL).
- No manual JSONL editing.
- No PPU-side stall ack — the SPU↔MFC handshake is
  self-contained.
- v4/SPURS diagnostic-only forever.
