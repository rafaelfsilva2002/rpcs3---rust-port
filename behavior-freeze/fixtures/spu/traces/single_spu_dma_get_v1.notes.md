# single_spu_dma_get_v1.notes.md

R6.7 A.5 — first replay-validated SPU DMA GET fixture. Captured
2026-05-03 from RPCS3 against a CC0 PSL1GHT homebrew authored for
this purpose. Adds the seventh oracle to the `behavior-freeze/`
harness and is the first one whose JSONL contains `spu_mfc_cmd` +
`mfc_dma_complete` events.

## Origem do homebrew

Autoral. CC0 1.0 (public domain). Source committed at
`behavior-freeze/fixtures/spu/sources/single_spu_dma_get_v1/` with
LICENSE.md. Two .c files (PPU `main.c` + SPU `spu/spu_dma_get.c`)
+ Makefile + README.md. Targets PSL1GHT runtime.

Comportamento (uma linha): PPU allocates a 128-byte BSS buffer
filled with `buf[i] = i & 0xFF`, passes its EA via
`thread_args.arg0`; SPU runs a complete MFC GET (ch16-21 wrch,
ch22-23 wait setup, ch24 RdTagStat blocking read), sums the 128
GET'd bytes (= 8128 = 0x1FC0), XORs with 0xDEADBEEF to produce
canonical status `0xDEADA12F`, writes that to OUT_MBOX, halts via
stop 0x101 (SYS_SPU_THREAD_STOP_GROUP_EXIT).

Race-free single-round, GET-only. The fixture is the load-bearing
R6.7 oracle: the only way to produce status `0xDEADA12F` is to
(a) actually load the pre-DMA EA bytes into LS, AND (b) compute
the deterministic post-DMA sum + XOR. A bridge bug that drops the
GET silently (zero-fills LS) would produce a different status.

## Toolchain

Same `ps3toolchain` Docker image as the prior 6 oracles
(`rpcs3-ps3dev-toolchain:local`, image
sha256 `ed2167a9ac59…`, content size 2.43 GB; backup at
`C:\docker-backup\rpcs3-ps3dev-toolchain-local.tar`).

Build command (in container):

```
cd behavior-freeze/fixtures/spu/sources/single_spu_dma_get_v1
PS3DEV=/usr/local/ps3dev PSL1GHT=/usr/local/ps3dev/psl1ght make
```

Output: `build/single_spu_dma_get_v1.self` — 939,475 bytes,
sha256 `7b0761849ff64048dd4852d8fa9361cb70cec2dfe08ec5ef54e911fc53b333a0`.

## RPCS3 version + capture hooks

RPCS3 build: ToT from this repository at capture time, with the
R5.9c + R5.9e.3 SPU trace writer **plus** R6.7 A.1 DMA writer
extension (commits `cda976d7…` scaffolding + `95bdcaae…` runtime
hooks). The runtime hooks emit `spu_mfc_cmd` (with EA snapshot →
content-addressed `<sha>.dmachunk` side-file) and
`mfc_dma_complete` events for plain GET (cmd 0x40, eah=0,
size in {1,2,4,8} ∪ multiples of 16, lsa+size ≤ 256 KiB, tag < 32).

Bridge patch `7d6b6bba…` unchanged (does not handle DMA in R6.7;
runtime DMA delegation is out of scope).

`bin/rpcs3.exe` for the capture:
- size 63,936,512 bytes
- sha256 `30f9e228bcb58dab…` (built 2026-05-03 from
  `rpcs3-upstream-clean` worktree with R6.7 A.1 patches applied
  unstaged on branch `spu-trace-jsonl-runtime-hooks`)

## Capture procedure

`SPU Decoder` and `PPU Decoder` are both temporarily set to
`Interpreter (static)` in `bin/config/config.yml` for the capture
run, then restored to `Recompiler (LLVM)`. With LLVM JIT the
recompiler bypasses the C++ `set_ch_value()` / `get_ch_value()`
host-side handlers and the R6.7 A.1 hooks never fire — only the
interpreter dispatches every wrch/rdch through the hooked path.
Existing 6 oracles work with LLVM because their replay needs only
`spu_image` + `spu_wrch ch28` + `spu_stop` + `final_state`; the
DMA fixture additionally requires the per-channel events and the
`spu_mfc_cmd` / `mfc_dma_complete` / `.dmachunk` triple.

Driven by `.r67a5_capture.bat` in the upstream-clean worktree:

1. `RPCS3_SPU_TRACE_JSONL` env var points at the canonical fixture
   path under `behavior-freeze/fixtures/spu/traces/`.
2. `rpcs3.exe --no-gui --headless` invoked on the .self.
3. After lv2 reads OUT_MBOX as the group-exit status, RPCS3 exits
   and the trace writer destructor flushes the JSONL.
4. .spuimg + .dmachunk side-files are written to per-trace
   `<jsonl>.images/` and `<jsonl>.dma/` subdirs first; this
   capture moves them to the canonical pool dirs under
   `behavior-freeze/fixtures/spu/{images,dma}/` to be shared
   across fixtures (loader prefers per-trace then falls back to
   canonical).

Captured artifacts:

- `behavior-freeze/fixtures/spu/traces/single_spu_dma_get_v1.jsonl`
  (15 events, 2,347 bytes)
- `behavior-freeze/fixtures/spu/images/97a38063…ef56.spuimg`
  (262,144 bytes — full LS at thread create, mostly zeros + .self
  loaded segments)
- `behavior-freeze/fixtures/spu/dma/471fb943…2be5.dmachunk`
  (128 bytes — exact EA bytes the PPU wrote, sum = 8128 = 0x1FC0)

## Trace contents (15 events)

```
seq  0: spu_image          sha=97a38063… load=0x0     size=0x40000  entry_pc=0x0
seq  1: spu_wrch  ch16=0x10000  pc=12   (MFC_LSA — LS destination)
seq  2: spu_wrch  ch17=0x0      pc=20   (MFC_EAH — high 32 bits of EA = 0 on PS3 32-bit user space)
seq  3: spu_wrch  ch18=0x10068400 pc=28 (MFC_EAL — low 32 bits of EA buffer)
seq  4: spu_wrch  ch19=0x80     pc=36   (MFC_Size — 128 bytes)
seq  5: spu_wrch  ch20=0x3      pc=44   (MFC_TagID — tag 3)
seq  6: spu_wrch  ch21=0x40     pc=52   (MFC_Cmd — GET 0x40)
seq  7: spu_mfc_cmd cmd=0x40 tag=3 size=128 lsa=0x10000 eah=0 eal=0x10068400
                                                 ea_chunk_sha256=471fb943…
seq  8: mfc_dma_complete tag=3 transferred_bytes=128
seq  9: spu_wrch  ch22=0x8      pc=60   (MFC_WrTagMask — mask bit 3)
seq 10: spu_wrch  ch23=0x2      pc=68   (MFC_WrTagUpdate — MFC_TAG_UPDATE_ALL = 2)
seq 11: spu_rdch  ch24=0x8      pc=72   (MFC_RdTagStat — returns mask 1<<3)
seq 12: spu_wrch  ch28=0xDEADA12F pc=144 (OUT_MBOX — canonical post-DMA checksum)
seq 13: spu_stop  stop_code=0x101 pc=148
seq 14: final_state  gpr={r1=0x3FFF0, r2=0x10000, r3=0x8, r4=0x10000, r6=0x80,
                          r7=0x3, r8=0x40, r9=0x8, r10=0x2, r18=0x1FC0,
                          r19=0xDEADBEEF, r20=0xDEADA12F, …}
                     channels={in_mbox=null, out_mbox=null, out_intr_mbox=null,
                               snr1=0, snr2=0}
```

Acceptance criteria (R6.7 A.5 contract):

- exactly 1 spu_image event                                                  ✓
- exactly 1 target_spu (256)                                                 ✓
- exactly 1 spu_mfc_cmd event with cmd=0x40 (GET)                            ✓
- exactly 1 mfc_dma_complete event with same tag (3) and size (128)         ✓
- ch16-23 wrch + ch24 rdch sequence in the parser-mandated order            ✓
- spu_wrch ch28 = 0xDEADA12F                                                 ✓ (canonical)
- spu_stop with stop_code = 0x101                                            ✓
- .dmachunk content sums to 8128 = 0x1FC0                                    ✓ (verified)
- .dmachunk SHA-256 matches `ea_chunk_sha256` in spu_mfc_cmd event           ✓ (verified)
- final_state r20 = 0xDEADA12F (canonical post-XOR)                          ✓
- final_state r18 = 8128 = 0x1FC0 (sum of GET'd bytes 0..127)                ✓
- final_state r19 = 0xDEADBEEF (the XOR mask)                                ✓

## Replay-validation

Drives the full pipeline from
`rust/rpcs3-spu-recompiler/tests/single_spu_dma_get_v1_replay.rs`:

```
parse_jsonl_trace
  -> captured_events_to_traces_per_spu (now accepts MFC events)
  -> build_spu_program_from_captured_image
  -> apply_mfc_dma_pre_replay  (R6.7 C.3 helper: A.3 loader resolves
     <sha>.dmachunk → A.4 MfcReplayState → injects 128 bytes into LS
     at lsa=0x10000 + populates tag_stat_queue with 0x8)
  -> replay_per_spu_traces::<InterpreterExecutor>
  -> replay_per_spu_traces_with(|_| RecompilerExecutor::new())
  -> diff_snapshots(interp, jit).is_identical()
```

Status: ✅ parser ok / transformer ok (drops MFC events as
context, no hard-reject) / chunk loader ok (per-trace fallback
canonical) / pre-replay LS injection ok / interp replay ok / JIT
replay ok / cross-backend snapshot diff identical.

## Engine-side fixes landed for this fixture

R6.7 A.2-A.4 + Phase C extensions (all general — kept landed):

1. **Parser extension** (`trace_fmt.rs` R6.7 A.2): recognizes
   `spu_mfc_cmd` + `mfc_dma_complete` events with full validation
   (cmd ∈ {0x40}, eah == 0, tag < 32, size ∈ {1,2,4,8} ∪ multiples
   of 16 ≤ 0x4000, lsa + size ≤ 256 KiB, ordering invariant
   ch21→spu_mfc_cmd). 8 new TraceParseError variants.

2. **DMA chunk loader** (`dma_chunk.rs` R6.7 A.3):
   `resolve_dma_chunk_side_file()` searches per-trace
   `<jsonl>.dma/<sha>.dmachunk` first, then canonical
   `behavior-freeze/fixtures/spu/dma/<sha>.dmachunk`. SHA-256 +
   size validated on load. 7 error variants.

3. **MfcReplayState** (`mfc_replay.rs` R6.7 A.4): standalone
   state machine for ch16-25 → process_mfc_cmd → tag_stat_queue
   semantics. Supports Immediate / Any / All wait modes.

4. **Phase C executor wiring** (R6.7 C.1-C.5):
   `rpcs3-spu-thread` adds ch16-25 to its `ch::` module +
   `SpuChannels` fields + a `tag_stat_queue: VecDeque<u32>`.
   `SpuProgram::with_mfc_tag_stat_queue` plumbs the queue into
   InterpreterExecutor + RecompilerExecutor.
   `apply_mfc_dma_pre_replay()` bridges A.3 loader + A.4 state
   machine into a `DmaPreReplayPlan`. Transformer accepts MFC
   events as pure context (no hard-reject post-Phase C).

## Stability

Once committed, this trace is a regression sentinel. Do NOT delete
or edit without recording the reason here (e.g. RPCS3 trace writer
schema change → recapture; SPU C source change → bump to
`single_spu_dma_get_v2`; toolchain rev → re-build .self with
matching tag).

The .dmachunk content is the load-bearing payload: it MUST hash to
the SHA-256 referenced in the JSONL `spu_mfc_cmd` event AND it
MUST sum to 8128. Either invariant breaking should be treated as a
suspected corruption — re-capture from a clean RPCS3 build before
editing anything by hand.

## Capture environment forensics

- VS BuildTools: 2022 BuildTools (MSVC 14.44.35225.0)
- VULKAN_SDK: `C:\Users\manod\VulkanSDK\local`
- Qt: 6.8.0 msvc2022_64
- `R:\` SUBST drive mapped to `rpcs3-upstream-clean/` repo root
  (legacy build configuration; `link.command.1.tlog` LIBPATHs are
  baked-in `R:\BUILD\LIB\RELEASE-X64\GLSLANG` etc. — without the
  SUBST in place, the linker silently falls through to
  `$(VULKAN_SDK)\Lib\glslang.lib` which is `/MD` and conflicts
  with the rpcs3 `/MT` build, surfacing as 52 unresolved
  `spvtools::Optimizer` externals from `glslang.lib(SpvTools.obj)`).
  Fix: re-establish `subst R: <repo>` before building. Documented
  in this file so the build path is reproducible.
