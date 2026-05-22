# single_spu_dma_getlf_v1.notes.md

R8.4f-a — first MFC GETLF list-DMA + fence replay-validated
oracle (16th oracle). Byte-identical to R8.4c GETL except
cmd=0x46 (GETL | MFC_FENCE_MASK). Bridge ON delegates
end-to-end (`total_steps=1598`).

## Fence semantics

Per RPCS3 `do_list_transfer` at `SPUThread.cpp:2887`:
`transfer.cmd = args.cmd & ~0xf` — the fence (0x02) bit is
stripped before the per-element copy. Per `do_dma_check` at
`SPUThread.cpp:2819`, the fence bit only affects whether THIS
command waits for prior commands on the same tag. This is a
single-SPU fresh-tag single-dispatch fixture, so:

- Data path: byte-identical to plain GETL.
- Ordering effect: not observable (no prior commands on the
  tag to wait for).

## Origem do homebrew

Autoral. CC0 1.0 (public domain). Source at
`behavior-freeze/fixtures/spu/sources/single_spu_dma_getlf_v1/`.

Behaviour identical to GETLB v1 except cmd=0x46 and status
mask 0xC0DEFAFF (last byte `0xFF` mnemonic: "Fence").

Canonical TTY:
`[dma_getlf_v1] OK cause=0x1 status=0xdf1eea7f`

## Toolchain + RPCS3 hooks

Same as GETLB. `.self` 939,514 bytes sha
`f3201485f5266327…`. Captured against R8.4f-a rpcs3.exe
(`f3d4e85f…`) with all 3 patches applied.

## Captured artifacts

- `traces/single_spu_dma_getlf_v1.jsonl` (15 events)
- `images/3bdc07e4bf7c5a05505b73d800431b9d5cf46b126fdf45474f26f3777ea66b0d.spuimg`
  (NEW SHA)
- `.dmalistdesc`: REUSES R8.4b/c/f-a (same EA layout)
- `.dmachunk` element 0/1: REUSES canonical pool

ZERO new `.dmachunk` / `.dmalistdesc`. ONE new `.spuimg`.

## Acceptance criteria

- captured TTY = `[dma_getlf_v1] OK cause=0x1 status=0xdf1eea7f` ✓
- 15-event JSONL                                                ✓
- spu_mfc_cmd cmd=0x46 + 5 additive fields                      ✓
- mfc_dma_complete transferred_bytes = 192                      ✓
- Rust parser accepts cmd=0x46                                  ✓
- Replay state machine routes GETLF via `process_mfc_list_cmd`  ✓
- Bridge ON delegates end-to-end (total_steps=1598)             ✓
- check_triple_symmetry.py --fixture get_list_f                 ✓
- All 16 oracles green                                          ✓
