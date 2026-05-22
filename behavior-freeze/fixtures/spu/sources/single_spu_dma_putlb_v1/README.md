# single_spu_dma_putlb_v1

CC0 1.0 (public domain). See LICENSE.md.

R8.4f-b — first MFC PUTLB list-DMA + barrier oracle (17th
oracle target). Symmetric inverse of R8.4f-a GETLB: same
8-byte BE descriptor format, LS → EA direction, cmd=0x25
(PUTL | `MFC_BARRIER_MASK = 0x01`).

## REUSE-PUTL justification

Per RPCS3 `do_list_transfer:2887`:
`transfer.cmd = args.cmd & ~0xf` strips the barrier bit
(0x01) before the per-element copy → data path byte-identical
to PUTL. Per `do_dma_check:2819`, `mfc_barrier` only gates
SUBSEQUENT cmds on the same tag (no observable effect for
single-SPU fresh-tag single-dispatch). Per `SPUThread.cpp:
4929-4937`, PUTL/PUTLB/PUTLF share the same case block.

## Canonical computation

```python
sum_ea1 = sum(i & 0xFF for i in range(128))   # 0x1FC0
sum_ea2 = 0x42 * 64                           # 0x1080
combined = (sum_ea1 << 16) | sum_ea2          # 0x1FC01080
ea_status = combined ^ 0xBEEFCABB             # 0xA12FDA3B
spu_sentinel = 0xC0FFEEBB                     # fixed
```

Status XOR mask `0xBEEFCABB` mirrors PUTL's `0xBEEFCAFE` with
the last byte `0xBB` = "Barrier"; SPU sentinel `0xC0FFEEBB`
mirrors PUTL's `0xC0FFEEBA` with last byte bumped to `0xBB`
to distinguish from regressions that route 0x25 through
PUTL's status formula.

Predicted canonical TTY:

```
[dma_putlb_v1] OK cause=0x1 spu=0xc0ffeebb ea_status=0xa12fda3b
```

## Behaviour

Identical to PUTL v1 except cmd=0x25 and the two distinct
constants above:
1. PPU allocates `ea_dst1` (128 B init 0xAA) + `ea_dst2`
   (64 B init 0xAA). Pre-PUTLB sentinel makes a dropped DMA
   surface as a distinct (wrong) `ea_status`.
2. PPU passes EAs via arg0/arg1.
3. SPU fills LS source with counting + constant 0x42.
4. SPU builds 2-element list_element[] in LS.
5. SPU dispatches MFC PUTLB (cmd=0x25).
6. SPU waits ch22/ch23/ch24 (mask=0x08, ALL).
7. SPU writes spu_sentinel `0xC0FFEEBB` to OUT_MBOX, halts.
8. PPU joins, sums both EA buffers, computes ea_status.
