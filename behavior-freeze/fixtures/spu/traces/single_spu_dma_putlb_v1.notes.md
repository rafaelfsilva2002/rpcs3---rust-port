# single_spu_dma_putlb_v1.notes.md

R8.4f-b — first MFC PUTLB list-DMA + barrier replay-validated
oracle (17th oracle). Byte-identical to R8.4e PUTL except
cmd=0x25 (PUTL | MFC_BARRIER_MASK). Bridge ON delegates
end-to-end via the existing PUTL callback.

## Barrier semantics

Same as R8.4f-a GETLB but on the PUT direction. Per
`SPUThread.cpp:2887` `do_list_transfer` strips the barrier bit
(0x01) before the per-element copy → data path byte-identical
to PUTL. Per `do_dma_check:2819`, `mfc_barrier` only gates
SUBSEQUENT cmds on the same tag (no effect for single-SPU
fresh-tag single-dispatch).

## Origem

Autoral. CC0 1.0. Source at
`behavior-freeze/fixtures/spu/sources/single_spu_dma_putlb_v1/`.

Behaviour: PPU allocates EA destinations init to 0xAA, SPU
fills LS source with counting+0x42, builds descriptor list,
dispatches PUTLB (cmd=0x25), waits, writes spu_sentinel
`0xC0FFEEBB` (BB = Barrier mnemonic), halts. PPU sums EA
buffers and computes `ea_status = ((sum_ea1<<16)|sum_ea2) ^
0xBEEFCABB = 0xA12FDA3B`.

Canonical TTY:
`[dma_putlb_v1] OK cause=0x1 spu=0xc0ffeebb ea_status=0xa12fda3b`

## Captured artifacts

- `.self`: 939,514 B sha `120f43ba27eb2123…`
- `.jsonl`: 15 events
- `.spuimg`: NEW
  `1a659f3225c59df282aa2d17c99404cf46a23ac42bb54b800d5b7b369dab6126`
- `.dmalistdesc`: REUSES `79238773…` (since R8.4b GETL)
- `.dmachunk`: REUSES canonical pool (since R6.7 / R8.2)

## Patches at R8.4f-b landing

- scaffolding: `d9d60bfa01a942c0523ac4ae5f8307c9bd89c57efc0736b432dc1e38db1d482c`
- runtime hooks: `e53518c4393e416d08ad09257ddf0af9c92ff7011a3f0524ff1db9c70593519e`
- bridge: `106ddede745c6487e3b1f4dbe61c272beb3c16835c164a952a0799ed4de3e899`
- rpcs3.exe: `85e6fe8d09f7ae02d0cc258f8087a0eb46ab25bde3c66e8eda0050682626f428`

## Acceptance

- captured TTY = `[dma_putlb_v1] OK cause=0x1 spu=0xc0ffeebb ea_status=0xa12fda3b` ✓
- 15-event JSONL                                                ✓
- spu_mfc_cmd cmd=0x25 with 5 additive fields                   ✓
- mfc_dma_complete transferred_bytes = 192                      ✓
- side-files dedup with R8.4b/c/e/f-a pool                      ✓
- existing 16 oracles remain green                              ✓
- replay state machine routes PUTLB via `process_mfc_list_cmd`
  (PUTL branch, `is_putl == true`, no LS mutation)              ✓
- runtime bridge PUTL callback handles PUTLB
  (interpreter routes 0x25 to dma_putl_callback)                ✓
- Bridge ON delegates end-to-end (`total_steps=1394`, identical
  to PUTL, no fallback)                                         ✓
- check_triple_symmetry.py --fixture put_list_b                 ✓
