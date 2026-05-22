# single_spu_dma_putlf_v1.notes.md

R8.4f-b — first MFC PUTLF list-DMA + fence replay-validated
oracle (18th oracle). Byte-identical to R8.4e PUTL except
cmd=0x26 (PUTL | MFC_FENCE_MASK). Bridge ON delegates
end-to-end via the existing PUTL callback.

## Origem

Autoral. CC0 1.0. Source at
`behavior-freeze/fixtures/spu/sources/single_spu_dma_putlf_v1/`.

Behaviour identical to PUTLB v1 except cmd=0x26, SPU sentinel
`0xC0FFEEBF` (BF = Fence mnemonic), and `ea_status` mask
`0xBEEFCAFF` → `0xA12FDA7F`.

Canonical TTY:
`[dma_putlf_v1] OK cause=0x1 spu=0xc0ffeebf ea_status=0xa12fda7f`

## Captured artifacts

- `.self`: 939,514 B sha `94e5474e8f62a4e0…`
- `.jsonl`: 15 events
- `.spuimg`: NEW
  `54640ed6b7fc956453be233b3239b2a8787fdd0001a65a962b2d60070706ab17`
- `.dmalistdesc` + `.dmachunk`: REUSE canonical pool

## Acceptance

- captured TTY = `[dma_putlf_v1] OK cause=0x1 spu=0xc0ffeebf ea_status=0xa12fda7f` ✓
- Bridge ON `total_steps=1394` (identical to PUTL)               ✓
- check_triple_symmetry.py --fixture put_list_f                  ✓
- All 18 oracles green                                           ✓
