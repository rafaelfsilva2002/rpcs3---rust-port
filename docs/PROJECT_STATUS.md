# Project Status — R8.2 LANDED (9th oracle: multi-DMA GET triple-symmetric)

**Authoritative current source of truth for the RPCS3 → Rust port.**

Last updated: **2026-05-20 (R8.2 landing)**. R8.2 (multi-DMA GET
+ ALL wait + 9th oracle) extends R8.1's GET+PUT triple-symmetric
DMA bridge with multi-tag in-flight semantics. The new fixture
`single_spu_dma_get_multi_v1` exercises two queued GETs (tags
3 + 5, distinct EAs / sizes / LSAs) + `WrTagMask = 0x28` +
`WrTagUpdate = ALL` + `RdTagStat = 0x28`. **Zero code changes
landed** — the 8-oracle baseline (parser, state machine, chunk
loader, executor wiring, runtime bridge callback) already
supported everything R8.2 exercises. The bridge ON delegation
log shows `total_steps=1584` (vs 1054 for single-GET, 1049 for
PUT), confirming both DMAs traversed the Rust executor end to
end via the same R7.2 callback invoked twice. See "R8.2 phase
closure (2026-05-20)" below.

Previously: **2026-05-19 (R8.1 landing)**. R8.1 (MFC PUT runtime
+ replay oracle) extends R7's triple-symmetric DMA bridge with the
inverse direction (LS → EA). The runtime bridge now executes BOTH
DMA-bound oracles end-to-end through the Rust executor under
`RPCS3_SPU_RUST_BRIDGE=1` with byte-identical canonical outputs:
GET = `0xdeada12f`, PUT spu sentinel = `0xc0ffeeca` + ea_status =
`0xcafea57e`. The replay oracle for `single_spu_dma_put_v1` is the
8th oracle and gates byte-identical Interpreter ↔ Recompiler
agreement plus a post-replay verification that final LS at the PUT
region matches the captured `.dmachunk`. See "R8.1 phase closure
(2026-05-19)" below.

Previously: **2026-05-18 (R7 closure)**. R7 phases R7.1 (Bridge
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

- **R8.2 is LANDED** (2026-05-20). First multi-DMA replay oracle
  + 9th oracle. Two queued GETs (tags 3 + 5) + ALL wait + 0x28
  RdTagStat + canonical status `0xE12DEA4E`. **Zero engine-side
  code changes**: the 8-oracle baseline (parser, state machine,
  loader, executor, bridge callbacks) already covered multi-tag
  in-flight + ALL wait semantics; R8.2 is a pure coverage gain
  on the existing implementation. Triple-symmetry green via
  `check_triple_symmetry.py --fixture get_multi`. See § 8.2.
- **R8.1 is LANDED** (2026-05-19). MFC PUT runtime extension +
  8th replay-validated oracle. The runtime bridge now delegates
  BOTH `single_spu_dma_get_v1.self` (cmd 0x40, R7.2) AND
  `single_spu_dma_put_v1.self` (cmd 0x20, R8.1) end-to-end through
  the Rust executor with byte-identical canonical outputs
  (`0xdeada12f` / `0xcafea57e`) versus bridge OFF and the replay
  oracles. See § 8.1 R8.1 closure summary.
- **R7 is formally CLOSED.** R7.1 (Bridge Phase B honest fallback)
  + R7.2 (Bridge Phase D runtime DMA GET via FFI callback into
  `vm::_ptr<u8>`) + R7.3 (triple-symmetry regression gate) all
  ACEITO 2026-05-18.
- **R6 is formally CLOSED** (2026-05-03). R6.2 first delegation →
  R6.3a/b/c per-oracle delegation → R6.4a outcome contract → R6.4b
  persistent handle → R6.5 / R6.5b bridge acceptance → R6.6 game_like
  cross-path → R6.7 design + A.1-A.5 + Phase C — all landed and
  gated green.
- **Nine replay-validated SPU oracles exist**, all
  `diff_snapshots(interp, jit).is_identical()` byte-identical
  across `InterpreterExecutor` and `RecompilerExecutor`.
- **`single_spu_dma_put_v1` is the 8th oracle** (R8.1 landed
  2026-05-19; runtime-delegated under bridge ON same day).
  Symmetric inverse of GET: SPU fills LS with `i & 0xFF` counting
  pattern, dispatches MFC PUT cmd 0x20, waits via ch22/23/24,
  writes sentinel `0xC0FFEECA` to OUT_MBOX, halts via stop 0x101.
  PPU reads EA back, computes `ea_status = sum_of_ea ^ 0xCAFEBABE
  = 0xCAFEA57E` — both invariants land identically across bridge
  OFF / bridge ON / replay paths.
- **`single_spu_dma_get_v1` is the first DMA-bound oracle**
  (R6.7 A.5, replay-validated 2026-05-03; runtime-delegated under
  bridge ON 2026-05-18 via R7.2). It exercises the full MFC GET
  sequence (ch16-21 wrch + `spu_mfc_cmd` + `mfc_dma_complete` +
  ch22-23 wrch + rdch ch24) and lands the canonical post-DMA
  status `0xDEADA12F` in OUT_MBOX across all three execution paths
  (bridge OFF / bridge ON / replay).
- **DMA / MFC GET+PUT replay pipeline is complete.** Writer
  extension (A.1 GET + R8.1 PUT) + parser (A.2 + R8.1 cmd 0x20
  accept) + chunk loader (A.3) + replay state machine (A.4 +
  R8.1 `process_mfc_cmd_pre_replay`) + executor wiring (Phase C
  + R8.1 PUT callback routing) + GET oracle (A.5) + PUT oracle
  (R8.1) — all landed.
- **The C++ ↔ Rust runtime bridge covers all 8 oracles.**
  Bridge ON byte-identical to bridge OFF on the 6 non-DMA oracles
  (per R6.5 / R6.5b / R6.6 acceptance) and on both DMA oracles
  (per R7.2 + R8.1 acceptance — GET: `DELEGATED EXECUTION OK
  total_steps=1054`; PUT: `DELEGATED EXECUTION OK
  total_steps=1049`). Bridge logs distinguish the two with
  `R7.2 DMA GET dispatched` vs `R8.1 DMA PUT dispatched`.
- **Runtime DMA bridge scope: GET + simple PUT.** List cmds,
  atomic primitives, MFC barriers / fence bits, and multi-SPU
  DMA races on shared EA remain **out of R8.1 scope** — they
  defer to R8.2+. Non-(GET|PUT) MFC ops still surface
  `MfcUnsupported` and the bridge falls back honestly to C++.
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

All nine oracles below pass cross-backend byte-identical
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
| 7 | [`single_spu_dma_get_v1`](../behavior-freeze/fixtures/spu/traces/single_spu_dma_get_v1.jsonl) | R6.7 A.5 | 15 | MFC GET (ch16-21 wrch + `spu_mfc_cmd` + `mfc_dma_complete` + ch22-23 + rdch ch24) + post-DMA sum + XOR | `0xDEADA12F` | yes (GET 0x40) | bridge ON delegated via R7.2 (`total_steps=1054 stall_iters=0`); triple-symmetric |
| 8 | [`single_spu_dma_put_v1`](../behavior-freeze/fixtures/spu/traces/single_spu_dma_put_v1.jsonl) | R8.1 | 15 | MFC PUT (symmetric inverse of GET — LS → EA) + 128-byte LS source pattern → ch16-21 + spu_mfc_cmd cmd=0x20 + mfc_dma_complete + ch22-23 + rdch ch24 + ch28 OUT_MBOX sentinel; PPU reads EA back → `ea_status = 0xCAFEA57E` | spu=`0xC0FFEECA`, ea_status=`0xCAFEA57E` | yes (PUT 0x20) | bridge ON delegated via R8.1 (`total_steps=1049 stall_iters=0`); triple-symmetric |
| 9 | **[`single_spu_dma_get_multi_v1`](../behavior-freeze/fixtures/spu/traces/single_spu_dma_get_multi_v1.jsonl)** | **R8.2** | **23** | **TWO QUEUED MFC GETs (cmd 0x40, tags 3 + 5, distinct EAs / sizes 128 + 64 / LSAs 0x10000 + 0x10100, both in-flight before any wait) + WrTagMask=0x28 + WrTagUpdate=ALL + RdTagStat=0x28 + ch28 OUT_MBOX status; FIRST multi-DMA oracle** | **`0xE12DEA4E`** | **yes (GET 0x40 × 2)** | **bridge ON delegated via R7.2 callback × 2 dispatches (`total_steps=1584 stall_iters=0`); triple-symmetric; ZERO engine-side code changes** |

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

**Out of R7 scope (deferred to R8+):** MFC PUT (now LANDED in
R8.1, see § 8.1 below), DMA list cmds (GETL/PUTL/GETLB/PUTLB),
atomic primitives (GETLLAR/PUTLLC/PUTLLUC/PUTQLLUC), MFC barriers
/ fence bits, multi-SPU DMA races on shared EA, SPURS production
support. The R6 hard rules above carry forward verbatim to R7 and
to R8+.

---

## 8.1 R8.1 closure summary (2026-05-19)

R8.1 landed in a single session: Rust core (parser + state machine
+ channels + interpreter routing + FFI) → Docker .self build →
RPCS3 OFF capture → bridge ON acceptance → 8th replay oracle →
triple-symmetry gate extended → patches regenerated → docs
updated.

**Scope.** First MFC PUT-bound oracle + symmetric runtime extension
on top of R7.2's runtime DMA GET. Mirrors the R6.7 A.5 / R7.2 GET
delivery but inverts DMA direction (LS → EA). The captured
`.dmachunk` now carries the SPU's LS-source bytes at dispatch
time; the runtime bridge writes them to RPCS3 EA via
`vm::_ptr<u8>`; the replay oracle verifies SPU LS at the PUT
region matches the chunk post-execution.

**Rust core extensions.**
- `trace_fmt.rs` parser accepts `spu_mfc_cmd.cmd ∈ {0x40 GET,
  0x20 PUT}`; the defensive subset-rejection canary moves to
  `0x44 GETL` (list variants still out of scope).
- `mfc_replay.rs` adds `MfcReplayError::PutLsBytesMismatch`
  (load-bearing PUT correctness gate) and a new public method
  `process_mfc_cmd_pre_replay`. The PRE-replay variant defers
  the PUT LS-bytes assertion (cannot inspect dispatch-time LS
  before the SPU runs); the AssertNow `process_mfc_cmd` remains
  the canonical at-dispatch state machine for future in-line
  executor wiring.
- `rpcs3-spu-thread::SpuChannels` adds `dma_put_callback:
  Option<DmaPutCallback>` symmetric to the GET callback. The
  `refuse_mfc` gate is RELAXED whenever EITHER callback is
  installed.
- `rpcs3-spu-interpreter` wrch ch21 intercept routes by cmd:
  0x40 → GET callback, 0x20 → PUT callback, other →
  `MfcUnsupported`. PUT source pointer is read via the SPU
  thread's LS at the dispatch lsa.
- `rpcs3-spu-ffi` adds `rust_spu_set_dma_put_callback` +
  `DmaPutCallbackFn` typedef. The C header is updated. FFI
  tests serialized via a new `CALLBACK_TEST_MUTEX` (static
  AtomicU32 observers cross-pollute under cargo's parallel test
  runner without it).

**C++ side extensions.**
- `SPUThread.cpp` writer hook (R6.7 A.1 extended): the same
  `record_spu_mfc_cmd` + `record_mfc_dma_complete` events now
  fire for cmd=0x20 PUT. For PUT the snapshot bytes come from
  `this->ls + mfc_lsa` (vs `vm::_ptr<u8>(mfc_eal)` for GET).
- `SPURustBridge.cpp` bridge: new static
  `bridge_dma_put_callback` reads `src_ls_ptr` size bytes (the
  SPU's LS source) and writes to `vm::_ptr<u8>(eal)`. Installed
  alongside the GET callback on every `rust_spu_new` in
  `try_delegate_execution()`. Success log: `R8.1 DMA PUT
  dispatched: cmd=0x20 eal=0x... size=N tag=T on '...'; real
  LS/EA path (vm::_ptr<u8>); tag-stat 1<<T queued for subsequent
  rdch ch24`.

**Fixture + capture.** `single_spu_dma_put_v1` source (PSL1GHT
homebrew, CC0): PPU allocates a 128-byte BSS buffer zero-filled,
passes EA via `thread_args.arg0`; SPU fills LS at `0x10000` with
counting pattern `i & 0xFF` (sum = 8128 = 0x1FC0), dispatches MFC
PUT tag 3 size 128, waits via ch22/23/24, writes sentinel
`0xC0FFEECA` to OUT_MBOX, halts via stop 0x101. PPU joins, reads
EA back, sums, XORs with `0xCAFEBABE` to produce `ea_status =
0xCAFEA57E`. Built in Docker via `rpcs3-ps3dev-toolchain:local`
(image sha `ed2167a9ac59…`); `.self` 939,475 bytes sha
`761414892bd3757a1a1d8238d6623f7270e5fee49321620b5c47b466e321f3c5`.
Capture run with `Core: SPU/PPU Decoder: Interpreter (static)`
(LLVM JIT bypasses `set_ch_value()` MFC hooks per R6.7 A.5
gotcha); trace is 15 events including the new `spu_mfc_cmd
cmd=0x20` event with `ea_chunk_sha256=471fb943…` — the SAME
content-addressed pool entry as the GET fixture (deduplicated
naturally because both fixtures use the same source pattern).

**Replay oracle.** New
`rust/rpcs3-spu-recompiler/tests/single_spu_dma_put_v1_replay.rs`:
- Parses JSONL via `parse_jsonl_trace`.
- Asserts cmd=0x20, tag=3, size=128, eah=0, lsa=0x10000,
  dma_complete_count=1, ch28 carries `0xC0FFEECA`, stop 0x101.
- Builds `SpuProgram` from `.spuimg`, seeds r3 with the PSL1GHT
  arg0 EA (SPU calling convention places u64 arg0 high in lane
  0, low in lane 1 — without this the final-state ExpectGprWord
  for r3 fails since PUT keeps EA in r3 through exit; GET
  overwrites r3 with tag_stat so doesn't need the seed).
- Calls `apply_mfc_dma_pre_replay` (PRE-replay PUT route via
  `process_mfc_cmd_pre_replay` — chunk SHA validated, LS-bytes
  assertion deferred).
- Runs replay × Interpreter + replay × Recompiler;
  `diff_snapshots.is_identical()`.
- **Post-replay deferred PUT verification:** loads the captured
  chunk via `resolve_dma_chunk_side_file` and asserts that BOTH
  backends' final LS at `[lsa..lsa+size]` matches the chunk
  byte-for-byte. This restores the dispatch-time contract for
  the canonical fixture (the SPU does not touch LS after PUT
  dispatch).

**Triple-symmetry extension.**
`behavior-freeze/harness/check_triple_symmetry.py` was refactored
to parametrize by fixture via `--fixture {get,put}` (default
`get`, R7.3 backwards-compatible). Both fixtures pass green:

| Path | GET | PUT |
|---|---|---|
| bridge OFF TTY | `[dma_get_v1] OK cause=0x1 status=0xdeada12f` ✓ | `[dma_put_v1] OK cause=0x1 spu=0xc0ffeeca ea_status=0xcafea57e` ✓ |
| bridge ON delegation | `R7.2 DMA GET dispatched ... total_steps=1054 stall_iters=0` ✓ | `R8.1 DMA PUT dispatched ... total_steps=1049 stall_iters=0` ✓ |
| replay oracle | `single_spu_dma_get_v1_replay` ok ✓ | `single_spu_dma_put_v1_replay` ok ✓ |

**Patch SHAs at R8.1 landing (`check_patch_separation.py` pin):**

| Patch | sha256 | Δ vs R7 closure |
|---|---|---|
| scaffolding | `cda976d7b7bace826b3e8c38475fab5e077c88201bd0b31768541f06635143a1` | unchanged |
| runtime hooks | `1f598d37c07d30f7e8b43fa0e53018b7c9277f628c2af58ab40bf987ffbd46ff` | **bumped** (was `95bdcaae…`; PUT-extended writer hook in SPUThread.cpp) |
| rust bridge | `0afda1c6943feb5d98329299a57dd68404095efb0a792839779febed13ab8a7e` | **bumped** (was `a1e810264d…`; PUT callback + bridge_dma_put_callback added) |

**rpcs3.exe at R8.1 landing:** sha256
`3ef63a825f9820373bb1df175bc975d5063f531b98206860fab36a50a8cd95d2`
(64 MB; built 2026-05-19 with R7.1 + R7.2 + R8.1 surface).

**Out of R8.1 scope (deferred to R8.2+):** DMA list cmds
(GETL/PUTL/GETLB/PUTLB), atomic primitives (GETLLAR/PUTLLC/
PUTLLUC/PUTQLLUC), MFC barriers / fence bits, multi-SPU DMA
races on shared EA, SPURS production support, in-line state
machine driven by the executor (would restore the dispatch-time
PUT assertion contract — currently deferred to post-replay). The
R6 / R7 hard rules carry forward verbatim to R8.1 and R8.2+.

---

## 8.2 R8.2 closure summary (2026-05-20)

R8.2 landed as the cleanest fixture-only delivery to date. The
9th oracle `single_spu_dma_get_multi_v1` exercises multi-tag
in-flight DMA + ALL wait semantics + multi-bit `WrTagMask`, on
top of the R8.1 baseline. **Zero engine-side code changes** —
the existing 8-oracle implementation already covered every
mechanic R8.2 required.

**Scope.** Two queued MFC GETs (tags 3 + 5, distinct EAs,
distinct sizes 128 + 64, distinct LSAs 0x10000 + 0x10100,
both in-flight before any wait) + `WrTagMask = 0x28` (=
`(1 << 3) | (1 << 5)`) + `WrTagUpdate = ALL` + `RdTagStat
= 0x28` (returned only after both completions fire). The SPU
computes a combined checksum `status = ((sum1 << 16) | sum2)
^ 0xFEEDFACE = 0xE12DEA4E` and halts via stop 0x101.

**Why this validates with no code changes.**

| Concern | Coverage |
|---|---|
| Parser accepts 2 `spu_mfc_cmd` events with cmd=0x40 | R6.7 A.2 already accepts cmd=0x40 unconditionally; events stream through the per-SPU transformer as pure context |
| State machine handles 2 tags in flight | R6.7 A.4 unit test `mfc_replay_handles_wr_tag_mask_update_basic` already covered 2-tag ALL mode (tags 3 + 5, same as R8.2) |
| Chunk loader resolves 2 distinct SHAs | R6.7 A.3 `resolve_dma_chunk_side_file` is content-addressed, indifferent to how many GETs the trace contains |
| Tag-stat queue with multi-bit value | R6.7 Phase C wired `mfc_tag_stat_queue: VecDeque<u32>`; ALL mode pushes exactly one entry (the mask) regardless of how many GETs preceded it |
| Bridge ON multi-dispatch | R7.2 callback is invoked per `wrch ch21` automatically; refuse_mfc gate already relaxed once the callback is installed |
| Executor reads from correct LS regions | `apply_mfc_dma_pre_replay` walks events linearly and copies each chunk into LS at the captured (lsa, size) — distinct lsa per GET means no overwriting |

The empirical "investigar quando acontecer" policy paid off:
implementing PUT in R8.1 also primed the engine for any multi-DMA
GET workload that doesn't introduce list / atomic / fence
semantics.

**Fixture + capture.** `single_spu_dma_get_multi_v1` source
(PSL1GHT homebrew, CC0): PPU allocates two distinct EA buffers
(ea_buf1 = 128 B counting pattern `i & 0xFF`, ea_buf2 = 64 B
constant 0x42), passes EA1 via `thread_args.arg0` and EA2 via
`arg1`. SPU dispatches both GETs back-to-back, waits via
`ch22 = 0x28` + `ch23 = ALL` + `ch24` read, sums both LS
regions, computes combined status, writes OUT_MBOX, halts.
Built in Docker via `rpcs3-ps3dev-toolchain:local` (image sha
`ed2167a9ac59…`); `.self` 940 KB sha
`7eb545af47a2c51e064b4d79090e2930d1cd6058edbd9d29032785d0ad535659`.
Capture run with `Core: SPU/PPU Decoder: Interpreter (static)`
(R8.1 gotcha carries forward); trace is 23 events (vs 15 for
single-DMA fixtures) including 2 `spu_mfc_cmd` + 2
`mfc_dma_complete` events + 2 distinct `.dmachunk` references.

**Content-addressed `.dmachunk` pool dedup.** Chunk #1
(`471fb943aa23c511f6f72f8d1652d9c880cfa392ad80503120547703e56a2be5`,
128 B counting pattern) deduplicates with R6.7 GET v1 + R8.1
PUT v1 — already in the canonical pool, NOT re-committed. Chunk
#2 (`c422e7070cb1cb455b5de9afee0d975e303d0239c72030cd7414ab5c382d3ae8`,
64 B constant 0x42) is new and lands in the pool. The pool now
holds 2 chunks total.

**Replay oracle.** New `rust/rpcs3-spu-recompiler/tests/
single_spu_dma_get_multi_v1_replay.rs`:
- Parses JSONL via `parse_jsonl_trace`.
- Asserts 2 `spu_mfc_cmd` events (tags 3 + 5, sizes 128 + 64,
  cmd 0x40, EAs distinct, chunk SHAs distinct).
- Asserts 2 `mfc_dma_complete` events matching tags + sizes.
- Asserts `ch22 = 0x28`, `ch23 = 2 (ALL)`, `ch24 rdch = 0x28`,
  ch28 = `0xE12DEA4E`, stop 0x101.
- Builds `SpuProgram` from `.spuimg`, seeds r3 = EA1 lane 1 +
  r4 = EA2 lane 1 (PSL1GHT arg0 + arg1 convention).
- Calls `apply_mfc_dma_pre_replay` (both chunks land in LS at
  captured LSAs; pre-application sanity-checks both regions
  carry the expected patterns).
- Runs replay × Interpreter + replay × Recompiler;
  `diff_snapshots.is_identical()`.
- Post-replay verifies BOTH backends' final LS at both regions
  matches the captured chunks byte-for-byte (mirrors the R8.1
  PUT shape).

**Triple-symmetry extension.** `check_triple_symmetry.py`
`FIXTURES` dict gained `get_multi`. Three fixtures green:

| Path | GET | PUT | GET_MULTI |
|---|---|---|---|
| bridge OFF TTY | `0xdeada12f` ✓ | spu=`0xc0ffeeca` ea_status=`0xcafea57e` ✓ | `0xe12dea4e` ✓ |
| bridge ON delegation | R7.2 total_steps=1054 ✓ | R8.1 total_steps=1049 ✓ | R7.2×2 total_steps=1584 ✓ |
| replay oracle | get_v1_replay ✓ | put_v1_replay ✓ | get_multi_v1_replay ✓ |

**Patch SHAs at R8.2 landing (`check_patch_separation.py` pin):**

| Patch | sha256 | Δ vs R8.1 |
|---|---|---|
| scaffolding | `cda976d7b7bace826b3e8c38475fab5e077c88201bd0b31768541f06635143a1` | unchanged |
| runtime hooks | `1f598d37c07d30f7e8b43fa0e53018b7c9277f628c2af58ab40bf987ffbd46ff` | unchanged |
| rust bridge | `0afda1c6943feb5d98329299a57dd68404095efb0a792839779febed13ab8a7e` | unchanged |

**rpcs3.exe at R8.2 landing:** sha256
`3ef63a825f9820373bb1df175bc975d5063f531b98206860fab36a50a8cd95d2`
(same binary as R8.1; no rebuild needed).

**Out of R8.2 scope (deferred to R8.3+):** DMA list cmds
(GETL/PUTL/GETLB/PUTLB), atomic primitives (GETLLAR/PUTLLC/
PUTLLUC/PUTQLLUC), MFC barriers / fence bits, multi-SPU DMA
races on shared EA, SPURS production support, ANY wait mode
(R8.2 covers ALL only; ANY exists in the state machine but
no oracle exercises it yet), in-line state-machine executor
wiring. The R6 / R7 / R8.1 hard rules carry forward verbatim.

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
