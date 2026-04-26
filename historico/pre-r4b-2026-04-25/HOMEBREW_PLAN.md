# Homebrew validation plan — SPU first

**Status:** **P3 + P4 done; pipeline self-test green.** `spu-runner` binário implementado, `spu_homebrew_runner.py --diff` funcional. Smoke test ELF sintético commitado em `fixtures/spu/synthetic_il_stop.elf`.
**Owner:** próxima fase = P1 (fixture homebrew real) + P2 (extensão de loader se necessário) + P5 (RPCS3 dump capture).
**Bloqueado por:** apenas P5 (RPCS3 standalone SPU dump capture) e P1 (homebrew real). P2/P3/P4 ✅.

## Por que SPU homebrew vem antes de PPU homebrew

1. **SPU é hermético:** roda sem syscalls, LV2, ou GPU. O contrato observável é {GPRs, LS, channels} ao terminar.
2. **PPU exige todo o resto:** memória virtual, syscalls LV2, FS virtual, módulos HLE. Validar PPU contra binário real exigiria 5+ subsistemas verdes simultaneamente.
3. **SPU já tem 89 instruções implementadas** (cobre subset iter-1 + float compares + branches indiretos + LQX/STQX). PPU também está completo no interpretador, mas a *plataforma ao redor* dele não.

## Pré-requisitos antes de rodar o harness

### P1. Fixture: SPU ELF homebrew mínimo
- **O quê:** ELF SPU ≤ 256 KB, standalone (não dependente de PPU runtime).
- **Comportamento esperado:** sequência determinística, escreve assinatura conhecida em LS[0..16], chama `stop` com código 0.
- **Onde achar:**
  - `ps3autotests/spu/` — bateria de testes públicos da scene
  - `ps3tests-cpu` — alternative
  - Custom: escrever um SPU asm de 5-10 linhas em `spu-as` (binutils), assemblar para ELF
- **Onde commitar:** `behavior-freeze/fixtures/spu/hello.elf` (≤ 1 MB → pode ir no repo)

### P2. Loader ELF SPU — ✅ **REUSE** (não precisou de crate novo)
- `rpcs3-loader-elf-self::parse_elf` (via `goblin`) já lê ELF SPU 32-bit BE corretamente.
- `ElfInfo::is_spu()` discrimina via `e_machine == EM_SPU (0x17)`.
- `pt_load_iter()` enumera segmentos PT_LOAD para deploy no LS.

### P3. Crate `spu-runner` (binário) — ✅ **DONE**
- `rust/spu-runner/` — CLI ~170 LOC, integration tests 5/5.
- Lê ELF via P2, popula `SpuThread.ls`, roda `run_n` até `Stop` / step limit / erro.
- Dumps em `--out-dir`:
  - `gpr.csv` — 128 linhas `r{i},{u128_hex}`
  - `pc.txt` — entry + final PC
  - `ls.bin` — 256 KB raw
  - `summary.txt` — steps + outcome
- Exit codes: 0 (Stop), 1 (max-steps), 2 (interpreter err), 3 (load err), 64 (CLI err).

### P4. Harness Python (`spu_homebrew_runner.py`) — ✅ **DONE**
- Modo 1 (run + dump único), Modo 2 (com RPCS3 — placeholder até P5), Modo 3 (`--diff` entre dois dumps).
- `diff_dumps()` compara GPR linha-a-linha + PC + LS em chunks de 16 bytes (reporta primeiros 5).
- Self-test em `harness/test_spu_homebrew_runner.py` valida pipeline completo: ELF sintético → dump A → dump B → IDENTICAL.
- Fixture commitada: `fixtures/spu/synthetic_il_stop.elf` (92 bytes, `il r3,0x1234; stop 0`).

### P5. RPCS3 C++ binary com flag headless
- RPCS3 já suporta `--headless`. Validar que aceita SPU ELF standalone (provavelmente sim via `--spu-test`).
- **Ou:** modificar harness para extrair traço de execução de RPCS3 via `rpcs3 --replay` se o standalone não funcionar.

## Critério de sucesso para a fase

✅ **Mínimo viável:** 1 SPU ELF homebrew roda nos dois interpretadores e produz o mesmo {GPR, LS} final.

✅ **Forte:** 5+ ELFs (uma bateria de smoke test) passam; cobertura de cada família de instrução SPU é exercida.

✅ **Ouro:** harness automaticamente diffa cada commit (CI-style) — qualquer regressão dispara alerta.

## O que **não** está nesse plano (e por quê)

- **Performance:** validação de correção primeiro. Hot-path otimização depois.
- **Recompilador SPU:** depende dessa fase estar ✅ Forte (precisamos de um interpretador de oráculo robusto).
- **Save state binário:** RPCS3 tem formato proprietário `.savestat`. Harness independente é mais útil pra nosso ciclo.
- **Jogos comerciais:** legal/decryption + escala muito além do nosso atual.

## Caminho mínimo para "primeira luz"

Estimativa de esforço (ULTRATHINK realista, sem multiplicador defensivo):

1. **Achar/escrever 1 SPU ELF homebrew:** 30 min – 2 h (depende de quão acessível é ps3autotests)
2. **Crate `rpcs3-spu-elf-loader`:** 1 – 2 h
3. **Crate `spu-runner` binário:** 1 – 2 h
4. **Implementar harness Python:** 2 – 4 h (subprocess + diff + report)
5. **Validar contra RPCS3 C++:** 2 – 6 h (provavelmente vai expor 1-3 bugs em opcodes — debugar)

**Total:** 6 h (otimista) – 16 h (com bugs realistas) = **1-2 dias de trabalho focado**.

A maior incerteza é **passo 5**: a primeira diff vai expor onde nosso interpretador divergiu do C++. Cada bug encontrado validade a abordagem.

## Próxima ação concreta (quando o usuário decidir avançar)

P3/P4 estão prontos. O caminho restante:

1. **P1:** Buscar `ps3autotests` no GitHub (homebrew SPU real). Se acessível, baixar e copiar para `fixtures/spu/`. Se não, escrever um SPU asm via `spu-as` (binutils) e commitar o ELF.
2. **P5:** Capturar dump equivalente do RPCS3 C++. Opções:
   - Adicionar pequeno patch ao RPCS3 expondo `--spu-test <ELF>` que dumps `{gpr,ls,pc}` no mesmo formato do nosso `spu-runner`.
   - Ou: rodar SPU via PPU process e dumpar via gdb / save state.
3. **Validação:** rodar `python spu_homebrew_runner.py --elf fixtures/spu/<homebrew>.elf --rust-runner ... --rpcs3-binary ... --output baselines/spu_<name>/` e ler `baselines/spu_<name>/diff.txt`. Cada divergência = bug no nosso interpretador (ou no nosso loader).
