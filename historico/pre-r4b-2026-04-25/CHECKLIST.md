# Checklist operacional

> 🧊 **FROZEN BASELINE — 2026-04-24**
>
> - **Plan baseline frozen** as of 2026-04-24
> - **230 crates** in workspace
> - **5186 tests passing** (was 5165 at freeze; +21 from iter-8 SPU expansion)
> - **229 autonomous iterations** consecutive
> - **0 regressions** across the entire session
> - **Next phase:** execution against real targets — see [`ROADMAP.md`](ROADMAP.md) and [`HOMEBREW_PLAN.md`](HOMEBREW_PLAN.md)
>
> Frozen snapshot at: [`CHECKLIST_FREEZE_2026-04-24.md`](CHECKLIST_FREEZE_2026-04-24.md)
>
> Plan is **substantially complete as scope artifact** — runtime parity is NOT complete.
> See [`CURRENT_STATE.md`](CURRENT_STATE.md), [`DECISIONS.md`](DECISIONS.md) ADR-011.
>
> **Post-freeze additions (2026-04-24 / 2026-04-25):**
> - Hardening: 4 SPU helpers otimizados (`split_lanes`/`join_lanes`/`broadcast_u32`/`read_qword_be`).
> - Iter-8 SPU: +17 instruções (FCGT/FCMGT/FCEQ/FCMEQ/FREST/FRSQEST/FSM/LQX/STQX/BI/BISL/IRET/HBR/BIZ/BINZ/BIHZ/BIHNZ).
> - Iter-9 SPU: +17 instruções (SHL/ROT/ROTM/ROTMA, ROTI/ROTMI/ROTMAI, NAND/EQV/ANDC/ORC, CEQH/CEQB/CGTH/CGTB/CLGTH/CLGTB, SYNC/DSYNC/STOPD).
> - Iter-10 SPU: +11 instruções (AH/SFH halfword arith, CG/BG carry/borrow, ORX, BRSL, HBRA/HBRR hints, CEQHI/CGTHI/CLGTHI halfword imm).
> - Phase B P3+P4 done: `spu-runner` binário (13 integration tests, 8 fixtures sintéticos) + `spu_homebrew_runner.py` com diff funcional + `build_synthetic_fixtures.py` reproducível.
> - **NEW crate `rpcs3-spu-differential`** (12 lib tests): `SpuExecutor` trait + diff harness — recompiler-ready. `spu-runner` agora usa o trait via `--backend interpreter`.
> - **`docs/SPU_RECOMPILER_PLAN.md`** — port plan completo do SPUCommonRecompiler.cpp (Cranelift v0 recomendado, fases R0-R5).
> - **Phase R1 done** — `rpcs3-spu-decoder` crate (28 tests) com two-pass block analysis; valida contra todos os 8 fixtures.
> - **Phase R2 done** — `rpcs3-spu-recompiler` scaffold (7 tests + 4 spu-runner differential) provando trait integration; delega ao interpreter por enquanto. Ready for R2.5 (Cranelift codegen) pendente aprovação.
> - **Critical bug fix descoberto durante R1:** opcodes A (0x180→0xC0), AND (0x181→0xC1), SF (0x080→0x40) — eram 2× canonical. Fixos sem regressão; fixtures regeneradas.
> - **R3 blocker resolved (channel snapshot):** `SpuStateSnapshot.channel_counts` agora populado de SpuChannels. 2 testes diferenciais verificam.
> - **Iter-11 SPU:** +8 halfword shifts (ROTH/ROTHM/ROTMAH/SHLH + RI7 variantes). +1 fixture `synthetic_halfword_shifts.elf`. SPU agora **126 tests**, ~70% ISA coverage.
> - **Phase R2.5 FIRST LIGHT (Cranelift JIT real):** subset il/ila/nop/lnop/stop/a/sf/and/or/xor/nor codegen via Cranelift; `RecompilerExecutor` JIT-first com fallback automático; 14 lib tests + 4 differential tests (interpreter ≡ JIT) verde.
> - **Phase R2.5 MULTI-BLOCK (2026-04-25):** Multi-block compile + branches diretos (br/bra/brnz/brz/brsl) + RI10 ALU+cmp (ai/andi/ori/xori/ceqi/cgti) + LoadImm completo (il/ila/ilh/ilhu/iohl). **synthetic_loop.elf agora roda inteiro via JIT (loop completo, zero-fallback).** 20 lib tests.
> - **Phase R2.5 EXPANSION (2026-04-25):** +RI7 shifts (shli/roti/rotmi/rotmai) +RR compares (ceq/cgt/clgt) +RR word shifts (shl/shr/sar/rot com SPU semantics) +bitwise extras (nand/eqv/andc/orc) +carry/borrow (cg/bg). **synthetic_arith.elf agora roda 100% via JIT.** 27 lib tests no recompiler.
> - **Phase R2.5 LOAD/STORE (2026-04-25):** +`lqd`/`stqd`/`lqx`/`stqx` quadword load/store via `ls_ptr` em `JitState` + bswap para BE/LE conversion. **synthetic_loadstore.elf agora roda 100% via JIT.** 30 lib tests no recompiler. **3 fixtures rodando 100% JIT** (loop, arith, loadstore).
> - **Phase R2.5 HALFWORD+ORX (2026-04-25):** +`ah`/`sfh` (per-halfword arith via split-mask-add-repack) + `orx` (Unary OR-across). **synthetic_orx_collapse.elf agora roda 100% via JIT.** 35 lib tests no recompiler. **4 fixtures rodando 100% JIT** (loop, arith, loadstore, orx_collapse).
> - **Phase R2.5 HALFWORD-SHIFTS (2026-04-25):** +`shlhi`/`rothmi`/`rotmahi`/`rothi` (per-halfword RI7 shifts) + decoder bugfix (0x07C..0x07F agora classificados como AluImm7). **synthetic_halfword_shifts.elf agora roda 100% via JIT.** 37 lib tests. **5 fixtures rodando 100% JIT** (loop, arith, loadstore, orx_collapse, halfword_shifts).
> - **Phase R2.5 FLOAT-FAMILY (2026-04-25):** +`fa`/`fs`/`fm` (per-lane f32) via Cranelift `fadd`/`fsub`/`fmul` + FTZ denormal flush helper para FM. **synthetic_float_dot.elf agora roda 100% via JIT.** 39 lib tests. **6 fixtures rodando 100% JIT** (loop, arith, loadstore, orx_collapse, halfword_shifts, float_dot — 7/9 com il_stop trivial). Apenas synthetic_brsl_ret (indirect branches) cai pro interpreter.
> - **Phase R2.5 SELB+BENCHMARK (2026-04-25):** +`selb` (RRR-form bit-select) — primeiro opcode RRR codegen'd. **Benchmark JIT vs interpreter:** 200 runs do synthetic_loop em release → interpreter 2.43ms vs JIT 1.54ms = **1.58× speedup real medido**. Prova que codegen entrega valor concreto. 41 lib tests.
> - **Phase R2.5 HALFWORD-COMPARES + FIBONACCI (2026-04-25):** +`ceqh`/`cgth`/`clgth` (per-halfword compares). Novo programa **fibonacci(10)** via SPU instructions (3 ila + ceqi + brnz + 3 a + ai + br back-edge + stop) roda **100% via JIT, zero fallback** retornando r3=55 broadcast. 43 lib tests. **Fibonacci é o primeiro programa "real" (não-trivial loop) provando JIT em código com aritmética de múltiplas variáveis + condicional + back-edge.**
> - **Phase R2.5 MULTIPLIES + SUM-OF-SQUARES (2026-04-25):** +`mpy` (signed 16×16→32) +`mpyu` (unsigned 16×16→32) per word lane. Helper `emit_word_mpy` com sextend/mask + Cranelift `imul`. Novo programa **sum_of_squares (1²+2²+...+10² = 385)** roda 100% via JIT, retornando r3 = 0x181 broadcast. 45 lib tests.
> - **Phase R2.5 MEGA-EXPANSION (2026-04-25):** +13 opcodes em uma única sub-onda: **float compares** (fcgt/fcmgt/fceq/fcmeq via Cranelift `fcmp` + FTZ flush + abs mask), **byte compares** (ceqb/cgtb/clgtb 16-lane via 4 byte slices per word), **halfword RR shifts** (shlh/roth/rothm/rotmah com count from rb halfword), **halfword imm compares** (ceqhi/cgthi/clgthi), **multiplies imm** (mpyi/mpyui), **halfword imm arith** (ahi), **word imm extras** (sfi/clgti). +Decoder fixes para classificar todos esses primaries. 50 lib tests.
> - **Phase R2.5 UNARY + EXTENDED MULTIPLIES (2026-04-25, mesmo turno):** +12 opcodes em mais 2 sub-ondas: **8 unary RR** (clz/cntb/xsbh/xshw/xswd/fsm/frest/frsqest com Cranelift `clz`/`popcnt`/`sextend`/`ireduce`/`fdiv`/`sqrt`) + **3 extended multiplies** (mpyh/mpyhh/mpys via signed half-×-half + ireduce/sextend pattern).
> - **Phase R2.5 QUADWORD-SHIFTS + CONVERTS + BRANCH-HINT (2026-04-25, mesmo turno):** +6 opcodes: **rotqbyi/shlqbyi** (quadword byte shift imm via gather-by-byte pattern) + **cflts/cfltu/csflt/cuflt** (float↔int converts com `fcvt_*_sat` + `scale_float` helper). +**BranchHint** (HBR/HBRA/HBRR) reconhecido como NOP no JIT.
> - **Phase R4a — DISPATCHER + INDIRECT BRANCHES (2026-04-25):** JIT outcomes refatorados (BAILOUT → CONTINUE_TO sem queda pro interpreter). Codegen para UncondIndirect (bi/bisl/iret) e CondIndirect (biz/binz/bihz/bihnz). **`RecompilerExecutor::execute` agora é dispatcher loop** com function cache por `(entry_pc, ls_hash)` + JitStats expandida (cache_hits, cache_misses, dispatcher_iterations). **synthetic_brsl_ret roda 100% via JIT zero-fallback** (subroutine call/return). 5 testes R4a + benchmark **1.40× speedup vs interpreter**.
> - **Phase RRR-COMPLETE (2026-04-25):** +`shufb` (byte permutation 16-lane com 3 constant patterns + dynamic byte indexing via address arithmetic) +`fma`/`fnms`/`fms` (f32 multiply-add family, non-fused para matchar interpreter via `fmul + fadd/fsub`). 4 unit tests novos. **Toda família RRR-form (5 opcodes) agora codegen'd.**
> - **Phase BYTE-IMM (2026-04-25):** +6 opcodes byte-immediate (`andbi`/`orbi`/`xorbi`/`ceqbi`/`cgtbi`/`clgtbi`). Decoder fix: byte-imm extracted via `(raw >> 16) & 0xFF` (16-bit position diferente de RI10). Helper `emit_byte_imm` com Or/And/Xor (broadcast splat) + per-byte CmpEq/GtSigned/GtUnsigned. JIT cobertura agora **~102 opcodes**.
> - **Phase R4b — CHAINED PATCHING SEGURO (2026-04-25):** dispatcher do JIT pula direto para função compilada na continuação quando seguro. Chain-table local `HashMap<u32, ChainEntry>` (entry_fn fn-pointer + ls_hash + function_size) substituiu o caminho `compile_or_fetch` no path quente. **Reversibilidade total:** se `ls_hash` mismatch (SMC) ou ausente, fall through ao path R4a sem perda de correctness. 5 stats novos: `chained_jumps`, `dispatcher_bypasses`, `patch_hits`, `patch_misses`, `invalid_chain_guards`. **+12 testes:** 4 safety (refusal not compiled, refusal ls_hash change, chain on 2nd execution, clear_cache drops chain) + 4 equivalence (synthetic_loop/brsl_ret/fibonacci/sum_of_squares × 10 repeats byte-exato) + 4 benchmarks. Atualização R4a tests para refletir cache_hits=0 (chain bypassa). **Speedup vs interpreter:** 1.44× synthetic_loop, 1.43× brsl_ret, 1.46× fibonacci, 1.36× sum_of_squares. Chain table satisfaz 408/410 dispatcher iters em brsl_ret e 204/205 em programas single-function.
> - **Phase R4c — SMC / CACHE INVALIDATION SEGURO (2026-04-25):** detecção proativa de self-modifying code. Cada função compilada agora carrega `CompiledMeta { code_start, code_end, exact_hash, function_size }` em mapa paralelo `compiled_meta`. Helper `smc_scan(ls)` roda no início de toda dispatcher iter, varrendo `compiled_meta`, recomputando `hash_ls_range(ls, code_start, code_end)` e invalidando atomicamente entries cujo hash divergiu (drop de `compiled` + `compiled_meta` + `cache` + chain quando entry_fn casa). 5 stats novos: `smc_invalidations`, `smc_chain_evictions`, `smc_full_flushes` (reservado), `smc_range_hits`, `smc_range_misses`. **+14 testes:** SMC detected (different prog at same pc), no-SMC equivalence (4 programs × 10 repeats), writes outside ranges don't invalidate (single-execute test com stqd para 0x200 fora dos code ranges), chain eviction com fn-pointer match, stats invariants, meta-size tracking, 4 benchmarks. R4b tests atualizados (`recompiler_caches_decoded_functions` + `r4b_chain_refuses_when_ls_hash_changes`) para refletir que R4c intercepta antes do R4b path. **Speedup vs interpreter:** 1.40× synthetic_loop, 1.46× brsl_ret, 1.45× fibonacci, 1.58× sum_of_squares — sem regressão material vs R4b.
> - Workspace agora **5323 lib + 19 spu-runner + 28 decoder = 5370 testes verdes, +12 R4b + 14 R4c → 5396 base SPU, zero regressão (27ª onda consecutiva)**.
> - Bloqueado em Phase B P1 (homebrew SPU ELF real) e P5 (RPCS3 dump capture) — ver `HOMEBREW_PLAN.md`. R3 blocker FPSCR ainda pendente (não usado por fixtures atuais).

## Fase 0 — Setup (concluída 2026-04-21)
- [x] Criar `behavior-freeze/` sem tocar produção
- [x] Documentar inventário P0/P1/P2 ([INVENTORY.md](INVENTORY.md))
- [x] Contracts GTest iniciais (3 arquivos) plugados via `behavior-freeze/contracts/`
- [x] Harness Python headless (run/capture/compare) com log canônico
- [x] Especificação de fixtures mínimas ([../fixtures/README.md](../fixtures/README.md))
- [x] **Visual Studio 2022 Build Tools + Rust MSVC + Zig 0.16 + zls instalados**
- [x] **CMake+corrosion+rustc integration validada end-to-end (7+ .lib produzidas)**
- [x] **PORT_PLAN.md §0.1-0.4 com matriz de decisão 8D e veredito crate-a-crate**
- [x] **4 hooks em .claude/settings.local.json (PreToolUse guard + PostToolUse cargo check + Stop log + SessionStart context)**
- [x] **AUTONOMOUS_LOG.md com trilha auditável de cada turn**

## Onda 1 — Folhas puras (concluída 2026-04-21)
- [x] `rpcs3-utilities` — 15 tests (get_file_extension + C ABI + 2 proptest fuzz)
- [x] `rpcs3-config` — 21 tests (games.yml byte-exact vs yaml-cpp)
- [x] `rpcs3-crypto` — 23 tests (AES 128/192/256 ECB/CBC/CTR + SHA-1 + HMAC + CMAC, todos KAT NIST/RFC)

## Onda 2 — Parsers determinísticos (parcial, 2026-04-21)
- [x] `rpcs3-loader-psf` — 19 tests (PARAM.SFO parse/emit round-trip)
- [x] `rpcs3-loader-elf-self` — 14 tests (ELF64 BE + SCE/SELF headers, PT_LOAD range validation)
- [x] `rpcs3-loader-pkg` — 16 tests (PKG header, entry types, content types)
- [x] `rpcs3-loader-pup` — 10 tests (firmware PUP parse + SHA-1 hash validation)
- [ ] `rpcs3-loader-self-decrypt` — **BLOQUEADO** (não parcial): precisa (a) binary fixtures de SELF reais, (b) `key_vault` com chaves AES/ECDSA do PS3, (c) port de `Crypto/unself.cpp`. Itens (a) e (b) têm implicações legais (copyright + console keys). Temos `rpcs3-loader-elf-self` (parser de header) e `rpcs3-crypto` (AES/SHA base) prontos, mas o decrypt end-to-end não cabe nesta wave.
- [x] `rpcs3-loader-tar` — 14 tests (iter #198, ustar header + octal parser + block alignment)

## Onda 3 — Enums/tabelas (iniciada 2026-04-21)
- [x] `rpcs3-emu-types` — 11 tests (GameBootResult, SystemState, CpuFlag, 48 CellError codes, compile-time asserts de ordinais)
- [~] `rpcs3-lv2-syscall-table` — **PORT PARCIAL** (14 tests passando, tabela de IDs+nomes+shape feita na Onda 4). **Assinaturas de argumentos `ppu_thread`** ainda pendentes — dependem do port completo de `ppu_thread` (gigante, fora de escopo desta wave).

## Onda 4-7 — Emu core, LV2 syscalls, HLE modules (em progresso, 2026-04-21..04-24)

### Estado atual: 🎉🎉🎉🎉 **230 crates** / 🎉🎉🎉🎉🎉 **5165 testes verdes** / 229 iterações autônomas / ZERO regressões (🎉 MARCO 210 CRATES iter #206, 🎉🎉🎉🎉🎉 MARCO 5000 TESTES iter #211, 🎉🎉🎉🎉 **MARCO 230 CRATES** iter #229. wave-8 total = 50 crates, 3 com crypto real. **PLANO SUBSTANCIALMENTE COMPLETO.**)

### Plano residual (mapa honesto do que falta)

**Ainda portável como pequenas crates byte-exato (~20-30 candidatos):**
- Emu/Cell/timers.hpp (helpers pequenos)
- Emu/Memory/vm.cpp (2508L — contract surface)
- Emu/NP/rpcn_client.cpp (3348L — RPCN wire format — port parcial viável)
- Emu/RSX/rsx_methods.cpp, rsx_vertex_data (101L), RSX helpers pequenos
- Emu/IdManager (86L — deps deep, pode ser problema)
- Emu/Cell/ErrorCodes (91L — thread-local state deep)
- Diversos lv2_* e RSX helpers < 500L

**🚫 FORA DE ESCOPO desta wave (gigantes de runtime, cada um = projeto dedicado):**
- SPUCommonRecompiler.cpp (9792L) — JIT x86 backend
- SPULLVMRecompiler.cpp (9497L) — JIT LLVM backend
- SPUASMJITRecompiler.cpp (4878L) — ASMJIT legacy backend
- PPUInterpreter.cpp (7888L), SPUInterpreter.cpp (3363L) — runtime (contract OK via `rpcs3-ppu/spu-interpreter`)
- PPUThread.cpp (5684L), SPUThread.cpp (7488L) — runtime (contract OK via `rpcs3-ppu/spu-thread`)
- PPUTranslator.cpp (5594L), PPUAnalyser.cpp (3278L), PPUModule.cpp (3254L) — PPU JIT tooling
- System.cpp (4823L) — Emulator singleton state machine
- RSXThread.cpp (3675L), VKGSRender.cpp (3009L) — GPU backend runtime
- `rpcs3qt/**` — Qt UI (framework-specific, não deveria portar)

**Decisão arquitetural**: giants precisam de projetos dedicados de semanas cada; não cabem em port linear. Cobertos por contract stubs suficientes pra behavior-freeze wave. Plano substancialmente completo quando ~230 crates cobertas.

### Marcos atingidos
- [x] **Marco 50 crates** (iter #51, hle-cellfs)
- [x] **Marco 1000 testes** (iter #46, hle-cellmsgdialog)
- [x] **Marco 60 crates** (iter #60, hle-cellkey2char)
- [x] **Marco 70 crates** (iter #71, hle-cellvoice)
- [x] **Marco 75 crates** (iter #74, hle-cellmic)
- [x] **Marco 2000 testes** (iter #75, hle-cellmusicexport)
- [x] **Marco 80 crates** (iter #76, hle-cellphotoexport)
- [x] **Marco 85 crates + 50 iters** (iter #50, hle-cellatrac)
- [x] **Marco 90 crates** (iter #90, hle-celljpgdec)
- [x] **Marco 2500 testes** (iter #93, hle-cellpngenc)
- [x] **Marco 100 crates** (iter #96, hle-sys-libc)
- [x] **Marco 100 iterações autônomas** (iter #100, hle-sys-prx-user)
- [x] **Marco 3000 testes** (iter #104, hle-sys-ppu-thread-user)
- [x] **Marco 120 crates** (iter #116, hle-libsnd3)
- [x] **Marco 120 iterações** (iter #120, hle-cellauthdialog)
- [x] **Marco 4000 testes** (iter #138, hle-cellsysutilmisc)
- [x] **Marco 140 crates / 140 iterações** (iter #140, hle-cellsysutilnpeula)
- [x] **Marco 150 crates** (iter #146, hle-cellsysutilavcext)
- [x] **Marco 150 iterações** (iter #150, hle-cellspursjq)
- [x] **Marco 170 crates** (iter #166, hle-lv2-hid)
- [x] **Marco 4500 testes** (iter #167, hle-lv2-tty)
- [x] **Marco 175 crates** (iter #171, hle-cellavconfext)
- [x] **Marco 180 crates** (iter #176, hle-scenp)
- [x] **Marco 180 iterações** (iter #180, hle-cell-l10n)
- [x] **Marco 185 crates** (iter #181, hle-celldmuxpamf)
- [x] **Marco 4700 testes** (iter #185, audio-dumper)
- [x] **Marco 190 crates** (iter #186, audio-backend) — wave-8 Audio (4 crates: utils/resampler/dumper/backend)
- [x] **Marco 190 iterações** (iter #190, io-turntable) — wave-8 Io (4 crates: buzz/ghltar/gametablet/turntable)
- [x] **Marco 4800 testes** (iter #192, io-kamenrider)
- [x] 🎉🎉🎉 **Marco 200 CRATES** (iter #196, io-usio) — wave-8 Io total 10 crates (buzz/ghltar/gametablet/turntable/guncon3/kamenrider/dimensions/skylander/infinity/usio)
- [x] **Marco 4900 testes** (iter #201, loader-iso)
- [x] 🎉🎉🎉 **Marco 210 CRATES** (iter #206, io-interception)
- [x] 🎉🎉🎉🎉🎉 **Marco 5000 testes** (iter #211, io-rb3drums-config)
- [x] **Marco 220 crates** (iter #219, rsx-vertex-data)
- [x] 🎉🎉🎉🎉 **Marco 230 CRATES** (iter #229, util-cheat-info) — **meta plano substancialmente completo**

### Onda 4 — emu-core + memory + threads
- [x] `rpcs3-memory` (14 tests) + `rpcs3-memory-backing` (23 tests)
- [x] `rpcs3-cpu-thread` + `rpcs3-ppu-thread` (19 tests) + `rpcs3-spu-thread` (22 tests)
- [x] `rpcs3-ppu-opcodes` (25 tests) + `rpcs3-ppu-interpreter` (136 tests, 9 iters)
- [x] `rpcs3-spu-interpreter` (126 tests, 11 iters — iter-11 added halfword shifts ROTH/ROTHM/ROTMAH/SHLH + ROTHI/ROTHMI/ROTMAHI/SHLHI)
- [x] `rpcs3-spu-differential` (14 tests — channel-counts snapshot populated; trait + diff harness recompiler-ready)
- [x] `rpcs3-spu-decoder` (20 lib + 8 fixture-driven integration = 28 tests; two-pass leader analysis; Phase R1 done)
- [x] `rpcs3-spu-recompiler` (66 lib tests + 4 spu-runner differential; Cranelift JIT — **~102 opcodes** incluindo R4a dispatcher (indirect branches), RRR family completa, **byte immediate ops (andbi/orbi/xorbi/ceqbi/cgtbi/clgtbi)**; **6 fixtures + 3 programas reais rodam 100% via JIT zero-fallback**; Phase R4a + RRR + byte-imm done)
- [x] `rpcs3-emu-core` (14 tests) — 🎉 MVP PPU+SPU end-to-end
- [x] `rpcs3-lv2-syscall-table` (14 tests) + `rpcs3-loader-mself` (9 tests)
- [x] `rpcs3-vfs-paths` (17 tests) + `rpcs3-vfs-mount` (13 tests)

### Onda 5 — LV2 syscalls (14 crates)
- [x] `rpcs3-lv2-process` (14) / `rpcs3-lv2-ppu-thread` (16) / `rpcs3-lv2-memory` (17)
- [x] `rpcs3-lv2-sync` (24) / `rpcs3-lv2-fs` (26) / `rpcs3-lv2-timer` (16)
- [x] `rpcs3-lv2-event` (17) / `rpcs3-lv2-cond` (19) / `rpcs3-lv2-lwmutex` (22)
- [x] `rpcs3-lv2-rwlock` (21) / `rpcs3-lv2-event-flag` (25)
- [x] `rpcs3-lv2-spu-image` (21) / `rpcs3-lv2-spu-group` (21) / `rpcs3-lv2-raw-spu` (13)

### Onda 6 — sysPrxForUser + sys-libc + sys-* (15 crates)
- [x] `rpcs3-hle-sys-libc` (49) / `rpcs3-hle-sys-heap` (30) / `rpcs3-hle-sys-mempool` (32)
- [x] `rpcs3-hle-sys-prx-for-user` (15) / `rpcs3-hle-sys-prx-user` (33) / `rpcs3-hle-sys-spu-user` (45)
- [x] `rpcs3-hle-sys-net-user` (36) / `rpcs3-hle-sys-io-user` (30) / `rpcs3-hle-sys-rsxaudio-user` (28)
- [x] `rpcs3-hle-sys-game-user` (32) / `rpcs3-hle-sys-crashdump` (27) / `rpcs3-hle-sys-lv2dbg` (28)
- [x] `rpcs3-hle-sys-lwmutex-user` (37) / `rpcs3-hle-sys-lwcond-user` (28) / `rpcs3-hle-sys-mmapper-user` (30)
- [x] `rpcs3-hle-sys-ppu-thread-user` (34) / `rpcs3-hle-libfs-utility` (21)

### Onda 7 — HLE modules (cellXxx + sceXxx + libxxx) — 90+ crates
**Audio/Codec**: cellaudio (24), cellatrac (25), cellatracmulti (29), celladec (25), cellvdec (26), cellvpost (20), cellaudioout (covered by cellavconf), cellvoice (44), cellmusic (36), cellmusicdecode (39), cellmusicexport (36), cellmic (44), libmixer (41), libsnd3 (40), libsynth2 (34)

**Graphics/Display**: cellgcm (25), cellvideoout (23), cellfont (24), cellfont-ft (26), cellresc (41), cellgifdec (39), celljpgdec (36), celljpgenc (38), cellpngdec (40), cellpngenc (45), cellscreenshot (25), cellgem (42), cellsail (42), cellsailrec (20), cellvideoplayerutility (25), cellvideoupload (23)

**Sysutil/UI**: cellsysutil (21), cellsysutilavc (24), cellsysutilavcext (27), cellsysutilavc2 (42), cellsysutilap (23), cellsysutilmisc (13), cellsysutilnpeula (23), cellmsgdialog (22), celloskdialog (43), cellsysmodule (16), cellrtc (21), cellrtcalarm (21), cellnetctl (19), cellnetaoi (24), cellsubdisplay (39), cellsysconf (22), cellauthdialog (21), cellprint (33), cellsearch (34), cellscreenshot (25), cellwebbrowser (22)

**Storage/Files**: cellgame (20), cellgameexec (24), cellgametexec (covered cellgameexec), cellsavedata (23), cellfs (22), cellfs-sdata (36), cellsyscache (19), cellstorage (25), cellbgdl (25), cellsheap (25)

**Input**: cellpad (20), cellkb (23), cellmouse (24), cellkey2char (29), cellgem (42), cellcrosscontroller (27)

**Network**: cellhttp (42), cellhttputil (42), cellhttps (36), cellrudp (42), cellssl (17), cellsync (19), cellsync2 (38)

**SPU/Threading**: cellspurs (20), cellspursjq (23), cellfiber (35), celldaisy (35), cellovis (29), cellpamf (29), celldmux (34), cellspudll (19), cellpesmutility (30)

**Camera/USB**: cellcamera (26), cellusbd (23), cellusbpspcm (31)

**NP/Network/Trophies**: scenpsns (21), scenpmatchingint (13), scenpplus (5), scenputil (21), celltrophy (56), cellremoteplay (22), celldtcpiputility (33)

**Misc/Stubs**: cellauthdialog (21), cellpesmutility (30), cellrec (40), cellphotoexport (34), cellphotoimport (27), cellphotodecode (25), cellvideoexport (36), libad-core (28), libad-async (26), libmedi (25), cellllibprof (21), static-hle (35), hle-patches (25), cellmusicselectioncontext (34)

### Em progresso ou próximos candidatos
- [x] `rpcs3-hle-cellatracxdec` (1015L, iter #179, contract-only — ffmpeg path fora)
- [x] `rpcs3-hle-cellavconfext` (617L, iter #171)
- [x] `rpcs3-hle-cellaudioout` (590L, iter #170, dedicated port)
- [x] `rpcs3-hle-cellimejp` (1295L, iter #177)
- [x] `rpcs3-hle-cell-l10n` (2854L, iter #180, 3º maior do workspace)
- [x] `rpcs3-hle-cell-freetype2` (1096L, iter #178, 155 stubs)
- [x] `rpcs3-hle-scenpclans` (1282L, iter #173)
- [x] `rpcs3-hle-scenpcommerce2` (1125L, iter #174)
- [x] `rpcs3-hle-scenptus` (1478L, iter #175)
- [x] `rpcs3-hle-celldmuxpamf` (2906L, iter #181, contract-only)
- [x] `rpcs3-hle-scenp` main (7590L, iter #176, **MAIOR módulo** 239 entries)
- [x] `rpcs3-hle-cellgcmsys` (1632L, iter #182, contract-only)

### Cobertura Cell/Modules/ = 100% (136 crates Rust / 135 cpp modules, cellSpursSpu é helper interno sem REG_FUNC)

## Onda 8 — Emu infrastructure (em progresso, 2026-04-24)

### Wave-8 Audio (4 crates, iters #183-186)
- [x] `rpcs3-audio-utils` (iter #183, 12 tests) — volume/mute + non-linear scaling
- [x] `rpcs3-audio-resampler` (iter #184, 12 tests) — SoundTouch params + AudioFreq/Channel/SampleSize enums
- [x] `rpcs3-audio-dumper` (iter #185, 14 tests) — WAV file layout + bookkeeping (56B header compile-time asserted)
- [x] `rpcs3-audio-backend` (iter #186, 20 tests) — DSP helpers (convert_to_s16, volume ramp, normalize soft-clip, channel layouts)

### Wave-8 Io (6 crates, iters #187-192)
- [x] `rpcs3-io-buzz` (iter #187, 14 tests) — Logitech Buzz! buzzer (VID 0x054c/PID 0x0002)
- [x] `rpcs3-io-ghltar` (iter #188, 11 tests) — Guitar Hero Live guitar (VID 0x12BA/PID 0x074B)
- [x] `rpcs3-io-gametablet` (iter #189, 14 tests) — THQ uDraw Game Tablet (VID 0x20d6/PID 0xcb17)
- [x] `rpcs3-io-turntable` (iter #190, 13 tests) — DJ Hero Turntable (VID 0x12BA/PID 0x0140)
- [x] `rpcs3-io-guncon3` (iter #191, 8 tests) — Namco GunCon 3 light-gun com **cipher byte-exato** (KEY_TABLE 256 bytes)
- [x] `rpcs3-io-kamenrider` (iter #192, 12 tests) — Kamen Rider Summoner NFC portal

### Wave-8 Io — restantes (4 crates, iters #193-196, crypto real em 3)
- [x] `rpcs3-io-dimensions` (iter #193, 15 tests) — LEGO Dimensions com **TEA cipher real** + Jenkins PRNG + SHA1+AES key derivation
- [x] `rpcs3-io-skylander` (iter #194, 13 tests) — Skylanders PortalMaster 8-slot USB
- [x] `rpcs3-io-infinity` (iter #195, 20 tests) — Disney Infinity com **SHA1+AES-128** + scramble bit-twiddling + Jenkins RNG variant
- [x] `rpcs3-io-usio` (iter #196, 18 tests) — 🎉 arcade USIO v406 (Taiko/Tekken) — MARCO 200 CRATES

### Wave-8 Audio (adicional)
- [x] `rpcs3-audio-device-enumerator` (iter #197, 11 tests) — Cubeb + FAudio device enumerator normalization

### Wave-8 Loader (4 crates, iters #198-201)
- [x] `rpcs3-loader-tar` (iter #198, 14 tests) — POSIX ustar
- [x] `rpcs3-loader-trp` (iter #199, 14 tests) — Trophy archive TRP_MAGIC=0xDCA24D00
- [x] `rpcs3-loader-tropusr` (iter #200, 11 tests) — TROPUSR.DAT per-user state
- [x] `rpcs3-loader-iso` (iter #201, 10 tests) — 🏆 PS3 3k3y ISO + CD001 magic — MARCO 4900 TESTES

### Wave-8 Loader extra + NP + SPU + Emu misc (iters #202-220)
- [x] `rpcs3-loader-disc` (iter #202, 10 tests) — DiscType classifier + SYSTEM.CNF
- [x] `rpcs3-loader-iso-cache` (iter #203, 9 tests) — FNV-1a-64 cache stem
- [x] `rpcs3-np-countries` (iter #204, 10 tests) — 72-country PSN RPCN table
- [x] `rpcs3-spu-mfc` (iter #205, 11 tests) — 39 SPU MFC DMA opcodes byte-exato
- [x] `rpcs3-io-interception` (iter #206, 8 tests) — 🎉 global input-interception state — MARCO 210 CRATES
- [x] `rpcs3-io-usb-vfs` (iter #207, 5 tests) — SMI USB DISK mass-storage
- [x] `rpcs3-ipc-config` (iter #208, 6 tests) — IPC server port clamp
- [x] `rpcs3-np-upnp-config` (iter #209, 4 tests) — UPnP DeviceUrl
- [x] `rpcs3-io-recording-config` (iter #210, 8 tests) — video/audio recorder config
- [x] `rpcs3-io-rb3drums-config` (iter #211, 5 tests) — 🎉🎉🎉🎉🎉 RB3 drum MIDI mapper — MARCO 5000 TESTES
- [x] `rpcs3-io-mouse-config` (iter #212, 5 tests)
- [x] `rpcs3-io-pad-config-types` (iter #213, 6 tests)
- [x] `rpcs3-io-midi-config-types` (iter #214, 12 tests)
- [x] `rpcs3-io-g27-config-types` (iter #215, 8 tests) — G27 device-type-id 64-bit packing
- [x] `rpcs3-localized-string` (iter #216, 10 tests) — 315 UI string IDs
- [x] `rpcs3-rsx-gsframe` (iter #217, 4 tests)
- [x] `rpcs3-system-config-random` (iter #218, 8 tests)
- [x] `rpcs3-rsx-vertex-data` (iter #219, 11 tests) — vertex_base_type + push buffer math
- [x] `rpcs3-perf-monitor` (iter #220, 11 tests)

### Wave-8 RSX/Util/Version final (iters #221-229)
- [x] `rpcs3-rsx-texture-cache-types` (iter #221, 12 tests) — invalidation cause flag bits
- [x] `rpcs3-rsx-gl-decompiler` (iter #222, 6 tests) — GL varying register table
- [x] `rpcs3-rsx-gl-common` (iter #223, 4 tests) — TLS primary-context marker
- [x] `rpcs3-io-camera-config` (iter #224, 9 tests) — CameraSetting 6-field CSV
- [x] `rpcs3-rsx-vk-decompiler` (iter #225, 10 tests) — Vulkan varying table + texture index
- [x] `rpcs3-hle-sys-spinlock` (iter #226, 9 tests) — spinlock 0xABADCAFE sentinel
- [x] `rpcs3-rsx-surface-store` (iter #227, 9 tests) — MRT target table + pitch align 256
- [x] `rpcs3-version` (iter #228, 11 tests) — version parsing + branch detection
- [x] `rpcs3-util-console` (iter #229, 5 tests) — stream flags + stderr format
- [x] `rpcs3-util-cheat-info` (iter #229, 11 tests) — 🎉🎉🎉🎉 CheatType + @@@ separator — **MARCO 230 CRATES**

### Fora de escopo declarado (bloqueadores técnicos/legais + gigantes de runtime)
- 🔒 **`rpcs3-loader-self-decrypt`** — BLOQUEADO: precisa fixtures SELF reais + `key_vault` PS3 (implicações legais) + port de `Crypto/unself.cpp`. Parser de header (`rpcs3-loader-elf-self`) e crypto base (`rpcs3-crypto`) já prontos.
- ⏳ **`rpcs3-lv2-syscall-table` (port parcial feito, 14 tests)** — tabela de IDs + nomes + shape já no workspace. Faltam **assinaturas de argumentos `ppu_thread`** → bloqueado pelo port completo do `ppu_thread` (gigante).
- 🚫 **SPU/PPU Recompilers, PPU Translator, RSX Thread, VKGSRender, System.cpp, Qt UI** — cada um é um projeto dedicado de semanas. Contract stubs suficientes pra behavior-freeze wave (`rpcs3-ppu/spu-interpreter`, `rpcs3-ppu/spu-thread`).

## Sessão 2026-04-24 (iters #177-229, 53 iterações autônomas, ZERO regressões)

| # | Crate | Tests | Marco |
|---|-------|-------|-------|
| 177 | hle-cellimejp | 7 | |
| 178 | hle-cell-freetype2 | 7 | |
| 179 | hle-cellatracxdec | 10 | |
| 180 | hle-cell-l10n | 9 | 🏆 180 iters |
| 181 | hle-celldmuxpamf | 12 | 🏆 185 crates |
| 182 | hle-cellgcmsys | 7 | Cell/Modules 100% |
| 183 | audio-utils | 12 | wave-8 iniciada |
| 184 | audio-resampler | 12 | |
| 185 | audio-dumper | 14 | 🏆 4700 tests |
| 186 | audio-backend | 20 | 🎉 190 crates |
| 187 | io-buzz | 14 | |
| 188 | io-ghltar | 11 | |
| 189 | io-gametablet | 14 | |
| 190 | io-turntable | 13 | 🏆 190 iters |
| 191 | io-guncon3 | 8 | cipher real |
| 192 | io-kamenrider | 12 | 🏆🏆🏆 4800 tests |
| 193 | io-dimensions | 15 | 🔐 TEA cipher byte-exato |
| 194 | io-skylander | 13 | |
| 195 | io-infinity | 20 | 🔐 SHA1+AES+scramble |
| 196 | io-usio | 18 | 🎉🎉🎉🎉 **200 crates** |
| 197 | audio-device-enumerator | 11 | |
| 198 | loader-tar | 14 | |
| 199 | loader-trp | 14 | |
| 200 | loader-tropusr | 11 | |
| 201 | loader-iso | 10 | 🏆🏆🏆 **4900 testes** |
| 202 | loader-disc | 10 | |
| 203 | loader-iso-cache | 9 | FNV-1a-64 |
| 204 | np-countries | 10 | |
| 205 | spu-mfc | 11 | 39 DMA opcodes |
| 206 | io-interception | 8 | 🎉🎉🎉 **210 crates** |
| 207 | io-usb-vfs | 5 | |
| 208 | ipc-config | 6 | |
| 209 | np-upnp-config | 4 | |
| 210 | io-recording-config | 8 | |
| 211 | io-rb3drums-config | 5 | 🎉🎉🎉🎉🎉 **5000 testes** |
| 212 | io-mouse-config | 5 | |
| 213 | io-pad-config-types | 6 | |
| 214 | io-midi-config-types | 12 | ßßß separator |
| 215 | io-g27-config-types | 8 | device-type-id 64-bit |
| 216 | localized-string | 10 | 315 UI IDs |
| 217 | rsx-gsframe | 4 | |
| 218 | system-config-random | 8 | |
| 219 | rsx-vertex-data | 11 | 220 crates |
| 220 | perf-monitor | 11 | |
| 221 | rsx-texture-cache-types | 12 | |
| 222 | rsx-gl-decompiler | 6 | |
| 223 | rsx-gl-common | 4 | |
| 224 | io-camera-config | 9 | |
| 225 | rsx-vk-decompiler | 10 | |
| 226 | hle-sys-spinlock | 9 | 0xABADCAFE |
| 227 | rsx-surface-store | 9 | MRT + pitch align |
| 228 | version | 11 | |
| 229 | util-console + util-cheat-info | 5+11 | 🎉🎉🎉🎉 **230 CRATES** |

**Delta sessão total**: crates 180→230 (+50), tests 4620→5165 (+545), iters 176→229 (+53).

## 🎉🎉🎉🎉 PLANO SUBSTANCIALMENTE COMPLETO (atingido iter #229, 2026-04-24)

**Métricas finais:**
- **230 crates** no workspace
- **5165 testes** verdes
- **229 iterações autônomas** consecutivas
- **ZERO regressões** em toda a sessão

**Cobertura por área:**
- Cell/Modules/ = 100% (136 crates HLE)
- Emu/Audio/ = 5 crates (utils, resampler, dumper, backend, enumerator)
- Emu/Io/ = 18+ crates (10 dispositivos USB emulados + 8+ configs)
- Loader/ = 10 crates (psf, elf-self, pkg, pup, mself, tar, trp, tropusr, iso, iso-cache, disc)
- Emu/NP/ = 2 crates (countries, upnp-config)
- Emu/RSX/ = 6 crates (gsframe, vertex_data, texture_cache_types, surface_store, gl/vk decompilers, gl_common)
- Utilities/ = 2 crates (console, cheat_info)
- Misc = version, ipc-config, perf-monitor, spu-mfc, localized-string, system-config-random, rsx-gsframe

**Crypto byte-exato real:**
- `guncon3` — 256-byte KEY_TABLE cipher (Namco GunCon 3)
- `dimensions` — TEA cipher + Jenkins PRNG (LEGO Dimensions)
- `infinity` — SHA1+AES-128 + scramble bit-twiddling + Jenkins variant (Disney Infinity)

## Fase 1 — Pré-requisitos para rodar testes contra emulador C++
- [ ] Build com `-DBUILD_RPCS3_TESTS=ON` e verificar que `rpcs3_test` passa verde hoje
- [ ] Integrar os 3 `test_contract_*.cpp` ao `rpcs3/CMakeLists.txt` (ver `contracts/README.md` — uma linha em `target_sources`)
- [ ] Obter ao menos 1 homebrew PPU open-source (ex.: `ps3-homebrew-pong` ou `ps3autotests`) como fixture real
- [ ] Rodar `python behavior-freeze/harness/capture_baseline.py --scenario=help_text` para gravar o primeiro golden

## Fase 2 — Expansão
- [ ] Cobrir BootGame com 5 inputs sintéticos (pasta vazia, PKG corrompido, EBOOT renomeado, path inexistente, SELF plaintext)
- [ ] Adicionar diff de `RPCS3.log` canonicalizado ao `compare_run.py`
- [ ] Adicionar hash de `.rrc` (RSX capture) para 1 homebrew de referência
- [ ] Adicionar WAV dump + FFT compare para áudio

## Fase 3 — Diferencial vs Rust
- [ ] Garantir que o binário Rust expõe as mesmas flags de CLI
- [ ] Rodar a mesma suíte `compare_run.py` apontando para o binário Rust
- [ ] Congelar baseline de divergências aceitáveis (ex.: timestamps)

## O que NÃO fazer nesta fase
- NÃO refatorar `rpcs3/` em busca de testabilidade.
- NÃO mockar BootGame só para ficar verde.
- NÃO adicionar testes P2 antes dos P0 estarem cobertos.
