# SPU PPU↔SPU trace fixtures

Esta pasta abriga **traces JSONL capturados de execuções reais sob RPCS3 patcheado** (R5.8 A.3). Cada trace é uma sequência determinística de eventos PPU↔SPU (mailbox, sinal, park, wake, stop, final_state) gerada pelo writer C++ em [`rpcs3/Emu/Cell/SPUTraceJsonl.{h,cpp}`](../../../../rpcs3/Emu/Cell/) e consumida pelo pipeline Rust em [`rust/rpcs3-spu-differential/src/trace_fmt.rs`](../../../../rust/rpcs3-spu-differential/src/trace_fmt.rs).

**Status atual:** **REPLAY-VALIDATED FIXTURE EXISTS (2026-04-29).** R5.9e.7 LANDED — `single_spu_mailbox_v1.jsonl` (5 events, 1.1 KB) é o primeiro trace replay-validated do projeto, com companion `single_spu_mailbox_v1.notes.md` (provenance) e content-addressed `.spuimg` side-file em `behavior-freeze/fixtures/spu/images/68cf203b…abac43.spuimg` (262 KB). Captured 2026-04-29 from `rpcs3.exe` (R5.9c+R5.9e.3 writer) executando uma homebrew CC0 PSL1GHT autoral compilada via from-source `ps3toolchain` em container Docker `debian:bookworm-slim`. A flag `REPLAY_VALIDATED_TRACE_EXISTS` em [`behavior-freeze/harness/check_trace_fixtures.py`](../../../harness/check_trace_fixtures.py) está `True`. Acceptance gate: [`rust/rpcs3-spu-recompiler/tests/single_spu_mailbox_v1_replay.rs`](../../../../rust/rpcs3-spu-recompiler/tests/single_spu_mailbox_v1_replay.rs) — drives the FULL pipeline (parse → transform → build → replay × Interpreter + replay × Recompiler) e asserta `diff_snapshots(...).is_identical()` no final snapshot. Provenance completa + lista dos engine-side fixes co-landed (3 fixes gerais não-single-fixture) em [`single_spu_mailbox_v1.notes.md`](./single_spu_mailbox_v1.notes.md).

**Diagnostic-only traces fora desta pasta:** o trace v3 (`spurs_test_v3_real.jsonl` — R5.9d-era multi-SPU SPURS) e o trace v4 (`spurs_test_v4_real.jsonl` — R5.10a..p ISA-coverage iteration) vivem em [`rust/rpcs3-spu-differential/tests/data/`](../../../../rust/rpcs3-spu-differential/tests/data/) e NÃO em `behavior-freeze/fixtures/`. São DMA-bound no protocol layer (`pc=0x74C wrch ch16 (MFC_LSA)` no v4) — per [`docs/SPU_TRACE_R5_9E_REPLAY_PLAN.md`](../../../../docs/SPU_TRACE_R5_9E_REPLAY_PLAN.md) § D.1, replay NÃO pode progredir além do MFC boundary sem (a) full DMA infrastructure, (b) writer that captures MFC events as oracle inputs (R5.9f deferred), ou (c) homebrew non-DMA (caminho que R5.9e.7 tomou). Diagnostic traces são `#[ignore]`d em `tests/real_trace_diagnostic.rs` — surfaced as ISA-coverage signals, NÃO promovidos a byte-identical contract.

**Nenhum trace fabricado** vai aqui. Hand-authoring um JSONL sintético "como se" tivesse vindo do C++ derrota o propósito do harness — o trace existe para ser uma fonte oracular de verdade contra a qual o stack Rust é validado. Se o trace fosse fabricado para passar, ele não detectaria nada. A pasta cresce somente via captura empírica adicional.

(Para fixtures sintéticas que NÃO fingem ser capturas reais, ver `R5_6_REFERENCE_JSONL` em `rust/rpcs3-spu-differential/src/trace_fmt.rs` — explicitamente marcada como "synthetic, hand-derived from R5.6".)

## Critérios de aceitação para NOVOS traces replay-validated (post R5.9e.7)

R5.9e.7 entregou o primeiro replay-validated fixture (`single_spu_mailbox_v1`). Para adicionar um novo trace replay-validated a esta pasta, confirmar TODOS os critérios abaixo. **Nada além de replay-validated traces vai aqui** — diagnostic-only traces ficam em `rust/rpcs3-spu-differential/tests/data/`.

1. **Origem do homebrew:** autoral, public-domain, ou explicitamente redistribuível (license-clean). Nunca conteúdo comercial / copyrighted / extraído de jogo PS3 publicado. Source CC0/MIT do homebrew commitado em `behavior-freeze/fixtures/spu/sources/<homebrew>/` com LICENSE.md (mesmo padrão que `single_spu_mailbox_v1` segue).
2. **Boota no RPCS3 instrumentado:** `rpcs3.exe` produzido a partir de `scaffolding patch + runtime_hooks patch` aplicados sobre upstream master, com sha256s pinned em `check_patch_separation.py`. Sem fork-forks fazendo emulação custom. **R5 closure pinned os hashes** — qualquer captura nova precisa rodar contra esses mesmos patches ou justificar a divergência em PR.
3. **Cria SPU thread group** (ou fluxo equivalente Raw SPU MMIO) — homebrew puramente PPU não satisfaz porque não dispara nenhum dos 8 hook events implementados.
4. **Exerce mailbox PPU↔SPU OU signal OU sequência observável:** o trace deve conter pelo menos um par balanceado dos eventos `ppu_push_inmbox` / `spu_rdch` (ou `spu_wrch` / `ppu_pop_outmbox`, ou `ppu_signal`). Trace só com `spu_stop` + `final_state` é tecnicamente válido mas exercita só 2 de 8 hooks — fica como fixture parcial documentado, não como reference oracle.
5. **Sem DMA (não-MFC):** zero `spu_wrch` events em ch21 (`MFC_Cmd`). Per § D.1 do `SPU_TRACE_R5_9E_REPLAY_PLAN.md`, replay não suporta DMA até R5.9f / R5.12 landar. Traces DMA-bound vão para `tests/data/` como diagnostic-only, não aqui.
6. **Gera `.jsonl` real:** writer C++ produz o file via `RPCS3_SPU_TRACE_JSONL=<path>` env var; arquivo nunca é editado manualmente após captura. Para auto-flush correto na destructor do trace writer, "Exit RPCS3 when process finishes: true" precisa estar habilitado em `R:\bin\config\config.yml` — ver `enable_autoexit_and_capture.cmd` em `single_spu_mailbox_v1`'s source dir como template.
7. **Companion `.notes.md` obrigatório:** documenta origem (homebrew + license), toolchain usado para build, comando exato de captura, comportamento esperado em uma linha, status dos engine-side fixes (se algum precisou landar). Ver `single_spu_mailbox_v1.notes.md` como referência canônica.
8. **Side-file `.spuimg` no layout centralizado:** `behavior-freeze/fixtures/spu/images/<sha256>.spuimg` (NÃO `<trace>.images/<sha>.spuimg` — alternate layout reservado para diagnostic traces). Hash sha256 do file MUST bater com o `image_sha256` field do evento `spu_image` no JSONL.
9. **Pipeline Rust passa fim-a-fim com `diff_snapshots(...).is_identical()`:**
   - `parse_jsonl_trace()` retorna `Ok(Vec<CapturedEvent>)`;
   - `captured_events_to_traces_per_spu()` retorna `Ok(BTreeMap<u32, Vec<TraceEvent>>)` — exatamente 1 entry para single-SPU traces;
   - `build_spu_program_from_captured_image()` retorna `Ok(SpuProgram)` (hash + size + entry_pc validados);
   - `replay_per_spu_traces::<InterpreterExecutor>(...)` retorna `Ok(BTreeMap<u32, TraceReplayReport>)` com `final_event_kind = Finished{stop_code}` esperado;
   - `replay_per_spu_traces_with(..., |_| RecompilerExecutor::new())` retorna o mesmo;
   - **`diff_snapshots(interp.final_snapshot, jit.final_snapshot).is_identical()` retorna `true`.** PC, GPRs, LS, channels, park_state — todos devem casar byte-by-byte. `total_steps` legitimamente difere entre backends e é EXCLUÍDO do contrato (per R5.9e.7 acceptance gate test).
10. **Acceptance test commitado:** novo file em `rust/rpcs3-spu-recompiler/tests/<homebrew>_replay.rs` chamado pelo CI, com mesmo shape do `single_spu_mailbox_v1_replay.rs`. Sem teste novo, o trace não está realmente sendo verificado.

Se algum estágio falhar, **preservar o trace as-is** — falha aponta para um real correctness gap entre o stack Rust e o C++. NÃO weakenar assertions Rust, NÃO editar o `.jsonl`, NÃO regenerar o trace para "passar". Engine-side fixes para gaps semantic (como os 3 que landed em R5.9e.7) são aceitáveis e devem ser GERAIS — não single-fixture hacks.

## Hard rules permanentes

- ✅ Nunca commitar `.jsonl` fabricado / hand-authored.
- ✅ Nunca commitar trace de jogo PS3 comercial.
- ✅ Nunca editar trace manualmente após captura (single-byte mods quebram o oracle property).
- ✅ Nunca enfraquecer Rust pipeline para aceitar trace divergente — divergence = real bug a investigar.
- ✅ Nunca aceitar trace gerado por RPCS3 com modificações além de `scaffolding + runtime_hooks` patches tracked — qualquer outra patch invalida a chain-of-custody.

## Schema e formato

- **Wire format:** JSONL (newline-delimited JSON), uma linha por evento, UTF-8.
- **Schema completo:** [`docs/SPU_TRACE_CAPTURE.md`](../../../../docs/SPU_TRACE_CAPTURE.md).
- **Integration patch (C++):** [`docs/SPU_TRACE_CAPTURE_PATCH.md`](../../../../docs/SPU_TRACE_CAPTURE_PATCH.md).
- **Runtime-hooks application guide:** [`docs/SPU_TRACE_CAPTURE_RUNTIME_HOOKS.md`](../../../../docs/SPU_TRACE_CAPTURE_RUNTIME_HOOKS.md).
- **Validação (Rust):** [`rust/rpcs3-spu-differential/src/trace_fmt.rs`](../../../../rust/rpcs3-spu-differential/src/trace_fmt.rs).
- **R5.9e.1 SPU image side-files** (replay prerequisite, schema doc-only landed 2026-04-28; writer/parser implementation in R5.9e.2/.3): traces that ship `spu_image` events reference content-addressed `.spuimg` files. For committed fixtures (replay-validated), the `.spuimg` side-files live at [`behavior-freeze/fixtures/spu/images/<sha256>.spuimg`](../images/) — sibling directory to `traces/`. Per-trace `<trace>.images/<sha>.spuimg` is the alternate layout for local-only diagnostic traces. Layout choice is per-trace and documented in the trace's `.notes.md` (`external-image: <sha256> @ <path>` directive). See `docs/SPU_TRACE_CAPTURE.md` § "R5.9e.1 — SPU image metadata + side-file layout" for the full schema and unsupported-cases catalogue. **No `.spuimg` files exist yet** in this repository — the directory is created at the moment of the first replay-validated fixture commit (R5.9e.7 deliverable).

## Convenção de naming

R5.9e.7 estabeleceu o padrão canonizado: **`<homebrew>_v<N>`** (versionado, sem sufixo de commit/data — provenance vai no `.notes.md`).

```
behavior-freeze/fixtures/spu/traces/
├── <homebrew>_v<N>.jsonl
└── <homebrew>_v<N>.notes.md
```

Exemplo (canônico, R5.9e.7):

```
single_spu_mailbox_v1.jsonl     ← 5 events, 1.1 KB
single_spu_mailbox_v1.notes.md  ← provenance + engine-fixes log
```

`<homebrew>` é descritivo do escopo (`single_spu_mailbox` = single-SPU mailbox handshake; `dual_spu_signal` seria signals em 2 SPUs; etc.). `v<N>` reserva espaço para variantes incrementais que exercitam features adicionais sem invalidar o trace anterior.

Image side-files seguem layout content-addressed centralizado em `behavior-freeze/fixtures/spu/images/<sha256>.spuimg` — multiple traces que carregam o mesmo SPU image bytes deduplican naturalmente.

O arquivo `.notes.md` companion descreve (ver `single_spu_mailbox_v1.notes.md` como referência canônica):

- Origem do homebrew (autoral CC0 / disponível publicamente / outro license-clean).
- Toolchain usado para build (compiler versions, flags load-bearing, container hash se hermético).
- Versão do RPCS3 + sha256 dos hooks aplicados (deve bater com `check_patch_separation.py`).
- Procedimento exato de captura (.cmd / shell script comitado em `behavior-freeze/fixtures/spu/sources/<homebrew>/`).
- Comportamento esperado do programa (resumo em uma linha — útil para debug se replay falhar).
- Decoded trace contents (lista de eventos para inspeção rápida sem precisar abrir o JSONL).
- Status dos testes Rust contra esse trace: parser ok / transformer ok / interp replay ok / JIT replay ok / `diff_snapshots(...).is_identical()` ok.
- Quaisquer engine-side fixes co-landed para casar a captura com o replay (com justificativa de que são GERAIS, não single-fixture hacks).

## Procedimento de captura

Resumo (detalhado em [`docs/SPU_TRACE_CAPTURE_PATCH.md`](../../../../docs/SPU_TRACE_CAPTURE_PATCH.md) § "Capture procedure"):

```bash
# 1. Aplicar o integration patch e adicionar SPUTraceJsonl.{h,cpp} ao build.
# 2. Buildar RPCS3.
# 3. Rodar com captura habilitada:
export RPCS3_SPU_TRACE_JSONL=/tmp/out.jsonl
./rpcs3 --headless /caminho/para/homebrew.elf

# 4. Smoke-test do JSONL via Rust:
cd rust/rpcs3-spu-differential
# (exemplo opcional `parse_jsonl_trace` ainda não commitado — TBD em A.3 final)
cargo test -p rpcs3-spu-differential --lib trace_fmt::tests::parse_

# 5. Commitar no destino com naming acima.
```

## Política de versionamento e licenciamento

- **Sem binários comerciais.** Como em [`fixtures/README.md`](../../README.md), nunca commitar homebrew protegido por copyright.
- **Traces de homebrew autoral / public domain:** OK commitar `.jsonl` + `.notes.md` + ELF (se < 1 MB).
- **Traces de homebrew com licensing restrito:** commitar APENAS `.jsonl` + `.notes.md` com referência à fonte. O `.jsonl` é uma observação comportamental do que a execução fez, não o binário em si — geralmente safe a commitar mesmo quando o binário não é.
- **Traces grandes (> 1 MB):** considerar gzipar (`.jsonl.gz`) e adicionar passo de descompactação no test harness; ou guardar offline e referenciar via `.notes.md`.
- **Estabilidade:** uma vez commitado um trace + `.notes.md`, ele vira regression sentinel. Não deletar nem editar sem registrar a razão (o trace estava errado / o homebrew foi atualizado / o RPCS3 foi atualizado e o trace foi recapturado).

## Validação esperada (forma canônica post-R5.9e.7)

O acceptance gate test segue o shape de [`rust/rpcs3-spu-recompiler/tests/single_spu_mailbox_v1_replay.rs`](../../../../rust/rpcs3-spu-recompiler/tests/single_spu_mailbox_v1_replay.rs):

1. Carregar o `.jsonl` via `std::fs::read_to_string` resolvido relativamente à crate manifest.
2. `parse_jsonl_trace()` → `Ok(Vec<CapturedEvent>)` (não vazio).
3. Verificar critérios estruturais (zero ch21 MFC_Cmd, ≥1 ch28 OUT_MBOX ou critério análogo, exatamente 1 spu_image, exatamente 1 target_spu para single-SPU traces).
4. `captured_events_to_traces_per_spu()` → `Ok(BTreeMap<u32, Vec<TraceEvent>>)`.
5. `build_spu_program_from_captured_image()` → `Ok(SpuProgram)` (hash + size + entry_pc validados).
6. `replay_per_spu_traces::<InterpreterExecutor>(...)` → `Ok(...)` com `final_event_kind = Finished{stop_code}` esperado.
7. `replay_per_spu_traces_with(..., |_| RecompilerExecutor::new())` → mesmo `Finished{stop_code}`.
8. **`diff_snapshots(interp.final_snapshot, jit.final_snapshot).is_identical()` retorna `true`** — PC, GPRs, LS, channels, park_state byte-by-byte. `total_steps` excluído (legitimamente difere entre backends).

**Se qualquer estágio falhar:** preserve o trace failing como está. NÃO ajuste assertions, NÃO weakene checks no parser/transformer, NÃO infira valores ausentes. O trace é oracle; uma falha aponta para um real correctness gap entre o stack Rust e o C++ — diagnostique antes de qualquer trabalho de bridge.

## Cross-references

- Wire format: [`docs/SPU_TRACE_CAPTURE.md`](../../../../docs/SPU_TRACE_CAPTURE.md).
- C++ integration patch: [`docs/SPU_TRACE_CAPTURE_PATCH.md`](../../../../docs/SPU_TRACE_CAPTURE_PATCH.md).
- Trace writer C++ source: [`rpcs3/Emu/Cell/SPUTraceJsonl.h`](../../../../rpcs3/Emu/Cell/SPUTraceJsonl.h) + [`SPUTraceJsonl.cpp`](../../../../rpcs3/Emu/Cell/SPUTraceJsonl.cpp).
- Rust parser + transformer: [`rust/rpcs3-spu-differential/src/trace_fmt.rs`](../../../../rust/rpcs3-spu-differential/src/trace_fmt.rs).
- Synthetic round-trip fixture (NOT real): `R5_6_REFERENCE_JSONL` em `trace_fmt.rs`.
- Replay engine: [`rust/rpcs3-spu-differential/src/lib.rs`](../../../../rust/rpcs3-spu-differential/src/lib.rs) — busca `pub fn replay_trace`.
- Status / próxima fase: [`docs/PROJECT_STATUS.md`](../../../../docs/PROJECT_STATUS.md).
