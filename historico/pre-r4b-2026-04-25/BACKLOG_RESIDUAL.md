# Backlog Residual

**Last updated:** 2026-04-24
**Scope:** items NOT closed by the current 230-crate / 5165-test baseline.
Items here are **viable but not yet ported**; truly blocked items live in [`DEFERRED.md`](DEFERRED.md).

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

> ⚠️ These are **explicitly out of scope for this wave by design**. Listed here for visibility, not as backlog.
> Each is a multi-week dedicated project.

- [ ] `SPUCommonRecompiler.cpp` (9792L) — JIT x86 backend
- [ ] `SPULLVMRecompiler.cpp` (9497L) — JIT LLVM backend
- [ ] `SPUASMJITRecompiler.cpp` (4878L) — ASMJIT legacy backend
- [ ] `PPUInterpreter.cpp` (7888L), `SPUInterpreter.cpp` (3363L) — runtime
- [ ] `PPUThread.cpp` (5684L), `SPUThread.cpp` (7488L) — runtime threads
- [ ] `PPUTranslator.cpp` (5594L), `PPUAnalyser.cpp` (3278L), `PPUModule.cpp` (3254L) — PPU JIT tooling
- [ ] `System.cpp` (4823L) — Emulator singleton state machine
- [ ] `RSXThread.cpp` (3675L), `VKGSRender.cpp` (3009L) — GPU runtime
- [ ] `rpcs3qt/**` — Qt UI (framework-specific, by design out of scope)

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
1. Move it from this file to a fresh entry in [`AUTONOMOUS_LOG.md`](AUTONOMOUS_LOG.md)
2. Add the new crate to [`CHECKLIST.md`](CHECKLIST.md) under the correct wave
3. Bump counters in [`CURRENT_STATE.md`](CURRENT_STATE.md)

When an item is reclassified as deferred (legal, fixtures, runtime giant):
- Move to [`DEFERRED.md`](DEFERRED.md) with reason / required input / unblock condition
