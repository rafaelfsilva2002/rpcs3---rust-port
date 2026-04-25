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
- Formato: ELF SPU standalone, carregado via PPU trampolim curto.
- Comportamento: roda sequência conhecida, escreve em LS[0], sinaliza done via out_mbox.
- Uso: dump de LS + canais.

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
