# single_spu_dma_putl_v1

CC0 1.0 (public domain). See LICENSE.md.

R8.4e — first MFC PUTL list-DMA oracle target. Symmetric inverse
of R8.4b/c GETL: a single MFC PUTL dispatch (cmd 0x24) with two
elements writes data FROM Rust/SPU LS TO RPCS3 EA via the same
8-byte BE descriptor format.

## Canonical computation (Python reference)

```python
sum1 = sum(i & 0xFF for i in range(128))   # 0x1FC0
sum2 = 0x42 * 64                           # 0x1080
combined = (sum1 << 16) | sum2             # 0x1FC01080
ea_status = combined ^ 0xBEEFCAFE          # 0xA12FDA7E
spu_sentinel = 0xC0FFEEBA                  # fixed
```

Predicted canonical TTY:

```
[dma_putl_v1] OK cause=0x1 spu=0xc0ffeeba ea_status=0xa12fda7e
```

## Behaviour (deterministic)

1. PPU allocates `ea_dst1` (128 B, init to sentinel `0xAA`) and
   `ea_dst2` (64 B, init to sentinel `0xAA`). The pre-PUTL
   sentinel exists so a dropped/silent PUTL would surface as a
   distinct (wrong) `ea_status` rather than a coincidental match.
2. PPU passes `ea_dst1` via `arg0`, `ea_dst2` via `arg1`.
3. SPU fills LS source regions with deterministic content:
   - `LS[0x10000..0x10080]`: counting pattern `i & 0xFF`.
   - `LS[0x10080..0x100C0]`: constant `0x42`.
   (Both patterns reuse R8.2 / R8.3 / R8.4c chunk SHAs, so
   `.dmachunk` side-files deduplicate with the canonical pool.)
4. SPU builds a 2-element `list_element[]` in LS:
   ```
   list[0] = { sb=0, pad=0, ts=128, ea=EA1 }
   list[1] = { sb=0, pad=0, ts= 64, ea=EA2 }
   ```
5. SPU dispatches MFC PUTL via ch16-21:
   ```
   ch16 MFC_LSA      = 0x10000           (source base, NOT dest)
   ch17 MFC_EAH      = 0
   ch18 MFC_EAL      = LS offset of list (descriptor pointer,
                                          same convention as
                                          GETL — Cell BE list-
                                          DMA puts the descriptor
                                          in LS, not in EA)
   ch19 MFC_Size     = 16                (descriptor size = 2 * 8)
   ch20 MFC_TagID    = 3
   ch21 MFC_Cmd      = 0x24 PUTL
   ```
6. RPCS3 reads the descriptor list from LS, walks elements,
   copies each `ts` bytes FROM `LS[mfc_lsa + cumulative_offset]`
   TO `EA = item.ea`:
   - element 0: `LS[0x10000..0x10080]` → `ea_dst1` (counting)
   - element 1: `LS[0x10080..0x100C0]` → `ea_dst2` (0x42)
7. SPU waits via ch22/ch23/ch24 (mask=0x08, ALL).
8. SPU writes `0xC0FFEEBA` (sentinel) to OUT_MBOX, halts via
   stop 0x101.
9. PPU joins, then reads back `ea_dst1` + `ea_dst2`:
   - `sum_ea1 = sum(ea_dst1[i])` should equal `0x1FC0`.
   - `sum_ea2 = sum(ea_dst2[i])` should equal `0x1080`.
   - `combined = (sum_ea1 << 16) | sum_ea2 = 0x1FC01080`
   - `ea_status = combined ^ 0xBEEFCAFE = 0xA12FDA7E`
10. PPU prints canonical TTY and exits.

## Failure mode catalogue

- List dispatch dropped (EA buffers stay 0xAA):
  `sum_ea1 = 0xAA * 128 = 0x5500`,
  `sum_ea2 = 0xAA * 64  = 0x2A80`,
  `ea_status = (0x5500 << 16 | 0x2A80) ^ 0xBEEFCAFE = 0xEBBFE43E`.
- Element 0 dropped only: `ea_status` reflects ea1 stayed 0xAA,
  ea2 got 0x42 — distinct wrong value.
- Element 1 dropped only: inverse.
- Descriptor format wrong (swapped elements / wrong EA
  interpretation): distinctively wrong sums.
- PUTL not dispatched at all (e.g., cmd lookup table missing
  0x24): `MfcUnsupported` from Rust runtime path; bridge falls
  back to C++ executor; canonical OK status still emerges
  because the C++ path handles 0x24 natively. The DELEGATED
  EXECUTION OK log line confirms the Rust runtime path.

## Side-file dedup expectations

- `.spuimg`: new SHA (PUTL SPU C source is new).
- `.dmalistdesc`: new SHA (descriptor `ea` field values for THIS
  fixture's EA layout differ from GETL's, so the descriptor
  bytes hash differently).
- `.dmachunk` element 0 (128 B counting pattern):
  REUSES `471fb943aa23c511f6f72f8d1652d9c880cfa392ad80503120547703e56a2be5`
  from R6.7 A.5 GET / R8.1 PUT / R8.2 / R8.3a / R8.3b / R8.3c /
  R8.4b. Counting pattern is content-identical across all
  these fixtures.
- `.dmachunk` element 1 (64 B constant `0x42`):
  REUSES `c422e7070ed4...` from R8.2 / R8.3a / R8.3b / R8.3c /
  R8.4b. Constant `0x42` pattern matches.

This is the SAME perfect-dedup property R8.4b/c achieved for
GETL — proves the content-addressed pool design across both
list-DMA directions.

## Build

Requires the PSL1GHT toolchain (see `behavior-freeze/docs/HOMEBREW_PLAN.md`
or the `rpcs3-ps3dev-toolchain:local` Docker image). From this
directory:

```sh
source /etc/profile.d/ps3dev.sh
make
```

Output: `single_spu_dma_putl_v1.self` (~940 KB).

## Capture

Same procedure as R8.4b GETL capture:

```sh
export RPCS3_SPU_TRACE_JSONL=/path/to/single_spu_dma_putl_v1.jsonl
# config.yml MUST set:
#   Core: SPU Decoder = Interpreter (static)
#   Core: PPU Decoder = Interpreter (static)
# (LLVM JIT bypasses C++ set_ch_value() for MFC channels — same
# constraint that landed in R6.7 A.5)
rpcs3.exe --no-gui --headless --stdout single_spu_dma_putl_v1.self
```

The trace MUST be captured against an rpcs3.exe built with the
R8.4e writer extension (relaxed `record_spu_mfc_list_cmd` cmd
gate to accept 0x24).
