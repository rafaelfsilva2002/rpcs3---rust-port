# SPU Recompiler — Port Plan

**Status:** scoping. Implementation **NOT STARTED**.
**Pre-conditions met:** ✅ interpreter at 118 tests / 50%+ ISA coverage; ✅ `SpuExecutor` trait via [`rpcs3-spu-differential`](../../rust/rpcs3-spu-differential/); ✅ 8 fixtures + 13 integration tests for differential validation.
**Owner:** next phase of the project.

---

## What we're porting

C++ source: `rpcs3/Emu/Cell/SPUCommonRecompiler.cpp` (9792 lines) + companion files:
- `SPULLVMRecompiler.cpp` (9497 lines) — LLVM backend
- `SPUASMJITRecompiler.cpp` (4878 lines) — legacy ASMJIT backend (deprecated upstream)

`SPUCommonRecompiler.cpp` is the **shared analysis + caching + runtime layer**. The actual codegen lives in the backend files.

### Responsibilities of `SPUCommonRecompiler.cpp` (mapped from line numbers)

| Section | Lines | Responsibility |
|---|---|---|
| `spu_cache` | 709–733+ | On-disk + in-memory cache of compiled functions; rebuilds on launch |
| `spu_program` ordering | 1218–1224 | Equality / less-than for function dedup (key: instruction bytes + entry) |
| `spu_runtime::rebuild_ubertrampoline` | 1307 | The "ubertrampoline" — dispatcher that routes any SPU PC to the right compiled function or falls back to the interpreter |
| `spu_runtime::find` / `make_branch_patchpoint` | 1892, 1917 | Block lookup; patch indirect branches once the target is compiled |
| `spu_recompiler_base::dispatch` / `branch` | 2012, 2116 | C++ helpers called *from* JIT'd code at block exits |
| `spu_recompiler_base::old_interpreter` | 2212 | Fallback path when the JIT can't (or hasn't yet) compiled a region |
| `reg_state_t` | 2479+ | Abstract per-register state: known-constant, known-mask, unknown, etc. |
| `block_reg_info` | 2763 | Per-block summary of register flow (live-in / live-out) |
| `spu_recompiler_base::analyse` | 2816 | The big one — control-flow analysis that produces a `spu_program` (basic-block graph + register state per block) |
| `spu_recompiler_base::dump` | 8709 | Disassembly emitter for debugging |
| `spu_llvm_worker` / `spu_llvm` | 8802, 8927 | The LLVM-based backend |
| `spu_fast : public spu_recompiler_base` | 9149 | Faster non-LLVM backend (uses templated lambdas) |
| `add_pattern` | 9766 | Registers known instruction patterns for fast-path codegen |

The whole file is essentially: **decode → analyse → cache → codegen → dispatch**.

---

## Decision tree (in dependency order)

### D1. Codegen backend — which compiler infrastructure?

Three options with honest trade-offs:

| Option | Crate / Dep | Pros | Cons | Weight |
|---|---|---|---|---|
| **Cranelift** | `cranelift` (pure Rust, ~5 MB) | Fast compile, easy integration, debuggable IR, MSVC-friendly | 10–30% slower output than LLVM | **Recommended for v0** |
| **LLVM via `inkwell`** | `inkwell` (binds to LLVM 18+, system dep) | Matches RPCS3 closely, best code quality, mature | LLVM build is a Windows nightmare; ~200 MB toolchain; slow compile times | Defer to v2 if Cranelift hits a wall |
| **Hand-rolled x86_64** | `iced-x86` or raw asm | Fastest startup, smallest dep, exact control | x86_64-only; mirrors `spu_fast` complexity; high engineering cost | Skip unless v1 is too slow |

**Recommendation:** Cranelift first. It's the lowest-risk path and the one that lets us reach "first working JIT" fastest. Re-evaluate after a working Cranelift backend exists and we can measure.

### D2. What does the recompiler implement?

[`SpuExecutor`](../../rust/rpcs3-spu-differential/src/lib.rs) — the same trait the interpreter uses. This makes the differential harness automatic:

```rust
let mut interp = InterpreterExecutor::default();
let mut recomp = CraneliftRecompilerExecutor::new();
let (a, b, diff) = run_and_diff(&mut interp, &mut recomp, &program);
assert!(diff.is_identical(), "{:#?}", diff);
```

### D3. Cache strategy?

C++ uses an on-disk cache keyed by `spu_program` bytes hash. For v0:
- **Skip the disk cache.** Compile everything in-process, hold in a `HashMap<u64, CompiledBlock>` keyed by program hash.
- Disk cache lands later if cold-start time becomes a bottleneck.

### D4. Block analysis depth?

C++ `spu_recompiler_base::analyse` (line 2816) is ~5000 lines of dense flow analysis. It tracks per-instruction `reg_state_t` (constant propagation, register taint, branch taken/not-taken predictions, etc.).

For v0: **skip register tracking.** Treat every register as unknown at every point. Codegen reads from a "register file" struct and writes back. Slow, but correct, and unlocks the JIT before we burn weeks on the analyzer.

For v1: port `reg_state_t` constant propagation. This unlocks ~3× speedup by inlining immediates into machine code.

### D5. Fallback?

C++ has `old_interpreter` as a safety net. We have it for free — if the JIT can't compile a block (unimplemented opcode, suspect input), fall back to `InterpreterExecutor`. Same backend instance, no duplicated state.

---

## Phased plan

### Phase R0 — Pre-flight (DONE — 2026-04-25)
- ✅ `SpuExecutor` trait + `run_and_diff` infra
- ✅ Interpreter at 118 tests, ~50% ISA coverage
- ✅ 8 fixtures + 13 integration tests
- ✅ `spu-runner` selects backend via `--backend` flag

### Phase R1 — Decode + analyze (DONE — 2026-04-25)
- ✅ New crate: `rpcs3-spu-decoder` (20 lib + 8 fixture-driven integration tests).
- ✅ `decode_inst(raw, pc) -> SpuInstruction` — pure single-instruction decode.
- ✅ `decode_function(ls, entry, max_blocks) -> SpuFunction` — two-pass leader analysis (collect all branch targets first, then cut blocks at every leader so blocks never overlap).
- ✅ `SpuInstKind` enum coarse-grained by family (Stop, Nop, AluRr, AluImm, Rrr, LoadStore*, Branch*, Channel, Unary, etc.).
- ✅ `BlockTerminator` enum: Stop / UncondDirect / UncondIndirect / CondDirect / CondIndirect / UnknownOpcode / FellThroughLimit.
- ✅ Integration tests against all 8 committed fixtures.
- 🔧 **Side-effect:** caught and fixed canonical-opcode bug for A (0x180→0xC0), AND (0x181→0xC1), SF (0x080→0x40). All fixtures regenerated. See "Critical fix" section below.

### Phase R2 — Recompiler scaffold (DONE — 2026-04-25)
- ✅ New crate: `rpcs3-spu-recompiler` (7 lib tests).
- ✅ `RecompilerExecutor` implements `SpuExecutor` — proves trait-based plug-in.
- ✅ Function cache (`HashMap<(entry, ls_hash), SpuFunction>`).
- ✅ Currently delegates to interpreter — **byte-identical by construction**.
- ✅ `spu-runner --backend recompiler` flag; 4 differential integration tests confirm interpreter≡recompiler on real fixtures.
- 🔜 **Next: codegen.** Backend (Cranelift recommended per D1) plugs in inside `execute()` between decode and delegate.

### Phase R2.5 — Cranelift backend (MULTI-BLOCK done — 2026-04-25)
- ✅ **APPROVED** — added `cranelift = "0.118"` + `cranelift-jit` + `cranelift-module` + `cranelift-native`.
- ✅ `JitBackend` em `rpcs3-spu-recompiler/src/jit.rs` — codegen real via Cranelift JIT.
- ✅ **Multi-block compile**: cada SPU basic block vira uma Cranelift block. Direct branches → `jump`; conditional brnz/brz → `brif`; brsl → write link + jump.
- ✅ **Subset suportado** (~44 opcodes):
  - Control: `stop`, `nop`, `lnop`, `br`, `bra`, `brnz`, `brz`, `brsl`
  - LoadImm: `il`, `ila`, `ilh`, `ilhu`, `iohl`
  - RR ALU word: `a`, `sf`, `and`, `or`, `xor`, `nor`, `nand`, `eqv`, `andc`, `orc`
  - RR compares word: `ceq`, `cgt`, `clgt`
  - RR word shifts: `shl`, `shr` (rotm), `sar` (rotma), `rot`
  - RR carry/borrow: `cg`, `bg`
  - RI10 ALU/cmp word: `ai`, `andi`, `ori`, `xori`, `ceqi`, `cgti`
  - RI7 word shifts: `shli`, `roti`, `rotmi`, `rotmai`
  - **Load/store qword: `lqd`, `stqd`, `lqx`, `stqx`** (via `ls_ptr` no JitState + bswap BE/LE)
  - **Halfword arith: `ah`, `sfh`** (split-mask-add-repack)
  - **Halfword shift imm: `shlhi`, `rothi`, `rothmi`, `rotmahi`** (split-mask-shift-repack with sextend for arith shr)
  - **Unary OR-across: `orx`** (collapse into preferred slot)
  - **Float arith: `fa`, `fs`, `fm`** (Cranelift fadd/fsub/fmul with i32↔f32 bitcast; FTZ denormal flush for `fm`)
  - **RRR-form bit-select: `selb`** (`(rc & rb) | (!rc & ra)` per lane)
  - **Halfword compares: `ceqh`, `cgth`, `clgth`** (split-mask-icmp-select-pack with sextend for signed gt)
  - **Multiplies: `mpy`, `mpyu`, `mpyi`, `mpyui`** (signed/unsigned 16×16→32 per lane via Cranelift `imul`; imm variants broadcast a 16-bit constant)
  - **Float compares: `fcgt`, `fcmgt`, `fceq`, `fcmeq`** (Cranelift `fcmp` + FTZ flush + abs mask for magnitude)
  - **Byte compares: `ceqb`, `cgtb`, `clgtb`** (16-lane via 4 byte slices per word, pack via OR-shifts)
  - **Halfword RR shifts: `shlh`, `roth`, `rothm`, `rotmah`** (dynamic per-halfword shifts with saturation >= 16)
  - **Halfword imm compares: `ceqhi`, `cgthi`, `clgthi`** (broadcast imm10 + per-halfword cmp)
  - **More word imm: `sfi` (sub-from), `clgti` (unsigned gt imm), `ahi` (halfword add imm)`**
  - **Extended multiplies: `mpyh`, `mpyhh`, `mpys`** (signed high×low<<16, high×high, low×low truncated)
  - **Unary RR ops: `clz`, `cntb`, `xsbh`, `xshw`, `xswd`, `fsm`, `frest`, `frsqest`** (Cranelift native ops + sign-extension chains + FTZ flush)
- ✅ **Performance benchmark:** JIT measured at **1.58× faster** than interpreter on `synthetic_loop` (200 runs, release build). Documents that codegen delivers concrete value, not just proof-of-concept.
- ✅ **Real-world program demos:**
  - Fibonacci(10) — 11-instruction SPU program with 3 register vars + conditional exit + back-edge → r3 = 55 broadcast.
  - Sum-of-squares(1..10) — 9-instruction SPU program with mpyu accumulator loop → r3 = 385 broadcast.
  - **Brsl_ret** (R4a) — subroutine call/return via brsl + bi → r3 = 17 broadcast (10+7).
  - All run **100% via JIT, zero fallback**, byte-exact vs interpreter.

- ✅ **RRR family completa:** `selb` (bit-select), `shufb` (byte permutation com 3 constant patterns + dynamic byte indexing), `fma`/`fnms`/`fms` (f32 multiply-add via fmul + fadd/fsub non-fused para matchar interpreter).
- ✅ **Byte immediate ops:** `andbi`/`orbi`/`xorbi` (broadcast splat + ALU op) + `ceqbi`/`cgtbi`/`clgtbi` (per-byte compares com pack via OR-shifts).
- ✅ **JIT cobertura total: ~102 opcodes.** Famílias broad cobertas: ALU word/halfword/byte, compares word/halfword/byte, shifts word/halfword RR e imm, multiplies word/halfword, float arith/compares/converts, branches (direct + indirect via R4a), branch hints, load/store qword (D-form e indexed), unary RR (clz/cntb/sign-ext/fsm/frest/frsqest), quadword byte shifts, RRR-form completa, byte immediate ops.
- ✅ Function-level pre-flight: rejeita funções com qualquer block terminando em indirect branch (`bi`/`bisl`/`iret`/`biz`/`binz`/`bihz`/`bihnz`) ou opcode unknown.
- ✅ `JitState` repr(C) com 128 GPRs × 4 lanes u32 + pc + stop_code.
- ✅ ABI: `extern "C" fn(*mut JitState) -> u32` retornando `JIT_OUTCOME_STOP`/`BAILOUT`/`STALL`.
- ✅ `RecompilerExecutor` JIT-first com fallback automático ao interpreter.
- ✅ `JitStats` tracking: `jit_runs`, `fallback_runs`, `compiled_functions`.
- ✅ **synthetic_loop.elf rodando ENTEIRO via JIT** (loop com brnz back-edge + stop). 1+2+...+10 = 55 verificado byte-a-byte vs interpreter, **zero fallback**.
- ✅ 20 lib tests + 4 spu-runner differential.
- 🔜 **Próxima expansão (R3 incremental):**
  - Indirect branches via target patching/dispatcher (com link register inspection)
  - Load/store qword (lqd/stqd/lqx/stqx) — necessita acesso ao LS via host pointer
  - Compares restantes (ceq/cgt RR + halfword/byte variants)
  - Shifts (shl/rot/rotm/rotma + immediate)
  - Float family (fa/fs/fm/fcgt/...) — usa `f32x4`

### Phase R4a — JIT Dispatcher + Indirect Branches (DONE — 2026-04-25)
- ✅ JIT outcomes redefinidos: STOP / CONTINUE_TO / UNKNOWN_OPCODE / STALL
- ✅ Codegen UncondIndirect (bi/iret/bisl com link write) e CondIndirect (biz/binz/bihz/bihnz com `brif`)
- ✅ Pre-flight aceita indirect terminators (era reject)
- ✅ Dispatcher loop em `RecompilerExecutor::execute`
- ✅ Function cache por `(entry_pc, ls_hash)` com cache_hits/misses tracking
- ✅ JitStats expandida: cache_hits, cache_misses, dispatcher_iterations
- ✅ synthetic_brsl_ret roda 100% via JIT zero-fallback (subroutine call/return byte-exact vs interpreter)
- ✅ 5 testes R4a + benchmark 1.40× speedup
- ✅ Self-loop bound via max_steps (test r4a_dispatcher_caps_iterations_to_max_steps)

### Phase R4b — Chained patching seguro (DONE — 2026-04-25)
- ✅ `ChainEntry { entry_fn, ls_hash, function_size }` em `JitCache`. Usa fn-pointer direto em vez de `*const CompiledFunction` — independente de rehash do `compiled` HashMap.
- ✅ `chain_lookup` enum `Hit / Stale / Miss`: dispatcher pula `compile_or_fetch` no Hit; Stale evict + fall through; Miss apenas fall through.
- ✅ `ls_hash` guard recomputado por iteração (mesma função `hash_ls_around` do path R4a) — proteção contra SMC ou aliasing pc cross-program.
- ✅ Chain table persiste entre `execute()` calls (vive no Mutex<JitCache>) — 2nd execução de programa idêntico = 100% chain hits.
- ✅ JitStats +5 fields: `chained_jumps`, `dispatcher_bypasses`, `patch_hits`, `patch_misses`, `invalid_chain_guards`. Invariante `dispatcher_iterations == patch_hits + patch_misses`.
- ✅ `clear_function_cache()` purga chain table junto com compiled.
- ✅ `chain_table_size()` exposto para tests.
- ✅ R4a tests atualizados: `cache_hits == 0` no 2nd run (chain bypassa global). `r4a_benchmark_brsl_ret_jit_vs_interpreter` agora valida `chained_jumps >= 408`.
- ✅ 12 novos testes R4b: 4 safety (refusal not compiled, refusal ls_hash change, chain on 2nd execution, clear_cache drops chain) + 4 equivalence (synthetic_loop/brsl_ret/fibonacci/sum_of_squares × 10 repeats byte-exato vs interpreter) + 4 benchmarks (synthetic_loop 1.44×, brsl_ret 1.43×, fibonacci 1.46×, sum_of_squares 1.36×).
- ✅ Reversibilidade total: zero correctness sacrificada — chain falha → path R4a inalterado.

### Phase R4c — SMC / cache invalidation seguro (DONE — 2026-04-25)
- ✅ Detecção proativa de self-modifying code: `smc_scan(ls)` corre no início de toda dispatcher iter, varrendo `compiled_meta`, recomputando `hash_ls_range(ls, code_start, code_end)` e invalidando entradas stale.
- ✅ `CompiledMeta { code_start, code_end, exact_hash, function_size }` por entrada compilada. `code_range_of(SpuFunction)` calcula `[min(block.start_pc), max(block.end_pc))` — cobre graphs gappy conservadoramente.
- ✅ Modelo escolhido: **per-entry detection com hash exato da range, sem full flush**. `smc_full_flushes` reservado para futuro (= 0 hoje).
- ✅ Invalidação atômica em 3 fases: snapshot stale keys (Phase 1) → drop de compiled+meta+chain matched (Phase 2) → drop decoded cache (Phase 3, separate Mutex). Chain só é evicted quando `chain[pc].entry_fn` casa com a fn invalidada (preserva chain entries apontando para compilação mais recente no mesmo pc).
- ✅ `hash_ls_range` separado de `hash_ls_around` — primeiro cobre range exato da função (captura SMC fora da janela 256-byte do segundo).
- ✅ JitStats +5 fields: `smc_invalidations`, `smc_chain_evictions`, `smc_full_flushes`, `smc_range_hits`, `smc_range_misses`.
- ✅ `clear_function_cache()` purga `compiled_meta` junto com compiled+chain.
- ✅ `compiled_meta_size()` exposto para tests.
- ✅ R4b tests atualizados: `recompiler_caches_decoded_functions` (cache size pós-3rd-run agora 1, não 2 — R4c evicts stale prog #1) + `r4b_chain_refuses_when_ls_hash_changes` (aceita tanto `invalid_chain_guards>=1` quanto `smc_chain_evictions>=1`).
- ✅ 14 novos testes R4c: SMC across executions, writes outside ranges, chain eviction com fn match, equivalence × 4 programas × 10 repeats, stats invariants, meta-size tracking, 4 benchmarks.
- ✅ Reversibilidade preservada: scan é puramente aditivo. Chain Stale (R4b) ainda funciona como segunda linha de defesa; SMC scan apenas chega primeiro com mais cobertura.
- ✅ Zero regressão de correctness; speedup vs interpreter mantém faixa R4b (1.40-1.58×).
- 🔜 R5+: generation counter (instrumentar store opcodes para bumpar generation; smc_scan só recompute hash quando generation diverge).
- 🔜 R5+: interpreter resume from JitState (UNKNOWN_OPCODE no meio de função → transferir state para interpreter sem full re-run).
- 🔜 R5+: IR-level patchpoint (Cranelift indirect call substituindo CONTINUE_TO); ganho marginal nos benchmarks atuais.

### Phase R3 — Coverage parity (estimate: 5–10 sessions)
- Extend Cranelift codegen to cover float lanes, indirect branches, channel ops, shifts, compares estendidos.
- Each new opcode group adds a unit test that runs both backends and asserts identity.

### Phase R4 — Constant prop + cache (estimate: 5–8 sessions)
- Port `reg_state_t` constant tracking from `SPUCommonRecompiler.cpp:2479+`.
- Add in-memory function cache.
- First benchmark vs interpreter (expect 5–20× on hot loops).

### Phase R5 — Optional: LLVM backend
- Re-evaluate. Only proceed if Cranelift output is the bottleneck.
- If proceeding: new crate `rpcs3-spu-recompiler-llvm` via `inkwell`. Same trait, slot-in alternative.

---

## What is **NOT** in scope (yet)

- ❌ Disk cache. Rebuild every launch initially.
- ⚠️ Patchpoint-based branch chaining (`make_branch_patchpoint`). **Substituído por R4b chain-table** no nível do dispatcher (HashMap<u32, ChainEntry> com ls_hash guard). IR-level patching ficaria como R5+ opcional caso bench mostre que chain-table ainda é gargalo.
- ✅ Self-modifying code detection. **Implementado em R4c** via `smc_scan` rodando no início de toda dispatcher iter, com hash exato por code-range. Detecta SMC em qualquer função compilada (não só a próxima target), invalida atomicamente `compiled` + `compiled_meta` + `chain` (quando casa) + decoded cache. R4b `ls_hash` guard mantido como segunda linha de defesa.
- ❌ `spu_fast` template-lambda backend port. Cranelift / LLVM make this unnecessary.
- ❌ MFC DMA path optimization. The DMA crate is independent and gets its own workstream.

---

## Risks & open questions

| Risk | Mitigation |
|---|---|
| **FPSCR not modeled** in interpreter — recompiler will need denormal flags, rounding mode, exception status. Diff harness can't catch divergences in state we don't snapshot. | Extend `SpuStateSnapshot` with `fpscr: u32` field (currently default-zeroed). Wire FPSCR updates into interpreter first; recompiler must match. **Block R1.** |
| **Channel state snapshot** — `ChannelCounts` in `SpuStateSnapshot` is currently default. Channels are observable (mailbox depths, signal pending bits). | Expose channel introspection from `SpuThread`; populate `channel_counts` in `snapshot_from_thread`. **Block R3.** |
| **Indirect branch target resolution** — JIT'd code has to call back into runtime to look up targets. Cost = indirect call per block exit. | Acceptable for v0; optimize via patchpoints in v1. |
| **Self-modifying SPU LS** — guest writes to instruction memory. C++ has invalidation logic. | Audit our fixtures: none self-modify. Document as a v1 issue with explicit test. |
| **Memory safety of generated code** — Cranelift output runs as `extern "C"` from Rust. Must establish ABI carefully. | Use Cranelift's `JITModule` standard pattern; treat the generated function as a single FFI surface. |
| **Performance regression** — recompiler exists but is *slower* than interpreter on cold start. | First benchmark in Phase R4. If so, investigate caching first; codegen second. |
| **LLVM backend on Windows** — `inkwell` requires LLVM 18+ in a discoverable location with the right CRT. | Defer LLVM to v2; revisit only if Cranelift output is unacceptable. |

---

## Critical fix during R1: opcode canonicalisation

While building the decoder we discovered three opcodes in the
interpreter were **double the canonical SPU ISA value** — internally
consistent (every test passed) but byte-incompatible with anything
produced by binutils, RPCS3 C++, or any external SPU toolchain:

| Opcode | Interpreter (pre-fix) | Canonical SPU | Status |
|---|---|---|---|
| `a rt, ra, rb` | `0x180` | `0xC0` | ✅ fixed |
| `and rt, ra, rb` | `0x181` | `0xC1` | ✅ fixed |
| `sf rt, ra, rb` | `0x080` | `0x40` | ✅ fixed |

Root cause: an early iter-1 commit shifted the opcode value left by 1
bit beyond the standard MSB-0 layout. Other opcodes (XOR, SHL, ROT,
…) were added later from the C++ source and used canonical values, so
the bug was localised to A/AND/SF.

Fix: changed `pack_rr(0x180, …)` → `pack_rr(0x0C0, …)` (and analogues)
in both encoder and dispatcher; regenerated all 4 fixtures that use
A/SF/AND. Workspace remained 100% green throughout.

**Implication:** the SPU port is now byte-compatible with canonical
ISA encodings. Future homebrew ELFs assembled with binutils will load
correctly. Phase R2.5 (Cranelift) and the eventual differential vs
RPCS3 C++ are now unblocked on this front.

## Required pre-conditions before starting Phase R1

These are the gates. Each must be ✅ before code starts.

- ✅ Interpreter has ≥50% of opcodes implemented (currently 118 tests, ~50%).
- ✅ Differential harness exists (`run_and_diff` in `rpcs3-spu-differential`).
- ✅ Multiple fixtures committed exercising distinct opcode families.
- ✅ Integration tests automatically catch divergences (8 fixtures × 13 tests).
- ⏳ **TODO before R3 (FPSCR):** extend `SpuStateSnapshot` with FPSCR field (today defaulted to 0). Without this, float divergences will silently slip past the diff. **Concrete action:** add `fpscr: u32` to `SpuStateSnapshot`; populate from `SpuThread.fpscr` once the latter exists in `rpcs3-spu-thread`. Decision deferred until first FP-heavy fixture demands it.
- ✅ **DONE 2026-04-25 (channel snapshot):** `snapshot_from_thread` now populates `channel_counts` directly from `SpuChannels` public fields (`in_mbox`, `out_mbox`, `out_intr_mbox`, `snr`). 2 new diff tests in `rpcs3-spu-differential` validate.

---

## How to start Phase R2.5 (Cranelift codegen — concrete next action)

R1 (decoder) and R2 (recompiler scaffold) are **DONE**. The remaining
gate is a **user decision** on the codegen backend (per D1).

If the user approves Cranelift:

1. Add `rust/rpcs3-spu-recompiler-cranelift/` (or upgrade
   `rust/rpcs3-spu-recompiler/` to add a `cranelift` feature).
2. Add `cranelift = "0.110"` (or current) and `cranelift-jit` deps.
   Both are pure-Rust crates published to crates.io; no system deps.
3. Implement `CraneliftBackend` that takes an `SpuFunction` (already
   decoded by `rpcs3-spu-decoder`) and emits IR for each block.
4. Inside `RecompilerExecutor::execute`, after `decode_with_cache`,
   try to compile every block. If every block compiles, run the JIT;
   otherwise fall back to the interpreter delegate (current path).
5. Existing 4 differential integration tests (`spu-runner --backend
   recompiler` byte-identical to `--backend interpreter`) act as the
   pass/fail gate. Add more as opcode coverage grows.

If the user prefers LLVM via `inkwell`:
- Setup is harder on Windows MSVC (LLVM 18+ install + linking).
- API is more familiar to RPCS3 maintainers (matches `SPULLVMRecompiler.cpp`).
- Defer until Cranelift output is benchmarked and shown insufficient.

---

## Reference: opcodes already implemented in interpreter (R0 baseline)

(See `rust/rpcs3-spu-interpreter/src/lib.rs` — `match bits(inst, …)` arms.)

**Done:** ALU word (a/sf/and/or/xor/nor/nand/eqv/andc/orc), shifts word (shl/rot/rotm/rotma + i7), compares (ceq/cgt/clgt + halfword/byte variants + immediate), float compare (fcgt/fcmgt/fceq/fcmeq), float arith (fa/fs/fm/fma/fnms/fms), reciprocal estimates (frest/frsqest naïve), branches direct (br/bra/brnz/brz) + indirect (bi/bisl/iret/biz/binz/bihz/bihnz/brsl) + hints (hbr/hbra/hbrr), load/store (lqd/stqd/lqx/stqx), immediate (il/ilh/ilhu/iohl/ila/andi/ori/xori/ai/ceqi/cgti/ceqhi/cgthi/clgthi), shuffle (selb/shufb/rotqbyi/shlqbyi), sign-extend (xsbh/xshw/xswd), channel (rdch/wrch/rchcnt), barriers (sync/dsync/stopd), halfword arith (ah/sfh), carry/borrow (cg/bg), or-across (orx), form-mask (fsm), convert (cflts/cfltu/csflt/cuflt).

**Outstanding (R1 should NOT block on these — fallback to interpreter is fine):** halfword shifts (roth/rothi/rothm/rothmi/rotmah/rotmahi/shlh/shlhi), more multiplies (mpyh/mpyhh/mpyhhu/mpys), full quadword bit shifts (rotqbi/shlqbi/etc.), gather bits (gb/gbh/gbb), MFC commands proper, mfspr/mtspr, hgt/hlgt/heq traps, double-precision family (df*), bisled (event check).
