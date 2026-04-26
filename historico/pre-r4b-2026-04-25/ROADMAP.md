# Roadmap — Post-Freeze

**Last updated:** 2026-04-24 (post-hardening + iter-8 SPU expansion)
**Baseline:** [`CURRENT_STATE.md`](CURRENT_STATE.md) — 230 crates / **5186** tests / 229 iters / 0 regressions

The plan is **substantially complete** as a port-plan artifact. The next phases shift from "module surface coverage" to "execution against real targets." This roadmap is **objective**: each phase has a binary done/not-done condition, not a calendar estimate.

> **Important reorder (2026-04-24):** Phase B was originally "PPU homebrew." Post-hardening insight: **SPU homebrew is a strictly easier first target** because the SPU is hermetic (no syscalls, no LV2, no GPU). See [`HOMEBREW_PLAN.md`](HOMEBREW_PLAN.md) for the SPU-first detailed plan. PPU homebrew remains as Phase B' after SPU validates the harness end-to-end.

---

## Phase A — Close residual documentation / backlog

- **Goal:** Get every residual item out of `BACKLOG_RESIDUAL.md` either ported, or moved to `DEFERRED.md` with explicit reason / required input / unblock condition.
- **Scope:**
  - Walk [`BACKLOG_RESIDUAL.md`](BACKLOG_RESIDUAL.md) section by section
  - For each item: port (small helper) OR defer (large / blocked) OR delete (already covered, just naming drift)
  - Update [`CHECKLIST.md`](CHECKLIST.md) per-item status
- **Done when:** `BACKLOG_RESIDUAL.md` either is empty or every item has a defer-or-do decision recorded.
- **Estimated effort:** 5-15 small iters, no risk to existing tests.

---

## Phase B — SPU homebrew differential (NEW — moved up from Phase B')

- **Goal:** A standalone SPU homebrew ELF runs through both RPCS3 C++ and the Rust port, with byte-identical {GPR, LS, channels} state at termination.
- **Why first:** SPU is hermetic — no syscalls, no LV2 scheduling, no GPU. Validates the diff harness end-to-end before any PPU plumbing risk.
- **Scaffold:** `behavior-freeze/harness/spu_homebrew_runner.py` (placeholder), `behavior-freeze/docs/HOMEBREW_PLAN.md` (P1..P5).
- **Blockers:** (P1) commit a SPU ELF fixture, (P2/P3) author `rpcs3-spu-elf-loader` + `spu-runner` Rust binary.
- **Done when:** `python harness/spu_homebrew_runner.py --elf fixtures/spu/hello.elf` exits 0 with empty diff.
- **Estimated effort (ULTRATHINK):** 6-16 h focused work (see HOMEBREW_PLAN §"Caminho mínimo").

## Phase B' — PPU homebrew differential

- **Goal:** Original Phase B — PPU homebrew binary committed and runnable through both runtimes.
- **Depends on:** Phase B done (SPU validation proves harness pattern works).
- **Scope:** Same as old Phase B — pick `ps3autotests` or similar, license check, build, capture, diff.

---

## Phase R — SPU Recompiler (NEW, parallel to Phase B)

- **Goal:** Port `SPUCommonRecompiler.cpp` (9792 LOC) to Rust as a JIT backend that implements `rpcs3_spu_differential::SpuExecutor`.
- **Pre-conditions met (R0):** ✅ interpreter at 118 tests, ~60% ISA; ✅ `SpuExecutor` trait; ✅ 8 fixtures + 13 integration tests; ✅ `spu-runner --backend` flag.
- **Plan:** see [`SPU_RECOMPILER_PLAN.md`](SPU_RECOMPILER_PLAN.md) for fases R0-R5, decisões D1-D5, riscos.
- **Backend recomendado v0:** Cranelift (pure Rust, MSVC-friendly).
- **R0..R2.5 done (2026-04-25):** Decoder + scaffold + Cranelift JIT cobrindo ~91 opcodes. 6 fixtures + 2 programas reais rodam 100% via JIT.
- **R4a done (2026-04-25):** **Dispatcher loop + indirect branches**. JIT permanece em JIT-land via CONTINUE_TO outcome + function cache por `(entry_pc, ls_hash)`. synthetic_brsl_ret (subroutine call/return) roda 100% via JIT zero-fallback. 1.40× speedup medido. JitStats expandida com cache_hits/misses.
- **R4b done (2026-04-25):** **Chained patching seguro**. Chain-table local `HashMap<u32, ChainEntry>` (entry_fn fn-pointer + ls_hash + function_size) substitui o `compile_or_fetch` global no path quente. Reversibilidade total: chain miss/stale → fall through ao path R4a. JitStats +5 (chained_jumps, dispatcher_bypasses, patch_hits, patch_misses, invalid_chain_guards). Speedups vs interpreter: 1.44× synthetic_loop, 1.43× brsl_ret, 1.46× fibonacci, 1.36× sum_of_squares; 408/410 dispatcher iters bypass global em brsl_ret. +12 testes (4 safety + 4 equivalence × 10 repeats + 4 benchmarks).
- **R4c done (2026-04-25):** **SMC / cache invalidation seguro**. `CompiledMeta { code_start, code_end, exact_hash, function_size }` por função; `smc_scan` no início de cada dispatcher iter recompute hash exato e invalida atomicamente entradas stale (compiled + meta + cache + chain matched). Modelo per-entry com hash exato (sem full flush). JitStats +5 (smc_invalidations, smc_chain_evictions, smc_full_flushes, smc_range_hits, smc_range_misses). Speedups mantêm faixa R4b (1.40-1.58×). +14 testes (SMC detection, no-SMC equivalence, writes outside ranges, chain eviction com fn match, stats invariants, 4 benchmarks). 2 R4b tests atualizados para semântica R4c.
- **R5+ roadmap (próximo recomendado):**
  - **Interpreter resume from JitState**: hoje UNKNOWN_OPCODE → re-run completa via interpreter desde program.entry_pc. Ideal: transferir JitState para interpreter e continuar de onde parou. Requer adapter SpuThread ↔ JitState e teste de equivalência sob fallback parcial.
  - Generation counter para SMC: instrumentar store opcodes no codegen para bumpar generation; smc_scan só recompute hash quando generation diverge. Reduz overhead no caso comum onde nenhum store aconteceu.
  - IR-level patchpoint: Cranelift indirect call substituindo CONTINUE_TO no nível do código JIT, removendo round-trip ao dispatcher Rust — só se bench mostrar chain-table ainda como gargalo.
- **Done when:** Cranelift backend runs every committed fixture with byte-identical output + indirect branches funcionam zero-fallback + SMC detected and invalidated automatically.

---

## Phase C — Homebrew differential validation

- **Goal:** `compare_run.py` runs the chosen homebrew through both RPCS3 C++ and the Rust crates, diffs canonicalized log + frame hash + WAV dump.
- **Scope:**
  - Wire `compare_run.py` to drive both runtimes
  - Define canonicalization rules for log lines (timestamps, addresses, thread IDs)
  - Capture frame hash (RSX capture or rendered output hash)
  - Capture audio (WAV → FFT → spectral diff)
  - Lock baseline as "acceptable divergence" file
- **Done when:** A clean run of the homebrew produces zero unacceptable divergences across 3 successive runs.
- **Depends on:** Phase B.

---

## Phase D — Save / load real validation

- **Goal:** Round-trip `cellSavedata` against real save data: create save → write → restart → read → delete → verify.
- **Scope:**
  - Ensure homebrew (Phase B) exercises cellSavedata, OR
  - Synthesize a save data fixture (PARAM.SFO + PNG icon + binary blob)
  - Run save scenario through both implementations, compare directory hash
- **Done when:** Save scenario diverges in zero bytes across 3 runs.
- **Depends on:** Phase C (validates the differential infrastructure first).

---

## Phase E — Sentinel commercial title

- **Goal:** A user-supplied commercial PS3 title runs through `compare_run.py` as the canonical regression sentinel. Locally only — never committed.
- **Scope:**
  - User picks a "simple" title (short startup, deterministic, single-player, low RSX requirement)
  - User provides ROM dump (legal acquisition, kept off-repo)
  - Wire `compare_run.py` to point at the title (locally configured)
  - Pin a "known good" reference run as the regression oracle
- **Done when:** Re-running the sentinel after any code change produces zero unacceptable divergences.
- **Depends on:** Phase C/D infrastructure.

---

## Phase F — Performance / RAM / VRAM profiling

- **Goal:** Once correctness is locked across phases A-E, measure performance against RPCS3 C++ baseline. Identify hotspots that would benefit from Zig (per ADR-009) or unsafe Rust optimization.
- **Scope:**
  - Pick benchmarks (homebrew + sentinel run-to-completion times, peak RAM, peak VRAM)
  - Profile both implementations with consistent tooling
  - Identify regressions / equivalences / wins
  - Flag candidates for hot-path optimization (Zig, SIMD, cache layout)
- **Done when:** A benchmark report exists with both implementations side-by-side, and any regression > 10% has an issue tracked with a fix plan.
- **Depends on:** Phases A-E (correctness first).

---

## What this roadmap does NOT include

These are explicitly out of scope (see [`DEFERRED.md`](DEFERRED.md) items 6-9):

- ❌ JIT recompilers (SPU/PPU) — separate "wave-9-runtime"
- ❌ RSX runtime backends (Vulkan/GL) — separate wave with backend choice
- ❌ Qt UI replacement — separate wave with UI framework choice
- ❌ Full `lv2-syscall-table` argument binding — depends on ported `ppu_thread`

Those waves can run in parallel with phases A-F if and when someone signs up. They are not blocking the differential validation track.

---

## Phase ordering rationale

A → B → C → D → E → F is **not arbitrary**:

- **A first** because cleaning the backlog reveals the true work-remaining without distraction.
- **B before C** because differential needs an actual fixture to differ on.
- **C before D** because save/load is a more demanding test than basic boot.
- **D before E** because sentinel testing reuses the differential plumbing.
- **F last** because optimizing before correctness invites perf regressions that mask correctness regressions.
