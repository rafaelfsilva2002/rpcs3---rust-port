# RPCS3 → Rust Port (behavior-freeze wave)

Port byte-exato de subsistemas do [RPCS3](https://github.com/RPCS3/rpcs3) (emulador PS3 em C++) para Rust, focado na **wave behavior-freeze**: replicar o comportamento observável do C++ tijolo a tijolo, com testes que validam paridade exata.

## Status

| Métrica | Valor |
|---------|-------|
| **Crates** | **230** |
| **Tests verdes** | **5165** |
| **Iterações autônomas** | **229** |
| **Regressões** | **0** |
| **Cobertura Cell/Modules/** | **100%** (136 crates HLE) |

🎉🎉🎉🎉 **Plano substancialmente completo** — todos os candidatos viáveis byte-exato cobertos. Os giants de runtime (SPU/PPU Recompilers, RSX Thread, VKGSRender, Qt UI) estão fora de escopo desta wave por design (cada um é um projeto dedicado de semanas).

## Como rodar os testes

```bash
cd rust/
cargo test --workspace --lib
```

Esperado: `5165 passed; 0 failed`.

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

behavior-freeze/              ← Documentação da wave
├── docs/
│   ├── CHECKLIST.md          ← Estado por wave + plano residual
│   ├── AUTONOMOUS_LOG.md     ← Trilha auditável das 229 iters
│   ├── INVENTORY.md          ← P0/P1/P2 inventory
│   └── PORT_PLAN.md          ← Decision matrix 8D
├── contracts/                ← GTest contracts (cpp side)
├── harness/                  ← Python test harness
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

Cada item abaixo é um projeto dedicado de semanas — não foi tentado nesta wave linear:

- `SPUCommonRecompiler.cpp` (9792L) — JIT x86 backend
- `SPULLVMRecompiler.cpp` (9497L) — JIT LLVM backend
- `SPUASMJITRecompiler.cpp` (4878L) — ASMJIT legacy backend
- `PPUInterpreter.cpp` (7888L), `SPUInterpreter.cpp` (3363L) — runtime
- `PPUThread.cpp` (5684L), `SPUThread.cpp` (7488L) — runtime threads
- `PPUTranslator.cpp` (5594L), `PPUAnalyser.cpp` (3278L), `PPUModule.cpp` (3254L) — PPU JIT tooling
- `System.cpp` (4823L) — Emulator singleton
- `RSXThread.cpp` (3675L), `VKGSRender.cpp` (3009L) — GPU runtime
- `rpcs3qt/**` — Qt UI (framework-specific)

Contract stubs suficientes pra behavior-freeze wave em `rpcs3-ppu-interpreter`/`rpcs3-spu-interpreter`/`rpcs3-ppu-thread`/`rpcs3-spu-thread`.

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

- [`behavior-freeze/docs/CHECKLIST.md`](behavior-freeze/docs/CHECKLIST.md) — Estado de cada wave + plano residual + lista do que está fora de escopo
- [`behavior-freeze/docs/AUTONOMOUS_LOG.md`](behavior-freeze/docs/AUTONOMOUS_LOG.md) — Trilha auditável das 229 iterações (cada iter = 1 entry)
- [`behavior-freeze/docs/INVENTORY.md`](behavior-freeze/docs/INVENTORY.md) — Inventário P0/P1/P2
- [`behavior-freeze/docs/PORT_PLAN.md`](behavior-freeze/docs/PORT_PLAN.md) — Decision matrix 8D para crate-a-crate

## Licença

GPL-2.0-only — mesma licença do RPCS3 upstream.

## Co-autoria

Este port foi desenvolvido em sessões assistidas por Claude (Anthropic), com 229 iterações autônomas consecutivas em ZERO regressões. Decisões arquiteturais, validação byte-exata e testes finais sob supervisão humana.
