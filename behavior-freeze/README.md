# RPCS3 Behavior Freeze — Rede de segurança pré-reescrita Rust

Esta pasta existe para **congelar o comportamento observável do RPCS3 C++** antes de qualquer reescrita em Rust. Ela **não altera** o código de produção.

## Princípios

1. **Oráculo temporário**: a build atual do RPCS3 é o padrão-ouro. A versão Rust deve reproduzir o mesmo comportamento observável para os cenários listados em [docs/INVENTORY.md](docs/INVENTORY.md).
2. **Black-box primeiro**: preferimos contratos externos (CLI, logs, frame hash, WAV, save files) a helpers internos.
3. **Diferencial**: cada teste gera um artefato capturável (log canônico, golden hash, estado final) que o binário Rust terá de bater.
4. **Separação**: zero mudança em `rpcs3/` de produção nesta fase — apenas `tests/` que já existe ganha arquivos `test_contract_*.cpp` novos, compilados só quando `-DBUILD_RPCS3_TESTS=ON`.

## Layout

```
behavior-freeze/
├── README.md                  # Este arquivo
├── docs/
│   ├── INVENTORY.md           # Inventário P0/P1/P2 com file:line anchors
│   ├── HOMEBREW_PLAN.md       # Plano homebrew real (P1+P5)
│   ├── DECISIONS.md           # ADR log point-in-time
│   ├── DEFERRED.md / BACKLOG_RESIDUAL.md  # Items deferidos/residuais
│   ├── AUTONOMOUS_LOG.md      # Stub + telemetry (.claude hooks)
│   └── SPU_RECOMPILER_PLAN.md # Stub (rust src doc-comment)
├── harness/
│   ├── run_headless.py         # Wrapper CLI --headless + captura de saída
│   ├── capture_baseline.py     # Roda um cenário e salva em baselines/
│   ├── compare_run.py          # Roda novamente e compara com baseline
│   └── lib/
│       ├── log_parser.py       # Canonicaliza RPCS3.log (remove timestamps, tid, caminhos)
│       └── frame_hash.py       # Hash estável de frames capturados
├── baselines/                  # Golden outputs (commit OU LFS — ver docs)
│   └── .gitkeep
├── fixtures/
│   └── README.md               # Especificação dos homebrews/PKGs/SELFs necessários
└── contracts/                  # Testes GTest de contrato plugados no rpcs3_test
    ├── README.md
    ├── test_contract_game_boot_result.cpp
    ├── test_contract_system_state.cpp
    └── test_contract_cpu_flag.cpp
```

## Como rodar (visão geral)

```bash
# 1. Construa com testes:
cmake -S . -B build -DBUILD_RPCS3_TESTS=ON
cmake --build build --target rpcs3_test
ctest --test-dir build --output-on-failure

# 2. Teste diferencial end-to-end (requer build do rpcs3.exe + fixtures):
python behavior-freeze/harness/capture_baseline.py --scenario=boot_no_fw
python behavior-freeze/harness/compare_run.py --scenario=boot_no_fw
```

Veja [docs/INVENTORY.md](docs/INVENTORY.md) para a lista completa do que congelar e [../docs/PROJECT_STATUS.md](../docs/PROJECT_STATUS.md) para o status atual da execução. O checklist operacional pré-cleanup está preservado em [../historico/pre-r4b-2026-04-25/CHECKLIST.md](../historico/pre-r4b-2026-04-25/CHECKLIST.md).
