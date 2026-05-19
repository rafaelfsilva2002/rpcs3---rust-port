# Project Status — R7 CLOSED (Triple-symmetric DMA bridge)

**Authoritative current source of truth for the RPCS3 → Rust port.**

Last updated: **2026-05-18 (R7 closure)**. R7 phases R7.1 (Bridge
Phase B honest fallback) + R7.2 (Bridge Phase D runtime DMA GET) +
R7.3 (triple-symmetry regression gate) all ACEITO. The runtime
bridge now executes the first DMA-bound oracle
`single_spu_dma_get_v1.self` end-to-end through the Rust executor
under `RPCS3_SPU_RUST_BRIDGE=1`, with byte-identical output
(`0xdeada12f`) versus bridge OFF and versus the replay oracle. See
"R7 phase closure (2026-05-18)" below.

Previously: **2026-05-03 (R6 closure)**. The text below describes the
current state as of R6 closure on that date. Long-form R5/R4 history
(R5.9e.7 / R5.11 / R5.11b material and the iteration-by-iteration
R5.4a..p timeline) has been moved verbatim to
[`docs/history/PROJECT_STATUS_R5_ARCHIVE.md`](./history/PROJECT_STATUS_R5_ARCHIVE.md).
Do NOT treat the archive as current.

---

## 1. Executive current status

- **R7 is formally CLOSED.** R7.1 (Bridge Phase B honest fallback)
  + R7.2 (Bridge Phase D runtime DMA GET via FFI callback into
  `vm::_ptr<u8>`) + R7.3 (triple-symmetry regression gate) all
  ACEITO 2026-05-18. The runtime bridge now executes the
  first DMA-bound oracle `single_spu_dma_get_v1.self` end-to-end
  through the Rust executor under `RPCS3_SPU_RUST_BRIDGE=1`, with
  byte-identical canonical status (`0xdeada12f`) vs bridge OFF and
  vs the replay oracle. See § 8 R7 closure summary.
- **R6 is formally CLOSED** (2026-05-03). R6.2 first delegation →
  R6.3a/b/c per-oracle delegation → R6.4a outcome contract → R6.4b
  persistent handle → R6.5 / R6.5b bridge acceptance → R6.6 game_like
  cross-path → R6.7 design + A.1-A.5 + Phase C — all landed and
  gated green.
- **Seven replay-validated SPU oracles exist**, all
  `diff_snapshots(interp, jit).is_identical()` byte-identical
  across `InterpreterExecutor` and `RecompilerExecutor`.
- **`single_spu_dma_get_v1` is the first DMA-bound oracle**
  (R6.7 A.5, replay-validated 2026-05-03; runtime-delegated under
  bridge ON 2026-05-18 via R7.2). It exercises the full MFC GET
  sequence (ch16-21 wrch + `spu_mfc_cmd` + `mfc_dma_complete` +
  ch22-23 wrch + rdch ch24) and lands the canonical post-DMA
  status `0xDEADA12F` in OUT_MBOX across all three execution paths
  (bridge OFF / bridge ON / replay).
- **DMA / MFC GET-only replay pipeline is complete.** Writer
  extension (A.1) + parser (A.2) + chunk loader (A.3) + replay
  state machine (A.4) + executor wiring (Phase C) + first oracle
  (A.5) — all landed in R6. R7.2 adds the runtime-bridge DMA GET
  execution path (FFI callback that reads EA bytes via RPCS3
  `vm::_ptr<u8>` and writes them into the Rust handle's LS).
- **The C++ ↔ Rust runtime bridge covers all 7 oracles.**
  Bridge ON byte-identical to bridge OFF on the 6 non-DMA oracles
  (per R6.5 / R6.5b / R6.6 acceptance) and on the DMA oracle (per
  R7.2 acceptance — `DELEGATED EXECUTION OK total_steps=1054`,
  `R7.2 DMA GET dispatched: cmd=0x40 ... real EA/LS path
  (vm::_ptr<u8>)`).
- **Runtime DMA bridge scope: GET-only.** PUT, list cmds, atomic
  primitives, MFC barriers / fence bits, and multi-SPU DMA races
  on shared EA remain **out of R7 scope** — they defer to R8+.
  Non-GET MFC ops still surface `MfcUnsupported` and the bridge
  falls back honestly to C++ for those.
- **`tests/data/spurs_test_v3_real.jsonl` and
  `tests/data/spurs_test_v4_real.jsonl` remain diagnostic-only.**
  v4 informed the ISA-coverage push (R5.10a..p) but is now retired;
  R6.7 A.5 + R7 closes the DMA cycle by delivering a fresh CC0
  oracle as the canonical first DMA-bound trace AND making it run
  through the runtime bridge. Commercial SPURS captures are not
  promoted to `behavior-freeze/`.

---

## 2. Current workspace roles

This project lives across two distinct top-level trees under the same
parent directory. **Do NOT merge them — they are complementary, not
duplicates.**

| Tree | Role |
|---|---|
| **`rpcs3-master/`** | The Rust port workspace. Contains the live `docs/` (this file), `behavior-freeze/` harness + fixtures + oracles, the entire `rust/` Cargo workspace (decoder + interpreter + recompiler + thread + differential + FFI), the C++ trace-writer + bridge patches (under `rpcs3/Emu/Cell/`), and historical snapshots. Tracked in git on branch `main`. **Source of truth for everything Rust + behavior-freeze.** |
| **`rpcs3-upstream-clean/`** | The C++ RPCS3 build / capture tree used to produce `rpcs3.exe` with the R6.7 A.1 trace hooks applied. Contains the upstream RPCS3 source + 3rd-party submodules + the MSBuild outputs (`build/lib/Release-x64/`, `bin/rpcs3.exe`). Branch `spu-trace-jsonl-runtime-hooks`. R6.7 A.1 patches are currently applied as unstaged source edits on top of upstream HEAD. **Source of truth for the rpcs3.exe binary that produces captures.** |

`rpcs3.exe` runs on Windows native (MSVC `/MT`). The PSL1GHT/ps3toolchain
side that produces `.self` binaries runs in a Docker image
(`rpcs3-ps3dev-toolchain:local`, sha `ed2167a9ac59…`, content 2.43 GB)
backed up at `C:\docker-backup\rpcs3-ps3dev-toolchain-local.tar`.

---

## 3. Current oracle matrix

All seven oracles below pass cross-backend byte-identical
(`diff_snapshots(interp, jit).is_identical()`). Each `.jsonl` has a
companion `.notes.md` documenting provenance, toolchain, capture
procedure, engine fixes co-landed, and acceptance criteria.

| # | Fixture | Phase landed | Events | Main behavior covered | OUT_MBOX / status | DMA? | Bridge runtime status |
|---|---|---|---|---|---|---|---|
| 1 | [`single_spu_mailbox_v1`](../behavior-freeze/fixtures/spu/traces/single_spu_mailbox_v1.jsonl) | R5.9e.7 | 5 | IN_MBOX (ch29) + OUT_MBOX (ch28) + stop 0x101 | `0x129` | no | bridge ON validated |
| 2 | [`single_spu_branch_loop_v1`](../behavior-freeze/fixtures/spu/traces/single_spu_branch_loop_v1.jsonl) | R5.11 | 5 | + branch/loop ISA (Fibonacci(10)=89) | `0x59` | no | bridge ON validated |
| 3 | [`single_spu_signal_v1`](../behavior-freeze/fixtures/spu/traces/single_spu_signal_v1.jsonl) | R5.11 | 5 | SNR1 (ch3) signal-notification + OUT_MBOX + stop | `0x129` | no | bridge ON validated (R6.3c Phase 1b SNR forwarding) |
| 4 | [`single_spu_loadstore_v1`](../behavior-freeze/fixtures/spu/traces/single_spu_loadstore_v1.jsonl) | R5.11b | 5 | + LS load/store (stqd/lqd + cwd/shufb/rotqby) | `0x129` | no | bridge ON validated |
| 5 | [`single_spu_mailbox_multi_v1`](../behavior-freeze/fixtures/spu/traces/single_spu_mailbox_multi_v1.jsonl) | R6.4b-replay | 5 | IN_MBOX round 1 + SNR1 round 2 + real park/wake (PPU `sysUsleep(100ms)`) | `0x453` | no | bridge ON validated (R6.4b persistent handle + `pop_wait`) |
| 6 | [`game_like_mailbox_signal_v1`](../behavior-freeze/fixtures/spu/traces/game_like_mailbox_signal_v1.jsonl) | R6.6 | 5 | IN_MBOX + LS load/store + branch/loop + SNR1 + real park/wake (cross-path sentinel) | `0x051A03C9` | no | bridge ON validated (`total_steps=488 stall_iters=1`) |
| 7 | **[`single_spu_dma_get_v1`](../behavior-freeze/fixtures/spu/traces/single_spu_dma_get_v1.jsonl)** | **R6.7 A.5** | **15** | **MFC GET (ch16-21 wrch + `spu_mfc_cmd` + `mfc_dma_complete` + ch22-23 + rdch ch24) + post-DMA sum + XOR** | **`0xDEADA12F`** | **yes (GET 0x40)** | **replay-valid only; runtime bridge DMA = R7** |

Notes on the seventh row:

- `single_spu_dma_get_v1` is **the first** fixture to carry
  `spu_mfc_cmd` + `mfc_dma_complete` events, plus a content-addressed
  `<sha>.dmachunk` side-file at
  `behavior-freeze/fixtures/spu/dma/471fb943…2be5.dmachunk` (128 bytes,
  sum = 8128 = 0x1FC0, counting pattern 0x00..0x7F).
- **Bridge runtime status for this oracle is replay-only.** The Rust
  bridge currently has NO `process_mfc_cmd()` callback; bridge ON
  attempting to delegate a `wrch ch21` would diverge from C++ ground
  truth. R7.1 (Phase B honest-fallback) + R7.2 (Phase D runtime DMA
  opt-in) are the next workstream.

---

## 4. Current verified gates — R6 closure, 2026-05-03

All gates below were re-run locally on 2026-05-03 against the R6
closure commit. Results recorded verbatim from the test runner.

| Command | Result |
|---|---|
| `cargo test -p rpcs3-spu-recompiler --test single_spu_dma_get_v1_replay` | passed (1) |
| `cargo test -p rpcs3-spu-recompiler --test single_spu_mailbox_v1_replay` | passed (1) |
| `cargo test -p rpcs3-spu-recompiler --test single_spu_branch_loop_v1_replay` | passed (1) |
| `cargo test -p rpcs3-spu-recompiler --test single_spu_signal_v1_replay` | passed (1) |
| `cargo test -p rpcs3-spu-recompiler --test single_spu_loadstore_v1_replay` | passed (1) |
| `cargo test -p rpcs3-spu-recompiler --test single_spu_mailbox_multi_v1_replay` | passed (1) |
| `cargo test -p rpcs3-spu-recompiler --test game_like_mailbox_signal_v1_replay` | passed (1) |
| `cargo test -p rpcs3-spu-differential --lib` | passed (137) |
| `cargo test -p rpcs3-spu-thread --lib` | passed (44) |
| `cargo test --workspace --lib --no-fail-fast` | passed (0 failed across all crates) |
| `python behavior-freeze/harness/check_trace_fixtures.py` | exit 0; 7 fixtures listed; `REPLAY_VALIDATED_TRACE_EXISTS = True` |
| `python behavior-freeze/harness/check_patch_separation.py` | exit 0; SHAs match |

C++ trace patches preserved unchanged (sha256, pinned by
`check_patch_separation.py`):

| Patch | sha256 |
|---|---|
| scaffolding (`rpcs3/Emu/Cell/SPUTraceJsonl.{h,cpp}`) | `cda976d7b7bace826b3e8c38475fab5e077c88201bd0b31768541f06635143a1` |
| runtime hooks (`SPUThread.cpp` + writer-side integration) | `95bdcaae4850f3b2a94b5aea59761263589efabeac71bd3cb8464ad59c3a6721` |
| rust bridge (`SPURustBridge.cpp`) | `7d6b6bba3d1c590ec16f2ff175b262a4f95bdf95ace92eb91636824488436c03` |

**`cargo test --workspace --release` is NOT asserted green.** A handful
of HLE crates (`rpcs3-hle-cellsysutilmisc`, `rpcs3-hle-celljpgdec`,
`rpcs3-hle-cellmusicselectioncontext`, `rpcs3-hle-cellvideoexport`)
have a pre-existing `no_std` / `global_allocator` build error that
surfaces under `--release`. This is unrelated to the SPU stack and
predates R4a. **`--workspace --lib` is the scoped green gate.** Do not
promote the workspace as "green" without specifying scope.

---

## 5. Current completed components

The following components are complete and exercised by the gates in § 4:

- **`behavior-freeze/` harness.** Python gates
  (`check_trace_fixtures.py`, `check_patch_separation.py`,
  `build_synthetic_fixtures.py`, `spu_homebrew_runner.py`,
  `test_spu_homebrew_runner.py`) + fixtures + canonical
  `.spuimg` / `.dmachunk` pools.
- **SPU decoder** (`rust/rpcs3-spu-decoder/`). Two-pass leader
  analysis, basic-block graphs, ~107 opcodes covered (including
  R5.10 ISA-coverage additions for LQA/STQA/LQR/STQR/CBD/CWD/CHD/CDD,
  FSM/FSMH/FSMB/FSMBI, ROTQBYI/ROTQMBYI/SHLQBYI/SHLQBII, byte-imm
  RI10s + Class-A RI10s, and the RRR-form rt/rc fix from R5.11b).
- **SPU interpreter** (`rust/rpcs3-spu-interpreter/`). ~70% ISA, FTZ
  denormal flush, halfword/byte ops, channel I/O snapshot, RR-form
  `rotqby` (R5.11b add), corrected C-family default mask byte-order
  (R5.11b fix), corrected RRR-form rt/rc dispatch (R5.11b fix).
- **SPU recompiler** (`rust/rpcs3-spu-recompiler/`). Cranelift-backed
  JIT covering the broad subset (ALU word/halfword/byte, compares,
  shifts, multiplies, float arith/compares/converts, RRR, branches
  direct + indirect, branch hints, qword load/store, byte-imm), plus
  R4a dispatcher loop, R4b safe chained patching with ls_hash guard,
  R4c per-entry SMC scan with exact range hash, R5 partial fallback
  to interpreter from `JitState`. Channel ops jitted via runtime
  helpers (`spu_helper_rdch` / `spu_helper_wrch` / `spu_helper_rchcnt`).
- **`rpcs3-spu-thread`** state machine. `SpuThread` + `SpuChannels`
  (with R6.7 Phase C MFC channel fields and `tag_stat_queue`
  `VecDeque`), park/wake API, `SpuWakeResult`, `SpuExecEvent`,
  single-threaded executor.
- **`rpcs3-spu-differential`** + `SpuExecutor` trait, `SpuProgram` +
  `initial_gpr_overrides` + `initial_mfc_tag_stat_queue`,
  `SpuStateSnapshot`, `diff_snapshots`,
  `InterpreterExecutor` reference oracle, the
  `SpuPpuLockstepDriver`, `replay_per_spu_traces` orchestrator,
  R6.7 modules `dma_chunk.rs` (A.3 loader) and `mfc_replay.rs` (A.4
  state machine), `apply_mfc_dma_pre_replay()` (Phase C helper).
- **FFI crate `rpcs3-spu-ffi`.** Static lib (`/MT`) consumed by the
  C++ bridge in `rpcs3/Emu/Cell/SPURustBridge.cpp`.
- **C++ ↔ Rust SPU bridge** for the **supported non-DMA workloads**
  (oracles 1-6). `try_delegate_execution()` +
  `stop_and_signal()` re-use + persistent
  `unordered_map<lv2_id, BridgeSession>` side-table + multi-round
  loop with `pop_wait` for Stalls (R6.4b). StallWrite ch28
  depth-1 overwrite (R6.5b). Default OFF preserved; opt-in via
  `bin/config/config.yml`.
- **JSONL trace capture pipeline.** Writer
  (`rpcs3/Emu/Cell/SPUTraceJsonl.{h,cpp}` + `SPUThread.cpp` hooks)
  emits the 10-original event kinds + R6.7 A.1 additions
  (`spu_mfc_cmd`, `mfc_dma_complete`). Env-var-gated via
  `RPCS3_SPU_TRACE_JSONL`. Noop when disabled.
- **`.spuimg` side-file pipeline.** Content-addressed by SHA-256;
  canonical pool at `behavior-freeze/fixtures/spu/images/`; loaded by
  `build_spu_program_from_captured_image()` with hash + size +
  entry_pc validation.
- **`.dmachunk` side-file pipeline.** Content-addressed by SHA-256;
  canonical pool at `behavior-freeze/fixtures/spu/dma/`; loaded by
  `resolve_dma_chunk_side_file()` (per-trace `<jsonl>.dma/` precedence
  + canonical fallback); validated against the `ea_chunk_sha256` +
  size fields in the corresponding `spu_mfc_cmd` event before any
  byte is touched.
- **DMA / MFC GET-only replay.** `MfcReplayState` supports
  Immediate / Any / All wait modes; `apply_mfc_dma_pre_replay()`
  walks the captured events, drives the state machine, loads the
  `.dmachunk` via the A.3 loader, and produces a `SpuProgram` whose
  LS already contains the post-DMA bytes plus a pre-populated
  rdch ch24 queue.
- **Seven replay-validated oracles.** Listed in § 3.

---

## 6. Current partially complete components

The following components are partially landed and have defined next
work (mostly R7 / R8+):

- **Runtime DMA bridge.** Bridge currently has NO callback into
  RPCS3's `process_mfc_cmd()`. Bridge ON cannot delegate a
  `wrch ch21` honestly. R7.1 (Phase B honest-fallback) + R7.2
  (Phase D runtime DMA opt-in) cover this. The replay path works
  end-to-end; the runtime path does not.
- **MFC PUT (LS → EA).** Symmetric to GET but requires capturing
  EA-before-PUT bytes for replay determinism. Out of R7. Defer to
  R8+.
- **DMA list commands** (`GETL`, `PUTL`, `GETLB`, `PUTLB`, etc.).
  Need per-list-element event sequencing. Out of R7. Defer to R8+.
- **Atomic primitives** (`GETLLAR`, `PUTLLC`, `PUTLLUC`, `PUTQLLUC`).
  LL/SC reservation tracking is its own work item. Out of R7. Defer.
- **MFC barriers / fence bits.** Defer until ≥2 overlapping DMAs
  are observed in a CC0 fixture.
- **Multi-SPU DMA races on shared EA regions.** R6+R7 are single-SPU
  only. Defer.
- **SPURS / v4 diagnostic traces.** `tests/data/spurs_test_v3_real.jsonl`
  (R5.9d-era multi-SPU SPURS, 6 SPUs) and
  `tests/data/spurs_test_v4_real.jsonl` (R5.10a..p iteration trace,
  DMA-bound at pc=0x74C `wrch ch16 MFC_LSA`) remain
  **diagnostic-only**. The R5.10p analysis catalogued the full MFC
  GET sequence in v4. Both contain commercial code and are never
  promoted. They surface ISA / protocol gaps as diagnostic signals
  only.
- **Production performance.** Speedup numbers reported in the R5
  archive are observed benchmarks against synthetic fixtures; no
  real-workload benchmark has been published. The R4b/R4c chained
  patching + SMC scan work is correct but performance under sustained
  game workloads is not characterized.
- **Broad RPCS3 subsystems outside SPU.** RSX runtime, PPU JIT,
  Qt UI, audio backends, full LV2 syscall fidelity, loader / game
  boot parity — all carry partial Rust scaffolding from earlier
  waves but none are production-ready. None gated.

---

## 7. Current out-of-scope / not yet done

These items are **not** part of R6 closure and are not active workstreams:

- **Runtime DMA execution through the bridge.** Moves to R7.1 + R7.2
  (see § 9).
- **R7 / R8 advanced DMA features** — PUT, list commands, atomics,
  barriers/fence, multi-SPU DMA races on shared EA. R8+ scope.
- **Full PPU JIT.** Out of every R5/R6 wave; no Rust PPU recompiler
  exists. PPU stays on the C++ side.
- **RSX runtime.** Out of scope; the Rust workspace has crates with
  RSX-adjacent helpers but no GS frame execution.
- **Full LV2 / syscall fidelity.** Many syscalls are Rust scaffolded
  for header / signature parity, not execution parity. Out of scope.
- **Complete loader / game boot parity.** PSF / PUP / PKG / SELF /
  decrypt paths are partially Rust-mirrored at the contract level
  (per `behavior-freeze/docs/INVENTORY.md`) but full boot of a
  commercial game does not run through the Rust stack.
- **UI / packaging.** No Qt UI port. No installer / packaging story.
- **Commercial game trace promotion.** Hard rule: traces of
  commercial PS3 games NEVER go into `behavior-freeze/`. Only CC0
  homebrew authored for this project. Same for any future DMA / SPURS
  fixture.

---

## 8. R6 closure summary

R6 is formally closed at R6.7 A.5 (2026-05-03). The closure delivers:

1. A C++ ↔ Rust runtime SPU bridge that executes real `.self`
   binaries through the Rust executor **for the supported non-DMA
   workloads** (oracles 1-6). Bridge ON / OFF byte-identical for
   those workloads.
2. A complete DMA capture + replay pipeline (R6.7 A.1-A.5 + Phase C)
   for plain MFC GET commands — writer extension, parser,
   content-addressed `.dmachunk` side-files, state machine, executor
   wiring, and the load-bearing CC0 fixture
   `single_spu_dma_get_v1`. **All 7 oracles are replay byte-identical
   across Interpreter and Recompiler.**
3. The seventh replay-validated oracle (`single_spu_dma_get_v1`) that
   distinguishes "no DMA" from "wrong DMA" from "right DMA" via the
   canonical `0xDEADA12F` status — the only value reachable when
   (a) the GET actually copied 128 bytes from EA into LS at
   lsa=0x10000, AND (b) the SPU computed the deterministic post-DMA
   sum + XOR.

**Wording discipline:**

- We say **"the bridge is validated for the supported non-DMA
  workloads"**. We do NOT say "full runtime bridge".
- We say **"all 7 oracles are replay byte-identical"**. We do NOT
  say "bridge ON / OFF is byte-identical on all 7 runtime
  workloads" — bridge ON has only been validated against the 6
  non-DMA oracles; oracle #7 (`single_spu_dma_get_v1`) is
  replay-valid but the runtime bridge cannot yet honestly execute
  it, because it would need `process_mfc_cmd()` delegation that
  Phase B / Phase D will land in R7.
- We say **"runtime bridge DMA moves to R7"**. Period.

**Trace shape of the seventh oracle** (15 events):

```
seq  0: spu_image       sha=97a38063…  size=0x40000  entry_pc=0
seq  1: spu_wrch ch16=0x10000     (MFC_LSA)
seq  2: spu_wrch ch17=0           (MFC_EAH)
seq  3: spu_wrch ch18=0x10068400  (MFC_EAL)
seq  4: spu_wrch ch19=128         (MFC_Size)
seq  5: spu_wrch ch20=3           (MFC_TagID)
seq  6: spu_wrch ch21=0x40        (MFC_Cmd = GET)
seq  7: spu_mfc_cmd  cmd=0x40 tag=3 size=128 lsa=0x10000 eah=0 eal=0x10068400
                                                  ea_chunk_sha256=471fb943…
seq  8: mfc_dma_complete  tag=3  transferred_bytes=128
seq  9: spu_wrch ch22=0x8         (WrTagMask = 1<<3)
seq 10: spu_wrch ch23=0x2         (WrTagUpdate = MFC_TAG_UPDATE_ALL)
seq 11: spu_rdch ch24=0x8         (RdTagStat returns mask 1<<3)
seq 12: spu_wrch ch28=0xDEADA12F  (OUT_MBOX = canonical post-DMA cs)
seq 13: spu_stop  stop_code=0x101
seq 14: final_state  r18=0x1FC0 r19=0xDEADBEEF r20=0xDEADA12F
```

**Capture requirements for re-capture** (load-bearing, documented in
the fixture's `.notes.md` and in
[`docs/SPU_DMA_MFC_R6_7_DESIGN.md`](./SPU_DMA_MFC_R6_7_DESIGN.md) § 13.3):

1. **`subst R: <repo-root>` active during build.** The MSBuild
   `link.command.1.tlog` for `rpcs3.exe` carries 545 burned-in `R:\`
   paths from a legacy SUBST build configuration. Without an active
   SUBST, the linker silently skips the missing `R:\` `/LIBPATH:`
   directives and falls through to `$(VULKAN_SDK)\Lib\glslang.lib`
   (75 MB, /MD CRT) which is incompatible with the rpcs3 `/MT`
   build → 52 LNK2001 unresolved `spvtools::Optimizer` externals
   from `glslang.lib(SpvTools.obj)`. Fix is one command before the
   build.
2. **`Core: SPU Decoder: Interpreter (static)`** (and PPU Decoder)
   in `bin/config/config.yml` for the CAPTURE run only. RPCS3 LLVM
   JIT bypasses the C++ `set_ch_value()` / `get_ch_value()` for MFC
   channels, and the R6.7 A.1 trace hooks live inside those
   functions — JIT inlining suppresses them. Restore to
   `Recompiler (LLVM)` after capture. Documented in the fixture's
   `.notes.md`.

**Hard rules carried forward to R7 and beyond (unchanged):**

- No fake JSONL.
- No manual JSONL editing after capture.
- No commercial trace promotion.
- No fake DMA — synthesising `MFC_Cmd=0x40` success without
  consulting an oracle (replay) or RPCS3 vm:: (runtime) is a hard
  reject.
- No fake `RdTagStat` — never return a fixed/zero/random tag stat
  for `rdch ch24`.
- No fake LS bytes after a GET — `.dmachunk` content must hash to
  the captured `ea_chunk_sha256`; the R7.2 runtime path reads via
  `vm::_ptr<u8>(eal)` (real RPCS3 memory).
- v4 / SPURS stays diagnostic-only forever.

---

## R7 closure summary (2026-05-18)

R7 closed in a single session: R7.1 (Bridge Phase B honest
fallback) → R7.2 (Bridge Phase D runtime DMA GET) → R7.3 (triple
symmetry regression gate), all ACEITO.

**R7.1 — Bridge Phase B (honest fallback for `MFC_Cmd`).**
Adds `rust_spu_set_refuse_mfc(handle, 1)` FFI + a new outcome
variant `rust_spu_outcome_t_MfcUnsupported`. When the C++ bridge
installs the refuse gate AND no callback is set, the Rust
interpreter short-circuits ANY `wrch ch16..=23` / `rdch ch24/25`
/ `rchcnt` on those channels BEFORE per-channel mutation and
surfaces `MfcUnsupported`. The bridge's outcome switch logs
`"MFC/DMA detected at ch%u (%s), total_steps=%u ...
falling back honestly to C++ executor before Rust-side MFC
mutation. Channel state intact"` and drops the session — RPCS3
state is byte-identical to entry, the C++ executor takes over
from the original PC. Acceptance on `single_spu_dma_get_v1.self`:
bridge ON fell back at **ch16 (MFC_LSA)** at `total_steps=4`
(entry prologue), then C++ ran the .self and produced the
canonical TTY.

**R7.2 — Bridge Phase D (runtime DMA GET via FFI callback).**
Adds `rust_spu_set_dma_get_callback(handle, &fn, &user_data)`
FFI. The C++ bridge installs a static callback
`bridge_dma_get_callback` that reads `size` bytes from RPCS3 EA
`eal` via `vm::_ptr<u8>` (the same path the C++ executor's
`process_mfc_cmd()` simple-GET branch uses at
`rpcs3/Emu/Cell/SPUThread.cpp:2091`) and `memcpy`s them into the
Rust handle's LS at the captured `mfc_lsa`. The Rust interpreter
intercepts `wrch ch21 (MFC_Cmd)` BEFORE delegating to
`SpuChannels::write` and invokes the callback when cmd=0x40.
Validation (cmd=0x40, eah=0, tag<32, size ∈ {1,2,4,8} ∪
multiples of 16 ≤ 16384, lsa+size ≤ 256 KiB) happens in Rust
before the callback fires. On success the interpreter pushes
`1 << tag` into the tag-stat queue and the SPU continues; a
subsequent `rdch ch24` pops the value and the SPU finishes
naturally. Non-GET cmds, validation failures, and NULL EA still
surface `MfcUnsupported` (R7.1 fallback). Acceptance on
`single_spu_dma_get_v1.self`: bridge ON now **delegates
end-to-end** (`DELEGATED EXECUTION OK total_steps=1054
stall_iters=0`), with success log
`"R7.2 DMA GET dispatched: cmd=0x40 eal=0x10011180 size=128
tag=3 ... real EA/LS path (vm::_ptr<u8>); tag-stat 0x8 queued
for subsequent rdch ch24"` and the canonical TTY
`[dma_get_v1] OK cause=0x1 status=0xdeada12f`.

**R7.3 — Triple-symmetry regression gate.** New harness:
`behavior-freeze/harness/check_triple_symmetry.py`. Runs all
three execution paths against `single_spu_dma_get_v1.self` and
asserts they converge on the canonical status `0xdeada12f`:

1. **bridge OFF real binary** — `rpcs3.exe` with bare C++ executor
2. **bridge ON real binary** — `rpcs3.exe` with
   `RPCS3_SPU_RUST_BRIDGE=1`; the Rust bridge delegates
   end-to-end via R7.2 runtime DMA GET (no fallback line in
   the Rust bridge log)
3. **replay oracle** —
   `cargo test single_spu_dma_get_v1_replay --release`
   asserts `diff_snapshots(interp, jit).is_identical() == true`

All three pass.

**Patch SHAs at R7 closure (`check_patch_separation.py` pin):**

| Patch | sha256 | Δ vs R6 closure |
|---|---|---|
| scaffolding | `cda976d7b7bace826b3e8c38475fab5e077c88201bd0b31768541f06635143a1` | unchanged |
| runtime hooks | `95bdcaae4850f3b2a94b5aea59761263589efabeac71bd3cb8464ad59c3a6721` | unchanged |
| rust bridge | `a1e810264d8d9474018c279606111b543eb3f6b6c5845839382e4a657e220e70` | **bumped** (was `7d6b6bba…` at R6, `eeb57616…` at R7.1; final R7.2 sha is the recorded one) |

**rpcs3.exe at R7 closure:** sha256
`81ac5f096b5b9e79d1d466f35f8d986129636c2801093a3a86d30cd65f2a4404`
(64 MB; built 2026-05-18 03:22 with R7.1 + R7.2 surface).

**Out of R7 scope (deferred to R8+):** MFC PUT, DMA list cmds
(GETL/PUTL/GETLB/PUTLB), atomic primitives (GETLLAR/PUTLLC/
PUTLLUC/PUTQLLUC), MFC barriers / fence bits, multi-SPU DMA
races on shared EA, SPURS production support. The R6 hard rules
above carry forward verbatim to R7 and to R8+.

---

## 9. R8+ recommended next scope

R7 is the closed phase (see § 8 R7 closure summary below). The
next-up phase is R8+, which covers the MFC features deliberately
deferred from R7's GET-only scope. **Do not begin R8 work without
re-reading the hard rules in § 8.** Recommended R8+ scope:

1. **R8.1 — MFC PUT (LS → EA writes).** Symmetric to R7.2's GET
   but inverts the direction: SPU writes `wrch ch21 = 0x20 (PUT)`,
   the bridge's runtime callback reads `mfc_size` bytes from the
   Rust handle's LS at `mfc_lsa` and writes them to RPCS3 EA at
   `mfc_eal` via `vm::_ptr<u8>` (matching the C++ executor's PUT
   branch). Acceptance: a fresh CC0 fixture (`single_spu_dma_put_v1`
   — to be authored) round-trips a counting buffer SPU → EA → PPU
   readback and the lv2 group-exit status carries a canonical
   marker that proves the PUT bytes landed.
2. **R8.2 — DMA list commands** (GETL / PUTL / GETLB / PUTLB).
   Per-list-element event sequencing in the JSONL writer +
   parser, then runtime support. PSL1GHT-style list of
   `(eal, size)` pairs in LS at the address pointed to by
   `MFC_Cmd`'s `mfc_lsa`.
3. **R8.3 — Atomic primitives** (GETLLAR / PUTLLC / PUTLLUC /
   PUTQLLUC). LL/SC reservation tracking is its own work item —
   the bridge needs a per-`raddr` reservation map shared between
   the Rust executor and RPCS3's existing C++ atomic infrastructure.
4. **R8.4 — MFC barriers / fence bits.** Defer until at least
   two overlapping DMAs are observed in a CC0 fixture.
5. **R8.5 — Multi-SPU DMA races on shared EA.** R6+R7 are
   strictly single-SPU; the bridge's persistent-handle table is
   keyed by `lv2_id`, so multi-SPU is a workstream of its own.
6. **R8.6 — SPURS production support.** Out of any current scope
   because SPURS captures contain commercial code; defer to a
   separate CC0 multi-SPU fixture authored for the purpose.

Same hard rules from § 8 apply throughout R8.

---

## 10. Historical archive

R5 / R4 long-form material — the full iteration-by-iteration timeline
from R4a through R5.9e.7 and the R5.11 / R5.11b additive expansions —
has been moved to:

- [`docs/history/PROJECT_STATUS_R5_ARCHIVE.md`](./history/PROJECT_STATUS_R5_ARCHIVE.md)

That archive carries the verbatim text as it stood on 2026-04-29 at
R5 closure plus the R5.11 / R5.11b expansions. The archive includes:

- The full R5 closure section (delivered components, what stayed out,
  confirmations at R5 closure).
- The full R5.4a..p ISA-coverage iteration log (R5.10a → R5.10p), which
  ended at the DMA / MFC boundary that R6.7 has since crossed.
- The full R5.8 A.1 / A.2 / A.3 capture pipeline narrative.
- The full R5.9a..R5.9e.7 multi-SPU schema + first replay-validated
  fixture story.
- All R5.11 + R5.11b additive fixture entries (`single_spu_branch_loop_v1`,
  `single_spu_signal_v1`, `single_spu_loadstore_v1`).
- The original "Next recommended phase" sections that recommended
  R5.8 / R6 — those are **obsolete / historical**. The current "next
  steps" are in § 9 above.

Hooks and other files that reference `behavior-freeze/docs/`
historical doc paths (`AUTONOMOUS_LOG.md` stub, `SPU_RECOMPILER_PLAN.md`
stub, `INVENTORY.md`, `DECISIONS.md`, `DEFERRED.md`,
`BACKLOG_RESIDUAL.md`, `HOMEBREW_PLAN.md`) are unchanged. Older
verbatim snapshots remain at
[`historico/pre-r4b-2026-04-25/`](../historico/pre-r4b-2026-04-25/).
