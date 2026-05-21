# single_spu_dma_getl_v1.notes.md

R8.4b — first MFC GETL (list-DMA EA→LS) capture. **R8.4c
PROMOTED TO 13TH REPLAY-VALIDATED ORACLE** (2026-05-21).

Update history:
- R8.4b (2026-05-21): writer extension + real capture only.
  Replay state machine rejected via R8.4a canary.
- R8.4c (2026-05-21): replay state machine landed
  (`MfcReplayState::process_mfc_list_cmd`), R8.4a canary
  lifted for cmd 0x44 only, side-file resolver
  `resolve_dma_listdesc_side_file` added, **promoted to
  13th replay-validated oracle**. Cross-backend
  byte-identical via `diff_snapshots(interp, jit)
  .is_identical()` confirmed.
- R8.4d (pending): runtime bridge GETL callback +
  triple-symmetry expansion. Until then, bridge ON falls
  back to C++ at the GETL dispatch.

Captured 2026-05-21 from RPCS3 against a CC0 PSL1GHT
homebrew authored for this purpose.

## Origem do homebrew

Autoral. CC0 1.0 (public domain). Source committed at
`behavior-freeze/fixtures/spu/sources/single_spu_dma_getl_v1/`
with LICENSE.md.

Behaviour: PPU prepares 2 EA buffers (128 B counting + 64 B
constant 0x42), passes both EAs via thread_args.arg0/arg1.
SPU builds a 2-element list_element[] in LS, dispatches MFC
GETL (cmd=0x44), waits via ch22/ch23/ch24 (mask=0x08, ALL),
sums both copied LS regions, writes canonical OUT_MBOX status
`0xDF1EEA5A` = `((sum1 << 16) | sum2) ^ 0xC0DEFADA`, halts
via stop 0x101.

## Toolchain

Same `rpcs3-ps3dev-toolchain:local` Docker image
(sha `ed2167a9ac59…`). `.self` 940 KB sha
`df041b4a67fa8dcf55bd9c93295177c9c530e04f7eeabae95b0908e7d59d8402`.

## RPCS3 version + capture hooks

R8.4b writer extension active. Patches:

- scaffolding: `5c170508a73e492d42784036d61a972edab7a85b7ea7105d6dde388a5e67d6c0`
  (R8.4b BUMP: added `record_spu_mfc_getl_cmd` + `write_dma_listdesc_side_file`)
- runtime hooks: `745945f4872f7d83541aa74d9a065b6a6bc3785af73510d026e6643a0985cd96`
  (R8.4b BUMP: SPUThread.cpp ch21 dispatch detects cmd=0x44,
  snapshots descriptor + per-element EA bytes, calls
  `record_spu_mfc_getl_cmd`, emits `mfc_dma_complete` with
  `transferred_bytes = sum(ts)`)
- rust bridge: `0afda1c6…` (UNCHANGED — no bridge runtime
  GETL yet, R8.4d scope)

`bin/rpcs3.exe`:
- size 63,942,656 bytes
- sha `3f2348de0e50b7dd4aadeac92aaec83b678cd84f7f7be88742835cc0c38a0b72`
- (R7.1 + R7.2 + R8.1 + R8.3a + R8.3b + R8.4b surface; built
  2026-05-21 with the GETL writer extension landed)

## Capture procedure

Same as prior R8.x captures: `Core: SPU/PPU Decoder:
Interpreter (static)` both decoders during capture; restored
to `Recompiler (LLVM)` after.

Captured artifacts:

- `behavior-freeze/fixtures/spu/traces/single_spu_dma_getl_v1.jsonl`
  (15 events, ~2 KB)
- `behavior-freeze/fixtures/spu/images/f0878b15077adc224ccfd89e62134952637923a9a5cb0ba2277d999dac366e18.spuimg`
  (262,144 bytes; NEW SHA — SPU C source is new for GETL)
- `behavior-freeze/fixtures/spu/dma/79238773912c38db59bf192072b2d89fcb1757d7be59870765cc2be911271126.dmalistdesc`
  (16 bytes; NEW kind of side-file: list descriptor array,
  2 × 8 bytes, format documented in DESIGN § 19.1)
- `.dmachunk` files: BOTH dedup with existing pool
  - element 0: `471fb943aa23c511f6f72f8d1652d9c880cfa392ad80503120547703e56a2be5`
    (128 B counting pattern, shared with R6.7 GET / R8.1
    PUT / R8.2 / R8.3a/b/c)
  - element 1: `c422e7070cb1cb455b5de9afee0d975e303d0239c72030cd7414ab5c382d3ae8`
    (64 B constant 0x42, shared with R8.2 / R8.3a/b/c)
  - ZERO new `.dmachunk` files — content-addressed pool at
    peak dedup.

## Trace contents (15 events)

```
seq  0: spu_image          sha=f0878b15…  size=0x40000  entry_pc=0
seq  1: spu_wrch  ch16=0x10000     pc=124  (LSA dest base)
seq  2: spu_wrch  ch17=0x0         pc=132  (EAH)
seq  3: spu_wrch  ch18=0x180       pc=136  (EAL = LS offset
                                              of descriptor list)
seq  4: spu_wrch  ch19=0x10        pc=144  (Size = 16 bytes
                                              = 2 elements × 8)
seq  5: spu_wrch  ch20=0x3         pc=152  (TagID = 3)
seq  6: spu_wrch  ch21=0x44        pc=160  (Cmd = GETL)
seq  7: spu_mfc_cmd cmd=0x44 tag=3 size=16 lsa=0x10000 eah=0
        eal=0x180 ea_chunk_sha256=79238773…
        descriptor_sha256=79238773… descriptor_size=16
        element_chunks=[471fb943…, c422e707…]
        element_sizes=[128, 64]
        element_eals=[0x10011180, 0x10011200]
seq  8: mfc_dma_complete tag=3 transferred_bytes=192
        (= 128 + 64)
seq  9: spu_wrch  ch22=0x08        pc=164  (MFC_WrTagMask)
seq 10: spu_wrch  ch23=0x2         pc=172  (MFC_WrTagUpdate
                                              = ALL)
seq 11: spu_rdch  ch24=0x08        pc=176  (RdTagStat)
seq 12: spu_wrch  ch28=0xDF1EEA5A  pc=304  (OUT_MBOX canonical)
seq 13: spu_stop  stop_code=0x101  pc=308
seq 14: final_state  r48=0xDF1EEA5A (= status), others
```

Diff vs prior DMA fixtures: cmd field is 68 (= 0x44 GETL)
instead of 64 (GET) / 32 (PUT); `spu_mfc_cmd` event carries
5 new fields (descriptor_sha256, descriptor_size,
element_chunks, element_sizes, element_eals).

## Acceptance criteria (R8.4b contract)

- captured TTY matches predicted canonical `0xDF1EEA5A`        ✓
- 15-event JSONL trace                                          ✓
- spu_mfc_cmd cmd=0x44 with all 5 additive fields populated     ✓
- mfc_dma_complete with transferred_bytes = sum(ts) = 192       ✓
- new side-file `<sha>.dmalistdesc` lands                        ✓
- both element `.dmachunk` files exist (dedup with pool ok)     ✓
- existing 12 replay oracles remain green                       ✓
- check_trace_fixtures.py green                                 ✓
- check_patch_separation.py green (3 patches verified)          ✓
- Rust parser deserializes additive fields into
  `SpuMfcCmdEvent::{descriptor_sha256,...,element_eals}`        ✓
- Rust parser still REJECTS at validate with
  `UnsupportedMfcListCmd { cmd: 0x44, .. }` (R8.4a canary
  preserved — replay/transform still rejects per design)        ✓

## R8.4c — IMPLEMENTED

- `MfcReplayState::process_mfc_list_cmd` ✓ landed: walks
  descriptor (8-byte BE slots: sb / pad / u16 ts / u32 ea),
  validates per-element (sb stall-and-notify bit MUST be 0,
  ts in (0, 0x4000], descriptor ts/ea must match trace
  event's `element_sizes` / `element_eals`), loads each
  element chunk via existing `.dmachunk` resolver, copies
  bytes into LS at cumulative offset (`lsa_base + sum of
  prior ts`), registers in-flight tag with the size = sum
  of ts.
- `resolve_dma_listdesc_side_file` ✓ added to
  `dma_chunk.rs` (mirror of `resolve_dma_chunk_side_file`
  with `.dmalistdesc` extension + 0x800 size cap).
- `apply_mfc_dma_pre_replay` ✓ dispatches cmd=0x44 to the
  new path via `process_mfc_cmd_pre_replay`.
- R8.4a canary `UnsupportedMfcListCmd` ✓ lifted for cmd=
  0x44 ONLY (PUTL/PUTLB/PUTLF/GETLB/GETLF still rejected;
  canary unit test updated to iterate only the 5
  unsupported codes).

## R8.4d (still pending)

- Runtime bridge `rust_spu_set_dma_getl_callback` FFI.
- C++ bridge handler in `SPURustBridge.cpp` (read
  descriptor from SPU's LS, walk elements, copy each from
  `vm::_ptr<u8>(ea)` to LS at cumulative offset).
- rpcs3.exe rebuild + bridge patch SHA bump.
- Triple-symmetry expansion for `--fixture get_list`.

## Stability

Once committed, this trace is a regression sentinel for the
WRITER. Re-capturing this fixture against a future rpcs3.exe
build MUST produce byte-identical JSONL (ignoring trace_path
absolutes that get redirected to `<trace_path>.dma/`). The
`.dmalistdesc` SHA `79238773…` IS the canonical descriptor
content for this SPU bytecode — if a future capture produces
a different descriptor SHA, either the SPU bytecode changed
(re-build) or the writer changed (debug).
