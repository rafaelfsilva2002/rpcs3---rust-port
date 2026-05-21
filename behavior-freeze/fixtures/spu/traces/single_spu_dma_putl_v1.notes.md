# single_spu_dma_putl_v1.notes.md

R8.4e — first MFC PUTL list-DMA (LS → EA) replay-validated
oracle (14th oracle). Symmetric inverse of R8.4c GETL:
identical 8-byte BE descriptor format, identical side-file
pool, opposite direction. Bridge ON delegates end-to-end.

Update history:
- R8.4e (2026-05-21): writer extension (relaxed
  `record_spu_mfc_getl_cmd` to also accept cmd=0x24,
  SPUThread.cpp ch21 dispatch detects PUTL + snapshots LS
  source bytes per element), parser canary lift (cmd=0x24
  moved out of `MFC_LIST_CMDS_UNSUPPORTED`), replay state
  machine extension (`process_mfc_list_cmd` PUTL branch
  validates without mutating LS), real CC0 fixture
  captured, replay test landed, runtime bridge PUTL
  callback (`rust_spu_set_dma_putl_callback` +
  `bridge_dma_putl_callback`), triple-symmetry
  `--fixture put_list` green. All 14 oracles green.

Captured 2026-05-21 from RPCS3 against a CC0 PSL1GHT
homebrew authored for this purpose.

## Origem do homebrew

Autoral. CC0 1.0 (public domain). Source committed at
`behavior-freeze/fixtures/spu/sources/single_spu_dma_putl_v1/`
with LICENSE.md.

Behaviour: PPU allocates two EA destination buffers
pre-initialized to sentinel `0xAA` (so a dropped PUTL
produces a distinct wrong ea_status), passes both EAs via
thread_args.arg0/arg1. SPU fills LS source regions with R8.2-
shared patterns (counting + constant 0x42), builds a
2-element list_element[] in LS, dispatches MFC PUTL
(cmd=0x24), waits via ch22/ch23/ch24 (mask=0x08, ALL),
writes fixed sentinel `0xC0FFEEBA` to OUT_MBOX, halts via
stop 0x101. PPU joins, sums both EA buffers, computes
`ea_status = ((sum_ea1<<16)|sum_ea2) ^ 0xBEEFCAFE =
0xA12FDA7E`, prints canonical TTY.

Canonical TTY: `[dma_putl_v1] OK cause=0x1 spu=0xc0ffeeba ea_status=0xa12fda7e`

## Toolchain

Same `rpcs3-ps3dev-toolchain:local` Docker image
(sha `ed2167a9ac59…`, backed up at
`C:\docker-backup\rpcs3-ps3dev-toolchain-local.tar`).
`.self` 939,511 bytes sha
`d7efc5629cca9fdfb05d07271d4b1813d7cf40c45a6066c1135acd27a9ae76b9`.

## RPCS3 version + capture hooks

R8.4e writer extension active (relaxed
`record_spu_mfc_getl_cmd` cmd gate to also accept 0x24 PUTL +
extended SPUThread.cpp dispatcher to detect 0x24 and snapshot
LS source bytes per element). Patches:

- scaffolding: `402c2d139526a4efd592ba6f052f0c59067aaf09b9d079d75d03ca4a09fe4e5a`
  (R8.4e BUMP from R8.4b/c/d `5c170508…`: relaxed cmd guard +
  doc comment update)
- runtime hooks: `3760b78c8854dd83157f5ef5e501ae85b4fd9b46dc143ae226fb19703bf4a974`
  (R8.4e BUMP from R8.4b/c/d `745945f4…`: SPUThread.cpp ch21
  detects 0x24 + per-element LS source snapshot path with
  cumulative LS bounds check)
- rust bridge: `e09b9c40b3187f89b559c5fcde949a86491974c836525338d51dd2e99600850e`
  (R8.4e BUMP from R8.4d `d2d531850f…`: added
  `bridge_dma_putl_callback` + install in
  `try_delegate_execution`)

`bin/rpcs3.exe`:
- size 64 MB
- sha `64ff57a1248ebb857fcffda2ff392fffa432deb7f0dd75deb07cbb670152cd33`
- built 2026-05-21 with the R8.4e PUTL writer + bridge
  extensions

## Capture procedure

Same as prior R8.x captures: `Core: SPU/PPU Decoder:
Interpreter (static)` both decoders during capture; restored
to `Recompiler (LLVM)` after. LLVM JIT bypasses C++
`set_ch_value()` for MFC channels — same constraint that
landed in R6.7 A.5.

Captured artifacts:

- `behavior-freeze/fixtures/spu/traces/single_spu_dma_putl_v1.jsonl`
  (15 events, ~3.1 KB)
- `behavior-freeze/fixtures/spu/images/3474dea93b83f18920eced5d37725ac19b3ffda6de67c0227a8496bd3a1189dd.spuimg`
  (262,144 bytes; NEW SHA — SPU C source is new for PUTL)
- `.dmalistdesc`: REUSES `79238773912c38db59bf192072b2d89fcb1757d7be59870765cc2be911271126`
  from R8.4b GETL (perfect dedup — PSL1GHT placed `ea_dst1`/
  `ea_dst2` at the same EA layout as R8.4b's `ea_buf1`/
  `ea_buf2`, so the descriptor bytes hash identically)
- `.dmachunk` files: BOTH dedup with existing pool
  - element 0: `471fb943aa23c511f6f72f8d1652d9c880cfa392ad80503120547703e56a2be5`
    (128 B counting pattern, shared with R6.7 GET / R8.1 PUT /
    R8.2..R8.4d)
  - element 1: `c422e7070cb1cb455b5de9afee0d975e303d0239c72030cd7414ab5c382d3ae8`
    (64 B constant 0x42, shared with R8.2..R8.4d)

ZERO new `.dmachunk` files (perfect content-addressed dedup).
ZERO new `.dmalistdesc` files (lucky EA layout match with
R8.4b GETL). ONE new `.spuimg` (different SPU bytecode).

## Trace shape

15 events (matches R8.4b/c GETL count):

1. `spu_image` (LS bytecode snapshot)
2-7. `spu_wrch` ch16-21 (MFC dispatch sequence)
8. `spu_mfc_cmd` cmd=0x24 PUTL with all 5 additive list fields
   populated (`descriptor_sha256`, `descriptor_size`,
   `element_chunks`, `element_sizes`, `element_eals`)
9. `mfc_dma_complete` tag=3, transferred_bytes=192 (= sum of
   ts)
10-11. `spu_wrch` ch22 (mask=0x08), ch23 (ALL=0x02)
12. `spu_rdch` ch24 (value=0x08)
13. `spu_wrch` ch28 (OUT_MBOX = `0xC0FFEEBA` sentinel)
14. `spu_stop` (stop_code=0x101)
15. `final_state`

## Acceptance criteria (R8.4e contract)

- captured TTY matches predicted canonical                       ✓
  `[dma_putl_v1] OK cause=0x1 spu=0xc0ffeeba ea_status=0xa12fda7e`
- 15-event JSONL trace                                          ✓
- spu_mfc_cmd cmd=0x24 with all 5 additive fields populated     ✓
- mfc_dma_complete with transferred_bytes = sum(ts) = 192       ✓
- `.dmalistdesc` dedup with R8.4b pool                          ✓
- both element `.dmachunk` files dedup with pool                ✓
- existing 13 replay oracles remain green                       ✓
- 14 oracles green total                                         ✓
- check_trace_fixtures.py green                                  ✓
- check_patch_separation.py green (3 patches verified)          ✓
- Rust parser accepts cmd=0x24 with the same additive-fields
  validation as cmd=0x44                                         ✓
- Rust replay state machine processes PUTL without mutating LS  ✓
- Bridge ON delegates end-to-end (total_steps=1394, no
  fallback)                                                      ✓
- check_triple_symmetry.py --fixture put_list                    ✓

## R8.4f / R8.5+ — deferred

- PUTLB (cmd=0x25) / PUTLF (cmd=0x26) — list + barrier/fence.
- GETLB (cmd=0x45) / GETLF (cmd=0x46) — list + barrier/fence.
- Stall-and-notify (descriptor `sb` bit 0x80) — needs SPU-to-
  PPU signaling integration via `mfc_notify` channel.
- 3+ element fixtures + descriptor edge cases (256-element
  max).

## Stability

Once committed, this trace is a regression sentinel. Re-
capturing this fixture against a future rpcs3.exe MUST
produce byte-identical JSONL (ignoring trace_path absolutes
that get redirected to `<trace_path>.dma/`). The
`.dmalistdesc` SHA `79238773…` IS the canonical descriptor
content for this SPU bytecode — if a future capture produces
a different descriptor SHA, either the SPU bytecode changed
(re-build) or the linker placed `ea_dst1`/`ea_dst2` at
different EAs (re-investigate dedup).
