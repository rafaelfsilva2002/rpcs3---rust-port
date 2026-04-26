# Fixtures

Esta pasta documenta as fixtures externas necessárias. **Nenhum binário de jogo comercial deve ser commitado aqui.**

## O que é necessário e por quê

### F1. Homebrew PPU mínimo (para diferencial de PPU)
- Formato: ELF PPU 64-bit descriptografado (SELF plaintext ok).
- Tamanho alvo: < 256 KB.
- Comportamento: escreve uma assinatura conhecida em um offset fixo da RAM, depois sai com exit code 0.
- Uso: rodar `rpcs3 --headless <elf>` e comparar dump de GPR/log final.
- Fonte sugerida: `ps3autotests`, `ps3tests-cpu`, ou um mini-crt0 custom.

### F2. Homebrew SPU mínimo (para diferencial de SPU)
- Formato: ELF SPU standalone (32-bit, big-endian, EM_SPU=0x17).
- Comportamento: roda sequência conhecida, escreve em LS[0], sinaliza done via `stop`.
- Uso: dump de {GPR, LS, PC} via `rust/spu-runner` + diff via `harness/spu_homebrew_runner.py`.

**Já commitados** (8 fixtures sintéticos, gerados por `harness/build_synthetic_fixtures.py`):

| Fixture | Bytes | Insts | Exercises |
|---|---:|---:|---|
| `spu/synthetic_il_stop.elf` | 92 | 2 | sentinel — il + stop |
| `spu/synthetic_arith.elf` | 120 | 9 | il + a + sf + shli + xor + or + and + stop |
| `spu/synthetic_loop.elf` | 116 | 8 | ila + ai + ceqi + brnz + br back-edge → soma 1..10 = 55 |
| `spu/synthetic_float_dot.elf` | 112 | 7 | il + shli + fa + fm chain → produz float 8.0 |
| `spu/synthetic_loadstore.elf` | 104 | 5 | il + ila + stqd + lqd round-trip via LS |
| `spu/synthetic_shifts.elf` | 112 | 7 | shli + rotmi + rotmai + roti exercitam toda família word-shift |
| `spu/synthetic_brsl_ret.elf` | 108 | 6 | brsl + bi (link register) — function call style |
| `spu/synthetic_orx_collapse.elf` | 104 | 5 | il + ah (halfword add) + orx (collapse word lanes) |
| `spu/synthetic_halfword_shifts.elf` | 104 | 5 | ilh + shlhi + rothmi + rothi → exercita toda família halfword-shift |

Para regenerar: `python behavior-freeze/harness/build_synthetic_fixtures.py`. Para upgrade a homebrew **real**, seguir P1 em `docs/HOMEBREW_PLAN.md`.

### F3. PARAM.SFO válido e dois malformados
- Um válido com TITLE_ID, TITLE, PARENTAL_LEVEL (≤ 4 KB).
- Um com magic ruim.
- Um truncado (primeiros 64 bytes apenas).
- Uso: alimentar `psf::load` no GTest.

### F4. SELF pequeno não-NPDRM (para unself.cpp)
- Homebrew SELF assinado mas sem KLIC (NPDRM=false).
- Comportamento: `decrypt_self` retorna o ELF plaintext byte-a-byte igual ao fixture de controle.
- Uso: GTest compara SHA-256 do output.

### F5. PKG mínimo
- PKG sem DRM ou com `.rap` publicamente disponível.
- Uso: fluxo de install, comparação de árvore extraída.

### F6. .rrc de RSX capture
- Gerar uma vez rodando `rpcs3 --rsx-capture=out.rrc <homebrew-gráfico>`.
- Uso: replay + frame hash.

## Convenção de caminho

```
fixtures/
├── ppu/hello.elf                   # F1
├── spu/hello.elf + hello.spu       # F2
├── psf/valid.sfo, bad_magic.sfo, truncated.sfo   # F3
├── self/non_npdrm_hello.self, non_npdrm_hello.elf.expected  # F4
├── pkg/sample.pkg                  # F5
└── rrc/hello_graphics.rrc          # F6
```

## Política de versionamento

- Fixtures **pequenas (< 1 MB cada)** podem ser commitadas diretamente.
- Fixtures maiores devem ser geradas via script `fixtures/build_all.sh` (a escrever na Fase 2) ou baixadas de releases do repositório (a decidir).
- Nunca comitar binários protegidos por direitos autorais.
