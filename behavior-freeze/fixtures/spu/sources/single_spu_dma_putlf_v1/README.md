# single_spu_dma_putlf_v1

CC0 1.0 (public domain). See LICENSE.md.

R8.4f-b — first MFC PUTLF list-DMA + fence oracle (18th
oracle target). Symmetric inverse of R8.4f-a GETLF: cmd=0x26
(PUTL | `MFC_FENCE_MASK = 0x02`).

## Canonical computation

```python
sum_ea1 = 0x1FC0; sum_ea2 = 0x1080
combined = (sum_ea1 << 16) | sum_ea2          # 0x1FC01080
ea_status = combined ^ 0xBEEFCAFF             # 0xA12FDA7F
spu_sentinel = 0xC0FFEEBF                     # fixed
```

Predicted canonical TTY:

```
[dma_putlf_v1] OK cause=0x1 spu=0xc0ffeebf ea_status=0xa12fda7f
```

Same behaviour as PUTLB v1 except cmd=0x26 and the masks
above.
