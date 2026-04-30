# RPCS3 → Rust Port (behavior-freeze wave)

Port byte-exato de subsistemas do [RPCS3](https://github.com/RPCS3/rpcs3) (emulador PS3 em C++) para Rust, focado na **wave behavior-freeze**: replicar o comportamento observável do C++ tijolo a tijolo, com testes que validam paridade exata.

## Status

> **Para o status autoritativo atual, ver [`docs/PROJECT_STATUS.md`](docs/PROJECT_STATUS.md).**
>
> Os números abaixo eram a snapshot do baseline de 2026-04-24 (frozen).
> Após esse marco, foi adicionado um SPU recompiler real (Cranelift JIT)
> — ver `docs/PROJECT_STATUS.md` para a versão verificada localmente.

| Métrica | Frozen baseline (2026-04-24) | Atual |
|---------|------------------------------|-------|
| **Crates** | 230 | ver PROJECT_STATUS.md |
| **Tests `--workspace --lib`** | 5165 | ver PROJECT_STATUS.md |
| **SPU recompiler** | scaffold delegando ao interpreter | Cranelift JIT real (R4a + R4b + R4c minimal SMC) |
| **Cobertura Cell/Modules/** | 100% (136 crates HLE) | igual |

**Plano substancialmente completo** como artefato de escopo — todos os
candidatos viáveis byte-exato de `Cell/Modules/`, `Audio/`, `Io/`,
`Loader/`, `RSX/` (helpers), `NP/`, LV2, HLE foram entregues. Runtime
giants ainda fora de escopo: PPU JIT completo (LLVM/ASMJIT backends),
PPU Translator, PPU Thread/Module/Analyser, RSX Thread/VKGSRender,
System.cpp, Qt UI. O SPU recompiler está parcialmente implementado
(ver `docs/PROJECT_STATUS.md` para detalhe do que está done vs partial).

## Como rodar os testes

```bash
cd rust/
cargo test --workspace --lib
```

Resultado esperado: passa sem failures. Para a contagem exata e o status
de cada sub-suite (SPU stack, recompiler --release, etc.), ver
[`docs/PROJECT_STATUS.md`](docs/PROJECT_STATUS.md).

## Estrutura

```
rust/                         ← 230 Rust crates (workspace Cargo)
├── Cargo.toml                ← workspace members list
├── rpcs3-utilities/          ← Onda 1: folhas puras
├── rpcs3-config/
├── rpcs3-crypto/             ← AES + SHA + HMAC + CMAC com KAT NIST
├── rpcs3-loader-psf/         ← Onda 2: parsers determinísticos
├── rpcs3-loader-elf-self/
├── rpcs3-loader-pkg/
├── rpcs3-loader-pup/
├── rpcs3-loader-tar/
├── rpcs3-loader-trp/
├── rpcs3-loader-tropusr/
├── rpcs3-loader-iso/
├── rpcs3-loader-iso-cache/
├── rpcs3-loader-disc/
├── rpcs3-emu-types/          ← Onda 3: enums + ABI contracts
├── rpcs3-emu-core/           ← Onda 4: PPU+SPU MVP end-to-end
├── rpcs3-memory/
├── rpcs3-memory-backing/
├── rpcs3-cpu-thread/
├── rpcs3-ppu-thread/
├── rpcs3-spu-thread/
├── rpcs3-ppu-opcodes/
├── rpcs3-ppu-interpreter/    ← 136 tests
├── rpcs3-spu-interpreter/    ← 68 tests
├── rpcs3-vfs-paths/
├── rpcs3-vfs-mount/
├── rpcs3-lv2-*/              ← Onda 5: 14 crates LV2 syscalls
├── rpcs3-hle-sys-*/          ← Onda 6: 17 crates sysPrxForUser + sys-libc
├── rpcs3-hle-cell*/          ← Onda 7: 100+ crates HLE PS3 modules
├── rpcs3-hle-scenp*/         ← NP/PSN HLE (sceNp main = 7590 linhas → 239 entries)
├── rpcs3-audio-*/            ← Onda 8: 5 crates Audio
├── rpcs3-io-*/               ← 18 crates Io (10 dispositivos USB + 8 configs)
├── rpcs3-rsx-*/              ← 6 crates RSX (gsframe, vertex, surface, decompilers)
├── rpcs3-np-*/               ← Network/PSN
├── rpcs3-spu-mfc/            ← SPU MFC DMA opcodes
├── rpcs3-localized-string/   ← 315 UI string IDs
└── ... (230 total)

docs/                            ← Status autoritativo atual
└── PROJECT_STATUS.md            ← Verified status, R4a/R4b/R4c done, next phase

historico/                       ← Snapshots históricos
└── pre-r4b-2026-04-25/         ← Docs antes da limpeza (CHECKLIST, ROADMAP, PORT_PLAN, AUTONOMOUS_LOG full, etc.)

behavior-freeze/                 ← Wave behavior-freeze (harness, fixtures, docs ainda em uso)
├── docs/
│   ├── INVENTORY.md             ← P0/P1/P2 inventory factual
│   ├── HOMEBREW_PLAN.md         ← Plano P1+P5 homebrew real (referenciado pelo Python harness)
│   ├── DECISIONS.md             ← ADR log (point-in-time records)
│   ├── DEFERRED.md              ← Deferred items
│   ├── BACKLOG_RESIDUAL.md      ← Backlog residual ao baseline 2026-04-24
│   ├── AUTONOMOUS_LOG.md        ← Stub + telemetry section (.claude hooks dependem)
│   └── SPU_RECOMPILER_PLAN.md   ← Stub (rust source comment depende)
├── contracts/                   ← GTest contracts (cpp side)
├── harness/                     ← Python test harness
└── fixtures/
```

## Highlights de crypto byte-exato real

Três dispositivos USB emulados portados com cipher byte-exato verificável:

- **`rpcs3-io-guncon3`** — Namco GunCon 3 light-gun: 256-byte `KEY_TABLE` + 3-round per-byte cipher com op selecionada por `KEY_TABLE[k] & 3`
- **`rpcs3-io-dimensions`** — LEGO Dimensions toypad: TEA cipher (delta 0x9E3779B9, 32 rounds) + Bob Jenkins small-noise PRNG (init_a 0xF1EA5EED, 42 warmup rounds, rotl 21/19/6) + figure-key derivation com `scramble`/`randomize`
- **`rpcs3-io-infinity`** — Disney Infinity Base: SHA1 + AES-128 ECB key derivation + 64-bit `SCRAMBLE_MASK = 0x8E55AA1B3999E8AA` bit-twiddling + Jenkins variant (23 warmup rounds vs Dimensions' 42)

## Cobertura por área

| Área | Crates | Status |
|------|--------|--------|
| `rpcs3/Emu/Cell/Modules/` (HLE PS3) | 136 | **100%** |
| `rpcs3/Emu/Audio/` | 5 | utils/resampler/dumper/backend/enumerator |
| `rpcs3/Emu/Io/` | 18 | 10 USB devices + 8 configs |
| `rpcs3/Loader/` | 10 | psf/elf/pkg/pup/mself/tar/trp/tropusr/iso/iso-cache/disc |
| `rpcs3/Emu/RSX/` | 6 | gsframe + vertex/texture/surface utils + GL/VK decompilers |
| `rpcs3/Emu/NP/` | 2 | countries + upnp-config |
| LV2 syscalls (`Emu/Cell/lv2/`) | 14 | + 17 sys-* user-side |
| Misc (version, perf, IPC, util) | ~10 | |

## Fora de escopo desta wave (gigantes de runtime)

Cada item abaixo é um projeto dedicado de semanas. **Nem todos seguem fora de escopo** —
o SPU recompiler em particular saiu da lista após a wave R1..R4c (ver `docs/PROJECT_STATUS.md`).

Status atualizado:

- `SPUCommonRecompiler.cpp` (9792L) — JIT x86 backend → **port em andamento**
  via `rpcs3-spu-recompiler` com Cranelift (~102 opcodes JIT-cobertos,
  R4a dispatcher, R4b chained patching, R4c SMC scan).
- `SPULLVMRecompiler.cpp` (9497L) — JIT LLVM backend → ainda fora de escopo;
  reservado para R5+ se Cranelift mostrar gargalo.
- `SPUASMJITRecompiler.cpp` (4878L) — ASMJIT legacy → não será portado
  (substituído por Cranelift).
- `PPUInterpreter.cpp` (7888L) — fora de escopo (contract stub em `rpcs3-ppu-interpreter`).
- `SPUInterpreter.cpp` (3363L) — substituído por `rpcs3-spu-interpreter` Rust real (~70% ISA).
- `PPUThread.cpp` (5684L), `SPUThread.cpp` (7488L) — runtime threads, fora de escopo.
- `PPUTranslator.cpp` (5594L), `PPUAnalyser.cpp` (3278L), `PPUModule.cpp` (3254L) — PPU JIT tooling, fora de escopo.
- `System.cpp` (4823L) — Emulator singleton, fora de escopo.
- `RSXThread.cpp` (3675L), `VKGSRender.cpp` (3009L) — GPU runtime, fora de escopo.
- `rpcs3qt/**` — Qt UI (framework-specific), fora de escopo.

Contract stubs em `rpcs3-ppu-interpreter`/`rpcs3-ppu-thread` cobrem o
suficiente para a wave behavior-freeze em PPU. SPU foi além do stub —
ver `docs/PROJECT_STATUS.md` SPU recompiler section.

## Bloqueado por dependências externas

- `rpcs3-loader-self-decrypt` — precisa fixtures SELF reais + `key_vault` PS3 (implicações legais de copyright + console keys)

## Setup local

Pré-requisitos:
- Rust **1.82+** (MSVC toolchain no Windows)
- `cargo` (vem com Rust)

```bash
git clone https://github.com/rafaelfsilva2002/rpcs3---rust-port.git
cd rpcs3---rust-port/rust
cargo build --workspace
cargo test --workspace --lib
```

Para uma única crate:
```bash
cargo test -p rpcs3-io-dimensions --lib
```

## Documentação

- [`docs/PROJECT_STATUS.md`](docs/PROJECT_STATUS.md) — **Status autoritativo atual** (testes verificados localmente, R4a/R4b/R4c done, próxima fase recomendada).
- [`behavior-freeze/docs/INVENTORY.md`](behavior-freeze/docs/INVENTORY.md) — Inventário P0/P1/P2 (estável, fatos por crate).
- [`behavior-freeze/docs/HOMEBREW_PLAN.md`](behavior-freeze/docs/HOMEBREW_PLAN.md) — Plano para validação diferencial vs RPCS3 com homebrew real (P1+P5 ainda bloqueados).
- [`historico/pre-r4b-2026-04-25/`](historico/pre-r4b-2026-04-25/) — Snapshots históricos completos dos docs antes da limpeza. Inclui `CURRENT_STATE.md`, `CHECKLIST.md`, `ROADMAP.md`, `PORT_PLAN.md`, `AUTONOMOUS_LOG.md` (versão completa com 230+ iters), `SPU_RECOMPILER_PLAN.md`, frozen snapshots `*_2026-04-24.md`, e arquivos `.bak` originais.
- `behavior-freeze/docs/AUTONOMOUS_LOG.md` e `behavior-freeze/docs/SPU_RECOMPILER_PLAN.md` — mantidos no caminho original como stubs porque hook do Claude Code (Stop/SessionStart) e doc-comment do Rust source (`rust/rpcs3-spu-recompiler/src/lib.rs`) dependem do path exato. Conteúdo completo está em `historico/`.

## Licença

GPL-2.0-only — mesma licença do RPCS3 upstream.
