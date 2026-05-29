# RPCS3 → Rust — Port Status & Optimization Roadmap

> **Audit date:** 2026-05-28 · **Method:** 6-agent code audit (one per
> subsystem) reading *actual source*, cross-checked against a freshly
> re-run green gate. Findings below are **code-verified**, not copied
> from the narrative docs. Companion to the canonical
> [`PROJECT_STATUS.md`](./PROJECT_STATUS.md) (R-stage history) and the
> operational baton [`../.planning/HANDOFF.md`](../.planning/HANDOFF.md).

---

## 0. Verified snapshot

| Item | Value | Evidence |
|---|---|---|
| **Test gate** | `cargo test --workspace --tests --release` = **280 blocks / 0 fail / 6015 tests**, exit 0 | re-run 2026-05-28 |
| **Workspace** | **242 crates** (137 are `rpcs3-hle-*`) | `rust/Cargo.toml` members |
| **Build cache** | `rust/target` = 10 GB (warm) | — |
| **Git** | branch `main`, **120 commits ahead of `origin/main` (UNPUSHED)**; tree clean except untracked `.planning/.loop_active`, `CLAUDE.md`, `_disas.txt` | `git status` |
| **HEAD** | `0bc50a541` (docs: HANDOFF baton for R13.5+) | `git log` |

> ⚠️ **The 120 unpushed commits are the single biggest operational
> risk** — all of R9→R13 lives only on this machine. Recommend pushing
> a backup before further work.

---

## 1. Big picture — what this port *is* today

This is a **correctness-first "behavior-freeze" port**: each RPCS3 C++
subsystem is re-implemented in Rust and validated *byte-for-byte*
against captured execution traces ("oracles") before moving on. It is
**not** a performance build and **not** a game-runner yet.

**What genuinely works (code-verified):**

- Boots a **tiny CC0 PSL1GHT homebrew `.self`** end-to-end through
  `EmuCore::run_self` to the canonical `0xC0DE` exit.
- **20 SPU replay oracles** pass byte-identical across the SPU
  interpreter *and* a real Cranelift JIT.
- A complete **RSX command-stream pipeline** (FIFO decode → register
  state → DrawTracker) replays a real PSL1GHT **clear + draw + flip
  frame** captured from real libgcm bytes.
- **LV2 sync primitives** (8 of 9 families) and a working PSL1GHT
  runtime-init pipeline (stack/heap/TLS/import-stub).

**What it is NOT (the headline gaps):**

- ❌ **Does not boot a commercial PS3 game** — no SELF decryption, no
  NPDRM/EBOOT path.
- ❌ **Renders no pixels** — RSX stops at draw-call records; there is no
  shader/texture/Vulkan/GL backend.
- ❌ **Not fast** — PPU is interpreter-only, the SPU JIT is built but
  *not wired in*, execution is single-threaded, guest memory is a
  hashmap-of-pages.

> The mental model: **"runs a 1 KB homebrew that prints `0xC0DE`" ≠
> "runs God of War."** Everything below measures that distance.

---

## 2. Done / Not-done matrix (by subsystem)

Legend: 🟢 solid & tested · 🟡 real but partial/unintegrated · 🔴 not started / deferred

### 2.1 SPU — 🟢 (waves R5–R8)

| Done | Evidence |
|---|---|
| 20 replay oracles, Interpreter ↔ Cranelift JIT byte-identical | `rust/rpcs3-spu-recompiler/tests/*_replay.rs` (20 files) |
| Real Cranelift 0.118 JIT: multi-block codegen, chain-table, self-modifying-code detection | `rpcs3-spu-recompiler/src/jit.rs` (3485 LOC) |
| Full 6-code list-DMA family (GETL/GETLB/GETLF/PUTL/PUTLB/PUTLF) + stall-and-notify | `rpcs3-spu-differential/src/mfc_replay.rs` (3068 LOC) |
| Real C↔Rust FFI bridge (20 panic-safe `extern "C"`) | `rpcs3-spu-ffi/src/lib.rs` |
| ~94 SPU primary opcodes, 0 stubs | `rpcs3-spu-interpreter/src/lib.rs` |

| Not done | Reason |
|---|---|
| Full SPU ISA | ~94/200 opcodes — driven by what the oracles execute |
| Atomic LL/SC (GETLLAR/PUTLLC…) | deferred R8.7+ (canary-rejected) |
| Concurrent multi-SPU scheduling | data-model only; no scheduler |
| JIT indirect branches (`bi`/`bisl`) | fall back to interpreter |

Tests (`#[test]`): decoder 42 · interpreter 260 · recompiler 165 · thread 49 · differential 172 · ffi 30 · mfc 11 · runner 19.

### 2.2 PPU — 🟢 interpreter, 🔴 no JIT (wave R11)

| Done | Evidence |
|---|---|
| Full scalar PowerPC ISA, 7508-LOC interpreter, 223 tests | `rpcs3-ppu-interpreter/src/lib.rs:652` (`step()`), 47 primary opcodes |
| ~110 VMX/AltiVec XO vector ops (vperm/vsel/vmaddfp/vcmp*/vpk*…) | `lib.rs:2668-3197`, CR6 record-form |
| Scalar FP with FPSCR result update | `lib.rs:2440` |
| **OE-form overflow arithmetic** (XER.OV/SO) | `lib.rs:1729` — **shipped (R11.4b), but docs still say "deferred"** |
| Atomics (lwarx/stwcx.), mtspr/mfspr, dcbz | `lib.rs:1141-1923` |

| Not done | Reason |
|---|---|
| **PPU JIT/recompiler** | **does not exist** — interpreter-only; dominant perf ceiling |
| mftb / time base | returns stub 0 (no real clock) |
| Supervisor mode / MSR / TLB | user-mode stubs |
| Real memory ordering (sync/isync) | no-ops (fine for single-thread only) |

### 2.3 LV2 kernel — 🟡 (waves R9, R10)

| Done | Evidence |
|---|---|
| 8/9 sync families (mutex/sema/cond/event/event-port/rwlock/event-flag/lwmutex) via trait containers | `rpcs3-lv2-sync/src/state.rs`, **109 tests** |
| 53 numeric syscalls + 23 NID import handlers (real bodies) | `rpcs3-emu-core/src/lib.rs:867`, `:1675` |
| PSL1GHT runtime init (sys_process_param, proc_prx_param, TLS) | `emu-core/src/lib.rs:616-819` |

| Not done | Reason |
|---|---|
| `lwcond` (R10.8) | placeholder variant; no `rpcs3-lv2-lwcond` crate |
| **Permissive catch-all syscall** | unknown syscalls return `0/CELL_OK` → **green boot ≠ correct kernel coverage** for real binaries |
| TTY emit from `printf` | R9 deferred (newlib `_write_r` linkage) |

### 2.4 RSX — 🟢 command-stream, 🔴 GPU (waves R12–R13)

| Done | Evidence |
|---|---|
| 12 RSX/GCM crates, all real + tested (**138 tests**) | `rust/rpcs3-rsx-*` + `rpcs3-hle-cellgcm[sys]` |
| FIFO decoder + FifoEngine (PUT/GET, call-stack, runaway guard) | `rpcs3-rsx-fifo/src/lib.rs` |
| Method register file (0x4000) + DrawTracker + descriptor parse (vertex/index/texture/surface) | `rpcs3-rsx-state/src/lib.rs` |
| R13 cellGcm HLE: 4 NID handlers; real **clear+draw+FLIP frame** from real libgcm bytes → `0xC0DE` | `emu-core/src/lib.rs:1463-1623`, `tests/rsx_gcm_*.rs` |

| Not done (the GPU "giant tail") | Reason |
|---|---|
| Shader decompilation | `gl/vk-decompiler` port only varying-name→location *tables*, **not** decompilers (misleading names) |
| Texture pixel decode | parse-only; format kept raw |
| Rasterization / framebuffer / Vulkan / GL backend | **zero** GPU deps in the tree |
| cellGcmSys runtime (flip queue/tile/vsync) | contract-only counter dispatcher |

### 2.5 HLE modules — 🟡 broad ABI ports, **not integrated**

| Done | Evidence |
|---|---|
| 137 `rpcs3-hle-*` crates, ~89k LOC, **3723 tests**, 0 stubs | `rust/rpcs3-hle-*` |
| Real ABI/lifecycle/error-code ports (cellrtc calendar math, celljpgdec state machine, cellspurs lifecycle) | per-crate `src/lib.rs` |
| 127/137 crates encode byte-exact error codes | grep `_ERROR_*: u32 = 0x` |

| Not done | Reason |
|---|---|
| **Integration into emu-core** | **ZERO** of the 137 crates are a dependency of any other crate — unconsumed islands; emu-core re-implements the only used HLE (cellGcm) inline |
| Real codec/DSP decode (JPEG/PNG/ATRAC/video pixels) | lifecycle ported, results *injected* (~318 in-code disclaimers) |
| Replay-oracle validation | covered by unit tests only, not capture-replay oracles |

### 2.6 emu-core / boot / loaders — 🟡

| Done | Evidence |
|---|---|
| `EmuCore::run_self` boots CC0 homebrew → `0xC0DE` (SCE/SELF parse → ELF64-BE load → init → PPU interp → syscall dispatch) | `emu-core/src/lib.rs:473`, 3404 LOC, 20+12 tests |
| Crypto primitives (AES/SHA1/HMAC/CMAC/MD5/SHA256) | `rpcs3-crypto` (30 tests) |
| Parse-only loaders (elf-self/pkg/pup/psf/iso/disc) | per-crate tests |

| Not done (headline gap) | Reason |
|---|---|
| **SELF decryption** | no self-decrypt crate; crypto primitives never wired into a decrypt pipeline |
| **Boot of a commercial game (NPDRM EBOOT)** | no decrypt + permissive syscalls |
| PKG/PUP/ISO install/extract/decrypt | parse/classify-only by design |
| Real threading / scheduler | single PPU thread; SPUs run synchronously inline |

---

## 3. Drift found in this audit (docs vs code)

| # | Doc claim | Code reality | Severity |
|---|---|---|---|
| 1 | "3 RSX crates" (PROJECT_STATUS title) | **12** RSX/GCM crates, all real+tested | medium |
| 2 | crate names `rsx-gl-decompiler` / `rsx-vk-decompiler` imply shader decompile | only port name→location lookup tables | medium |
| 3 | "deferred: OE-arith overflow" (R11 note + interpreter header) | **implemented** at `ppu-interpreter/lib.rs:1729` (R11.4b) | medium |
| 4 | §4 "`cargo test --workspace --release` is NOT green" (HLE build errors) | **gate is green** — 280 blocks, exit 0 | medium |
| 5 | crate count implies broad emulator completion | 137 HLE crates exist but **0 are integrated** into the run path | high |
| 6 | "green boot" framing | unknown syscalls/NIDs return `0` silently — boot ≠ kernel correctness | medium |
| 7 | SPU recompiler "delegates to interpreter; JIT lands in R2" (Cargo.toml) | a **real** Cranelift JIT exists in `jit.rs`, but it is **not wired into emu-core/FFI** | low |

Minor cruft: stale `rpcs3-spu-interpreter/src/lib.rs.bak-20260424-222205` in tree; `rpcs3-spu-mfc` name implies an executor but only holds constants (executor lives in `spu-differential`).

---

## 4. Optimization roadmap — "run on a toaster" 🍞

**Current posture (code-verified):** correctness-first, *not* a perf
build. The real Cranelift SPU JIT is **dead on the live path**
(`emu-core` depends only on `rpcs3-spu-interpreter`; `rpcs3-spu-ffi`
lists the recompiler as a dep but has **0 references** and calls the
interpreter's `run_n`; `spu-runner` accepts only `--backend
interpreter`). The PPU is interpreter-only. Execution is
single-threaded. Guest memory is `HashMap<u32, Box<[u8;4096]>>` with a
per-access flag-check loop. Vector ops are scalarized. The release
profile lacks `panic="abort"`, `strip`, PGO/BOLT. There are **zero
benchmarks** and `rpcs3-perf-monitor` measures nothing.

### Prioritized levers (highest impact first)

| # | Lever | Impact | Effort | State |
|---|---|---|---|---|
| 0 | **Criterion benchmark harness** — can't optimize what you can't measure; do this first | enabler | low | missing |
| 1 | **Wire the existing SPU JIT into `emu-core` + FFI** (swap `InterpreterExecutor`→`RecompilerExecutor` behind a flag) | high | low–med | **mostly EXISTS** |
| 2 | **Flat mmap memory backend** (`LinearBackend`, `*mut u8` + masked offset) replacing the hashmap-of-pages | high | med | designed, missing |
| 3 | **Multi-threaded SPU scheduling** (1 PPU + 6 SPUs on host threads; reuse atomic page/reservation tables) | high | high | missing |
| 4 | **PPU recompiler** (Cranelift, mirror the proven SPU JIT for PPC64) | high | high | missing |
| 5 | **Aggressive release profile** (`panic="abort"`, `strip`, try `lto="fat"`) | low–med | trivial | missing |
| 6 | **Real SIMD** for vector ops (Cranelift `I32X4`/`F32X4`; `std::arch` in interp) — replaces `for lane in 0..4` scalar loops | med | med | missing |
| 7 | **PGO, later BOLT** — esp. effective on the giant interpreter `match` dispatch | med | low–med | missing |
| — | **GPU rendering backend** (separate giant tail; nothing renders without it) | — | very high | deferred |

> **Cheapest big wins:** #1 (JIT already built, just unplugged) and #2
> (already designed) + #5 (profile flags). Landing those three is the
> most credible first step toward "toaster-playable" — *before* the
> much larger PPU-JIT, threading, and GPU efforts.

---

## 4.1 Backend choice — Cranelift vs LLVM (researched 2026-05-28)

**Question:** is Cranelift a mistake — would the LLVM the original RPCS3
uses be faster? **Verdict: keep Cranelift.** It will *not* generate
faster code than LLVM, but that is the wrong axis to optimize for this
project. Reserve LLVM only as an optional, feature-flagged **Tier-2**
if benchmarks ever prove a profiled-hot block needs it.

### The trade-off is two-axis, not one

| | Generated-code quality (throughput) | Compile latency |
|---|---|---|
| **LLVM -O2/-O3** | 🏆 best | 🐢 slow |
| **Cranelift** | ≈ LLVM **-O0/-O1** | 🏆 fast & predictable |

Verified numbers (CGO 2024, Engelke & Schwarz — same IR through both):

- Cranelift code ≈ **LLVM -O0** quality; LLVM -O2/-O3 is **~1.1–2×
  faster** on general code, and **several×** on tight loops needing
  cross-function inlining + autovectorization (which Cranelift lacked
  until Wasmtime 36, Nov 2025, still off-by-default).
- The gap is **structural**, not closing: Cranelift's e-graph mid-end
  adds only **~2%** over its own classical pipeline (cfallin, Apr 2026).
- Compile latency: Cranelift is only **~20–35% faster** than *optimized*
  LLVM for like-for-like IR (the folklore "10×" only holds vs full
  -O3). Baseline-vs-optimizing in general: **~15–20× faster compile for
  ~1.1–1.5× slower code** (Wasmtime Winch RFC).

### The original RPCS3 already proves the point

From the local C++ source (`rpcs3-upstream-clean/`):

- **PPU** = LLVM-only recompiler (+ interpreter); no ASMJIT.
- **SPU** = 4 modes: interp static, interp dynamic, **ASMJIT** (x64-only
  fast baseline), **LLVM** (default, optimizing, cross-arch).
- LLVM is only viable because RPCS3 **caches compiled native code to
  disk** (`ppu-<sha1>/v7-kusa-*.obj`, `spu-…-v1-tane.dat`, compile on
  first run, reuse after). There is **no runtime tier-up** — the backend
  is a static config choice.
- **LLVM is not universally faster:** in some titles ASMJIT beats LLVM
  even in steady state (NFS: Most Wanted = 15–18 FPS ASMJIT vs 9–12 FPS
  LLVM, RPCS3 #14752).

→ RPCS3's **ASMJIT SPU recompiler is the precedent for a fast baseline
tier.** Our Cranelift JIT is the *ASMJIT equivalent*, not the LLVM one —
we'd be replicating their fast tier, not their slow one.

### Why Cranelift is right *here* specifically

1. **Pure-Rust, zero toolchain.** `llvm-sys`/`inkwell` work but demand a
   version-matched LLVM C++ toolchain on every build machine (the same
   class of pain already hit via the SPIRV-Tools/glslang submodule),
   add tens of MB to the binary, and forfeit the memory-safe pure-Rust
   win. Cranelift = `cargo build`.
2. **Already built.** The SPU JIT exists in Cranelift and just needs
   wiring; switching to LLVM = a rewrite.
3. **Low-end favors fast, predictable compile.** Felt performance =
   frame-time consistency; a runtime LLVM compile hitch blows the frame
   budget = visible stutter. Cranelift's fast compile + disk caching is
   the better low-end first-run experience.

### Backend choice is the *smallest* lever

```
interpreter → any JIT        5–50×    ← we are HERE (JIT not even wired)
block chaining (we HAVE it!) decisive for hot loops (SPU JIT R4b chain table + R4c SMC)
fastmem (HashMap → mmap)     large
PPU has no JIT at all        large
threading (1 PPU + 6 SPU)    large
─────────────────────────────────────
Cranelift vs LLVM codegen    ~14%–2×  ← the smallest of all
```

### Target architecture — tiered (matches RPCS3 + every production VM)

- **Tier 0** — interpreter (cold code). *Have it.*
- **Tier 1** — Cranelift baseline (warm code). *Built; wire it in.*
- **Tier 2** *(optional, later, feature-flagged)* — LLVM for
  profile-hot blocks only, compiled on a background thread + disk cache.
  *Only if a measured hot path proves codegen quality is the wall.*

This is RPCS3's model (interp → ASMJIT → LLVM) plus automatic
profile-guided tier-up (which RPCS3 itself does not do). Same shape as
V8 (Ignition → Sparkplug → Maglev → TurboFan) and JSC (LLInt →
Baseline → DFG → FTL/LLVM).

**Sources:** CGO 2024 (home.cit.tum.de/~engelke/pubs/2403-cgo.pdf) ·
cfallin.org/blog/2026/04/09/aegraph · Wasmtime Winch RFC
(github.com/bytecodealliance/rfcs) · Wasmtime cranelift/docs/compare-llvm.md
· RPCS3 #14752 (LLVM slower than ASMJIT) · local `rpcs3-upstream-clean`
`system_config_types.h`, `PPUTranslator.h`, `SPURecompiler.h`,
`SPUThread.cpp:1817`, `PPUThread.cpp:5180-5682` (LLVM .obj cache).

---

## 4.2 Measured baseline — interpreter vs Cranelift JIT (2026-05-28)

First numbers from the new `cargo bench -p rpcs3-spu-recompiler`
(`benches/spu_executors.rs`, lever #0). Time is per full `execute()`
(fresh `SpuThread` + deploy + run-to-stop). Machine-relative — a baseline
to track regressions against, not an absolute truth.

| Workload | Interpreter | JIT cold | JIT warm | Warm verdict |
|---|---|---|---|---|
| `branch_loop` (Fibonacci, tiny) | 16.5 µs | 209 µs | 35 µs | JIT **2.1× slower** |
| `mailbox` (channel/fallback) | 19.2 µs | 121 µs | 36 µs | JIT **1.9× slower** |
| **`hot_loop_30k`** (30k-iter loop) | **252 µs** | — | **67 µs** | JIT **3.8× FASTER** |

**Reading:** on the tiny correctness oracles the JIT *loses* — its
compile + dispatch overhead isn't amortized, and per-`execute` cost is
dominated by the 256 KB `SpuThread` setup (note the interpreter is ~the
same ~17–19 µs regardless of program). On a real hot loop (30k
iterations) the JIT is **3.8× faster** — empirical proof that the JIT
pays off *only* for hot code.

**Consequence for lever #1:** do NOT wire the JIT as "always-JIT" — that
would slow down short-lived SPU tasks. The correct design is **tiered**
(interpret cold/short, JIT hot), matching §4.1. Two sub-findings worth
their own levers: (a) the 256 KB `SpuThread` setup dominates short runs
(reuse / avoid re-zeroing), and (b) the JIT break-even sits in the
low-thousands of executed instructions.

---

## 4.3 Lever #1 — wired AND validated end-to-end (commit `35eecb4af`)

Lever #1 is no longer just "mostly EXISTS": the Cranelift SPU JIT is wired into
`emu-core` behind the `spu-recompiler` feature (runtime `EmuCore::spu_backend`,
default Interpreter) **and validated end-to-end through `EmuCore::run_self`
with a real PSL1GHT `.self`** — both backends produce a byte-identical SPU
`OUT_MBOX`.

- Fixture `single_spu_selfcompute_v1` (self-contained: no IN_MBOX / DMA / args —
  the only shape that boots through the synchronous single-SPU run_self path).
- Test `rust/rpcs3-emu-core/tests/spu_selfcompute_jit.rs`: interpreter +
  recompiler both yield `OUT_MBOX = 0x7A314`. Gate **281 blocks / 0 fail /
  6016 tests** (was 280/6015 — zero regression).
- **Bug fixed along the way:** emu-core's `sysSpuImageImport` (NID `0xebe5f72f`)
  over-read a fixed 256 KB blob and silently swallowed failures, so **no
  PSL1GHT homebrew had ever actually run its SPU through `run_self`** (R9
  validated SPU only via the `run_spu_group_single` helper). Now reads the
  largest mappable span + logs failures.

Remaining (does not block the JIT validation, tracked as follow-ups):
- **Tiered promotion** — the JIT is still all-or-nothing per the selector; the
  §4.2 data says the right model is *interpret cold/short, JIT hot* (hotness
  counter + background compile). Not built.
- **Faithful PPU exit code for SPU homebrews** — `sysSpuImageClose` / the
  newlib exit import (`sysPrxForUser 0xe0da8efd`) are unimplemented, so the PPU
  *process* exit code is unreliable through run_self (the test asserts on the
  SPU OUT_MBOX instead).

---

## 5. Honest reality check

Running a **real commercial game on low-end hardware is far off** —
months of focused performance + GPU work, not a tuning pass. Today the
port is a proof-of-concept that boots PSL1GHT homebrew through 20 SPU
oracles and replays a real libgcm frame's command stream. It has **no
PPU JIT, no live SPU JIT, no GPU backend, single-threaded execution,
and a hashmap memory model** — so it cannot render a frame of an actual
game at any speed yet. The genuinely encouraging part: the two
highest-impact levers are unusually cheap because the SPU JIT is
already written (just unwired) and the flat-mmap backend is already
designed. The behavior-freeze foundation is solid and well-tested — it
is the right base to optimize *from*.

---

## 6. How this audit was produced

6 parallel subagents, one per subsystem (SPU · PPU/LV2 · RSX · HLE ·
emu-core/loaders · performance), each instructed to read real `.rs`
source, distinguish real implementations from stubs (`grep
unimplemented!/todo!`, LOC, `#[test]` counts), and report **evidence
(file:line)** rather than trust the narrative docs. Cross-checked
against a freshly re-run `cargo test --workspace --tests --release`
(green: 280 blocks / 6015 tests). Two agents were re-run after a
transient API/network drop.
