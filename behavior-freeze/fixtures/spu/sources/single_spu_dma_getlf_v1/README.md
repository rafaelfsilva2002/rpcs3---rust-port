# single_spu_dma_getlf_v1

CC0 1.0 (public domain). See LICENSE.md.

R8.4f-a — first MFC GETLF list-DMA + fence oracle target.
Identical to R8.4c GETL (cmd=0x44) except cmd=0x46 (GETL +
`MFC_FENCE_MASK = 0x02`).

## Fence semantics for this fixture

Per RPCS3 `do_dma_check` + `do_list_transfer`:
- `MFC_FENCE_MASK` (0x02) makes THIS command wait until all
  prior commands on the same tag have completed before
  starting.
- For a single-SPU synchronous fixture with ONE list-DMA on a
  FRESH tag (no prior commands on the tag), the fence has NO
  observable effect — `mfc_fence & mask == 0` at dispatch.
- `do_list_transfer` strips the fence bit before the
  per-element copy (`transfer.cmd = args.cmd & ~0xf`), so
  the byte-level data movement is byte-identical to GETL.

## Canonical computation (Python reference)

```python
sum1 = sum(i & 0xFF for i in range(128))   # 0x1FC0
sum2 = 0x42 * 64                           # 0x1080
combined = (sum1 << 16) | sum2             # 0x1FC01080
status = combined ^ 0xC0DEFAFF             # 0xDF1EEA7F
```

XOR mask `0xC0DEFAFF` (last byte `0xFF` mnemonic: "Fence").

Predicted canonical TTY:

```
[dma_getlf_v1] OK cause=0x1 status=0xdf1eea7f
```

## Behaviour

Same as GETLB v1 except cmd=0x46.
