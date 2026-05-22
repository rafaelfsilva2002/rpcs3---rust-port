# single_spu_dma_getlb_v1

CC0 1.0 (public domain). See LICENSE.md.

R8.4f-a — first MFC GETLB list-DMA + barrier oracle target.
Identical to R8.4c GETL (cmd=0x44) except cmd=0x45 (GETL +
`MFC_BARRIER_MASK = 0x01`).

## Barrier semantics for this fixture

Per RPCS3 `do_dma_check` + `do_list_transfer`:
- `MFC_BARRIER_MASK` (0x01) makes the per-tag `mfc_barrier`
  register persist after this dispatch, so SUBSEQUENT
  commands on the same tag must wait.
- For a single-SPU synchronous fixture with ONE list-DMA on a
  FRESH tag and NO subsequent commands on the same tag, the
  barrier persistence has NO observable effect.
- `do_list_transfer` strips the barrier bit before the
  per-element copy (`transfer.cmd = args.cmd & ~0xf`), so
  the byte-level data movement is byte-identical to GETL.

This is why R8.4f-a can reuse R8.4c's GETL state machine,
runtime bridge callback, and side-file format — only the
parser/replay/bridge cmd-acceptance lists change.

## Canonical computation (Python reference)

```python
sum1 = sum(i & 0xFF for i in range(128))   # 0x1FC0
sum2 = 0x42 * 64                           # 0x1080
combined = (sum1 << 16) | sum2             # 0x1FC01080
status = combined ^ 0xC0DEFABB             # 0xDF1EEA3B
```

XOR mask `0xC0DEFABB` mirrors GETL's `0xC0DEFADA` with the
last byte = `0xBB` (mnemonic: "Barrier"), so a regression
that accidentally accepts cmd=0x45 but uses GETL's status
formula would surface as `0xDF1EEA5A` instead of
`0xDF1EEA3B`.

Predicted canonical TTY:

```
[dma_getlb_v1] OK cause=0x1 status=0xdf1eea3b
```

## Behaviour

Identical to GETL v1, byte-for-byte:
1. PPU allocates `ea_buf1` (128 B counting pattern) and
   `ea_buf2` (64 B constant 0x42). `.dmachunk` files
   deduplicate with the canonical pool.
2. PPU passes EA1 via arg0, EA2 via arg1.
3. SPU builds a 2-element list_element[] in LS at
   `&list_descriptors[0]`.
4. SPU dispatches MFC GETLB:
   ```
   ch16 MFC_LSA      = 0x10000           (dest base)
   ch17 MFC_EAH      = 0
   ch18 MFC_EAL      = LS offset of list (descriptor pointer)
   ch19 MFC_Size     = 16                (= 2 * 8)
   ch20 MFC_TagID    = 3
   ch21 MFC_Cmd      = 0x45 GETLB        (= 0x44 GETL | 0x01 BARRIER)
   ```
5. RPCS3 walks the descriptor + per-element memcpys
   EA → LS at cumulative offset (raw `ts` sum advance).
6. SPU waits via ch22/ch23/ch24 (mask=0x08, ALL).
7. SPU sums both LS regions, computes status, writes to
   OUT_MBOX, halts via stop 0x101.

## Build / capture

Same procedure as R8.4c GETL — see
`single_spu_dma_getl_v1/README.md` for the full instructions.
SPU bytecode is identical to GETL except for the single byte
change at the `MFC_Cmd` immediate.
