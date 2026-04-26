# Project Status — post R4b/R4c/R5 SPU recompiler update

**Authoritative current source of truth for the RPCS3 → Rust port.**

Last updated: 2026-04-25. Latest landed layers: **R5 interpreter resume from JitState (partial fallback)** on top of **R4c minimal SMC / cache invalidation** on top of **R4b safe chained patching** on top of **R4a JIT dispatcher loop**. All landed in the same 2026-04-25 session.

For historical document snapshots taken at the time of this cleanup, see [`historico/pre-r4b-2026-04-25/`](../historico/pre-r4b-2026-04-25/).

---

## Current verified status

Tests below were executed locally during this update. Results recorded as of 2026-04-25:

| Command | Result | Tests |
|---|---|---|
| `cargo test -p rpcs3-spu-decoder --lib` | passed | 20 |
| `cargo test -p rpcs3-spu-differential --lib` | passed | 14 |
| `cargo test -p rpcs3-spu-interpreter --lib` | passed | 126 |
| `cargo test -p rpcs3-spu-recompiler --release` | passed | 98 |
| `cargo test -p spu-runner` | passed | 19 (5 smoke + 14 fixture/differential) |
| `cargo test --workspace --lib` | passed | 5355 |

Notes:

- `cargo test --workspace --lib` runs lib-only unit tests across every workspace crate. The 5355 figure is the sum of per-crate `passed` counts; **0 failed, 0 errors**.
- Integration tests not included in `--workspace --lib`: `spu-runner` (19), `rpcs3-spu-decoder` fixture-driven (8). Combined with workspace lib that's 5382 distinct tests passing today.
- `cargo test --workspace --release` (full workspace, release profile) is **NOT** asserted green here. A few HLE crates (e.g. `rpcs3-hle-cellsysutilmisc`, `rpcs3-hle-cellmusicselectioncontext`, `rpcs3-hle-celljpgdec`, `rpcs3-hle-cellvideoexport`) have a pre-existing `no_std`/`global_allocator` build error that surfaces only under `--release`. This error is unrelated to the SPU recompiler stack and was present before the R4a/R4b/R4c work.

**Do not promote the workspace as "green" without specifying scope.** `--workspace --lib` is green (5355 passed today); `--workspace --release` has the pre-existing HLE compile error documented above.

---

## Executive summary

- The Rust port already replaces the broad coverage layer of `Cell/Modules/`, `Audio/`, `Io/`, `Loader/`, `RSX/` helpers, `NP/`, LV2 syscalls, and many HLE modules byte-exact.
- A pure-Rust SPU recompiler (`rpcs3-spu-recompiler`) is **operational** with a Cranelift-backed JIT covering ~102 SPU opcodes, indirect-branch dispatcher (R4a), safe chained patching (R4b), minimal SMC/cache invalidation (R4c), and **interpreter resume from JitState (R5 partial fallback)** so that an unsupported opcode mid-program no longer forces a re-run from the entry PC.
- Real homebrew validation, RPCS3 dump capture, full PPU JIT, LLVM backend, RSX runtime, Qt UI: still out of scope.

---

## What is complete

- Behavior-freeze harness: `compare_run.py`, `capture_baseline.py`, `run_headless.py`, contracts, fixture spec.
- Synthetic fixture pipeline for SPU: 8 ELF fixtures committed, `build_synthetic_fixtures.py` reproduces them, `spu_homebrew_runner.py` diffs interpreter vs recompiler.
- 230+ Rust crates covering deterministic surface (parsers, crypto, HLE module signatures, Audio/Io device emulation, RSX helpers, LV2/sysPrxForUser stubs). The "230+" figure is an **approximate** carry-over from the 2026-04-24 frozen baseline (230 crates) plus the post-freeze SPU stack additions (`rpcs3-spu-decoder`, `rpcs3-spu-differential`, `rpcs3-spu-recompiler`, `spu-runner`); a fresh exact count is not asserted here unless the workspace member list is re-counted.
- SPU interpreter: 126 unit tests (executed locally above), ~70% ISA coverage with FTZ denormal flush in `fm`, channel-count snapshot for differential harness.
- SPU decoder: 20 lib unit tests + 8 fixture-driven integration tests; two-pass leader analysis builds basic-block graphs for Cranelift codegen.
- SPU differential trait crate: 14 tests; `SpuExecutor` is the backend-agnostic interface used by both interpreter and recompiler.
- SPU runner CLI: 19 tests (5 smoke + 14 fixture/differential); `--backend interpreter|recompiler` flag for ad-hoc validation.
- SPU recompiler (`rpcs3-spu-recompiler`): 92 lib tests (release-profile run above), all green; 0 fallback to interpreter on the 8 committed synthetic fixtures.

---

## What is partially complete

- SPU recompiler opcode coverage (~102 of the full SPU ISA). Common opcodes are codegen'd; rare/edge cases (channel ops, double-precision float, etc.) are still unsupported. As of R5, an unsupported opcode mid-program triggers a *partial* fallback (interpreter resumes from the JIT-exit PC with all GPRs/LS preserved) rather than a full re-run from `program.entry_pc` — see the SPU recompiler R5 section below.
- SMC invalidation: minimal R4c is in place (per-entry hash scan at dispatcher iter start). Not yet driven by codegen-instrumented store opcodes; reactive scan only.
- Homebrew differential validation: synthetic fixtures only. No real PPU/SPU homebrew ELF committed yet, so byte-exact parity vs RPCS3 C++ on real workloads is not asserted.

---

## SPU interpreter status

- Crate: [`rust/rpcs3-spu-interpreter`](../rust/rpcs3-spu-interpreter)
- Tests: 126 lib tests, executed locally now → all passed.
- Coverage: ~70% of SPU ISA. Includes float ops with FTZ denormal flush, halfword/byte ops, channel I/O snapshot, branch family.
- Role: reference oracle for the differential harness. The recompiler must produce byte-identical state.

## SPU decoder status

- Crate: [`rust/rpcs3-spu-decoder`](../rust/rpcs3-spu-decoder)
- Tests: 20 lib + 8 fixture-driven integration. Lib tests executed locally → 20 passed.
- Approach: two-pass — first pass collects branch leaders, second pass cuts blocks. Produces `SpuFunction { entry, blocks: BTreeMap<u32, SpuBasicBlock> }`.
- Used by: recompiler (compile path), tooling (block visualization).

## SPU differential harness status

- Crate: [`rust/rpcs3-spu-differential`](../rust/rpcs3-spu-differential)
- Tests: 14 lib, executed locally → all passed.
- Provides `SpuExecutor` trait, `SpuProgram`, `SpuStateSnapshot`, `ExecutionStopReason`, `diff_snapshots`, `run_and_diff`, `error_result`.
- Channel snapshot populated (`SpuStateSnapshot::channel_counts`) — was an R3 blocker, now resolved.

## SPU runner / fixtures status

- Binary: [`rust/spu-runner`](../rust/spu-runner) — CLI driver with `--backend interpreter|recompiler`.
- Tests: 19 (5 smoke + 14 fixture/differential), executed locally → all passed.
- Committed fixtures: 8 synthetic SPU ELFs in `behavior-freeze/fixtures/spu/` covering il/stop, arith, loop, float dot product, load/store, halfword shifts, brsl+ret, orx collapse.
- Each committed fixture runs 100% via JIT with `fallback_count = 0` (verified by recompiler tests).

## SPU recompiler status

- Crate: [`rust/rpcs3-spu-recompiler`](../rust/rpcs3-spu-recompiler) — backend-agnostic `SpuExecutor` impl wrapping a Cranelift JIT.
- Tests: 92 lib tests, executed locally now under `--release` → all passed.
- JIT is **NOT** a delegate-to-interpreter scaffold anymore. It is a real Cranelift JIT generating x86-64 code from `SpuFunction` graphs with multi-block compile.

### R1 — Decoder (DONE)
Two-pass leader analysis; 8 fixtures decode without errors.

### R2 — Scaffold (DONE)
Trait impl + cache + harness wiring.

### R2.5 — Cranelift JIT codegen (DONE for broad subset)
~102 opcodes across ALU word/halfword/byte, compares, shifts, multiplies, float arith/compares/converts, RRR (selb/shufb/fma/fnms/fms), branches direct + indirect, branch hints, load/store qword (D-form + indexed), unary RR (clz/cntb/sign-ext/fsm/frest/frsqest), quadword byte shifts, byte-immediate ops.

### R4a — JIT dispatcher loop + indirect branches (DONE)
- New JIT outcomes: `STOP`, `CONTINUE_TO`, `STALL` (reserved), `UNKNOWN_OPCODE`.
- Codegen for `bi`/`bisl`/`iret` (`UncondIndirect`) and `biz`/`binz`/`bihz`/`bihnz` (`CondIndirect`).
- Function cache keyed by `(entry_pc, ls_hash_around)`.
- `try_jit_run` is a dispatcher loop: compile-or-fetch → call → outcome → loop or return.
- `synthetic_brsl_ret.elf` runs 100% via JIT (entry function + continuation function, both compiled, dispatcher iterates twice per call).
- Reported speedup vs interpreter: ~1.40× on `brsl_ret` (200 release runs at the time R4a landed).

### R4b — Safe chained patching (DONE)
- **R4b is dispatcher-level chain table, NOT machine-code/IR patching.** The chain lives in `JitCache.chain: HashMap<u32, ChainEntry>` and is consulted by Rust at the start of each dispatcher iter.
- `ChainEntry { entry_fn: extern "C" fn(*mut JitState) -> u32, ls_hash: u64, function_size: u64 }`. The `entry_fn` is a stable Cranelift fn-pointer; `ls_hash` is the safety guard.
- `chain_lookup` returns `Hit { entry_fn, function_size }` / `Stale` / `Miss`:
  - `Hit` skips `compile_or_fetch` entirely.
  - `Stale` (chain entry exists but `ls_hash` diverged) evicts the chain entry, falls through.
  - `Miss` falls through to the R4a path; chain is then installed for next time.
- Chain table persists across `execute()` calls. A repeated execution of the same `SpuProgram` hits the chain on every dispatcher iteration after the first.
- `clear_function_cache()` purges both the compiled cache and the chain table.
- New stats in `JitStats`: `chained_jumps`, `dispatcher_bypasses`, `patch_hits`, `patch_misses`, `invalid_chain_guards`. Invariant: `dispatcher_iterations == patch_hits + patch_misses`.
- Reversibility: any failure (Miss / Stale) falls through to the R4a path with no correctness loss. Byte-exact equivalence with the interpreter is preserved.

### R4c — Minimal SMC / cache invalidation (DONE in same session as R4b)
- Per-compile metadata `CompiledMeta { code_start, code_end, exact_hash, function_size }` stored in a parallel map `JitCache.compiled_meta`.
- `smc_scan(ls)` runs at the start of every dispatcher iter. It walks `compiled_meta`, recomputes `hash_ls_range(ls, code_start, code_end)`, and atomically invalidates entries whose hash diverged. Removes from `compiled` + `compiled_meta` + decoded cache + chain table (only when the chain entry's `entry_fn` matches the just-evicted function).
- New stats: `smc_invalidations`, `smc_chain_evictions`, `smc_full_flushes` (reserved escape hatch, currently 0), `smc_range_hits`, `smc_range_misses`.
- The R4b `ls_hash` chain guard remains as a second line of defense; SMC scan now catches modifications proactively before chain lookup runs.
- SMC detection model chosen: per-entry exact-range hash, no full flush. Conservative by construction; `smc_full_flushes` stays at 0 unless the implementation is changed.
- Reversibility: scan is purely additive on top of R4a/R4b. Removing the `smc_scan` call would weaken proactive SMC detection and rely only on the remaining cache/chain guards. The scan should be treated as the authoritative R4c safety layer; the R4b `ls_hash` chain guard exists as a secondary check, not as an equivalent substitute.

### R5 — Interpreter resume from JitState (partial fallback) (DONE in same session)

- Problem solved: previously, when the JIT could not continue (compile failure on a target function, or runtime `JIT_OUTCOME_UNKNOWN_OPCODE`), the dispatcher gave up and the executor re-ran the program from `program.entry_pc` via the interpreter — discarding all JIT-prefix progress (GPRs, LS writes, PC, link registers).
- New API: `InterpreterExecutor::resume_from_state(gpr, ls, pc, max_steps_remaining, prior_steps)` in [`rust/rpcs3-spu-differential/src/lib.rs`](../rust/rpcs3-spu-differential). Builds an `SpuThread`, populates it from the snapshot, runs the interpreter from `pc`, and folds `prior_steps` into the returned `steps_executed`.
- New helper: `RecompilerExecutor::partial_fallback_to_interpreter(state, ls, total_steps, program)` in [`rust/rpcs3-spu-recompiler/src/lib.rs`](../rust/rpcs3-spu-recompiler). Snapshots the in-flight `JitState` (128 GPRs × 4 lanes via `state.load_gpr`), passes the existing `ls` buffer (already mutated by JIT stores) and `state.pc` to `resume_from_state`. Stats updated atomically.
- Both former hit_unsupported paths in `try_jit_run` now route through this helper:
  - `compile_or_fetch` returns `Err` → partial fallback from current `state.pc` (= target_pc of the failing function).
  - `JIT_OUTCOME_UNKNOWN_OPCODE` runtime exit → partial fallback from current `state.pc`.
- The old "full fallback from entry_pc" path (`stats.fallback_runs += 1; self.interp.execute(program)`) is now dead code in practice — `try_jit_run` always returns `Some(...)` for these conditions. Kept as defensive scaffold; `fallback_runs` should remain 0 in normal operation.
- New stats in `JitStats`: `partial_fallbacks`, `unknown_opcode_exits` (subset), `channel_stall_exits` (reserved, currently 0), `resumed_interpreter_runs`, `resumed_interpreter_steps`. Channel-stall handoff is reserved for future R5+ once the JIT gains channel codegen.
- Scope guarantees: byte-exact equivalence vs interpreter-only is preserved (verified by `r5_partial_fallback_*_equivalence_*` tests). 100%-JIT-supported fixtures still execute with `partial_fallbacks == 0` and `fallback_runs == 0` — R5 only fires when the JIT genuinely cannot continue.
- Known limit (carried forward): channel state is **not** propagated from JIT into the resume call. Today's JIT does not codegen channel ops, so any channel touch triggers the partial fallback that lands in `resume_from_state` with channels at default (empty) state. This is consistent — a channel op would be the first thing the interpreter executes, and the SpuThread it constructs starts with empty channels just as a fresh `execute` would.

---

## Performance status

The numbers below are **reported benchmark output** from `cargo test -p rpcs3-spu-recompiler --release` benches printed to stderr. They were captured at the time R4b landed and re-confirmed during R4c benches; both are within the same noise band. Numbers are not a guarantee — they vary across machines and runs.

| Program | Speedup vs interpreter (R4b) | Speedup vs interpreter (R4c) |
|---|---|---|
| `synthetic_loop` | ~1.44× | ~1.40× |
| `brsl_ret` | ~1.43× | ~1.46× |
| `fibonacci` | ~1.46× | ~1.45× |
| `sum_of_squares` | ~1.36× | ~1.58× |

R4c added a per-iter SMC scan (hash recompute over each compiled function's exact code range). The wall-clock cost is small enough that R4c speedups stay within the same noise band as R4b. **No material performance regression** observed.

The R4b chain table satisfies, on `brsl_ret` at 200 release runs, ~408 of ~410 dispatcher iterations (chained_jumps); on single-function programs it satisfies ~204 of ~205. This is reported observability, not a guarantee.

---

## Known limitations

- **Self-modifying-code detection is reactive, not codegen-driven.** R4c scans every iter rather than triggering on store opcodes. Generation counter / per-store invalidation is a future improvement.
- **Channel state is not propagated through partial fallback.** R5 hands the SpuThread to the interpreter with channels at default (empty). The JIT does not yet codegen channel ops, so any channel-touching instruction is always the first thing executed by the interpreter side and sees the same default state a fresh `execute()` would. If the JIT ever gains channel codegen, channel state will need to flow through `resume_from_state` too.
- **PPU JIT is not the focus of this wave.** No PPU recompiler started.
- **No complete LLVM backend yet.** The decision matrix in [`SPU_RECOMPILER_PLAN`](../historico/pre-r4b-2026-04-25/SPU_RECOMPILER_PLAN.md) calls for evaluating LLVM (`inkwell`) only after the Cranelift backend hits a clear ceiling. It has not.
- **RSX runtime and Qt UI remain out of scope.** Helpers in `rpcs3-rsx-*` exist; the runtime thread (`RSXThread.cpp`, `VKGSRender.cpp`) and the Qt UI (`rpcs3qt/`) do not.
- **HLE crates have a pre-existing `no_std`/`global_allocator` build error under `--release`.** Unrelated to SPU recompiler. Documented above; not yet investigated.

---

## What not to claim yet

- **Do not** claim "RPCS3 Rust port complete". The runtime giants (PPU JIT, RSX runtime, Qt UI) are out of scope by design and would each be multi-week dedicated projects.
- **Do not** claim the SPU JIT is "byte-exact on real homebrew". It is byte-exact on the 8 committed synthetic SPU fixtures only. No real homebrew SPU ELF is committed yet.
- **Do not** claim "workspace green" without specifying scope. Current truth is `cargo test --workspace --lib` passes 5355 tests; `cargo test --workspace --release` does not, due to pre-existing HLE build issues.
- **Do not** claim performance speedups as guaranteed. The numbers above are reported benchmark output, machine- and run-dependent.
- **Do not** claim the recompiler "delegates to the interpreter". It does not (anymore). The recompiler's Cranelift JIT runs every committed fixture end-to-end with `fallback_count = 0`. Interpreter is used as the differential oracle and, via R5 partial fallback, as the resume target when the JIT cannot continue (unsupported opcode). The current 8 synthetic fixtures never trigger R5 — the partial-fallback path is exercised by dedicated `r5_*` tests with channel ops, not by general workloads.

---

## Next recommended phase

R5 partial fallback is now done. Two candidates for R5.x / R6, in order of expected payoff:

**Option A — JIT codegen for channel ops (`rdch`/`wrch`/`rchcnt`).** Now that R5 lets us safely fall back from any unsupported op without losing state, the next high-value codegen target is the channel family. Today every channel touch triggers `partial_fallback_to_interpreter` from the start of its function (because the channel op is what makes the function fail to compile). Adding codegen for channel ops in `rpcs3-spu-recompiler/src/jit.rs` `supported_check` + an `emit_channel_*` family would keep more programs in JIT-land. Once the codegen exists, R5's `channel_stall_exits` stat becomes meaningful — channel ops that *would* stall would propagate `JIT_OUTCOME_STALL` and resume the interpreter with channel state intact (which requires extending `resume_from_state` to also accept channel state).

**Option B — Generation counter for R4c SMC.** Today `smc_scan` recomputes the exact-range hash for every compiled function on every dispatcher iter. For non-SMC programs that's pure overhead. Instrumenting store opcodes (`stqd`/`stqx`) in the JIT codegen to bump a global "LS generation" counter would let `smc_scan` short-circuit when no store has happened since the last scan. Reduces per-iter overhead in the common case. Requires modifying JIT codegen — a clean R5+ wave. Don't pursue unless `smc_range_misses` per iter becomes a measurable hotspot.

**R6 candidate (deferred until needed): IR-level patchpoint.** Replace `CONTINUE_TO` in the JIT body with an indirect call to the next `entry_fn` read from a chain table, eliminating the round-trip to the Rust dispatcher. The current dispatcher loop with R4b chain table is fast enough that this hasn't been a bottleneck on existing fixtures. Only pursue if a real-workload benchmark shows the Rust-side dispatcher as the bottleneck.

---

## Test commands and latest observed results

All commands below were executed locally during this update. Full output not reproduced here; results are pasted from the actual `test result:` summary line of each `cargo test` run.

```bash
# SPU stack — each crate independently:
cargo test -p rpcs3-spu-decoder --lib
# → test result: ok. 20 passed; 0 failed.   (verified locally now)

cargo test -p rpcs3-spu-differential --lib
# → test result: ok. 14 passed; 0 failed.   (verified locally now)

cargo test -p rpcs3-spu-interpreter --lib
# → test result: ok. 126 passed; 0 failed.  (verified locally now)

cargo test -p rpcs3-spu-recompiler --release
# → test result: ok. 98 passed; 0 failed.   (verified locally now — 92 + 6 R5)

cargo test -p spu-runner
# → 14 passed (fixture/differential) + 5 passed (smoke). (verified locally now)

# Full workspace lib tests:
cargo test --workspace --lib
# → 5355 passed total, 0 failed.            (verified locally now — 5349 + 6 R5)

# Full workspace release: NOT GREEN.
cargo test --workspace --release
# → pre-existing build error in rpcs3-hle-cellsysutilmisc, rpcs3-hle-cellmusicselectioncontext,
#   rpcs3-hle-celljpgdec, rpcs3-hle-cellvideoexport (no_std + missing global_allocator).
#   Unrelated to SPU recompiler stack; existed before R4a/R4b/R4c work.
```

Speedup numbers in the Performance section above are reported benchmark output from `cargo test -p rpcs3-spu-recompiler --release -- --nocapture r4b_benchmark r4c_benchmark`. Treat them as observed, not guaranteed.

---

## Historical docs location

All status/plan markdown files as they were at the start of this cleanup live at [`historico/pre-r4b-2026-04-25/`](../historico/pre-r4b-2026-04-25/). Each file is a verbatim copy of what was at `behavior-freeze/docs/<name>.md` at that moment.

After the move, `behavior-freeze/docs/` keeps **only** files that have an active reason to be at that path:

- `AUTONOMOUS_LOG.md` — stub. Kept at this path because `.claude/settings.local.json` Stop hook appends a `turn ended` timestamp every turn, and SessionStart reads `tail -5`. Full pre-cleanup content is at [`historico/pre-r4b-2026-04-25/AUTONOMOUS_LOG.md`](../historico/pre-r4b-2026-04-25/AUTONOMOUS_LOG.md).
- `SPU_RECOMPILER_PLAN.md` — stub. Kept at this path because `rust/rpcs3-spu-recompiler/src/lib.rs` cites it in a doc comment that is intentionally not edited (cleanup task scope is docs-only). Full content at [`historico/pre-r4b-2026-04-25/SPU_RECOMPILER_PLAN.md`](../historico/pre-r4b-2026-04-25/SPU_RECOMPILER_PLAN.md).
- `INVENTORY.md` — full content. Factual P0/P1/P2 inventory of code with file:line anchors. Referenced by `README.md`, `behavior-freeze/README.md`, and `behavior-freeze/contracts/README.md`.
- `HOMEBREW_PLAN.md` — full content. Plan for P1+P5 (real homebrew + RPCS3 dump capture). Referenced by `behavior-freeze/harness/spu_homebrew_runner.py:62` and `behavior-freeze/fixtures/README.md:33`.
- `DECISIONS.md` — full content + new header note. ADR log; numbers in individual ADRs are point-in-time records. Header now redirects to this document for current numbers.
- `DEFERRED.md` — full content. Items explicitly deferred with reason / required input / unblock condition.
- `BACKLOG_RESIDUAL.md` — full content + new header note. Backlog at the 2026-04-24 frozen baseline; header now redirects to this document for current status and updated workflow.

Files **moved out** of `behavior-freeze/docs/` (originals removed; verbatim copies preserved in `historico/pre-r4b-2026-04-25/`):

- Stubs that were only cross-linked from sibling docs: `CURRENT_STATE.md`, `CHECKLIST.md`, `ROADMAP.md`, `PORT_PLAN.md`.
- Per-day frozen snapshots: `CURRENT_STATE_2026-04-24.md`, `CHECKLIST_FREEZE_2026-04-24.md`, `PLAN_FREEZE_2026-04-24.md`.
- Backup files: `CHECKLIST.md.bak-20260424-213908`, `PORT_PLAN.md.bak-20260424-213908`.

External readers were updated to point either to this document or to the historical copy in `historico/`:

- `README.md` — status section, scope clarification, doc index.
- `rust/README.md` — line 3 + line 401: PORT_PLAN.md references redirected.
- `behavior-freeze/README.md` — layout tree + "veja docs/CHECKLIST.md" line redirected to PROJECT_STATUS / historico.
- `behavior-freeze/docs/DECISIONS.md` — ADR-011 + ADR-012 cross-links updated.
- `behavior-freeze/docs/BACKLOG_RESIDUAL.md` — "When a residual item gets picked up" steps updated.

R5 (interpreter resume from JitState) added new code without changing the doc structure:

- `rust/rpcs3-spu-differential/src/lib.rs` — added `InterpreterExecutor::resume_from_state(&self, gpr, ls, pc, max_steps_remaining, prior_steps) -> SpuExecutionResult`.
- `rust/rpcs3-spu-recompiler/src/lib.rs` — added `JitStats { partial_fallbacks, unknown_opcode_exits, channel_stall_exits, resumed_interpreter_runs, resumed_interpreter_steps }` fields, helper `partial_fallback_to_interpreter`, and rewired both former `hit_unsupported` paths in `try_jit_run` through it. Added 6 R5 tests (`r5_partial_fallback_*` + `r5_no_fallback_*` + `r5_benchmark_*`).
