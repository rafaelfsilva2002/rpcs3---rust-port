# Inventário de comportamentos a congelar

Todos os anchors abaixo foram verificados no commit atual de `rpcs3-master/`. Quando um item diz "não encontrei evidência suficiente no repositório", significa literalmente isso.

Legenda de colunas: `Prio | Subsystem | Tipo | Nome real | Arquivo real | Símbolo/comando/fluxo | Evidência | Observável | Entrada | Saída/efeito | Como validar | Tipo de teste | Dependências | Motivo do risco`

---

## P0 — Núcleo sem o qual não há reescrita viável

### P0.1 Enum `game_boot_result` (contrato de retorno do BootGame)

- Prioridade: **P0**
- Subsistema: **boot**
- Tipo: **contrato** (ABI/API interna, impacta logs e UI)
- Nome real: `game_boot_result`
- Arquivo real: [rpcs3/Emu/System.h:42-62](../../rpcs3/Emu/System.h#L42-L62)
- Símbolo: `enum class game_boot_result : u32 { no_errors, generic_error, nothing_to_boot, wrong_disc_location, invalid_file_or_folder, invalid_bdvd_folder, install_failed, decryption_error, file_creation_error, firmware_missing, firmware_version, unsupported_disc_type, savestate_corrupted, savestate_version_unsupported, still_running, already_added, currently_restricted, database_config_missing }`
- Evidência: 18 valores ordenados; usado por `Emulator::BootGame`, `Emulator::Load` e por `is_error(res)` (linha 64-67). Mensagens em [rpcs3/Emu/System.cpp:139-161](../../rpcs3/Emu/System.cpp#L139-L161) traduzem cada valor para string observável.
- Comportamento observável: valor numérico retornado + string formatada no log.
- Entrada: chamada `Emu.BootGame(path, ...)`.
- Saída/efeito: um valor do enum + log `SYS: Boot failed: reason=<string>`.
- Como validar: teste GTest em `contracts/test_contract_game_boot_result.cpp` que fixa ordem, total e mapeia cada valor a uma string; regride se alguém mexer no enum ou na tabela de mensagens.
- Tipo de teste: **contract**
- Dependências: `Utilities/StrFmt.cpp`.
- Motivo do risco: Rust vai redefinir esse enum. Se ordem/numeração mudar, patchers externos, logs automatizados e testes diferenciais quebram em silêncio.

### P0.2 Enum `system_state`

- Prioridade: **P0**
- Subsistema: **boot**
- Tipo: **contrato**
- Nome real: `system_state`
- Arquivo real: [rpcs3/Emu/System.h:30-40](../../rpcs3/Emu/System.h#L30-L40)
- Símbolo: `enum class system_state : u32 { stopped, loading, stopping, running, paused, frozen, ready, starting }`
- Evidência: usado pelo `Emulator` para expor state machine externa (GUI e CLI `--error` observam).
- Observável: valor + efeitos `EmuCallbacks::on_run/on_pause/on_resume/on_stop/on_ready` ([rpcs3/Emu/System.h:71-78](../../rpcs3/Emu/System.h#L71-L78)).
- Entrada: transições `BootGame → Load → Run → Pause → Resume → Kill`.
- Saída/efeito: sequência de callbacks e entradas de log.
- Validar: teste de contrato + cenário headless que força boot de SELF inválido e observa ordem de callbacks via log.
- Tipo: **contract + characterization**
- Dependências: `EmuCallbacks`.
- Risco: ordem das transições é silent contract. Reescrita Rust pode emitir `on_ready` antes de `on_run` e ninguém nota.

### P0.3 CLI flags do binário `rpcs3`

- Prioridade: **P0**
- Subsistema: **CLI**
- Tipo: **contrato**
- Nome real: conjunto de 18 flags registradas via `QCommandLineParser`
- Arquivo real: [rpcs3/rpcs3.cpp:387-410](../../rpcs3/rpcs3.cpp#L387-L410), parser em [rpcs3/rpcs3.cpp:807-856](../../rpcs3/rpcs3.cpp#L807-L856)
- Símbolos: `arg_headless, arg_no_gui, arg_installfw, arg_installpkg, arg_config, arg_input_config, arg_user_id, arg_savestate, arg_rsx_capture, arg_high_dpi, arg_fullscreen, arg_gs_screen, arg_decrypt, arg_verbose_curl, arg_error, arg_updating, arg_q_debug, arg_timer`
- Evidência: cada flag é `constexpr auto` no topo do arquivo; parser processado em `run_rpcs3`.
- Observável: `rpcs3 --help` produz texto estável + cada flag afeta caminho de boot.
- Entrada: `rpcs3 --headless --no-gui /path/to/EBOOT.BIN`.
- Saída/efeito: modo headless, renderer Null, exit code e log.
- Validar: snapshot test de `rpcs3 --help` (golden-master). Teste smoke: `rpcs3 --headless --stopOnError` sem arquivo ⇒ exit code != 0 + log contendo `nothing_to_boot`.
- Tipo: **golden-master + headless-run**
- Dependências: Qt6 (QCommandLineParser).
- Risco: a reescrita Rust muito provavelmente usará `clap`. Nomes e comportamento têm de bater.

### P0.4 Fluxo `Emulator::BootGame` — detecção de tipo de caminho

- Prioridade: **P0**
- Subsistema: **boot/loader**
- Tipo: **fluxo**
- Nome real: `Emulator::BootGame(path, title_id, direct, cfg_mode, cfg_path, db_config)`
- Arquivo real: [rpcs3/Emu/System.cpp:936](../../rpcs3/Emu/System.cpp#L936) (declaração em [rpcs3/Emu/System.h:428](../../rpcs3/Emu/System.h#L428))
- Evidência: ponto único que decide entre PKG, pasta, ISO, EBOOT.BIN, SELF raw → retorna `game_boot_result`.
- Observável: valor de retorno + logs `LDR:`, `SYS:` + alteração de `system_state`.
- Entrada: string de caminho.
- Saída: enum + log.
- Validar: cenários headless com 5 inputs sintéticos (pasta vazia, PKG corrompido, EBOOT renomeado, path inexistente, SELF decryptado vs criptografado) e golden log canônico.
- Tipo: **differential + golden-master**
- Dependências: `Loader/PSF`, `Crypto/unself`, `Loader/unpkg`.
- Risco: cada branch de detecção é um ponto silencioso de regressão. Um SELF tratado como ELF basta para corromper o código Rust em silêncio.

### P0.5 SELF decrypt / NPDRM

- Prioridade: **P0**
- Subsistema: **loader/crypto**
- Tipo: **contrato + output**
- Nome real: `decrypt_self` e `verify_npdrm_self_headers`
- Arquivo real: [rpcs3/Crypto/unself.cpp:1361](../../rpcs3/Crypto/unself.cpp#L1361), [rpcs3/Crypto/unself.cpp:1438](../../rpcs3/Crypto/unself.cpp#L1438)
- Símbolo: `fs::file decrypt_self(const fs::file& elf_or_self, const u8* klic_key, SelfAdditionalInfo* out_info)` / `bool verify_npdrm_self_headers(...)`
- Evidência: entry-point para descriptografar SELF usando AES-128-CBC por section + SHA-1 HMAC. Keys em `key_vault.h`.
- Observável: ELF plaintext exato (byte-for-byte).
- Entrada: SELF criptografado + KLIC (p/ NPDRM).
- Saída: ELF plaintext (ou vazio em erro).
- Validar: fixture de SELF não-NPDRM pequeno (homebrew); teste GTest feeds bytes → espera SHA-256 do output.
- Tipo: **golden-master + differential**
- Dependências: OpenSSL/mbedtls, chaves compiladas.
- Risco: qualquer bit incorreto impede o ELF de carregar. AES CBC IV/paddings são diferentes de gosto de biblioteca.

### P0.6 PSF (PARAM.SFO) parser — contratos públicos

- Prioridade: **P0**
- Subsistema: **loader**
- Tipo: **contrato**
- Nome real: `psf::load`, `psf::get_string`, `psf::get_integer`, enum `psf::error`
- Arquivo real: [rpcs3/Loader/PSF.h](../../rpcs3/Loader/PSF.h), [rpcs3/Loader/PSF.cpp:32-35](../../rpcs3/Loader/PSF.cpp#L32-L35)
- Símbolos: `psf::load_result_t psf::load(const fs::file&, const string&)` → `{registry, error}`
- Observável: dict `registry` preenchido + enum `error ∈ {ok, stream, not_psf, corrupt}`.
- Validar: feed 3 fixtures (válido, magic wrong, truncado) → asserir valores de `registry["TITLE"]`, `registry["TITLE_ID"]`, `registry["PARENTAL_LEVEL"]`.
- Tipo: **contract + characterization**
- Dependências: nenhuma externa.
- Risco: TITLE_ID extraído errado → toda a UI, save path e config loading ficam offset.

### P0.7 PPU thread state ABI

- Prioridade: **P0**
- Subsistema: **PPU**
- Tipo: **contrato + output**
- Nome real: `ppu_thread` register file
- Arquivo real: [rpcs3/Emu/Cell/PPUThread.h:137-348](../../rpcs3/Emu/Cell/PPUThread.h#L137-L348)
- Símbolos: `gpr[32]`, `fpr[32]`, `vr[32]`, `cr`, `lr`, `ctr`, `xer`, `vrsave`, `cia`, reservation state (`raddr`, `rtime`, `rdata[128]`).
- Observável: estado pós-execução de cada opcode; serializável.
- Validar: reutilizar cenário `test_rsx_fp_asm` como modelo para `test_ppu_opcodes_golden`. Alternativa pragmática: homebrew PPU testcase (ex: ps3tests/cpu) roda headless e dumpa GPRs para log → comparar.
- Tipo: **differential**
- Dependências: LLVM (quando JIT ligado).
- Risco: semântica de XER (carry/overflow), modo Java/non-Java FP, modos de CR — pontos clássicos de bug silencioso.

### P0.8 SPU thread state ABI + LS

- Prioridade: **P0**
- Subsistema: **SPU**
- Tipo: **contrato + output**
- Nome real: `spu_thread`
- Arquivo real: [rpcs3/Emu/Cell/SPUThread.h:627-905](../../rpcs3/Emu/Cell/SPUThread.h#L627-L905)
- Símbolos: `gpr[128]` (v128), `fpscr`, `pc`, `mfc_queue[16]`, `raddr`, `rtime`, `rdata[128]`, channels `ch_in_mbox`, `ch_out_mbox`, `ch_tag_stat`, `ch_stall_stat`, `ch_atomic_stat`, LS 256KB.
- Evidência: todos acessíveis via `spu_thread` público.
- Validar: SPU homebrew conhecido (p.ex. `ps3autotests/spu`) roda headless, dump de LS final e gpr via log. Comparar SHA-256 do dump.
- Tipo: **differential + golden-master**
- Dependências: `Utilities/JITLLVM.cpp`, `Utilities/JITASM.cpp`.
- Risco: semântica de canais bloqueantes e ordering de MFC é exatamente onde uma reescrita erra. Se LS final diferir em 1 byte, o jogo diverge em frames.

### P0.9 Tabela de syscalls LV2

- Prioridade: **P0**
- Subsistema: **syscall**
- Tipo: **contrato**
- Nome real: `g_ppu_syscall_table`
- Arquivo real: [rpcs3/Emu/Cell/lv2/lv2.cpp:141-1024](../../rpcs3/Emu/Cell/lv2/lv2.cpp#L141-L1024)
- Símbolo: `const std::array<...> g_ppu_syscall_table` — 1024 entradas.
- Observável: numeração estável (ex: 52 = `sys_ppu_thread_create`, 41 = `sys_ppu_thread_exit`).
- Validar: teste GTest que checa N valores âncora: linha 141, a tabela deve conter função com nome X no índice Y. Tudo por reflexão de macro `BIND_SYSC`.
- Tipo: **contract**
- Dependências: `rpcs3/Emu/Cell/lv2/*.cpp`.
- Risco: uma mudança de índice = todos os jogos travam. Congelar mapa índice → nome é barato e essencial.

### P0.10 Syscalls de FS (sys_fs_open/read/write/close/stat/mkdir/opendir/readdir/closedir)

- Prioridade: **P0**
- Subsistema: **filesystem/syscall**
- Tipo: **contrato**
- Nome real: conjunto `sys_fs_*`
- Arquivo real: [rpcs3/Emu/Cell/lv2/sys_fs.h:647-678](../../rpcs3/Emu/Cell/lv2/sys_fs.h#L647-L678), implementação em `sys_fs.cpp`
- Observável: códigos de erro Cell (`CELL_ENOENT`, `CELL_EACCES`, `CELL_EIO`, `CELL_EISDIR`, `CELL_ENOTDIR`).
- Validar: expandir `tests/test_sys_fs.cpp` com casos cobrindo cada branch de erro via chamadas diretas a `lv2_fs_object::get_path_root_and_trail`. Teste diferencial: homebrew que faz `open(/dev_hdd0/..)` → espera mesmo errno.
- Tipo: **contract + characterization**
- Dependências: VFS, `vfs_config`.
- Risco: Rust devolver `EACCES` onde C++ devolvia `ENOENT` quebra jogos inteiros.

### P0.11 VFS mount table

- Prioridade: **P0**
- Subsistema: **filesystem/config**
- Tipo: **contrato**
- Nome real: `cfg_vfs`
- Arquivo real: [rpcs3/Emu/vfs_config.h:6-43](../../rpcs3/Emu/vfs_config.h#L6-L43)
- Símbolos: `dev_hdd0, dev_hdd1, dev_flash, dev_flash2, dev_flash3, dev_bdvd, games_dir, app_home, dev_usb`
- Observável: YAML serializado estável.
- Validar: teste de round-trip: serializa default → parse → compara.
- Tipo: **contract + snapshot**
- Dependências: cfg parser.
- Risco: renomear `dev_hdd0` quebra cada config pré-existente.

### P0.12 Canal de logs + caminho do `RPCS3.log`

- Prioridade: **P0**
- Subsistema: **config**
- Tipo: **contrato + output**
- Nome real: `logs::channel` + `LOG_CHANNEL` macro
- Arquivo real: [rpcs3/util/logs.hpp:11-142](../../rpcs3/util/logs.hpp#L11), [rpcs3/util/logs.cpp:81-95](../../rpcs3/util/logs.cpp#L81-L95), caminho gravado em [rpcs3/rpcs3.cpp:606](../../rpcs3/rpcs3.cpp#L606)
- Símbolos/exemplos: `rsx_log("RSX")`, `sys_log("SYS")`, `ppu_loader`, `ppu_validator`, `screenshot_log`.
- Observável: canal emite linha prefixada pelo canal; severidade ∈ `{always, fatal, error, todo, success, warning, notice, trace}` ([rpcs3/util/logs.hpp:13-25](../../rpcs3/util/logs.hpp#L13-L25)).
- Validar: test de contrato enumera canais observados no log e sua ordem; `log_parser.py` canonicaliza log em formato estável.
- Tipo: **contract + snapshot**
- Dependências: zlib (para compressão do log).
- Risco: logs são o 80% da nossa visibilidade diferencial. Se formato quebrar, parser quebra e todo o diffing quebra.

### P0.13 RSX Frame Capture (.rrc)

- Prioridade: **P0**
- Subsistema: **RSX**
- Tipo: **integração/output**
- Nome real: `frame_capture_data` / replay `.rrc`
- Arquivo real: [rpcs3/Emu/RSX/Capture/rsx_capture.h](../../rpcs3/Emu/RSX/Capture/rsx_capture.h), [rpcs3/Emu/RSX/Capture/rsx_capture.cpp:39](../../rpcs3/Emu/RSX/Capture/rsx_capture.cpp#L39), [rpcs3/Emu/RSX/Capture/rsx_replay.h:13-14](../../rpcs3/Emu/RSX/Capture/rsx_replay.h#L13-L14) (`c_fc_magic="RRC"`, `c_fc_version=0x6`).
- Observável: arquivo `.rrc` binário; pode ser reproduzido por `rsx_replay_thread`. CLI já tem flag `--rsx-capture` ([rpcs3/rpcs3.cpp:408](../../rpcs3/rpcs3.cpp#L408)).
- Validar: rodar headless de um homebrew → captura `.rrc` → replay → compara hash do frame final (Null renderer com readback). Diferencial Rust vs C++ é direto.
- Tipo: **replay + golden-master**
- Dependências: `utils::serial`.
- Risco: é o único mecanismo de replay pixel-perfeito já presente — precisa ser preservado no Rust.

### P0.14 PSF e config de `games.yml`

- Prioridade: **P0**
- Subsistema: **config**
- Tipo: **contrato + output**
- Arquivo real: [rpcs3/Emu/games_config.cpp:161](../../rpcs3/Emu/games_config.cpp#L161)
- Observável: YAML emitido na pasta de config (`fs::get_config_dir()` em [Utilities/File.cpp:2128](../../Utilities/File.cpp#L2128)).
- Validar: criar fixture mínima, chamar `games_config::add_game` de teste, comparar YAML byte-a-byte com baseline.
- Tipo: **snapshot**
- Risco: pequenas divergências de ordem de chave rompem compatibilidade com configs já existentes dos usuários.

### P0.15 `cpu_flag` bitset

- Prioridade: **P0**
- Subsistema: **PPU/SPU/scheduler**
- Tipo: **contrato**
- Arquivo real: [rpcs3/Emu/CPU/CPUThread.h:14-38](../../rpcs3/Emu/CPU/CPUThread.h#L14-L38)
- Símbolo: `enum class cpu_flag : u32 { stop, exit, wait, pause, suspend, yield, preempt, notify, pending, memory, ... }`
- Evidência: usado por `check_state()` ([rpcs3/Emu/CPU/CPUThread.cpp:830](../../rpcs3/Emu/CPU/CPUThread.cpp#L830)).
- Validar: teste de contrato bloqueia ordem dos valores. Fluxo: teste unitário simples instancia `atomic_bs_t<cpu_flag>`, seta/limpa flags, asserta.
- Tipo: **contract**
- Risco: bit order ≠ ordinal, mas qualquer reordem quebra serialização de savestate.

---

## P1 — compatibilidade e fluxos sensíveis

### P1.1 cellAudio sample format / 256 samples @ 48kHz

- Prioridade: **P1**
- Subsistema: **áudio**
- Tipo: **contrato + output**
- Arquivo real: [rpcs3/Emu/Cell/Modules/cellAudio.cpp:64-134](../../rpcs3/Emu/Cell/Modules/cellAudio.cpp) (log line 87)
- Observável: callback do backend recebe 256 samples; sample rate configurável ∈ {32K, 44.1K, 48K, 88.2K, 96K, 176.4K, 192K} (`AudioBackend.h:19-27`). `AudioDumper` produz WAV quando ativado.
- Validar: rodar com `Null` audio backend + dump WAV de homebrew com tom puro. Comparar FFT por bin.
- Tipo: **golden-master**
- Risco: resampler SoundTouch; jitter de timing do callback.

### P1.2 cellPadGetData estrutura

- Prioridade: **P1**
- Subsistema: **input**
- Tipo: **contrato**
- Arquivo real: `rpcs3/Emu/Cell/Modules/cellPad.cpp` + `cellPad.h:37-40` (filtros IIR)
- Observável: `CellPadData` struct — bytes exatos, ordem de campos.
- Validar: teste de tamanho/offsetof no GTest. Teste diferencial com trace gravado (keyboard handler).
- Tipo: **contract + replay**
- Risco: reordenar campo derruba todos os jogos.

### P1.3 sys_spu DMA MFC ordering

- Prioridade: **P1**
- Subsistema: **SPU/scheduler**
- Arquivo real: [rpcs3/Emu/Cell/SPUThread.h:680-853](../../rpcs3/Emu/Cell/SPUThread.h), [rpcs3/Emu/Cell/MFC.h:5-99](../../rpcs3/Emu/Cell/MFC.h)
- Validar: não-trivial. TODO em `contracts/test_contract_mfc_cmd.cpp` — fixa magic numbers e formato de `spu_mfc_cmd`. Teste funcional real requer homebrew com `fence`/`barrier`.
- Tipo: **contract (hoje) + differential (quando houver fixture)**
- Risco: reordenar DMA tag/fence = travas intermitentes em jogos.

### P1.4 Shader decompiler output

- Prioridade: **P1**
- Subsistema: **RSX/shaders**
- Arquivo real: [rpcs3/Emu/RSX/Program/FragmentProgramDecompiler.h:25-100](../../rpcs3/Emu/RSX/Program/FragmentProgramDecompiler.h)
- Observável: string GLSL gerada por microcódigo RSX fixo.
- Validar: alimentar binário de microcódigo conhecido → comparar GLSL com golden.
- Tipo: **golden-master + differential**
- Risco: divergência de tokens = cache miss permanente; Rust quase certamente emite strings diferentes.

### P1.5 Reservation (LL/SC) timing

- Prioridade: **P1**
- Subsistema: **PPU/SPU/memory**
- Arquivo real: [rpcs3/Emu/Memory/vm_reservation.h:17-323](../../rpcs3/Emu/Memory/vm_reservation.h)
- Validar: homebrew de stress test atômico; difficulty: alta, porque o teste precisa ser determinístico sob concorrência.
- Tipo: **differential + replay**
- Risco: clássico de corrida que só reproduz em alguns jogos.

### P1.6 PKG install

- Prioridade: **P1**
- Subsistema: **loader**
- Arquivo real: `rpcs3/Loader/unpkg.h:140-200` (header existe; confirmar em `rpcs3/Loader/` — **não encontrei evidência suficiente no repositório de que todos os 4 arquivos de unpkg existem como listados, algumas rotas estão em `rpcs3qt/pkg_install_dialog.cpp`**).
- Validar: PKG pequeno → install → comparar árvore de arquivos extraídos com baseline.
- Tipo: **golden-master**
- Risco: divergência na hash/integridade abortando install silenciosamente.

### P1.7 cellSaveData callback flow

- Prioridade: **P1**
- Subsistema: **syscall/filesystem**
- Arquivo real: `rpcs3/Emu/Cell/Modules/cellSaveData.cpp`
- Observável: ordem de callbacks `stat → alloc → read/write → close`.
- Validar: homebrew que salva + carrega + valida bytes.
- Tipo: **integration + golden-master**
- Risco: um callback fora de ordem = save corrompido silencioso.

### P1.8 Trophy storage

- Prioridade: **P1**
- Subsistema: **syscall**
- Arquivo real: `rpcs3/Emu/Cell/Modules/sceNpTrophy.cpp`
- Validar: unlock → inspecionar EDAT output.
- Tipo: **golden-master**
- Risco: EDAT incompatível = perda de troféus para o usuário final.

---

## P2 — secundário

### P2.1 Cache JIT no disco

- Prioridade: **P2**
- Subsistema: **JIT/cache**
- Arquivo real: `Utilities/JITLLVM.cpp`, `Utilities/JITASM.cpp`
- Observável: pasta de cache PPU/SPU; conteúdo não precisa bater bit-a-bit entre versões (por design o cache é invalidável).
- Tipo: **characterization**
- Risco: baixo — cache pode ser regenerado.

### P2.2 GUI Qt e overlays

- Prioridade: **P2**
- Subsistema: **GUI**
- Arquivo real: `rpcs3/rpcs3qt/`
- Tipo: **characterization** (snapshots de UI são caros e frágeis).

### P2.3 Camera/microphone HLE

- Prioridade: **P2**
- Subsistema: **syscall**
- Arquivo real: `rpcs3/Emu/Cell/Modules/cellCamera.cpp`, `cellMic.cpp`
- Risco: baixo — poucos jogos usam.

### P2.4 Network/PSN

- Prioridade: **P2**
- Subsistema: **NP**
- Arquivo real: `rpcs3/Emu/NP/np_handler.cpp/h`
- Risco: complicado testar offline; fora do escopo inicial.

---

## Itens não confirmados

- ISO9660 parser real: `rpcs3/Loader/ISO.h` — não encontrei evidência suficiente no repositório de código direto de ISO9660 (apenas iso_archive stub). Verificar antes de depender disso.
- Alguns arquivos de `rpcs3/Loader/` (TAR, TROPHY, mself) estão listados mas não inspecionados linha-a-linha neste passo. Marcar para auditoria.
