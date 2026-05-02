# R6 Live Bridge — RPCS3 C++ ↔ Rust SPU Executor

**Status:** R6.0 (design + Rust-side FFI scaffolding) **DONE — 2026-04-29**;
R6.0b (build artifact validation) **DONE — 2026-04-29**; R6.0c
(C header generation) **DONE — 2026-04-29**.
R6.1+ pending explicit user authorization (C++ patch changes start at R6.1).

This doc enumerates the phased plan to replace RPCS3's C++ SPU
executor with the Rust stack at runtime. The R5 phase closure
delivered 4 replay-validated oracle fixtures
(`single_spu_mailbox_v1`, `single_spu_branch_loop_v1`,
`single_spu_signal_v1`, `single_spu_loadstore_v1`), all passing
`diff_snapshots(interp, jit).is_identical()` byte-identically.
Those 4 fixtures are R6's regression sentinel — every bridge
change must keep all 4 acceptance gates green.

## Phasing

| Phase | Scope | Touches C++? | Authorization |
|---|---|---|---|
| **R6.0** ✅ DONE 2026-04-29 | Plan + Rust-side FFI scaffolding (`rpcs3-spu-ffi` crate with C-ABI surface). No C++ changes; existing patches preserved. | NO | implicit (continuation of R5 closure) |
| **R6.0b** ✅ DONE 2026-04-29 | Build artifact validation: `cargo build --release -p rpcs3-spu-ffi` produces `target/release/rpcs3_spu_ffi.lib` (12 MB COFF archive) + `librpcs3_spu_ffi.rlib` (103 KB). All 14 `rust_spu_*` symbols verified present in the staticlib. Tests + workspace lib green. | NO | (continuation) |
| **R6.0c** ✅ DONE 2026-04-29 | C header generation via cbindgen. `rust/rpcs3-spu-ffi/cbindgen.toml` config + `rust/rpcs3-spu-ffi/include/rpcs3_spu_ffi.h` auto-generated header (with C++-compat `extern "C"` block + `#pragma once` + `#include <stdint.h>`/`<stddef.h>` + opaque `typedef struct rust_spu_t rust_spu_t` + flat `rust_spu_outcome_t` enum prefixed to avoid Windows.h macro collision). Rust struct `RustSpuHandle` renamed to `RustSpu` for cleaner C-side name. Smoke-tested via `gcc 12.2 -Wall -Wextra -Werror -c smoke_test.c` inside the `ps3-build` container — produces a 2760-byte object file successfully. The `rpcs3-spu-ffi/include/smoke_test.c` is tracked as a regression sentinel. | NO | (continuation) |
| **R6.1** | C++ side: add a new SPU backend stub that links against the Rust FFI crate and delegates `cpu_task()` to it for opt-in threads. New patch `spu_rust_bridge.patch` (additive, doesn't modify existing scaffolding/runtime_hooks patches). | YES (NEW patch) | **REQUIRES EXPLICIT AUTH** — first time C++ patches grow beyond the trace writer |
| **R6.2** | Wire opt-in via env var `RPCS3_SPU_RUST_BRIDGE=1` (or similar). Defaults OFF — production path unchanged. Add a hello-world headless test: launch `single_spu_mailbox_v1.self` with the bridge on, expect same trace output. | YES | requires R6.1 |
| **R6.3** | Connect the C++ trace writer to the Rust executor's snapshot. The captured trace under bridge-on must match the captured trace under bridge-off (= existing fixtures). | YES | requires R6.2 |
| **R6.4** | Run all 4 oracle fixtures with bridge ON; assert byte-identical to the existing captures. This is the load-bearing acceptance gate for R6. | YES | requires R6.3 |
| **R6.5** | Performance polish — reduce FFI overhead (batch instructions per call, share LS pointer instead of copying, etc.). | YES | requires R6.4 (correctness baseline) |
| **R6.6** | Multi-thread support: bridge multiple cooperative SPU threads simultaneously. Per-thread Rust state via thread-local handles. | YES | requires R6.5 |
| **R6.7** | Production-readiness: stress tests on real homebrew (e.g., the v3/v4 spurs_test diagnostic that's currently DMA-bound). DMA support is out of scope here — that's R5.12. | YES | requires R6.6 |

## R6.0 deliverables (this iteration)

1. This doc.
2. New crate `rust/rpcs3-spu-ffi/` exposing a C-ABI surface that
   wraps the existing `rpcs3-spu-{thread,interpreter,differential,recompiler}`
   stack.
3. Internal Rust tests calling the C-ABI as if from C (via `unsafe`
   blocks + raw pointers) — exercise the same flows the 4 oracle
   fixtures exercise, but through the FFI surface.
4. `rpcs3-spu-ffi` added to the workspace `Cargo.toml` as a member.
5. `cargo test --workspace --lib` and `--tests` continue to pass.
6. C++ patches NOT modified (`check_patch_separation.py` exits 0,
   sha256s preserved).

R6.0 is purely additive. The 4 oracle fixtures continue to pass
unchanged. No production code path touches the FFI crate yet.

## C-ABI surface (`rpcs3_spu_ffi.h` — auto-generated header is R6.1+)

```c
// Opaque handle.
typedef struct rust_spu_handle rust_spu_handle_t;

// === Lifecycle ===
rust_spu_handle_t* rust_spu_new(void);
void rust_spu_drop(rust_spu_handle_t* h);

// === Initial-state setup (called before first run) ===
// Load `size` bytes from `bytes` into the Rust executor's LS at offset 0.
// Caller-side LS layout assumed contiguous 256 KiB.
int32_t rust_spu_load_ls(
    rust_spu_handle_t* h,
    const uint8_t* bytes,
    uint32_t size
);

// Set GPR `reg` (0..127) to the 128-bit value in `bytes` (BE-order).
int32_t rust_spu_set_gpr(
    rust_spu_handle_t* h,
    uint32_t reg,
    const uint8_t bytes[16]
);

// Set initial PC.
int32_t rust_spu_set_pc(rust_spu_handle_t* h, uint32_t pc);

// === Channel ops (PPU-side) ===
int32_t rust_spu_push_inmbox(rust_spu_handle_t* h, uint32_t value);
int32_t rust_spu_pop_outmbox(
    rust_spu_handle_t* h,
    /* out */ uint32_t* value
);
int32_t rust_spu_signal(
    rust_spu_handle_t* h,
    uint32_t slot,
    uint32_t value
);

// === Run / step ===
typedef enum {
    RUST_SPU_OUTCOME_CONTINUE     = 0,
    RUST_SPU_OUTCOME_STOP         = 1,
    RUST_SPU_OUTCOME_STALL_READ   = 2,
    RUST_SPU_OUTCOME_STALL_WRITE  = 3,
    RUST_SPU_OUTCOME_ERROR        = 4,
} rust_spu_outcome_t;

rust_spu_outcome_t rust_spu_step(
    rust_spu_handle_t* h,
    /* out */ uint32_t* code_or_channel
);
rust_spu_outcome_t rust_spu_run_until_event(
    rust_spu_handle_t* h,
    uint32_t max_steps,
    /* out */ uint32_t* code_or_channel,
    /* out */ uint32_t* steps_taken
);

// === Query state (after stop / stall) ===
int32_t rust_spu_get_pc(rust_spu_handle_t* h, /* out */ uint32_t* pc);
int32_t rust_spu_get_gpr(
    rust_spu_handle_t* h,
    uint32_t reg,
    /* out */ uint8_t bytes[16]
);
int32_t rust_spu_get_ls(
    rust_spu_handle_t* h,
    /* out */ uint8_t* bytes,
    uint32_t size
);
```

## Design choices (R6.0)

- **Opaque handle.** C side never sees Rust's `SpuThread` directly.
  The handle is allocated via `Box::leak` and freed on `rust_spu_drop`.
  No shared mutable state between threads (each handle is `!Sync`).
- **Mutex-free.** R6.0 assumes single-threaded access per handle.
  R6.6 will add multi-thread support if needed; the simplest path is
  one handle per SPU thread (lifecycle owned by the C++ `spu_thread`).
- **No `panic` across the FFI boundary.** Every entry point catches
  Rust panics via `std::panic::catch_unwind` and returns a
  non-zero error code.
- **Return-code convention.** All non-outcome functions return
  `int32_t`: 0 on success, non-zero on error. `rust_spu_outcome_t`
  is the typed return for run/step.
- **Big-endian byte order on the wire.** GPRs and LS are passed as
  `uint8_t[]` in BE byte order (matching SPU architecture). This
  decouples the FFI from host endianness and matches what the
  existing trace writer + replay engine already do.

## What R6.0 does NOT do

- ❌ Build a C header file (deferred to R6.1; for now the contract is
  documented in this file + the Rust code's `extern "C"` declarations).
- ❌ Modify any C++ source or existing patches.
- ❌ Wire the bridge into RPCS3's runtime path.
- ❌ Add cbindgen / build.rs / dynamic-library packaging.
- ❌ Define cross-language error reporting (stack-trace passthrough,
  etc.). R6.0 just returns numeric error codes.

## Open questions (deferred to R6.1)

1. Static library vs dynamic library? (cdylib in Cargo.toml gives a
   shared lib; staticlib gives a `.a`. RPCS3's build system prefers
   static linkage; lean toward `staticlib`.)
2. Memory ownership: C++ allocates the LS as part of `spu_thread`.
   Should Rust read from that pointer directly (zero-copy), or copy
   in/out per call? Zero-copy is faster but requires careful lifetime
   management. R6.5 perf decision.
3. Calling convention: `extern "C"` is the default. No `extern
   "system"` / Windows-specific concerns since RPCS3 builds on Win/
   Mac/Linux uniformly.

## Acceptance gate for R6.0

| Check | Expected |
|---|---|
| `cargo test -p rpcs3-spu-ffi --lib` | PASS (Rust-side FFI tests) |
| `cargo test --workspace --lib --no-fail-fast` | PASS, 0 failed |
| `cargo test --workspace --tests --no-fail-fast` | PASS, 0 failed |
| 4 oracle fixture acceptance gates | PASS (unchanged) |
| `python behavior-freeze/harness/check_patch_separation.py` | exit 0; sha256s preserved |
| `python behavior-freeze/harness/check_trace_fixtures.py` | exit 0; `REPLAY_VALIDATED_TRACE_EXISTS = True` |
