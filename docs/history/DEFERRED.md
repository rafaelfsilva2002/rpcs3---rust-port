# Deferred Items

**Last updated:** 2026-04-24 (entries written against the 2026-04-24 baseline; some have since been partially or fully addressed — see notes per item).

**Purpose:** Items explicitly NOT pursued in this wave. Each entry must include
**reason**, **required input**, and **unblock condition** so future maintainers
know exactly what would be needed to take the item up.

> Important: this file is **point-in-time**. The current authoritative status
> (including the incremental Rust SPU recompiler done up through R4c) lives in
> [`../../docs/PROJECT_STATUS.md`](../../docs/PROJECT_STATUS.md). Entries below
> that distinguish "Rust incremental recompiler" from "line-by-line C++ port"
> have been updated with cross-references. Do **not** read this file as
> evidence that the incremental Rust SPU recompiler is deferred — it isn't.

---

## 1. `rpcs3-loader-self-decrypt`

- **Reason:** PS3 SELF (Signed-ELF) decryption requires both the encrypted binary AND the corresponding console keys. Both are proprietary Sony assets with copyright + DMCA implications. Distributing keys or shipping fixtures derived from them is legally hazardous for the project.
- **Required input:**
  1. SELF binary fixtures (e.g. an open-source homebrew that the author ships in encrypted form)
  2. `key_vault` data structure with PS3 ECDSA / AES keys (one of: KaKaRoTo's keyvault snapshot, NoPaystation table, or homebrew-built equivalent)
  3. Port of `Crypto/unself.cpp` (depends on items 1 and 2 to validate against)
- **Unblock condition:** A homebrew-distributed encrypted SELF + a publicly-acknowledged keyvault layout that this repo can ship as test fixture without legal risk. We already have `rpcs3-loader-elf-self` (header parser) and `rpcs3-crypto` (AES/SHA primitives), so the algorithmic side is ready.

---

## 2. TAR / PUP install flow end-to-end

- **Reason:** `rpcs3-loader-tar` (POSIX ustar) and `rpcs3-loader-pup` (firmware container) are individually closed (28 tests combined), but the **end-to-end install flow** (extract PUP → run TAR through SELF decrypt → install firmware tree) is gated by item #1.
- **Required input:** firmware PUP + working SELF decrypt path.
- **Unblock condition:** item #1 unblocks this automatically.

---

## 3. Homebrew differential validation

- **Reason:** Need at least one open-source PS3 PPU homebrew binary to run as fixture against both RPCS3 C++ and our crates. None committed yet.
- **Required input:**
  1. Pick a homebrew (candidates: `ps3autotests`, `ps3-homebrew-pong`, similar)
  2. Get its source + license + build instructions
  3. Build it once with stable tooling, commit the PPU binary to `behavior-freeze/fixtures/` (license permitting)
  4. Wire `compare_run.py` to run it through both implementations
- **Unblock condition:** picked + committed fixture + green compare run.

---

## 4. Save / load real validation

- **Reason:** `rpcs3-hle-cellsavedata` is byte-exact against the C++ class shape (23 tests), but exercising the **real save/load cycle** (create save, write, restart, read, delete) requires either:
  1. A homebrew that exercises cellSavedata
  2. A commercial title with simple save semantics
- **Required input:**
  1. Save data fixture (synthesized or from homebrew)
  2. Disk-backed save directory with proper PARAM.SFO
  3. `compare_run.py` mode that captures save folder hash before/after a scenario
- **Unblock condition:** item #3 partially overlaps; if the chosen homebrew touches cellSavedata, this gets validated for free.

---

## 5. Sentinel commercial title

- **Reason:** A commercial PS3 game pinned as the canonical regression sentinel — runs through the same compare path on every change, catches drift that homebrew might miss.
- **Required input:**
  1. ROM dump (legally acquired by the user, NOT shipped in repo)
  2. PSN account / RPCN setup if title is online
  3. Choice of "simple" title — short startup, deterministic flow, single-player, minimal RSX requirements
  4. Approved fixture list (saves, configs) that lives outside the repo
- **Unblock condition:** user picks a title + acquires ROM + we wire a `compare_run.py` mode that points at it (locally only, not published).

---

## 6. Line-by-line ports of the original C++ JIT backends (SPU/PPU)

> **Status update (post-2026-04-24):** the *incremental* Rust SPU recompiler
> (`rust/rpcs3-spu-recompiler`, Cranelift-backed) is **operational** with R1
> decoder + R2 scaffold + R2.5 broad-subset codegen + R4a dispatcher + R4b
> chained patching + R4c minimal SMC. See
> [`../../docs/PROJECT_STATUS.md`](../../docs/PROJECT_STATUS.md). What remains
> deferred below is the **linear, file-by-file port of the original C++
> backends**, not Rust SPU recompilation in general.

- **Reason:** `SPUCommonRecompiler.cpp` (9792L), `SPULLVMRecompiler.cpp` (9497L),
  `SPUASMJITRecompiler.cpp` (4878L), `PPUTranslator.cpp` (5594L),
  `PPUAnalyser.cpp` (3278L), `PPUModule.cpp` (3254L). Each is a multi-week
  dedicated project. Porting linearly would dominate the timeline; porting in
  parallel would risk regressions on the parts already covered. The
  incremental Rust SPU recompiler chose a different path: build a Cranelift
  JIT from scratch covering the same `SpuFunction` graphs the original C++
  produces, validated byte-exact against the Rust interpreter, instead of
  translating the C++ source. That covers SPU; the C++ files themselves
  remain unported, and PPU (Translator/Analyser/Module) has no Rust
  counterpart yet.
- **Required input:** dedicated focus session(s), mentor/reviewer with
  experience in JIT design, decision on whether the Cranelift approach used
  for SPU should be extended to PPU or whether a different backend
  (`llvm-sys`, hand-rolled) is warranted there.
- **Unblock condition:** explicit go-ahead for a "wave-9-runtime PPU JIT"
  effort. The SPU side is already covered incrementally and tracked in
  `PROJECT_STATUS.md`; behavior-freeze contracts in `rpcs3-ppu-interpreter` /
  `rpcs3-spu-interpreter` act as the validation oracle for any further work.

---

## 7. RSX runtime backends (Vulkan / GL render threads)

- **Reason:** `RSXThread.cpp` (3675L), `VKGSRender.cpp` (3009L), and the GL backend are GPU-API-bound and require linking native Vulkan / OpenGL drivers. Out of scope for a pure-Rust port without a defined backend choice (`wgpu`? `vulkano`? `ash`?).
- **Required input:** GPU backend decision + Vulkan/GL bindings choice + RSX command stream emitter port.
- **Unblock condition:** RSX-runtime wave decision + dedicated team-time.

---

## 8. Qt UI (`rpcs3qt/**`)

- **Reason:** Qt is a framework-specific C++ UI toolkit. Porting to Rust would require choosing a different framework (egui? slint? iced?) and rewriting from scratch. Not part of behavior-freeze (UI is observable but not byte-deterministic).
- **Required input:** UI framework decision.
- **Unblock condition:** explicit "UI wave" with framework chosen.

---

## 9. `rpcs3-lv2-syscall-table` — full signature binding

- **Reason:** The table-of-IDs + names + shape side is **already done** (14 tests passing). What's still missing is the **per-syscall argument type bindings** (e.g. `sys_process_exit(s32)` vs `sys_lwmutex_create(vm::ptr<lwmutex_t>, vm::ptr<lwmutex_attr_t>)`), which require a fully-ported `ppu_thread` (a runtime giant — see item #6).
- **Required input:** ported `ppu_thread` argument-passing convention.
- **Unblock condition:** item #6 unblocks this.

---

## Summary table

| # | Item | Category | Blocker |
|---|------|----------|---------|
| 1 | self-decrypt | Legal / fixtures | Sony keys + SELF fixtures |
| 2 | PUP install flow | Dependent | Item 1 |
| 3 | Homebrew differential | Fixture | Pick + commit homebrew |
| 4 | Save/load real | Fixture | Save data fixture |
| 5 | Sentinel commercial | Fixture / legal | User-provided ROM |
| 6 | C++ JIT giants (line-by-line port) | Effort / scope | Dedicated wave; SPU side already covered incrementally — see `PROJECT_STATUS.md` |
| 7 | RSX runtime | Architectural | GPU backend choice |
| 8 | Qt UI | Architectural | UI framework choice |
| 9 | Syscall signature binding | Dependent | Item 6 |
