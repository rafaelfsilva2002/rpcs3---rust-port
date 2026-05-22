# single_spu_dma_getlb_v1.notes.md

R8.4f-a — first MFC GETLB list-DMA + barrier replay-validated
oracle (15th oracle). Byte-identical to R8.4c GETL except
cmd=0x45 (GETL | MFC_BARRIER_MASK). Bridge ON delegates
end-to-end (`total_steps=1598`).

## Barrier semantics

Per RPCS3 `do_list_transfer` at `SPUThread.cpp:2887`:
`transfer.cmd = args.cmd & ~0xf` — the barrier (0x01) bit is
stripped before the per-element copy. Per `do_dma_check` at
`SPUThread.cpp:2819`, the barrier persistence in `mfc_barrier`
only affects SUBSEQUENT commands on the same tag. This is a
single-SPU fresh-tag single-dispatch fixture, so:

- Data path: byte-identical to plain GETL.
- Ordering effect: not observable.

This is the load-bearing justification for the R8.4f-a
"reuse GETL semantics" strategy across parser, replay state
machine, runtime bridge callback, and triple-symmetry
expectations.

## Origem do homebrew

Autoral. CC0 1.0 (public domain). Source at
`behavior-freeze/fixtures/spu/sources/single_spu_dma_getlb_v1/`.

Behaviour mirrors R8.4c GETL: PPU allocates `ea_buf1` (128 B
counting) + `ea_buf2` (64 B constant 0x42), passes EAs via
arg0/arg1. SPU builds 2-element list_element[] in LS,
dispatches MFC GETLB (cmd=0x45) via ch16-21, waits ch22-24
(mask=0x08, ALL), sums both copied LS regions, computes
`status = ((sum1<<16)|sum2) ^ 0xC0DEFABB = 0xDF1EEA3B`,
writes to OUT_MBOX, halts via stop 0x101.

Status XOR mask `0xC0DEFABB` mirrors GETL's `0xC0DEFADA` with
the last byte `0xBB` (mnemonic: "Barrier") so a regression
that accidentally accepts cmd=0x45 but routes through GETL's
status formula would surface as `0xDF1EEA5A` instead of
`0xDF1EEA3B`.

Canonical TTY:
`[dma_getlb_v1] OK cause=0x1 status=0xdf1eea3b`

## Toolchain + RPCS3 hooks

Same `rpcs3-ps3dev-toolchain:local` Docker image as prior R8.x
captures. `.self` 939,514 bytes sha
`f490be23d1af05f8…`. Build with `Core: SPU/PPU Decoder:
Interpreter (static)` (restored to LLVM post-capture).

R8.4f-a writer extension active:

- scaffolding: `5085c4afaa5dd2df7526999b7f7f0ed33b763ce4c66d4decef55a2fa2b427364`
  (R8.4f-a BUMP: relaxed `record_spu_mfc_getl_cmd` cmd guard
  to accept 0x44/0x24/0x45/0x46)
- runtime hooks: `67bef0455eeedc511443c7d283841fd5080d703dac0b8bc11743b97a971a3dc8`
  (R8.4f-a BUMP: SPUThread.cpp ch21 dispatcher detects
  GETL/GETLB/GETLF as `getl_family`)
- rust bridge: `b9e5e977bc3f97b5e1a86f56a5d6affd79d831f3c9f4b47226511a242a45a713`
  (R8.4f-a BUMP: GETL log line generalized for GETL/GETLB/GETLF)

`bin/rpcs3.exe` sha
`f3d4e85f3d2e375bb9d58e8414a3e2f9699c3a25a6210eba998d3a869ee665ac`
(R8.4f-a rebuild).

## Captured artifacts

- `traces/single_spu_dma_getlb_v1.jsonl` (15 events, ~3 KB)
- `images/9ab0058de577e6fd8aa1caa6fde58b3c8f744a7d0afc018cab724df05c19df99.spuimg`
  (NEW SHA, different SPU bytecode)
- `.dmalistdesc`: REUSES R8.4b/c GETL's
  `79238773912c38db59bf192072b2d89fcb1757d7be59870765cc2be911271126`
  (lucky EA layout dedup)
- `.dmachunk` element 0: REUSES
  `471fb943aa23c511f6f72f8d1652d9c880cfa392ad80503120547703e56a2be5`
  (counting pattern, since R6.7)
- `.dmachunk` element 1: REUSES
  `c422e7070cb1cb455b5de9afee0d975e303d0239c72030cd7414ab5c382d3ae8`
  (constant 0x42, since R8.2)

ZERO new `.dmachunk` / `.dmalistdesc` files (perfect dedup).
ONE new `.spuimg`.

## Acceptance criteria (R8.4f-a contract)

- captured TTY matches predicted canonical `0xDF1EEA3B`       ✓
- 15-event JSONL                                              ✓
- spu_mfc_cmd cmd=0x45 with all 5 additive fields populated   ✓
- mfc_dma_complete transferred_bytes = sum(ts) = 192          ✓
- side-files dedup with R8.4b/c GETL pool                     ✓
- existing 14 oracles remain green                            ✓
- Rust parser accepts cmd=0x45 with same additive-fields
  validation as GETL                                          ✓
- Rust replay state machine routes GETLB to
  `process_mfc_list_cmd` (reuses GETL's EA→LS copy)           ✓
- Runtime bridge GETL callback handles GETLB
  (interpreter routes 0x45 to dma_getl_callback)              ✓
- Bridge ON delegates end-to-end
  (total_steps=1598, no fallback)                             ✓
- check_triple_symmetry.py --fixture get_list_b               ✓

## Stability

The descriptor SHA `79238773…` dedup with R8.4b GETL is
LUCKY — depends on PSL1GHT static-allocator placing
`ea_buf1`/`ea_buf2` at identical EAs across builds. If a
future re-capture produces a different `.dmalistdesc` SHA,
investigate whether the linker layout drifted or the source
changed.

## R8.4f-b deferred

PUTLB (cmd=0x25) and PUTLF (cmd=0x26) — symmetric inverse
(LS→EA with barrier/fence). Same pattern as R8.4f-a:
reuse PUTL data path, lift canary for 0x25/0x26.

R8.5+ deferred: stall-and-notify bit 0x80 (needs SPU→PPU
signaling), 3+ element fixtures, descriptor edge cases.
