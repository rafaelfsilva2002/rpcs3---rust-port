# Plano de porte RPCS3 C++ → Rust

**Regra-zero (imposta pelo usuário):** o binário nunca pode quebrar. Cada merge mantém a build funcional e passando nos `behavior-freeze/contracts/*` e `compare_run.py`.

## 0. Dimensão

| Subsistema | Linhas C++ (.cpp + .h) |
|---|---:|
| rpcs3/Emu/Cell/Modules (HLE) | 129 684 |
| rpcs3/Emu/RSX (GPU) | 120 055 |
| rpcs3/Emu/Cell/lv2 (syscalls) | 42 058 |
| Utilities | 24 068 |
| rpcs3/Input | 18 459 |
| rpcs3/Emu/CPU (PPU+SPU cores) | 17 398 |
| rpcs3/Crypto | 11 657 |
| rpcs3/Loader | 4 761 |
| rpcs3/Emu/Memory | 4 171 |
| rpcs3/Emu/Audio | 3 039 |
| **Total (núcleo)** | **~375 000** |

Fora isso ainda há `rpcs3qt/` (Qt GUI), `3rdparty/` e `rpcs3/Emu/NP/`.

## 0.1 Matriz de decisão: Rust vs Zig vs C++-remanescente

O driver primário é **memory safety** (o usuário declarou: "esse emulador tem muitos erros de memória"). A linguagem default é **Rust**. Zig entra **se e somente se** oferece ganho técnico mensurável vs Rust em um subsistema específico, e esse ganho compensa o custo de polyglot.

### Dimensões aplicadas crate-a-crate

Cada subsistema é pontuado em 8 eixos. Cada eixo favorece Rust (R), Zig (Z), ou é neutro (=). A recomendação é a linguagem com maior score ponderado, mas eixos `A` e `E` têm peso dobrado (são o custo de errar irreversível).

| Eixo | Descrição | Rust ganha quando... | Zig ganha quando... |
|---|---|---|---|
| **A** | Safety surface | Código chamado por muitos módulos; UB propaga | (quase nunca) |
| **B** | Raw memory / ponteiros compartilhados | `unsafe` isolado + `miri` + auditável | (quase nunca) |
| **C** | Metaprogramação compile-time | Macros/const fn cobrem | Tabelas gigantes, bit-parsing de opcodes via `comptime` |
| **D** | SIMD central (v128 hot loop) | `std::simd` estabilizado | `std::simd` ainda nightly e `@Vector(N,T)` oferece portabilidade |
| **E** | Interop C++ tipada (cxx bridge) | Precisa `std::string`, `unique_ptr`, classes | Fronteira fica em C ABI simples |
| **F** | Ecossistema de crates maduro | Crate conhecida cobre 80%+ (aes, goblin, ash, cpal, gilrs, yaml-rust2, tracing) | Nenhuma lib Rust razoável |
| **G** | Hot loop de runtime | (empate — mesmo LLVM) | (empate) |
| **H** | Testing/validação | `miri`, `cargo-mutants`, `cargo-fuzz`, `proptest`, `criterion` | `comptime test` elegante mas sem miri |

## 0.2 Veredito crate-a-crate

Tabela prescritiva. Cada linha é uma crate com a decisão final e a razão dominante.

| Crate / módulo | Onda | Linguagem | Razão dominante |
|---|:-:|:-:|---|
| `rpcs3-utilities` | 1 | **Rust** | F: ecossistema trivial. A=baixa, irrelevante. |
| `rpcs3-config` (games.yml, cfg nodes) | 1 | **Rust** | F: `yaml-rust2` + cfg nodes querem `derive(Serialize)` |
| `rpcs3-crypto` | 1 | **Rust** | F: RustCrypto é padrão-ouro. A=baixa mas correctness crítica |
| `rpcs3-loader-psf` | 2 | **Rust** | F: `binrw`/`nom`. A=baixa. |
| `rpcs3-loader-elf-self` | 2 | **Rust** | F: `goblin`. E=alta (alimenta PPU load). |
| `rpcs3-loader-pkg-pup` | 2 | **Rust** | F: `flate2`/`zstd` + crypto Rust. |
| `rpcs3-emu-types` (enums, Cell errors) | 3 | **Rust** | E=máxima: C++ ainda consulta via cxx bridge; `repr(u32)` + `static_assert` |
| `rpcs3-lv2-syscall-table` | 3 | **Rust** | C=alta (1024 slots) mas macros Rust + `inventory` crate cobrem. E=crítica (tipo-safety de args) |
| `rpcs3-memory` (vm, reservation) | 4 | **Rust** | A/B=máximas. Exatamente onde borrow checker + miri + `unsafe` auditável ganham. Zig seria retrocesso técnico |
| `rpcs3-ppu-interpreter` | 4 | **Rust** | A=máxima, G=máxima. C=alta (~300 opcodes) mas `build.rs` + macro gera tabela. |
| **`rpcs3-spu-interpreter-core`** | 4 | **Rust** | A=máxima. Thread state, MFC queue, channels. |
| **`rpcs3-spu-simd-kernel`** (sub-crate) | 4 | **Rust + módulo Zig opt-in** | D=crítica. Rust com `std::arch::x86_64` é default; se bench mostrar ≥15% de ganho, inner kernel migra pra Zig com `@Vector`. Decisão data-driven |
| `rpcs3-lv2-fs` / `sync` / `process` / `timer` / `mmapper` / `net` | 5 | **Rust** | A=alta, E=crítica, F=boa (parking_lot, crossbeam, tokio) |
| `rpcs3-hle-*` (cellGame, cellPad, cellAudio, cellSaveData, cellSpurs, sceNpTrophy, ...) | 6 | **Rust** | Volume alto + A=alta. cellSpurs pode ter sub-módulo SPU via mesma regra do spu-simd |
| `rpcs3-audio` (`cpal`) | 7 | **Rust** | F=perfeito. |
| `rpcs3-input` (`gilrs`) | 7 | **Rust** | F=perfeito. |
| `rpcs3-rsx-null` | 7 | **Rust** | Trivial. |
| `rpcs3-jit-llvm` (PPU/SPU) | 8 | **Rust** | F=`inkwell` maduro. A=alta. |
| `rpcs3-rsx-core` (FIFO, state) | 9 | **Rust** | A=alta, E=crítica, C=alta mas manejável |
| **`rpcs3-rsx-shader-decompiler`** | 9 | **Zig é candidato sério** | A=**baixa** (output é texto validável offline), B=zero, C=**máxima** (microcódigo RSX → GLSL via bit-parsing de packed structs), F=neutral (`naga` não fala RSX). Único lugar onde Zig `comptime` + `packed struct` produz código **mensuravelmente mais limpo**. Trava-se em PR de avaliação quando chegar a Onda 9 |
| `rpcs3-rsx-backend-vulkan` (`ash`) | 9 | **Rust** | F=`ash` é a lib Vulkan mais madura do ecossistema |
| `rpcs3-rsx-texture-cache` | 9 | **Rust** | A=alta, B=pesada |
| `rpcs3-gui` (`egui`/`iced`) | 10 | **Rust** | F=integral |

**Total esperado de crates Zig no projeto finalizado:** 1 a 2 (shader decompiler certo; SPU SIMD kernel condicional).

**Total de crates Rust:** 25-35 (a maioria do projeto).

**C++ remanescente:** tudo não-portado ainda. Meta é levar a zero, mas JIT LLVM e RSX Vulkan são os últimos pedaços (Ondas 8-9).

## 0.3 Critérios objetivos para reavaliar Zig na Onda 9

Quando chegar em `rpcs3-rsx-shader-decompiler`, decidir Zig vs Rust com base em:

1. Primeiro protótipo Rust usando `binrw` ou manual bit parsing. Medir LoC e clareza.
2. Segundo protótipo Zig usando `packed struct` + `comptime`. Medir LoC e clareza.
3. Benchmark de shader decompilation de 100 shaders reais (golden set).
4. **Regra de corte:** Zig ganha se e somente se (LoC −25% **E** tempo de decompilação −10%) **OU** (LoC −40% com tempo empatando).
5. Se Zig for escolhido, a crate fica como único ponto Zig — regra "ilha isolada": nenhuma outra crate depende dela via interop Zig; fronteira é C ABI.

## 0.4 Toolchain instalado nesta sessão

| Ferramenta | Versão | Uso |
|---|---|---|
| Visual Studio 2022 Build Tools | 17.14.30 | MSVC cl.exe + Windows SDK + cmake |
| Rust (rustup) | 1.94.1 | stable-x86_64-pc-windows-msvc default |
| Rust toolchain alternativa | — | stable-x86_64-pc-windows-gnu (inativa) |
| Zig | 0.16.0 (winget) | Para Onda 9 eventual — instalado preventivamente |
| zls (Zig language server) | 0.16.0 (winget) | Suporte LSP no VS Code |

**Observação:** após `winget install zig.zig`, o `zig` pode não estar no PATH do shell atual. Abrir novo terminal ou rodar `refreshenv` (PowerShell) / reabrir Git Bash.

## 1. Estratégia: Ship-of-Theseus com FFI

**Recusadas explicitamente:**
- *Big-bang rewrite*: 3–5 anos sem nenhum produto intermediário.
- *Rust-first paralelo* (forked project): perde o oráculo, testes diferenciais perdem valor.

**Escolhida: substituição módulo-a-módulo dentro do mesmo binário**, via fronteira FFI mínima. O CMake continua sendo o motor de build. Rust entra como libs estáticas que o linker C++ agrega. Ferramentas:

- **[`cxx`](https://cxx.rs/)** — ponte segura C++ ↔ Rust, geração de headers `.rs.h` automática.
- **[`corrosion`](https://github.com/corrosion-rs/corrosion)** — integra `cargo build` dentro do CMake. Adiciona um subdiretório e expõe a crate Rust como target CMake.
- **`cbindgen`** — só se precisarmos expor ABI C pura (ex.: callbacks chamados do JIT).

### Arquitetura proposta

```
rpcs3-master/
├── rust/                                   # workspace Cargo
│   ├── Cargo.toml                          # [workspace] members
│   ├── rpcs3-utilities/                    # 1ª crate — folhas
│   ├── rpcs3-config/
│   ├── rpcs3-crypto/
│   ├── rpcs3-loader/
│   ├── rpcs3-memory/
│   ├── rpcs3-cpu-core/                     # traits comuns PPU/SPU
│   ├── rpcs3-ppu-interpreter/
│   ├── rpcs3-spu-interpreter/
│   ├── rpcs3-lv2-fs/
│   ├── rpcs3-lv2-sync/
│   ├── ...                                 # uma crate por “módulo observável”
│   └── rpcs3-ffi/                          # glue cxx::bridge (single source of truth)
├── CMakeLists.txt                          # + add_subdirectory(rust)
└── rust/CMakeLists.txt                     # corrosion_import_crate(...)
```

Regras do workspace:
1. Uma crate = uma unidade observável do inventário (P0/P1).
2. Zero dependência de Qt/LLVM/Vulkan nas crates core.
3. `#![forbid(unsafe_code)]` por default; `unsafe` só nas crates de FFI e JIT, isolado em `SAFETY:` comentado.

## 2. Ordem de porte (folhas → núcleo)

Cada item: **pré-requisitos**, **critério de pronto** (o `compare_run.py` bate), **escopo de linhas**.

### Onda 1 — Folhas puras (sem FFI ainda)
Servem para validar toolchain, cxx, benchmarks, CI. Não mudam o binário produzido.

1. **rpcs3-utilities** — subset de `Utilities/` sem dependência do emulador
   - Pré: nenhum
   - Alvos: `BitField.h`, `StrUtil.h`, `CRC.h`, `date_time.{cpp,h}`, `version.{cpp,h}`, parte de `StrFmt`.
   - Pronto: property tests via `proptest`; `criterion` benchmark ≥ C++.
   - Linhas ≈ 3k.
2. **rpcs3-config** — parser de `games.yml`, `config.yml`
   - Pré: utilities.
   - Alvos: `Config.cpp`, `system_config.cpp`, `games_config.cpp`.
   - Pronto: `test_contract_games_yaml_roundtrip` (golden YAML byte-a-byte).
   - Linhas ≈ 6k.
3. **rpcs3-crypto** — AES, SHA1, key_vault
   - Pré: utilities.
   - Alvos: `Crypto/aes.cpp`, `sha1.cpp`, `key_vault.cpp`.
   - Crates Rust: `aes`, `sha1`, `cbc`, `hmac`. Substituição 1-para-1.
   - Pronto: vetores de teste KAT + fixture F4 (SELF non-NPDRM) decrypta para o mesmo ELF.
   - Linhas ≈ 4k.

### Onda 2 — Parsers determinísticos
Substituem funções pure-in-pure-out. FFI mínimo: `extern "C"` simples.

4. **rpcs3-loader-psf** — `Loader/PSF.{cpp,h}`
   - Pré: utilities.
   - Crates: `binrw` para parser binário.
   - Pronto: 3 fixtures (válida, bad magic, truncada) produzem mesmo `registry` e mesmo `psf::error`.
   - Linhas ≈ 0.4k.
5. **rpcs3-loader-elf-self** — descriptografia + parsing de ELF
   - Pré: crypto, psf.
   - Alvos: `Crypto/unself.cpp`, `Loader/ELF.h` (parte).
   - Crate Rust: `goblin` para ELF; AES/CBC já em `aes`.
   - Pronto: SHA-256 do plaintext bate byte-a-byte com C++ para F4.
   - Linhas ≈ 5k.
6. **rpcs3-loader-pkg-pup** — `Loader/PUP.{cpp,h}`, `Loader/unpkg.*`
   - Pronto: install de PKG de teste produz a mesma árvore de arquivos.
   - Linhas ≈ 2k.

### Onda 3 — Tabelas e enums (contratos puros)
Já temos os `test_contract_*.cpp`. Agora o lado Rust entra via `cxx::bridge` e o C++ passa a gerar as mensagens via Rust.

7. **rpcs3-emu-types** — enums `game_boot_result`, `system_state`, `cpu_flag`, códigos Cell (`CELL_ENOENT`, ...)
   - Pré: nada.
   - Implementação: `#[repr(u32)] enum` espelhando exatamente os ordinais. Macros de conversão `from_c_repr()`.
   - Pronto: GTest em C++ reconhece os valores idênticos via `cxx::bridge`.
   - Linhas ≈ 1k.
8. **rpcs3-lv2-syscall-table** — registro de syscalls
   - Pré: emu-types.
   - Estratégia: crate expõe `fn lookup(id: u16) -> Option<&'static SyscallSpec>` com nome + assinatura. C++ passa a consultar isso em vez de ter a tabela hardcoded. Implementações continuam em C++ por ora.
   - Pronto: snapshot `syscall_table.txt` idêntico antes/depois.

### Onda 4 — Memória e CPU (coração)
Aqui mora o risco. A partir daqui, FFI custa performance — precisa bench.

9. **rpcs3-memory** — `vm::*`, reservation
   - Pré: utilities.
   - Cuidado: `g_base_addr`, `g_sudo_addr`, `g_exec_addr` são memória compartilhada com o JIT. `unsafe` isolado em wrapper.
   - Pronto: programa PPU de teste faz load/store/ll-sc; estado final idêntico.
   - Linhas ≈ 4k.
10. **rpcs3-ppu-interpreter** — sem JIT ainda, só o interpreter
    - Pré: memory, emu-types.
    - Alvos: `PPUInterpreter.cpp`, `PPUAnalyser.cpp` (parte).
    - Estratégia: port opcode-por-opcode, comparando com C++ via replay de trace (F1 homebrew).
    - Pronto: homebrew PPU hello-world produz GPRs finais idênticos.
    - Linhas ≈ 8k + os 32k da implementação de ops.
11. **rpcs3-spu-interpreter**
    - Pré: memory.
    - Linhas ≈ 8k + ops.

### Onda 5 — Syscalls LV2 por família
Dividir por arquivo. Cada família vira uma crate.

12. **rpcs3-lv2-fs** — `lv2/sys_fs.cpp` (o maior; ~9k linhas)
13. **rpcs3-lv2-sync** — mutex/cond/event/semaphore (~8k)
14. **rpcs3-lv2-process-thread** — `sys_process`, `sys_ppu_thread`, `sys_spu` (~10k)
15. **rpcs3-lv2-timer-interrupt** — timing (~3k)
16. **rpcs3-lv2-memory-mmap** — `sys_mmapper`, `sys_memory` (~4k)
17. **rpcs3-lv2-net** — `sys_net` (~5k, mas P2)

### Onda 6 — Módulos HLE (Cell/Modules, ~130k linhas)
Port por impacto. Começa por cellGame (carregar jogo), cellPad (input), cellAudio, cellSaveData.

18. **rpcs3-hle-cellgame**
19. **rpcs3-hle-cellpad** — recebe estado do `rpcs3-input`
20. **rpcs3-hle-cellaudio** — fala com `rpcs3-audio`
21. **rpcs3-hle-cellsavedata**
22. **rpcs3-hle-sceNpTrophy**
23. **rpcs3-hle-cellSysutil**
24. **rpcs3-hle-cellSpurs** (delicado — runtime SPU)
25. ... até cobrir os módulos ativamente usados

### Onda 7 — I/O backends
Aqui Rust brilha com o ecossistema.

26. **rpcs3-audio-cpal** — substitui Cubeb/FAudio. Crate `cpal`.
27. **rpcs3-input-gilrs** — substitui handlers ds3/ds4/xinput/sdl. Crate `gilrs`.
28. **rpcs3-rsx-null** — renderer Null (1 arquivo, serve de cavalo de Tróia).

### Onda 8 — JIT
O ponto mais caro. Opções:

- **a) Bindings LLVM via `inkwell`** — mantém a mesma estratégia; custo de integrar com código nativo (frame pointer, SEH no Windows, `jit_announce`) é alto.
- **b) Migrar para Cranelift** — JIT em Rust puro, bem mais simples. Custo: performance inferior ao LLVM atual; alguns workloads PPU podem regredir.
- **c) Deixar C++ JIT intacto** — Rust só ataca o interpreter; JIT continua C++ indefinidamente.

**Recomendação**: começar por (c). Mover para (a) só depois de paridade funcional. (b) é experimento paralelo.

### Onda 9 — RSX Vulkan
Último grande pedaço. `ash` para raw Vulkan, ou `wgpu` se aceitar abstração a mais. Shader decompiler (`rpcs3/Emu/RSX/Program/*`) vira crate própria `rpcs3-rsx-shader-decompiler` com golden tests sérios (item P1.4 do inventário).

### Onda 10 — GUI
Desacoplável. `egui` (imediato, ótimo para debug) ou `iced` (mais refinado). Qt fica até o último momento.

## 3. Fases com marcos mensuráveis

| Fase | Duração estimada | Marco (o que funciona) | Métrica de sucesso |
|---|---|---|---|
| 0. Onboarding | 2 sem | `rust/` workspace + corrosion + 1 função em `rpcs3-utilities` chamada do C++ | CI verde: cmake build + ctest + cargo test |
| 1. Folhas + crypto | 4–6 sem | Ondas 1-2 completas | Contratos P0 da crypto/loader passam com binário misto |
| 2. Núcleo estático | 6–8 sem | Onda 3 (tipos/syscall table) | Enum ordinals congelados em Rust; C++ consulta Rust |
| 3. Memory + interpreters | 3–5 meses | Ondas 4 parciais (só interpreters, sem JIT) | Homebrew PPU/SPU passa no diferencial |
| 4. LV2 first wave | 3–4 meses | Onda 5 (fs + sync + process) | Jogo homebrew simples roda 100% Rust para syscalls core |
| 5. HLE MVP | 6–9 meses | Onda 6 (cellGame/Pad/Audio/SaveData em Rust) | 1 jogo “sentinela” completa boot e 1 min de gameplay via Rust HLE |
| 6. Backends I/O | 2–3 meses | Onda 7 | Áudio cpal + input gilrs em produção; `rsx-null` Rust |
| 7. RSX Vulkan | 6–12 meses | Onda 9 | Frame capture replay com hash idêntico; frametime ≤ +10% |
| 8. JIT + GUI | Aberto | Ondas 8, 10 | Paridade de performance + GUI ≠ Qt |

Total realista full-time até fase 5 (jogo rodando com HLE Rust): **12–18 meses**. Paridade completa com C++: **3+ anos**.

## 4. Rede de segurança em todo commit

- **Contracts GTest** (já criados): P0.1, P0.2, P0.15 em `behavior-freeze/contracts/*`. Adicionar um por crate portada.
- **compare_run.py** (já criado): rodar após cada merge.
- **`cargo test`** por crate + `proptest` para parsers.
- **Mutation testing**: `cargo-mutants` nas crates core; alvo ≥ 80% killed por onda.
- **Fuzz**: `cargo fuzz` em parsers (PSF, ELF, SELF, PKG, PUP).
- **Benchmarks diferenciais**: `criterion` crate; target regressão ≤ 10% por onda.
- **CI**: cada PR roda build Windows+Linux+macOS; ctest + cargo test + fuzz curto + compare_run sobre cenários P0.

## 5. Ferramentas de editor já alinhadas

- **clangd** enxerga tudo via `compile_commands.json` (flag já ligada em `CMakeLists.txt:12`).
- **rust-analyzer** toma conta do workspace `rust/` sem conflito.
- Arquivo `.clangd` na raiz para mutar warnings de `3rdparty/` (a criar antes da Onda 1).

## 6. Riscos e mitigações

| Risco | Probabilidade | Impacto | Mitigação |
|---|---|---|---|
| Overhead de FFI em hot paths (PPU exec) | alta | alto | Manter hot loop todo em Rust; FFI só em boundaries grossas. Bench antes de merge. |
| Divergência silenciosa de ordens de bits / endianness | alta | alto | `#[repr(C)]` + `static_assert` via `const _: () = assert!(...)`. Contracts GTest. |
| Ordem de `unsafe` em `vm::*` (compartilhado com JIT) | média | crítico | Isolar em wrapper com 1 `unsafe` dedicado; auditoria manual por PR. |
| Escopo criativo: refatorar enquanto porta | alta | alto | Regra dura: *copiar antes de refatorar*. Primeiro espelho 1:1, depois melhora. |
| Abandono por exaustão | alta | fatal | Marcos pequenos; cada onda entrega valor observável. |
| LLVM JIT quebrar com Rust ao lado | média | alto | Deixar JIT por último; ondas anteriores não tocam em `Utilities/JIT*.cpp`. |
| Licença GPL vs dependências Rust | baixa | médio | RPCS3 é GPLv2. Auditar licenças de crates (quase todas MIT/Apache). |

## 7. O que já foi feito (sessão 2026-04-21)

- [x] Visual Studio 2022 Build Tools instalado (MSVC + SDK + cmake)
- [x] Rust rustup migrado para `stable-x86_64-pc-windows-msvc`
- [x] Zig 0.16 + zls 0.16 instalados preventivamente
- [x] `rust/` workspace + `rust/CMakeLists.txt` com corrosion + FetchContent
- [x] `CMakeLists.txt` raiz: `option(USE_RUST_CRATES OFF)` + `add_subdirectory(rust)` condicional (byte-identical default)
- [x] `.clangd` na raiz (suprime 3rdparty/Qt noise)
- [x] `.cargo/config.toml` com `target-dir = rust/target`
- [x] **Onda 1 — 3 crates, 50 testes verdes sob MSVC:**
  - `rpcs3-utilities` — 15 testes (unit + C ABI + parity table + 2 proptest fuzz contra oráculo C++)
  - `rpcs3-config` — 21 testes (games.yml parse/emit byte-exato vs yaml-cpp, incluindo 3 filtros silenciosos)
  - `rpcs3-crypto` — 16 testes (AES-128/192/256 ECB+CBC com IV-update, SHA-1, HMAC-SHA-1, KAT FIPS-197/SP 800-38A/FIPS-180/RFC 2202)
- [x] `behavior-freeze/contracts/test_contract_*.cpp` (3 testes GTest aguardando integração em `rpcs3/CMakeLists.txt`)
- [x] `behavior-freeze/harness/{run_headless, capture_baseline, compare_run}.py`
- [x] `behavior-freeze/docs/{INVENTORY, CHECKLIST, PORT_PLAN}.md`

## 8. Playbook para modo autônomo (o que eu faço enquanto você dorme)

Lista ordenada e atômica. Cada passo é **verificável**: passa nos critérios OU falha — sem "quase pronto". Se falhar, paro naquele passo e deixo log no final do `rust/README.md` com o motivo.

### Passo 1 — Validar integração CMake+corrosion (smoke test standalone)

```bash
cd "c:/Users/manod/Downloads/Emulador Ps2, ps1 e ps3 nativos/rpcs3-master"
cmake -S rust -B build-rust-smoke -G "Visual Studio 17 2022" -A x64
cmake --build build-rust-smoke --target rpcs3_utilities_rs --config Release
```

**Critério de sucesso:** existe `build-rust-smoke/**/rpcs3_utilities.lib` (ou `rpcs3_utilities_rs.lib`) após o build.
**Em caso de falha:** anotar erro no `rust/README.md`, seguir para passo 2 assim mesmo (é bloqueante só pra integração no binário final, não para progredir nas crates).

### Passo 2 — Onda 2.a: `rpcs3-loader-psf` (PARAM.SFO parser)

Criar `rust/rpcs3-loader-psf/` espelhando o schema de `rpcs3/Loader/PSF.{h,cpp}`. Contratos a replicar:

- `psf::load(file) -> (registry, error)` com `error ∈ {ok, stream, not_psf, corrupt}`.
- Tipos de entrada: `array`, `string`, `integer` ([PSF.h](../../rpcs3/Loader/PSF.h)).
- `get_string(registry, key, default="")` e `get_integer(registry, key, default=0)`.

**Dependências:** `binrw = "0.14"` ou parser manual (decidir: binrw é mais limpo para este binário).
**Testes mínimos (12+):** magic inválido, truncated, roundtrip (parse → serialize → parse), valores conhecidos (TITLE_ID, PARENTAL_LEVEL, TITLE).
**Critério:** 12+ testes verdes.

### Passo 3 — Onda 2.b: `rpcs3-loader-elf-self` (ELF + SELF plaintext)

Nota: **somente a parte não-criptográfica por enquanto.** Decrypt pleno de SELF NPDRM exige key_vault (Onda 2 tardia). Nesta fase, cobrir:

- Parse de ELF64 big-endian PPU via `goblin = "0.8"` (feature `elf64`).
- Detectar magic `0x7F454C46` (ELF) vs `0x53434500` (SCE/SELF) e retornar enum discriminante.
- Para SELF: parsear cabeçalho SCE + metadata headers (sem descriptografar).
- Validar program headers (PT_LOAD) caem no range 0x00000000-0x30000000.

**Testes mínimos (10+):** ELF válido plaintext, SELF header parseado, magic errado, arquivo truncado.

### Passo 4 — Onda 2.c: `rpcs3-loader-self-decrypt`

Aqui junta tudo: SELF cripto → ELF plaintext. Integra `rpcs3-crypto` (AES-128 CBC) + `rpcs3-loader-elf-self`.

- `decrypt_self(bytes) -> Result<Vec<u8>, DecryptError>` mirror de `unself.cpp:1361`.
- Começar por caminho **não-NPDRM** (mais simples).
- NPDRM KLIC derivation adiado.

**Critério:** roundtrip funcional com fixture conhecida (se disponível) + teste de estrutura.

### Passo 5 — Onda 2.d: `rpcs3-loader-pkg` (PKG install)

- `rpcs3/Crypto/unpkg.cpp` + `rpcs3/Loader/` coleção de arquivos.
- PKG header parsing + descoberta de entries + extração.
- Integra com `rpcs3-crypto` (AES-CTR vai ser necessário — **adicionar `aes_crypt_ctr` à crate crypto primeiro**).

### Passo 6 — Estender `rpcs3-crypto` com funções pendentes

Antes do Passo 5, quando necessário:
- `aes_crypt_ctr` (CTR mode) — vetor NIST SP 800-38A F.5.1
- `aes_crypt_cfb128` — vetor NIST SP 800-38A F.3.13
- `aes_cmac` — RFC 4493 vetores

Adicionar em `rust/rpcs3-crypto/src/lib.rs` + KAT tests.

### Passo 7 — Relatório final

Após cada passo, atualizar:
- `rust/README.md` — tabela de crates com test counts
- `behavior-freeze/docs/CHECKLIST.md` — caixas marcadas
- Se passo falhou: log explícito no fim do README com erro + próximo diagnóstico sugerido

### Invariantes que NÃO podem ser violados

1. **Não tocar `rpcs3/` de produção** exceto os 2 lugares já tocados no `CMakeLists.txt` raiz.
2. **Não mudar default de `USE_RUST_CRATES`** — fica OFF para não quebrar build C++ atual.
3. **Não remover crates ou testes existentes** — só adicionar.
4. **Não mudar toolchain rustup** — fica MSVC.
5. **Não deletar `build-rust-smoke/` ou `rust/target/`** sem critério (são grandes mas cache é ok).
6. **Commit? Não.** O repositório não é git. Não iniciar `git init`.

### Quando parar

- Se 3 passos consecutivos falharem por razão não-trivial.
- Se chegar ao fim do Passo 7 (objetivo alcançado).
- Se um teste existente (dos 50 atuais) ficar vermelho após mudança — reverter e parar.
