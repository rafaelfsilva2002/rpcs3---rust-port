# SPU PPU↔SPU trace fixtures

Esta pasta abriga **traces JSONL capturados de execuções reais sob RPCS3 patcheado** (R5.8 A.3). Cada trace é uma sequência determinística de eventos PPU↔SPU (mailbox, sinal, park, wake, stop, final_state) gerada pelo writer C++ em [`rpcs3/Emu/Cell/SPUTraceJsonl.{h,cpp}`](../../../../rpcs3/Emu/Cell/) e consumida pelo pipeline Rust em [`rust/rpcs3-spu-differential/src/trace_fmt.rs`](../../../../rust/rpcs3-spu-differential/src/trace_fmt.rs).

**Status atual:** **13 REPLAY-VALIDATED FIXTURES LANDED.** The 13th oracle `single_spu_dma_getl_v1` was captured in R8.4b (writer) and PROMOTED in R8.4c (replay state machine for list cmds — `process_mfc_list_cmd` walks the captured `.dmalistdesc` descriptor + loads per-element chunks). Runtime bridge GETL (R8.4d) still pending — bridge ON for the GETL fixture would currently hit `MfcUnsupported` fallback. R5.9e.7 introduced `single_spu_mailbox_v1` (the first oracle); R5.11 + R5.11b added three more; R6.4b-replay added the fifth (`single_spu_mailbox_multi_v1`); R6.6 added the sixth (`game_like_mailbox_signal_v1`); R6.7 A.5 added the seventh (`single_spu_dma_get_v1` — first DMA-bound oracle); R8.1 added the eighth (`single_spu_dma_put_v1` — first PUT-bound oracle); R8.2 added the ninth (`single_spu_dma_get_multi_v1` — first multi-DMA oracle, two queued GETs + ALL wait); R8.3a added the tenth (`single_spu_dma_get_any_v1` — first ANY-wait-mode oracle, surfaced + co-fixed the ch24 drain-aggregate semantic); R8.3b added the eleventh (`single_spu_dma_tag_poll_v1` — first repeated-RdTagStat oracle, surfaced + co-fixed the persistent `completed_tags` semantic); R8.3c added the twelfth (`single_spu_dma_tag_immediate_v1` — first IMMEDIATE wait-mode oracle, surfaced + co-fixed a legacy clear-on-read in the replay state machine) — all twelve `diff_snapshots(interp, jit).is_identical()` byte-identically:

| Fixture | Layer | Channels / ISA exercised | Engine fix co-landed |
|---|---|---|---|
| [`single_spu_mailbox_v1`](./single_spu_mailbox_v1.jsonl) | R5.9e.7 | IN_MBOX (ch29) + OUT_MBOX (ch28) + stop 0x101 | 3 fixes (transformer initial-state, lv2 stop-0x101 OUT_MBOX drain, `SpuProgram.initial_gpr_overrides`) |
| [`single_spu_branch_loop_v1`](./single_spu_branch_loop_v1.jsonl) | R5.11 | + branch/loop ISA (Fibonacci) | None — rode existing fixes |
| [`single_spu_signal_v1`](./single_spu_signal_v1.jsonl) | R5.11 | SNR1 (ch3) + OUT_MBOX (ch28) + stop 0x101 | 1 fix (Cell BE SPU SNR-blocking semantics in `SpuChannels::read`) |
| [`single_spu_loadstore_v1`](./single_spu_loadstore_v1.jsonl) | R5.11b | + LS load/store (stqd/lqd + cwd/shufb/rotqby/stqd) | 3 fixes (rotqby RR-form added; C-family default mask byte-order corrected; RRR-form rt/rc field positions corrected in pack_rrr + selb/shufb/fma/fnms/fms dispatch) |
| [`single_spu_mailbox_multi_v1`](./single_spu_mailbox_multi_v1.jsonl) | R6.4b-replay | IN_MBOX (round 1) + SNR1 (round 2) + OUT_MBOX = 0x453 + stop 0x101; FIRST fixture with both `ppu_push_inmbox` and `ppu_signal` events + a real `spu_park`/`spu_wake` cycle (PPU `sysUsleep(100ms)` between writes forces SPU to actually park on rdch ch3 before round 2 input arrives) | None — rode existing fixes; the load-bearing transformer change was already there from R5.11 signal fixture work |
| [`game_like_mailbox_signal_v1`](./game_like_mailbox_signal_v1.jsonl) | R6.6 | IN_MBOX + LS load/store (16-word volatile buffer) + branch/loop (16-iter mix loop + 8-iter final mix) + SNR1 + real `spu_park`/`spu_wake` + OUT_MBOX = 0x051A03C9 + stop 0x101; FIRST "game-like" fixture combining FIVE bridge code paths in a single SPU program (cross-path interaction sentinel) | None — rode existing fixes; bridge ON observed `total_steps=488 stall_iters=1` |
| [`single_spu_dma_get_v1`](./single_spu_dma_get_v1.jsonl) | R6.7 A.5 | MFC GET (ch16-21 wrch + ch22-23 wait setup + ch24 RdTagStat) + 128-byte EA → LS DMA + post-DMA sum + XOR → OUT_MBOX = 0xDEADA12F + stop 0x101; FIRST fixture exercising MFC channels and the R6.7 A.1 DMA writer extension (`spu_mfc_cmd` + `mfc_dma_complete` events + content-addressed `<sha>.dmachunk` side-file at `../dma/`) | R6.7 A.2 parser + A.3 chunk loader + A.4 MfcReplayState + Phase C executor wiring; **interpreter mode required for capture** (LLVM JIT bypasses `set_ch_value()` for MFC channels) |
| [`single_spu_dma_put_v1`](./single_spu_dma_put_v1.jsonl) | R8.1 | MFC PUT (cmd 0x20, LS → EA — symmetric inverse of GET) + 128-byte LS source pattern → ch16-21 wrch + ch22-23 wait + ch24 RdTagStat + ch28 OUT_MBOX = 0xC0FFEECA (sentinel) + stop 0x101; PPU reads EA back, sums + XOR → `ea_status = 0xCAFEA57E`; FIRST PUT-bound oracle, content-addressed `.dmachunk` deduplicates with GET fixture (same counting pattern, same SHA `471fb943…`) | R8.1 parser (accept cmd 0x20 alongside 0x40, new defensive canary 0x44 GETL) + state machine PUT branch (`PutLsBytesMismatch` error; pre-replay defers assertion via `process_mfc_cmd_pre_replay`, post-replay verifies final LS in the test) + `DmaPutCallback` on `SpuChannels` + FFI `rust_spu_set_dma_put_callback` + C++ writer extension (snapshots LS bytes on PUT dispatch) + bridge `bridge_dma_put_callback` (`vm::_ptr<u8>` write path) |
| [`single_spu_dma_get_multi_v1`](./single_spu_dma_get_multi_v1.jsonl) | R8.2 | TWO QUEUED MFC GETs (cmd 0x40, tags 3 + 5, distinct EAs, distinct sizes 128 + 64, distinct LSAs 0x10000 + 0x10100, both in-flight before any wait) + WrTagMask = 0x28 = (1<<3)\|(1<<5) + WrTagUpdate = ALL + RdTagStat returns 0x28 once both completions fire; combined status = ((sum1 << 16) \| sum2) ^ 0xFEEDFACE = 0xE12DEA4E; FIRST multi-DMA oracle. Content-addressed pool: chunk #1 (`471fb943…`, counting pattern) deduplicates with GET v1 + PUT v1; chunk #2 (`c422e7070…`, constant 0x42) is new. | **NONE** — R8.2 is a pure coverage gain on the existing 8-oracle baseline. Parser (R6.7 A.2), state machine multi-tag in-flight (R6.7 A.4), 2-tag ALL mode (existing unit test `mfc_replay_handles_wr_tag_mask_update_basic`), chunk loader (R6.7 A.3), executor wiring (Phase C `tag_stat_queue: VecDeque`), and bridge multi-dispatch (R7.2 callback invoked twice via the same install) ALL work without modification. |
| [`single_spu_dma_get_any_v1`](./single_spu_dma_get_any_v1.jsonl) | R8.3a | TWO QUEUED MFC GETs (same shape as R8.2 multi) + WrTagMask = 0x28 + **WrTagUpdate = ANY (= 1)** + RdTagStat returns 0x28 (RPCS3 sync-DMA returns full mask in ANY mode); SPU embeds the actual ch24 returned value in the canonical via `(tag_stat << 24) ^ 0xBEEFBEAD`, status = 0x892FAE2D; FIRST ANY-wait-mode oracle. Content-addressed pool: chunk #1 + chunk #2 ZERO new entries (perfect dedup with R8.2 patterns). | **R8.3a engine fix landed** — the R8.3a embedded tag_stat surfaced a divergence between Rust runtime (popping single-tag bits one at a time) and C++ executor (returning `completed_tags & wr_tag_mask` aggregate). Fix: `SpuChannels::read(MFC_RD_TAG_STAT)` now drains the queue with bitwise OR + intersects with `mfc_wr_tag_mask` on each read. Pre-replay path (queue carries pre-aggregated value) and runtime path (callbacks push individual bits, drain aggregates) unified observationally for one-shot reads. **rpcs3.exe rebuilt to relink the new `rpcs3_spu_ffi.lib`.** |
| [`single_spu_dma_tag_poll_v1`](./single_spu_dma_tag_poll_v1.jsonl) | R8.3b | TWO QUEUED MFC GETs (tags 3 + 5) + TWO ch24 reads in the same SPU session with distinct masks (0x08, then 0x20, both ANY mode); SPU embeds BOTH returned tag_stat values into the canonical via `((tag_stat_1 << 24) \| (tag_stat_2 << 16)) ^ 0xCAFEBADC`, status = 0xDD1EAA5C; FIRST repeated-poll oracle. Forces persistent `completed_tags` semantics. ZERO new `.dmachunk` files (perfect dedup with R8.2 / R8.3a). | **R8.3b engine fix landed** — drain-clear from R8.3a stalled on the second ch24 read because the queue was empty. Predicted + observed: both replay test (parked at pc=140) and bridge ON (stall fallback to C++, total_steps=36). Fix: added `completed_tags: u32` field on `SpuChannels`, persistent across reads; ch24 read drains queue OR-into `completed_tags`, returns `completed_tags & mfc_wr_tag_mask` WITHOUT clearing. Multiple reads same session observe per-mask subsets of the same `completed_tags`. rpcs3.exe rebuilt; SHA pins unchanged. |
| [`single_spu_dma_tag_immediate_v1`](./single_spu_dma_tag_immediate_v1.jsonl) | R8.3c | TWO QUEUED MFC GETs + TWO ch24 reads with **`WrTagUpdate = IMMEDIATE` (= 0)** and overlapping masks (0x08 ⊂ 0x28); SPU embeds BOTH returned tag_stat values into the canonical via `((ts1 << 24) \| (ts2 << 16)) ^ 0xCAFE5A1E`, status = 0xDD164A9E; FIRST IMMEDIATE-wait-mode oracle. Captured `ts2 = 0x28` (full mask) proves IMMEDIATE does NOT clear `completed_tags`. ZERO new `.dmachunk` files. | **R8.3c engine fix landed (replay layer only)** — overlapping masks surfaced a latent clear-on-read in `MfcReplayState::process_rdch_tagstat` (legacy from R6.7 A.4 when no oracle exercised mask overlap). R8.3b SpuChannels::read fix had aligned the runtime side already; R8.3c removes the matching clear in the state machine oracle validator. rpcs3.exe does NOT require rebuild (fix is in `rpcs3-spu-differential`, replay-only path; not in `rpcs3_spu_ffi.lib`). SHA pins unchanged. |

A flag `REPLAY_VALIDATED_TRACE_EXISTS` em [`behavior-freeze/harness/check_trace_fixtures.py`](../../../harness/check_trace_fixtures.py) está `True` desde R5.9e.7. Acceptance gates ficam em [`rust/rpcs3-spu-recompiler/tests/`](../../../../rust/rpcs3-spu-recompiler/tests/) (12 test files, um por fixture). Cada `.notes.md` companion documenta provenance + engine fixes específicos.

**Diagnostic-only traces fora desta pasta:** o trace v3 (`spurs_test_v3_real.jsonl` — R5.9d-era multi-SPU SPURS) e o trace v4 (`spurs_test_v4_real.jsonl` — R5.10a..p ISA-coverage iteration) vivem em [`rust/rpcs3-spu-differential/tests/data/`](../../../../rust/rpcs3-spu-differential/tests/data/) e NÃO em `behavior-freeze/fixtures/`. São DMA-bound no protocol layer (`pc=0x74C wrch ch16 (MFC_LSA)` no v4) — per [`docs/SPU_TRACE_R5_9E_REPLAY_PLAN.md`](../../../../docs/SPU_TRACE_R5_9E_REPLAY_PLAN.md) § D.1, replay NÃO pode progredir além do MFC boundary sem (a) full DMA infrastructure, (b) writer that captures MFC events as oracle inputs (R5.9f deferred), ou (c) homebrew non-DMA (caminho que R5.9e.7 tomou). Diagnostic traces são `#[ignore]`d em `tests/real_trace_diagnostic.rs` — surfaced as ISA-coverage signals, NÃO promovidos a byte-identical contract.

**Nenhum trace fabricado** vai aqui. Hand-authoring um JSONL sintético "como se" tivesse vindo do C++ derrota o propósito do harness — o trace existe para ser uma fonte oracular de verdade contra a qual o stack Rust é validado. Se o trace fosse fabricado para passar, ele não detectaria nada. A pasta cresce somente via captura empírica adicional.

(Para fixtures sintéticas que NÃO fingem ser capturas reais, ver `R5_6_REFERENCE_JSONL` em `rust/rpcs3-spu-differential/src/trace_fmt.rs` — explicitamente marcada como "synthetic, hand-derived from R5.6".)

## Critérios de aceitação para NOVOS traces replay-validated (post R5.9e.7)

R5.9e.7 entregou o primeiro replay-validated fixture (`single_spu_mailbox_v1`). Para adicionar um novo trace replay-validated a esta pasta, confirmar TODOS os critérios abaixo. **Nada além de replay-validated traces vai aqui** — diagnostic-only traces ficam em `rust/rpcs3-spu-differential/tests/data/`.

1. **Origem do homebrew:** autoral, public-domain, ou explicitamente redistribuível (license-clean). Nunca conteúdo comercial / copyrighted / extraído de jogo PS3 publicado. Source CC0/MIT do homebrew commitado em `behavior-freeze/fixtures/spu/sources/<homebrew>/` com LICENSE.md (mesmo padrão que `single_spu_mailbox_v1` segue).
2. **Boota no RPCS3 instrumentado:** `rpcs3.exe` produzido a partir de `scaffolding patch + runtime_hooks patch` aplicados sobre upstream master, com sha256s pinned em `check_patch_separation.py`. Sem fork-forks fazendo emulação custom. **R5 closure pinned os hashes** — qualquer captura nova precisa rodar contra esses mesmos patches ou justificar a divergência em PR.
3. **Cria SPU thread group** (ou fluxo equivalente Raw SPU MMIO) — homebrew puramente PPU não satisfaz porque não dispara nenhum dos 8 hook events implementados.
4. **Exerce mailbox PPU↔SPU OU signal OU sequência observável:** o trace deve conter pelo menos um par balanceado dos eventos `ppu_push_inmbox` / `spu_rdch` (ou `spu_wrch` / `ppu_pop_outmbox`, ou `ppu_signal`). Trace só com `spu_stop` + `final_state` é tecnicamente válido mas exercita só 2 de 8 hooks — fica como fixture parcial documentado, não como reference oracle.
5. **DMA permitido a partir de R6.7 A.5 (GET) + R8.1 (PUT):** `spu_wrch` ch21 (`MFC_Cmd`) é aceito SE acompanhado de evento `spu_mfc_cmd` (cmd 0x40 GET ou cmd 0x20 PUT, eah=0, tag<32, size em {1,2,4,8} ∪ multiplos de 16 ≤ 0x4000, lsa+size ≤ 256 KiB) E evento `mfc_dma_complete` casando tag/size, E side-file `<sha>.dmachunk` em `behavior-freeze/fixtures/spu/dma/` (canonical) ou `<jsonl>.dma/` (per-trace) cujo SHA-256 + size validam contra os campos do `spu_mfc_cmd`. Para PUT, o `.dmachunk` carrega os bytes LS-source no momento do dispatch (não bytes EA — replay assert ocorre post-execution comparando LS final). List/atomic/lock-line variants ainda NÃO suportados (R8.1 fica GET+PUT simples). Pre-R6.7 traces DMA-bound (v3/v4 spurs) seguem em `rust/rpcs3-spu-differential/tests/data/` como diagnostic-only.
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
