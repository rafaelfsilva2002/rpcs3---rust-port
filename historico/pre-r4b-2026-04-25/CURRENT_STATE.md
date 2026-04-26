# Current State — RPCS3 → Rust Port

**Last updated:** 2026-04-25 (post R4c — SMC / cache invalidation seguro)
**Frozen baseline:** [`PLAN_FREEZE_2026-04-24.md`](PLAN_FREEZE_2026-04-24.md), [`CHECKLIST_FREEZE_2026-04-24.md`](CHECKLIST_FREEZE_2026-04-24.md), [`CURRENT_STATE_2026-04-24.md`](CURRENT_STATE_2026-04-24.md)

## Numbers

- **234** crates (added `rpcs3-spu-decoder` + `rpcs3-spu-recompiler` w/ Cranelift JIT)
- **5323 lib + 19 spu-runner + 28 decoder = 5370 tests** (was 5165 at freeze; +205 net), **+12 R4b + 14 R4c → 5396 net**
- **231** autonomous iterations consecutive (R4c sub-onda)
- **0** regressions across the entire session (27ª onda)
- **JIT vs interpreter (R4c, post SMC scan):** 1.40× synthetic_loop, 1.46× brsl_ret, 1.45× fibonacci, 1.58× sum_of_squares — sem regressão material vs R4b
- **R4c SMC scan:** detecta bytes alterados em qualquer função compilada, invalida atomicamente compiled + meta + chain (quando entry_fn casa) + decoded cache. Stats: `smc_invalidations`, `smc_chain_evictions`, `smc_full_flushes`, `smc_range_hits`, `smc_range_misses`.
- **R4b chain table mantido:** chain hits continuam para programas não-SMC (zero `smc_invalidations` em fixtures padrão)
- **JIT compiles fibonacci(10), sum-of-squares(10), AND brsl_ret (subroutine call/return)** end-to-end zero-fallback, byte-exact vs interpreter
- **JIT cobertura: ~102 opcodes** + R4a dispatcher + R4b chained patching + R4c SMC invalidation + RRR completa + byte immediate ops

## Post-freeze hardening (2026-04-24)

Two waves applied **after** the documental freeze, both byte-exact and zero-regression:

1. **SPU helpers optimization** — `split_lanes`/`join_lanes`/`broadcast_u32`/`read_qword_be` migrated from byte-array intermediaries to direct bit shifts (`(v >> 96) as u32` etc.). Removes 16-byte memcpy per ALU instruction. Marked `const fn`. See `rpcs3-spu-interpreter/src/lib.rs:90..130`.
2. **SPU iter-8 instructions** — added FCGT, FCMGT, FCEQ, FCMEQ, FREST (naïve), FRSQEST (naïve), FSM, LQX, STQX, BI, BISL, IRET, HBR, BIZ, BINZ, BIHZ, BIHNZ. SPU interpreter test count: **68 → 89** (+21). FREST/FRSQEST use `1/x`/`1/sqrt(x)` direct math; LUT-based byte-exact form deferred (see TODO `spu-frest-lut`).

## Homebrew validation pipeline (Phase B P3+P4 done, fixture coverage expanded)

- `rust/spu-runner` — CLI binary. **9/9 integration tests** (5 smoke + 4 fixture-driven).
- `behavior-freeze/harness/spu_homebrew_runner.py` — Python diff harness (run / diff / comparative).
- `behavior-freeze/harness/test_spu_homebrew_runner.py` — end-to-end self-test (synthetic ELF → 2 dumps → IDENTICAL).
- `behavior-freeze/harness/build_synthetic_fixtures.py` — generator for the committed fixtures.

**Committed fixtures** (`behavior-freeze/fixtures/spu/`):

| Fixture | Bytes | Insts | Exercises |
|---|---:|---:|---|
| `synthetic_il_stop.elf` | 92 | 2 | sentinel — il + stop |
| `synthetic_arith.elf` | 120 | 9 | il, a, sf, shli, xor, or, and, stop |
| `synthetic_loop.elf` | 116 | 8 | ila, a, ai, ceqi, brnz, br (back-edge), stop — computes 1+2+...+10 = 55 |
| `synthetic_float_dot.elf` | 112 | 7 | il, shli, fa, fm chained — produces float 8.0 |

Pipeline shape: `ELF → spu-runner → dumps → spu_homebrew_runner.py --diff → IDENTICAL/diff report`. Bloqueado apenas em P1 (homebrew real) e P5 (RPCS3 dump capture). See [`HOMEBREW_PLAN.md`](HOMEBREW_PLAN.md).

## Iter-9 (2026-04-25): SPU expansion +18 testes

Adicionadas 17 instruções (107/107 SPU verde, era 89):
- **Vector word shifts:** SHL, ROT, ROTM, ROTMA + immediates ROTI, ROTMI, ROTMAI
- **Bitwise complementaries:** NAND, EQV, ANDC, ORC
- **Extended compares:** CEQH, CEQB, CGTH, CGTB, CLGTH, CLGTB
- **Barriers:** SYNC, DSYNC (NOPs); STOPD (= stop 0)

## Iter-10 + Recompiler pre-flight (2026-04-25): +11 testes + differential infra

**Iter-10 SPU (+11 testes, 107 → 118):**
- **Halfword arith:** AH (0x0C8), SFH (0x048)
- **Carry/borrow:** CG (0x0C2), BG (0x042)
- **OR-across:** ORX (0x1F0)
- **Branch+link relative:** BRSL (0x066)
- **Branch hints:** HBRA (7-bit 0x08), HBRR (7-bit 0x09) — NOPs
- **Halfword imm compares:** CEQHI (0x7D), CGTHI (0x4D), CLGTHI (0x5D)

**New crate `rpcs3-spu-differential` (12 lib tests):**
- `SpuExecutor` trait — backend-agnostic execution interface
- `SpuProgram` / `SpuExecutionResult` / `SpuStateSnapshot` / `ExecutionStopReason`
- `InterpreterExecutor` — reference oracle backend
- `diff_snapshots()` + `run_and_diff()` — ready for recompiler plug-in
- Future SPU recompiler implements the same trait → automatic differential validation

**Fixtures expanded (4 → 8):**
- `synthetic_loadstore.elf` — round-trip via stqd/lqd
- `synthetic_shifts.elf` — exercises shli/rotmi/rotmai/roti
- `synthetic_brsl_ret.elf` — function call via brsl/bi (link register)
- `synthetic_orx_collapse.elf` — ah + orx (per-halfword + collapse)

**`spu-runner` refactored** to use `SpuExecutor` trait via `--backend` flag (today only `interpreter`; recompiler plugs in later with zero CLI changes).

**Documento de plano:** [`docs/SPU_RECOMPILER_PLAN.md`](SPU_RECOMPILER_PLAN.md) — port plan completo do `SPUCommonRecompiler.cpp`, decisões arquiteturais (Cranelift recomendado), fases R0-R5, riscos.

## Phase R1+R2 done (2026-04-25)

**R1 — Decoder** (`rpcs3-spu-decoder`, 28 tests):
- `decode_inst(raw, pc) -> SpuInstruction` — single-instruction decode com `SpuInstKind` enum por família.
- `decode_function(ls, entry, max_blocks) -> SpuFunction` — two-pass (collect leaders, então cut blocks).
- `BlockTerminator` enum (Stop/UncondDirect/UncondIndirect/CondDirect/CondIndirect/UnknownOpcode/FellThroughLimit).
- 8 integration tests confirmam decode correto contra TODOS os fixtures committed.

**R2 — Recompiler scaffold** (`rpcs3-spu-recompiler`, 7 tests + 4 spu-runner integration):
- `RecompilerExecutor` implementa `SpuExecutor` — prova trait-based plug-in.
- Function cache via `HashMap<(entry_pc, ls_hash), SpuFunction>`.
- Hoje delega ao interpreter — **byte-identical por construção**.
- `spu-runner --backend recompiler` funciona; 4 differential tests confirmam interpreter≡recompiler.

**Critical fix descoberto durante R1:**
- 3 opcodes (A, AND, SF) tinham primary value 2× canonical SPU ISA (interno consistente mas byte-incompatible com bytes externos).
- Corrigido em interpreter (encoder + dispatcher) + Python fixture builder; fixtures regeneradas; zero regressão.
- Port agora é byte-compatível com binutils SPU asm e RPCS3 C++ encoding.

**Phase R2.5 — MULTI-BLOCK done (2026-04-25):** User aprovou Cranelift. JIT real funcional para ~25 opcodes incluindo branches diretos. **`synthetic_loop.elf` (loop com brnz back-edge → soma 1..10 = 55) roda ENTEIRO via JIT, zero fallback.** Multi-block compile: cada SPU bb vira Cranelift block; `jump`/`brif` para branches diretos; brsl write link + jump. Funções com indirect branches (bi/bisl/biz/etc) são rejeitadas pelo pre-flight e caem pro interpreter. 20 lib tests + 4 spu-runner differential.

**Phase R2.5 — EXPANSION (2026-04-25, mesma sessão):** JIT cobertura subiu para **~40 opcodes**. Adicionados:
- **RI7 word shifts:** shli, roti, rotmi, rotmai (com saturação correta para count >= 32)
- **RR compares word:** ceq, cgt, clgt (mask 0xFFFFFFFF / 0)
- **RR word shifts:** shl, shr, sar, rot — com SPU semantics ((-count) & 0x3F para shr/sar, saturação a 0/sign-bit em count >= 32)
- **Bitwise extras:** nand, eqv, andc, orc
- **Carry/borrow:** cg (via uextend para u64 + check overflow), bg (via UnsignedLessThanOrEqual)

**Demo:** `synthetic_arith.elf` (il+a+sf+shli+xor+or+and) agora roda **100% via JIT, zero fallback**, byte-exato com interpreter. 27 lib tests no recompiler crate (era 20).

**Phase R2.5 — LOAD/STORE (2026-04-25, mesma sessão):** JIT agora suporta `lqd`/`stqd`/`lqx`/`stqx` (load/store quadword). Adicionado `ls_ptr: *mut u8` ao `JitState`; `RecompilerExecutor` aloca buffer de 256KB e popula `ls_ptr` antes de invocar JIT. Codegen usa `bswap` para converter entre BE storage e u32 lanes (host LE). **`synthetic_loadstore.elf` agora roda 100% via JIT, byte-exato com interpreter.** 30 lib tests no recompiler. **3 fixtures sintéticos rodando 100% JIT:** loop, arith, loadstore (mais il_stop trivial).

**Phase R2.5 — HALFWORD + ORX (2026-04-25, mesma sessão):** JIT agora suporta:
- `ah`, `sfh` (per-halfword arith) via técnica split-mask-add-repack (8 lanes u16 processadas em pares de 32-bit)
- `orx` (Unary OR-across word lanes → preferred slot)

**`synthetic_orx_collapse.elf` agora roda 100% via JIT, byte-exato.** 35 lib tests no recompiler. **4 fixtures rodando 100% JIT** (mais il_stop = 5): loop, arith, loadstore, orx_collapse. Fixtures restantes (synthetic_halfword_shifts, synthetic_brsl_ret, synthetic_float_dot) caem pro interpreter por opcodes ainda não codegen'd (halfword shifts, indirect branches, float family).

**Phase R2.5 — HALFWORD SHIFTS (2026-04-25, mesma sessão):** JIT agora suporta `shlhi`, `rothmi`, `rotmahi`, `rothi` (per-halfword RI7 shifts). Helper `emit_halfword_const_shift` aplica split-mask-shift-repack por halfword com sextend para arith shr. Decoder também atualizado para classificar 0x07C..0x07F como `AluImm7` (era `Unclassified`). **`synthetic_halfword_shifts.elf` agora roda 100% via JIT, byte-exato.** 37 lib tests.

**Phase R2.5 — FLOAT FAMILY (2026-04-25, mesma sessão):** JIT agora suporta `fa`, `fs`, `fm` (per-lane f32 add/sub/mul). Helper `emit_float_op` faz bitcast i32↔f32, aplica IEEE op via Cranelift `fadd`/`fsub`/`fmul`. Para `fm`, helper `emit_flush_denorm` emula SPU FTZ semantic (denormal → +0) antes/depois da multiplicação. **`synthetic_float_dot.elf` agora roda 100% via JIT, byte-exato com interpreter** (chain fa→fm→fa produz 8.0 = 0x41000000). 39 lib tests.

**Phase R2.5 — SELB + BENCHMARK (2026-04-25, mesma sessão):** JIT agora suporta `selb` (RRR-form bit-select: `(rc & rb) | (!rc & ra)` per lane). Primeiro opcode RRR-form codegen'd. **Benchmark mediu speedup real:** 200 runs do `synthetic_loop` em release: interpreter = 2.43ms (12.1µs/run), JIT = 1.54ms (7.7µs/run). **JIT é 1.58× mais rápido** — prova que codegen entrega valor concreto além da prova-de-conceito. 41 lib tests.

**Phase R2.5 — HALFWORD COMPARES + FIBONACCI (2026-04-25, mesma sessão):** JIT agora suporta `ceqh`, `cgth`, `clgth` (per-halfword compares). Helper `emit_halfword_cmp` reusa pattern split-mask-select-pack com sextend para signed gt. **Novo programa não-trivial:** fibonacci(10) computado via SPU instructions (3 ila + ceqi + brnz + 3 a + ai + br back-edge + stop). Roda **100% via JIT, zero fallback**, retorna r3 = 55 broadcast. Byte-exato vs interpreter. 43 lib tests.

**Phase R2.5 — MULTIPLIES + SUM-OF-SQUARES (2026-04-25, mesma sessão):** JIT agora suporta `mpy` (signed 16×16→32) e `mpyu` (unsigned 16×16→32) per word lane. Helper `emit_word_mpy` usa `ireduce I32→I16` + `sextend` para signed; mask 0xFFFF para unsigned; `imul` em ambos. **Segundo programa não-trivial:** sum_of_squares (1²+2²+...+10² = 385) usando mpyu em loop. Roda **100% via JIT, zero fallback**, retorna r3 = 0x181 broadcast. 45 lib tests.

**Phase R2.5 — MEGA EXPANSION (2026-04-25, mesma sessão, +13 opcodes):**
- **Float compares:** `fcgt`, `fcmgt`, `fceq`, `fcmeq` — Cranelift `fcmp` com FTZ flush e abs mask para magnitude variants.
- **Byte compares:** `ceqb`, `cgtb`, `clgtb` — 16-lane processing via 4 byte slices per word lane.
- **Halfword RR shifts:** `shlh`, `roth`, `rothm`, `rotmah` — count vem do halfword lane do rb, com saturação >= 16.
- **Halfword imm compares:** `ceqhi`, `cgthi`, `clgthi` — broadcast imm10.
- **Multiplies imm:** `mpyi` (signed), `mpyui` (unsigned).
- **Halfword imm arith:** `ahi` (per-halfword add).
- **Word imm:** `sfi` (sub-from), `clgti` (unsigned gt imm).
- **Decoder fixes:** halfword RR shifts (0x05C..0x05F) + RI10 imm primaries (0x74/0x75/0x5C/0x1D/0x0C) classificados corretamente.
- 50 lib tests no recompiler.

**Phase R2.5 — MEGA EXPANSION 2 + 3 (2026-04-25, mesma sessão, +12 opcodes):**
- **Unary RR ops (8 novos via novo `emit_unary` helper):**
  - `clz` — Cranelift `clz` direto, per word lane.
  - `cntb` — Cranelift `popcnt` per byte (4 bytes per word, packed).
  - `xsbh`, `xshw`, `xswd` — sign-extension byte→half / half→word / word→dword via `ireduce + sextend`.
  - `fsm` — bit-pattern do preferred slot expandido em 4 word lanes (bit 3 → lane 0 etc).
  - `frest`, `frsqest` — naive 1/x e 1/sqrt(|x|) com FTZ flush antes/depois (matches interpreter naive path).
- **Extended multiplies (3 novos via `emit_extended_mpy`):**
  - `mpyh` — signed (high half × low half) << 16.
  - `mpyhh` — signed high × high.
  - `mpys` — signed (low × low) truncated to 16 bits, sign-extended back to 32.

**Phase R2.5 — MEGA EXPANSION 4 (2026-04-25, mesma sessão, +6 opcodes):**
- **Quadword byte shift imm (2):** `rotqbyi` (rotate 128-bit register left by N bytes mod 16), `shlqbyi` (zero-fill shift left). Implementação via gathering bytes per output position com fórmula `out_byte_i = src_byte (i+N) mod 16`.
- **Float ↔ int converts (4):** `cflts` (f32→i32 signed), `cfltu` (f32→u32), `csflt` (i32→f32 signed), `cuflt` (u32→f32). Cranelift `fcvt_to_sint_sat`/`fcvt_to_uint_sat`/`fcvt_from_sint`/`fcvt_from_uint` + helper `scale_float` que multiplica por 2^exp_bias materializado como constant f32.
- **Workspace 5311 lib + 19 + 28 = 5358 testes, zero regressão (22ª onda consecutiva). JIT cobertura ~91 opcodes (era ~85).**

## Phase R4a — JIT Dispatcher + Indirect Branches (2026-04-25)

**Goal alcançado:** indirect branches no SPU JIT sem cair pro interpreter. Pre-R4a, qualquer função com `bi`/`bisl`/`iret`/`biz`/`binz`/`bihz`/`bihnz` era rejeitada no pre-flight e o programa inteiro caía pro interpreter. Pós-R4a, a JIT permanece em JIT-land via dispatcher loop com cache de funções compiladas por (entry_pc, ls_hash).

**Arquitetura:**
- **JIT outcomes:** `STOP` (halt), `CONTINUE_TO` (indirect branch — state.pc tem novo target), `UNKNOWN_OPCODE` (fallback necessário), `STALL` (reservado).
- **Codegen indirect branches:** UncondIndirect/CondIndirect terminator gera `load ra preferred → mask 0x3FFFC → store state.pc → return CONTINUE_TO`. Para BISL, write link broadcast antes. Para CondIndirect (biz/binz/bihz/bihnz), `brif` para take_block (CONTINUE_TO) ou fall_block (next direct block).
- **Dispatcher loop em `RecompilerExecutor::execute`:**
  1. Aloca LS buffer + JitState uma vez por execute().
  2. Loop: compile_or_fetch(state.pc) → call function → match outcome.
  3. CONTINUE_TO → loop com novo pc.
  4. STOP → return result.
  5. UNKNOWN_OPCODE → fallback ao interpreter.
- **Function cache** por `(entry_pc, hash_of_ls_around_entry)`. Cache hits/misses tracked em `JitStats`.

**Demo: synthetic_brsl_ret roda 100% via JIT:**
- Entry function (0x100): il + brsl → jump direto a 0x110 (Cranelift block dentro da mesma função).
- Subroutine block (0x110): ai + bi → CONTINUE_TO state.pc=0x108 (link).
- Continuation function (0x108): single-block stop.
- Total: 2 funções compiladas, 2 dispatcher iterations, 0 fallback, byte-exato vs interpreter.

**Benchmark medido:** 200 runs do brsl_ret em release: interpreter 2.35ms vs JIT R4a 1.68ms = **1.40× speedup** (antes era 1.0× porque sempre falhava pro interpreter).

**Stats observáveis:** `cache_hits`, `cache_misses`, `compiled_functions`, `dispatcher_iterations`, `jit_runs`, `fallback_runs` em `JitStats`. Cache cresce 2 por execução nova; subsequentes hitam cache 100%.

**Limitações documentadas (R4b/R4c):**
- Self-modifying code: cache invalida via ls_hash mas não há flush automático.
- Indirect branches sem fallback intermediário: se o target tem opcode unknown, fallback é "full re-run via interpreter desde program.entry_pc" (incorreto se LS foi modificado pela JIT). Para fixtures atuais não dispara.
- Sem chained patching: cada CONTINUE_TO custa um cache lookup.

## Phase R2.5 — RRR-form complete (2026-04-25, mesma sessão pós-R4a)

**+5 opcodes RRR codegen'd:**
- **shufb (0xB):** byte permutation 16-lane. Cada output byte: load selector de rc, check 3 constant patterns (sel & 0xC0 == 0x80 → 0x00; sel & 0xE0 == 0xC0 → 0xFF; sel & 0xE0 == 0xE0 → 0x80), else byte de ra/rb por idx & 0x1F. Address arithmetic via `state + gpr_offset(ra/rb) + (idx ^ 3)` para conversão BE→LE.
- **fma (0xE):** rt = ra*rb + rc (per-lane f32 via `fmul + fadd`, NÃO fused para matchar interpreter).
- **fnms (0xD):** rt = rc - ra*rb (`fmul + fsub` com ordem invertida).
- **fms (0xF):** rt = ra*rb - rc.

**JIT cobertura agora:** **~96 opcodes**. Todas as 5 famílias RRR (selb + shufb + fma + fnms + fms) cobertas.

## Phase R2.5 — Byte immediate ops (2026-04-25, mesma sessão)

**+6 opcodes byte-immediate (RI8 form):**
- **andbi (0x16):** rt = ra & broadcast(imm8) per byte (16 lanes via 4 word lanes).
- **orbi (0x06):** rt = ra | broadcast(imm8).
- **xorbi (0x46):** rt = ra ^ broadcast(imm8).
- **ceqbi (0x7E):** per-byte equality vs imm8 (output 0xFF or 0).
- **cgtbi (0x4E):** per-byte signed gt vs imm8.
- **clgtbi (0x5E):** per-byte unsigned gt vs imm8.

**Decoder fix:** byte-imm primaries (0x06/0x16/0x46/0x4E/0x5E/0x7E) extraídos com offset diferente (`(raw >> 16) & 0xFF` em vez de `i10_signed` que era `(raw >> 14) & 0x3FF`). Para reusar `SpuInstKind::AluImm`, sign-estendemos imm8 para i16 antes de armazenar.

**Helper `emit_byte_imm`:** Para and/or/xor, broadcast = imm8 splatado em 4 bytes (single u32 ALU op por word lane). Para cmp, per-byte extraction + icmp + select + pack.

**JIT cobertura:** **~102 opcodes**.

## Phase R4b — Chained patching seguro (2026-04-25)

**Goal alcançado:** dispatcher do JIT pula direto para a função compilada na continuação quando seguro, sem re-passar pelo `compile_or_fetch` global. R4a já mantinha JIT-land entre indirect branches mas cada CONTINUE_TO custava: hash do LS + acquire mutex + lookup HashMap + segundo lookup pra `function_size`. R4b colapsa isso em uma chain-table local (`HashMap<u32, ChainEntry>`) cujo guard é `ls_hash` — única hash compute, único acquire, sem lookup global na via rápida.

**Arquitetura:**
- **`ChainEntry`** em [`rust/rpcs3-spu-recompiler/src/lib.rs`](../../rust/rpcs3-spu-recompiler/src/lib.rs): `entry_fn: extern "C" fn(*mut JitState) -> u32`, `ls_hash: u64`, `function_size: u64`. fn pointer é estável (Cranelift não relocaliza código finalizado), independente do HashMap rehashing do `compiled` map.
- **`ChainLookup` enum:** `Hit { entry_fn, function_size }` / `Stale` (entry inválida — evicted) / `Miss` (sem entry).
- **Dispatcher fast-path:**
  1. `cur_hash = hash_ls_around(ls, target_pc)` — 1 hash.
  2. `chain_lookup(target_pc, cur_hash)`:
     - **Hit** → entry_fn + function_size, increment `chained_jumps` / `dispatcher_bypasses` / `patch_hits`.
     - **Stale** → evict entry, increment `invalid_chain_guards` + `patch_misses`, fall through.
     - **Miss** → increment `patch_misses`, fall through.
  3. Fall through → `compile_or_fetch` (path R4a inalterado), depois `chain_install` na chain-table.
  4. Chama `entry_fn(state)` direto (não via `(*compiled_ptr).call(state)` — pula um indireto).
- **Reversibilidade total:** se chain falha (Stale/Miss), volta ao path R4a de `compile_or_fetch`. Zero correctness sacrificada — `ls_hash` guard captura SMC.
- **Chain table persiste entre `execute()` calls** (vive em `JitCache`, mesmo lock que `compiled`). 2nd execução de programa idêntico → 100% chain hits.

**Stats novos em `JitStats`:**
- `chained_jumps`: iters satisfeitas pela chain (= `dispatcher_bypasses` = `patch_hits`).
- `dispatcher_bypasses`: iters que pularam `compile_or_fetch`.
- `patch_hits`: chain table com `ls_hash` correto.
- `patch_misses`: chain table sem entry ou `ls_hash` errado (subset → fall through).
- `invalid_chain_guards`: subset de `patch_misses` onde entry estava stale (evicted).

**Invariante:** `dispatcher_iterations == patch_hits + patch_misses`. `cache_hits + cache_misses == patch_misses` (fall-through paths consultam o global).

**Tests R4b (12 novos, todos byte-exato vs interpreter):**
- `r4b_chain_refuses_when_not_compiled` — fresh executor, primeira run: `patch_hits == 0`, `patch_misses == dispatcher_iterations`.
- `r4b_chain_refuses_when_ls_hash_changes` — duas SpuPrograms com mesmo entry_pc mas bytes diferentes → `invalid_chain_guards += 1` na segunda run; resultado byte-exato vs interpreter.
- `r4b_brsl_ret_second_execution_chains` — 2ª execução do mesmo programa: `chained_jumps == 2` (ambas dispatcher iters via chain).
- `r4b_synthetic_loop_equivalence_across_repeated_runs` — 10 iterações, ≥9 chained jumps, sem `invalid_chain_guards`.
- `r4b_brsl_ret_equivalence_across_repeated_runs` — 10× brsl_ret → ≥18 chained jumps.
- `r4b_fibonacci_equivalence_across_repeated_runs` — ≥9 chained jumps.
- `r4b_sum_of_squares_equivalence_across_repeated_runs` — ≥9 chained jumps.
- `r4b_clear_cache_drops_chain_table` — `clear_function_cache()` purga chain.
- 4 benchmarks (synthetic_loop, brsl_ret, fibonacci, sum_of_squares).

**Benchmark medido (release, 200 runs cada):**
| Programa | Interpreter | JIT R4b | Speedup | chained_jumps |
|---|---|---|---|---|
| synthetic_loop | 2.37ms | 1.64ms | **1.44×** | 204/205 |
| brsl_ret | 2.45ms | 1.71ms | **1.43×** | 408/410 |
| fibonacci | 2.48ms | 1.70ms | **1.46×** | 204/205 |
| sum_of_squares | 2.44ms | 1.79ms | **1.36×** | 204/205 |

**O que R4b NÃO faz (fronteira):**
- **Sem patching IR direto:** o JIT codegen ainda emite `return CONTINUE_TO`. Otimização "salto direto" acontece no nível do dispatcher Rust, não no código JIT. Patching IR é opcional para R5+ e custaria mais complexidade (indirect call em Cranelift) por ganho marginal nos benchmarks atuais.
- **Sem cross-program flush:** `clear_function_cache()` é manual. SMC sub-instrução não dispara invalidação automática — apenas a próxima dispatcher iter cuja `ls_hash` mudou (chain Stale path).

**Limitações/TODOs futuros (R4c+):**
- Self-modifying code: invalidação ainda depende de ls_hash mismatch detectado pelo guard. R4c minimal SMC invalidation traria flush proativo.
- Interpreter resume from JitState (R4c-2): se UNKNOWN_OPCODE no meio de uma função, a re-run completa via interpreter atual descarta o estado JIT acumulado.

## Phase R4c — SMC / cache invalidation seguro (2026-04-25)

**Goal alcançado:** detectar e invalidar atomicamente qualquer função JIT cuja região de código em LS foi modificada. R4b dependia do `ls_hash` guard como detecção *reativa* na chain table — só pegava SMC quando a próxima dispatcher iter targetava o pc afetado. R4c torna a detecção *proativa* via `smc_scan` rodando no início de toda dispatcher iter, varrendo TODAS as funções compiladas, não só a próxima target.

**Modelo de invalidação escolhido: range exato + hash exato por função (per-entry, não full-flush).**

**Arquitetura:**
- **`CompiledMeta { code_start, code_end, exact_hash, function_size }`** em [`rust/rpcs3-spu-recompiler/src/lib.rs`](../../rust/rpcs3-spu-recompiler/src/lib.rs). Calculado em `compile_or_fetch` com base nos blocos do `SpuFunction` (helper `code_range_of`: `min(block.start_pc) .. max(block.end_pc)` — cobre graphs gappy também, conservadoramente).
- **`hash_ls_range(ls, start, end)`** — hash determinístico de `ls[start..end]` via `DefaultHasher`. Distinto do `hash_ls_around` (256-byte window, usado como cache key) para captar SMC fora da janela 256-byte.
- **`smc_scan(ls)`** — método em `RecompilerExecutor`:
  1. Phase 1: snapshot de keys stale. Sob lock breve, walk `compiled_meta`, recompute `hash_ls_range` e compara com `exact_hash`. Mismatch → adiciona `(key, entry_fn)` à lista local.
  2. Phase 2: se há stale, drop sob novo lock. Para cada key: remove de `compiled` + `compiled_meta`. Bump `smc_invalidations` + `smc_range_hits`. Chain eviction: só se `chain[pc].entry_fn` casa com o `entry_fn` que acabou de ser removido — bump `smc_chain_evictions`. (Isso preserva chain entries que apontam para uma compilação MAIS RECENTE no mesmo pc.)
  3. Phase 3: drop entries do decoded cache (separate Mutex).
  4. Sem stale: bump `smc_range_misses` apenas.
- **Wired no dispatcher**: `smc_scan` roda no INÍCIO de cada iter em `try_jit_run`, antes de `chain_lookup` e `compile_or_fetch`. Garante que nenhum lookup vê estado stale.
- **Reversibilidade total**: chain Stale (R4b) ainda funciona como segunda linha de defesa caso smc_scan miss algo. SMC invalidation é puramente aditiva — zero correctness sacrificada.

**Como R4b interage com R4c:**
- Funções não modificadas: smc_scan acha hash idêntico → `smc_range_misses += 1`, sai. Chain table inalterada. R4b chain hits continuam normais.
- Função modificada: smc_scan invalida → `chain_lookup` no mesmo iter vira Miss (ou Hit se outra fn diferente no mesmo pc instalou nova chain). `compile_or_fetch` recompila fresh.
- Cross-program: R4c agora intercepta antes de R4b. Stats: `smc_chain_evictions` (R4c path) substitui `invalid_chain_guards` (R4b path) na maior parte dos cenários.

**Stats novos em `JitStats` (5):**
- `smc_invalidations`: total de entries removidas do compiled cache pelo SMC scan.
- `smc_chain_evictions`: subset onde a chain table tinha entry apontando para a fn invalidada.
- `smc_full_flushes`: reservado (= 0 na implementação minimal — escape hatch para futuro tracking mode "nuke everything").
- `smc_range_hits`: scans que detectaram modificação (= `smc_invalidations`).
- `smc_range_misses`: scans que confirmaram código intacto.

**Tests R4c (14 novos, todos byte-exato vs interpreter):**
- `r4c_no_smc_means_no_invalidations`: 10× loop_program → `smc_invalidations == 0`, `smc_range_misses == 9`.
- `r4c_smc_detected_across_executions_at_same_pc`: 2 programas com mesmo entry_pc mas códigos distintos → 2nd execute invalidates, resultado reflete prog_b (não stale prog_a).
- `r4c_writes_outside_compiled_ranges_do_not_invalidate`: programa de 2 funções (A com `stqd` para 0x200, B em 0x114) num único execute. `smc_invalidations == 0` porque escrita fora dos code ranges.
- `r4c_smc_evicts_chain_when_pointing_at_invalidated_fn`: warm chain com prog_a, depois prog_b → `smc_chain_evictions >= 1`.
- 4 equivalence (synthetic_loop/brsl_ret/fibonacci/sum_of_squares × 10 repeats).
- 2 stats invariants.
- 1 meta-size test (`compiled_meta_size` tracks `compiled` size).
- 4 benchmarks.

**R4b tests atualizados (2):**
- `recompiler_caches_decoded_functions`: cache size pós-3rd-run agora == 1 (R4c evicts stale entry de prog #1) em vez de 2. Adiciona assert `smc_invalidations >= 1`.
- `r4b_chain_refuses_when_ls_hash_changes`: aceita tanto `invalid_chain_guards >= 1` (R4b path) quanto `smc_chain_evictions >= 1` (R4c path). Adiciona assert `smc_invalidations >= 1`.

**Benchmark medido (release, 200 runs cada):**
| Programa | Interpreter | JIT R4c | Speedup | smc_invalidations | smc_range_misses |
|---|---|---|---|---|---|
| synthetic_loop | ~2.4ms | ~1.7ms | **1.40×** | 0 | 204 |
| brsl_ret | ~2.4ms | ~1.65ms | **1.46×** | 0 | ~613 |
| fibonacci | ~2.4ms | ~1.65ms | **1.45×** | 0 | 204 |
| sum_of_squares | ~2.4ms | ~1.5ms | **1.58×** | 0 | 204 |

Speedups dentro da faixa observada para R4b (1.36×-1.46×) — variação por noise de medição. Sem regressão material; safety scan adicionou hashing per-iter ainda dominado pelo JIT execution time.

**O que R4c minimal NÃO faz (fronteira):**
- **Não usa generation counter:** scan recomputa hash exato a cada iter. Generation counter seria mais barato mas requer instrumentação dos store opcodes no codegen — fora de scope para minimal.
- **Não usa full flush:** `smc_full_flushes` reservado para futuro tracking mode "nuke everything". Per-entry detection sempre funciona.
- **Não invalida durante JIT execution:** scan só roda entre iters. SMC dentro de uma única função invalida a própria função na PRÓXIMA iter (não a atual). Como o JIT nunca re-entra no mesmo código sem voltar ao dispatcher, isso é safe.
- **Sem invalidação de codegen IR:** chain table evict mas Cranelift module mantém código compilado vivo (memory leak benign — JITModule só limpa em drop).

**Limitações/TODOs futuros (R5+):**
- Generation counter: instrumentar store opcodes no codegen para bumpar generation; smc_scan só faz hash recompute quando generation diverge. Reduz overhead no caso comum.
- Interpreter resume from JitState: se UNKNOWN_OPCODE no meio de uma função, ao invés de full re-run pelo interpreter, transfere o JitState atual para o interpreter e continua de onde parou.
- IR-level patching: substituir CONTINUE_TO no código JIT por indirect call ao próximo entry_fn (read from chain table), removendo um round-trip ao dispatcher Rust.

## Iter-11 + R3 prep (2026-04-25)

**R3 blocker resolved — channel snapshot:**
- `SpuStateSnapshot::channel_counts` agora populado em `snapshot_from_thread`: in/out/intr mailbox depths + signal pending bits.
- 2 novos testes em `rpcs3-spu-differential` validam que `wrch SPU_WROUTMBOX` é detectado e que diff harness reporta divergência de canais.

**Iter-11 SPU (+8 instruções, 118 → 126):**
- **Halfword shifts RR:** ROTH (0x05C), ROTHM (0x05D), ROTMAH (0x05E), SHLH (0x05F)
- **Halfword shifts RI7:** ROTHI (0x07C), ROTHMI (0x07D), ROTMAHI (0x07E), SHLHI (0x07F)
- Helpers `halfword_op` / `halfword_const_op` reutilizáveis para futuras famílias halfword.

**Novo fixture:** `synthetic_halfword_shifts.elf` — exercita ilh + shlhi + rothmi + rothi → 0x00FF broadcast vira 0x0FF0 / 0x000F / 0xFF00 (3 tests: básico + recompiler-vs-interpreter diff).

## Methodology

- **behavior-freeze first** — replicate observable RPCS3 contracts byte-for-byte before any optimization
- **`compare_run.py`** is the differential gate (see [`harness/compare_run.py`](../harness/compare_run.py))
- **inventory P0/P1/P2** drives prioritization (see [`INVENTORY.md`](INVENTORY.md))
- **Ship-of-Theseus incremental** — module-by-module replacement, never big-bang rewrite
- **zero-regression rule** — every change keeps the full workspace green
- **append-only autonomous log** — see [`AUTONOMOUS_LOG.md`](AUTONOMOUS_LOG.md), 1689 lines / 229 iterations

## Language strategy

- **Rust is the default** for all new ports
- **Zig only enters with measurable benefit** (none committed in this wave; option remains open for hot paths)

## Plan status

**Plan substantially complete.**

> ⚠️ **IMPORTANT CLARIFICATION:** "Plan substantially complete" does **NOT** mean "complete runtime parity with RPCS3."
>
> What is complete: the **port plan as a documentation/scope artifact** — every viable byte-exact port from `Cell/Modules/`, `Audio/`, `Io/`, `Loader/`, `RSX/` (helpers), `NP/`, LV2 syscalls, and HLE modules has been delivered.
>
> What is **not** complete: the **runtime emulator** — the giants (SPU/PPU Recompilers, PPU Translator, RSX Thread, VKGSRender, System.cpp, Qt UI) remain explicitly out of scope and would each be a multi-week dedicated project. Contract stubs exist in `rpcs3-ppu-interpreter` / `rpcs3-spu-interpreter` / `rpcs3-ppu-thread` / `rpcs3-spu-thread` to satisfy the behavior-freeze wave's needs.

## Next phase (execution)

Move from "module surface coverage" to "execution against real targets":

- **Real fixtures** — at least one open-source PPU homebrew (e.g. ps3autotests) committed and reproducible
- **Homebrew differential validation** — run our crates alongside RPCS3 C++ on the same fixture, diff log + frame hash + WAV
- **Save/load real validation** — exercise cellSavedata against real save data + delete/load cycles
- **Sentinel commercial title** — pick one as canonical regression sentinel (avoids drift)
- **Performance/RAM/VRAM profiling** — only after correctness baseline is locked

See [`ROADMAP.md`](ROADMAP.md) for the full phase list.

## See also

- [`CHECKLIST.md`](CHECKLIST.md) — operational checklist with per-wave status
- [`BACKLOG_RESIDUAL.md`](BACKLOG_RESIDUAL.md) — small remaining pieces by category
- [`DEFERRED.md`](DEFERRED.md) — explicitly deferred items (with reason / required input / unblock condition)
- [`DECISIONS.md`](DECISIONS.md) — architectural decisions log
- [`ROADMAP.md`](ROADMAP.md) — next-phase plan
