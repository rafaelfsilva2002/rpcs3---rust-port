# Backlog Residual

**Last updated:** 2026-04-24 (numbers below refer to the 2026-04-24 frozen baseline; for current verified counts see [`../../docs/PROJECT_STATUS.md`](../../docs/PROJECT_STATUS.md)).

**Scope:** items NOT closed by the 230-crate / 5165-test baseline that
was frozen on 2026-04-24. Items here are **viable but not yet ported**;
truly blocked items live in [`DEFERRED.md`](DEFERRED.md).

> Note: this file is a residual backlog written against the 2026-04-24 baseline.
> Some items below have since been at least partially addressed by the SPU
> recompiler work (R1..R4c). Cross-reference with [`../../docs/PROJECT_STATUS.md`](../../docs/PROJECT_STATUS.md)
> before assuming an entry is still open.

---

## Small residual helpers

Tiny C++ helpers that didn't make it into a dedicated crate but could without major effort:

- [ ] `Emu/Cell/timers.hpp` — header-only timer helpers (`get_timebased_time`, `get_system_time`); pure math + atomic counter, ~50L of useful constants
- [ ] `Emu/Io/buzz_config.h` / `ghltar_config.h` / `turntable_config.h` — default pad-button bindings (already covered indirectly by the device emulator crates, but a `default-bindings` crate would centralize)
- [ ] `Emu/RSX/Overlays/overlay_compile_notification.cpp` — needs `localized_string_id` (already ported) + UI plumbing (deferred)
- [ ] `Emu/RSX/Core/RSXContext.cpp` (50L) — has deps on tile/iomap structs not yet portable
- [ ] `Emu/Cell/Modules/cellOvis.cpp` / `cellSysconf.cpp` already covered by existing crates; verify naming alignment

## VM / Memory remaining candidates

- [ ] `Emu/Memory/vm.cpp` (2508L) — partial contract surface possible (page table layout, address translation constants)
- [ ] `Emu/Memory/vm_locking.cpp` — reservations / atomic primitives
- [ ] Per-region page protection maps — `vm::ptr` bound checks that the HLE crates already trust

## RPCN / Network remaining candidates

- [ ] `Emu/NP/rpcn_client.cpp` (3348L) — large but the **wire format** (packet headers, command IDs, login flow) is portable as data-only crate even if the network I/O isn't ported
- [ ] `Emu/NP/np_handler.cpp` — similarly: protocol logic separable from socket layer
- [ ] RPCN message types enum (already partially covered; needs full enum dump)
- [ ] WolfSSL TLS handshake constants (referenced by RPCN_ERROR_WOLFSSL in our localized-string crate)

## RSX helpers

- [ ] `Emu/RSX/rsx_methods.cpp` — RSX command method table (large, but each method = small struct)
- [ ] `Emu/RSX/Common/BufferUtils.cpp` — buffer copy + format conversion helpers
- [ ] `Emu/RSX/Common/TextureUtils.cpp` — texture format conversion math
- [ ] `Emu/RSX/Capture/rsx_capture.cpp` — capture file format (RRC) parser
- [ ] `Emu/RSX/Capture/rsx_replay.cpp` — capture replay state machine

## Deferred large runtime systems

> ⚠️ These were **explicitly out of scope for the 2026-04-24 baseline by
> design**. Listed here for visibility, not as backlog. Each is a multi-week
> dedicated project. Some have since been *partially* addressed by
> incremental Rust replacements — see notes per item.

- [x] `SPUCommonRecompiler.cpp` (9792L, JIT x86 backend) — **superseded** by
      the incremental Rust SPU recompiler `rust/rpcs3-spu-recompiler` (Cranelift,
      operational up to R4c). The C++ source itself is **not** being ported
      line-by-line; the Rust crate is a fresh implementation validated
      byte-exact against the Rust SPU interpreter. See
      [`../../docs/PROJECT_STATUS.md`](../../docs/PROJECT_STATUS.md).
- [ ] `SPULLVMRecompiler.cpp` (9497L, JIT LLVM backend) — still deferred. The
      Rust port chose Cranelift for v0; an LLVM backend (`inkwell` /
      `llvm-sys`) is reserved for R5+ if benchmarks show the Cranelift
      backend hitting a ceiling.
- [ ] `SPUASMJITRecompiler.cpp` (4878L, ASMJIT legacy backend) — will not be
      ported; superseded by the Cranelift-based Rust recompiler.
- [x] `SPUInterpreter.cpp` (3363L) — **superseded** by `rust/rpcs3-spu-interpreter`
      (~70% ISA, used as the byte-exact reference oracle for the Rust
      recompiler).
- [ ] `PPUInterpreter.cpp` (7888L) — still C++ only (Rust side has a contract
      stub in `rpcs3-ppu-interpreter`, not a working interpreter).
- [ ] `PPUThread.cpp` (5684L), `SPUThread.cpp` (7488L) — runtime threads,
      still C++ only.
- [ ] `PPUTranslator.cpp` (5594L), `PPUAnalyser.cpp` (3278L), `PPUModule.cpp`
      (3254L) — PPU JIT tooling, still C++ only. No Rust counterpart yet;
      candidate for a "wave-9-runtime PPU JIT" effort.
- [ ] `System.cpp` (4823L) — Emulator singleton state machine, still C++
      only.
- [ ] `RSXThread.cpp` (3675L), `VKGSRender.cpp` (3009L) — GPU runtime, still
      C++ only.
- [ ] `rpcs3qt/**` — Qt UI (framework-specific, by design out of scope).

## Blocked legal / fixture-dependent work

> See [`DEFERRED.md`](DEFERRED.md) for full reason / required input / unblock condition.

- [ ] `rpcs3-loader-self-decrypt` — needs SELF binary fixtures + `key_vault` PS3 keys
- [ ] Real save data fixtures for `cellSavedata` differential validation
- [ ] Sentinel commercial title — needs ROM dump + game serial + USP
- [ ] PSN packet captures for RPCN replay tests
- [ ] Trophy XML fixtures for full TROPUSR roundtrip

---

## Triage convention

When a residual item gets picked up:
1. Update the appropriate "What is complete / partially complete" section in [`../../docs/PROJECT_STATUS.md`](../../docs/PROJECT_STATUS.md)
2. Remove the entry from this file
3. Re-run the test command panel in `PROJECT_STATUS.md` so the verified counts reflect the new addition

When an item is reclassified as deferred (legal, fixtures, runtime giant):
- Move to [`DEFERRED.md`](DEFERRED.md) with reason / required input / unblock condition
