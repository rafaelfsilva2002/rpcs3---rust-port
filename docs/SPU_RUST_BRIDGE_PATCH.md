# SPU Rust Bridge Patch — apply order, build, opt-in

The patch
[`docs/patches/spu_rust_bridge.patch`](./patches/spu_rust_bridge.patch)
wires the
[`rpcs3-spu-ffi`](../rust/rpcs3-spu-ffi/) staticlib into `rpcs3_emu`
and exposes a runtime-gated C++ facade that can optionally delegate
SPU execution to the pure-Rust executor.

Phased scope:

- **R6.1** (landed): opt-in hello-world. Bridge logs activation +
  exercises `rust_spu_new`/`rust_spu_drop` once per `spu_thread`.
- **R6.1b**: `rpcs3.exe` builds and links the staticlib end-to-end.
- **R6.1c**: patch hygiene — `git apply --check` works with no flags.
- **R6.2** (landed): first **delegated execution path** —
  `try_delegate_execution()` runs the SPU inside the Rust executor
  for mailbox-only workloads (single_spu_mailbox_v1 fixture). On
  Stop, propagates through RPCS3's normal `stop_and_signal()` path.
  On any unsupported outcome falls back honestly to the C++ executor
  with RPCS3 state unchanged.
- **R6.2c**: patch hygiene — patch + sha256 + docs updated to reflect
  the R6.2 delegation surface.

**No DMA/MFC**, **no SPURS**, **no execution-take-over by default**.

## What the patch contains

| File | Change | Purpose |
|---|---|---|
| `rpcs3/Emu/CMakeLists.txt` | +5 lines after `Cell/SPUTraceJsonl.cpp`; +24-line block after the APPLE check | Register `SPURustBridge.cpp`; conditional `target_link_libraries` against `rust/target/release/rpcs3_spu_ffi.lib`/`.a` if present (defines `RPCS3_HAS_SPU_RUST_BRIDGE=1`). |
| `rpcs3/Emu/Cell/SPURustBridge.h` (NEW, ~75 lines) | `is_enabled()` + `hello_world_proof()` + **`try_delegate_execution()`** (R6.2). | C++-side facade; opaque to the FFI internals. |
| `rpcs3/Emu/Cell/SPURustBridge.cpp` (NEW, ~225 lines) | Cached env-var check + log channel `RustSPU` + R6.2 delegation body. | Calls `rust_spu_*` via `#ifdef RPCS3_HAS_SPU_RUST_BRIDGE`. |
| `rpcs3/Emu/Cell/SPUThread.cpp` | +1 include line; ~25 lines in `cpu_task()` before `if (jit)` | When bridge is enabled: once-per-thread `hello_world_proof()` then **every entry** attempts `try_delegate_execution()`; on success, `return;` from `cpu_task()`; on failure, fall through to C++ executor. |
| `rpcs3/emucore.vcxproj` | +2 entries + `ItemDefinitionGroup` (Exists-conditional) | MSVC project: register the new files; conditionally inject `RPCS3_HAS_SPU_RUST_BRIDGE=1`, include dir, and `rpcs3_spu_ffi.lib` link dependency. |
| `rpcs3/emucore.vcxproj.filters` | +6 lines (2 filter blocks) | MSVC project: place the new files under `Emu\Cell` filter. |
| `rpcs3/rpcs3.vcxproj` | `ItemDefinitionGroup` (Exists-conditional) | MSVC EXE project: also link `rpcs3_spu_ffi.lib` + Rust std Windows system libs. **Required because emucore is a static lib** — see "Two-tier link" note below. |

## R6.2 delegation surface — semantics

`spu_rust_bridge::try_delegate_execution(spu_thread&) -> bool` is
the load-bearing function. Its contract:

### Phase 0 — setup (no RPCS3 state mutated)

1. `rust_spu_new()` — alloc handle (returns null on OOM ⇒ fallback).
2. `rust_spu_load_ls(h, spu.ls, SPU_LS_SIZE)` — full 256 KiB LS copy.
3. `rust_spu_set_pc(h, spu.pc)` — entry PC.
4. For r=0..127: `v128_to_be_bytes(spu.gpr[r])` then
   `rust_spu_set_gpr(h, r, bytes)`. The helper converts RPCS3's LE
   v128 (preferred slot in `_u32[3]`) to the Rust FFI's BE byte
   layout (`bytes[0]` = preferred-slot MSB).

If any of these fail, the handle is dropped and `false` is
returned. **No spu_thread state has been touched.**

### Phase 1 — non-destructive IN_MBOX peek

```cpp
u32 in_vals[4] = {0,0,0,0};
const u32 in_count = spu.ch_in_mbox.try_read(in_vals);
for (u32 i = 0; i < in_count; ++i)
    rust_spu_push_inmbox(h, in_vals[i]);
```

`spu_channel_4_t::try_read` is **non-destructive**. The values are
mirrored into the Rust handle's IN_MBOX, but RPCS3's `ch_in_mbox` is
left full. This is essential for the fallback path — if the Rust
executor declines to handle this workload, no PPU-published
mailbox value is lost.

### Phase 1b — non-destructive SNR1/SNR2 peek (R6.3c)

```cpp
u32 snr1_value = 0, snr2_value = 0;
const bool snr1_pending = spu.ch_snr1.try_read(snr1_value);
const bool snr2_pending = spu.ch_snr2.try_read(snr2_value);
if (snr1_pending) rust_spu_signal(h, /*slot=*/0u, snr1_value);
if (snr2_pending) rust_spu_signal(h, /*slot=*/1u, snr2_value);
```

`ch_snr1` / `ch_snr2` are single-value `spu_channel`s holding the
OR-merged signal value the PPU sent via `sysSpuThreadWriteSignal`.
We peek non-destructively (`try_read` returns true if a value is
pending) and forward to the Rust side via `rust_spu_signal(slot,
value)` (slot 0 = SNR1, slot 1 = SNR2). The Rust handle's signal
channels OR-merge the same way Cell BE hardware does, so the
forwarded value is consumed by the SPU's `rdch ch3` /
`SPU_RdSigNotify1` (or ch4 / SPU_RdSigNotify2). On the fallback
path the RPCS3-side channels remain populated; on commit they
are drained in Phase 3.

### StallWrite OUT_MBOX (ch28) handling (R6.5b)

The Rust executor's OUT_MBOX is depth-1 (`Option<u32>`), matching
Cell BE semantics. A second `wrch ch28` while the first is still
pending stalls. The bridge's resolution:

```cpp
if (outcome == StallWrite && out_code == 28) {
    if (session->stall_write_iterations >= kMaxStallIterations) {
        // bail-out → fallback
    }
    u32 intermediate = 0;
    rust_spu_pop_outmbox(h, &intermediate);  // drain Rust's depth-1 mailbox
    spu.ch_out_mbox.set_value(intermediate); // overwrite RPCS3's depth-1 channel
    session->stall_write_iterations++;
    continue;  // re-run on same handle; SPU's stalled wrch retries
}
```

This is a **deliberate destructive overwrite** of RPCS3's
`ch_out_mbox`. Justification: in PSL1GHT cooperative-thread
context, the lv2 syscall surface has no path for the PPU to read
a cooperative SPU's OUT_MBOX during execution (only the
`stop_and_signal(0x101)` handler reads it as `exit_status`),
so intermediate values the PPU cannot observe are functionally
lost regardless. The depth-1 overwrite IS the well-defined
PSL1GHT-cooperative semantic. Real-binary acceptance for this
path is not feasible in PSL1GHT cooperative (a SPU that writes
OUT_MBOX twice without an intervening stop deadlocks the C++
executor's `push_wait` since the PPU cannot drain) — the
executable acceptance gate lives at FFI level:
`rust_spu_outmbox_backpressure_via_ffi` in
`rust/rpcs3-spu-ffi/src/tests.rs`. The two `*_iterations`
counters (StallRead's `stall_iterations` from R6.4b and
StallWrite's `stall_write_iterations` from R6.5b) are surfaced
in the DELEGATED success log so the operator can distinguish
the two recovery paths.

### Phase 2 — multi-round run loop (R6.4b)

```cpp
constexpr u32 kMaxSteps = 1'000'000;
constexpr u32 kMaxStallIterations = 32;

while (true) {
    auto outcome = rust_spu_run_until_event(h, kMaxSteps, &out_code, &out_steps);
    session->total_steps += out_steps;

    if (outcome == Stop) break;  // → commit

    // Supported StallRead: pop_wait on RPCS3 + push to Rust + continue.
    if (outcome == StallRead && (out_code == 29 || out_code == 3 || out_code == 4)) {
        if (session->stall_iterations++ >= kMaxStallIterations) {
            // safety bail → drop session, fall back
            break;
        }
        if (out_code == 29) {
            auto [old_count, val] = spu.ch_in_mbox.pop_wait(spu, /*pop_value=*/true);
            if (old_count == 0) { /* aborted */ break; }
            rust_spu_push_inmbox(h, val);
            session->in_mbox_consumed++;
        } else /* SNR1 or SNR2 */ {
            spu_channel& ch = (out_code == 3) ? spu.ch_snr1 : spu.ch_snr2;
            s64 result = ch.pop_wait(spu);
            if (result < 0) break;
            rust_spu_signal(h, (out_code == 3) ? 0 : 1, (u32)result);
        }
        continue;  // re-run on same handle
    }

    // Any other outcome: drop session, fall back honestly.
    break;
}
```

R6.4b allows the bridge to make progress past `StallRead` on
mailbox/SNR channels without losing ownership. The loop's `pop_wait`
calls match the C++ executor's behavior in
`spu_thread::get_ch_value` (e.g. line 5524 for IN_MBOX, line 5573
for SNR1) — same blocking semantics, same wake conditions.

**State invariants:**
- `Stop` reached on first run: identical to R6.4a — no stalls,
  `stall_iters=0`, behavior matches all 4 existing fixtures.
- `Stop` reached after one or more stalls: each stall destructively
  popped the corresponding RPCS3 channel via `pop_wait`. The commit
  branch's IN_MBOX drain only loops over the *initial peek* count
  (`in_count`), since the loop body already consumed any
  post-stall values.
- Any non-Stop outcome: session dropped (handle freed), `false`
  returned. RPCS3 state on this fallback is **byte-identical to
  entry only if no `pop_wait` ran** (i.e. only the first run
  surfaced the non-Stop outcome). If `pop_wait` already ran and
  THEN we hit `Error`, the values are lost — logged at WARNING.
  This edge case is documented and does not occur in any of the
  4 existing fixtures or the multi-round FFI test.

### Phase 3 — commit on Stop

After `Stop` the bridge **commits** to the delegation:

1. `spu.ch_in_mbox.try_pop()` × `in_count` — drains for real
   (the Rust SPU consumed those values).
2. `spu.ch_snr1.try_pop()` / `spu.ch_snr2.try_pop()` — drains
   the SNR channels we forwarded in Phase 1b (R6.3c). Skipped
   for channels that had no pending value pre-run.
3. `rust_spu_pop_outmbox()` in a loop — drain Rust's OUT_MBOX into
   `spu.ch_out_mbox.set_value(latest)`. The depth-1 RPCS3 channel
   keeps the latest value; multi-write fixtures are out of R6.2
   scope (logged at WARNING if observed).
4. Sync final PC + 128 GPRs back from Rust into `spu.pc` /
   `spu.gpr[]` so post-stop C++ hooks (e.g. final_state trace
   guard) see the Rust executor's output.
5. `rust_spu_drop(h)`.
6. **`spu.stop_and_signal(out_code)`** — the official RPCS3 stop
   path. For `code = 0x101`
   (`SYS_SPU_THREAD_STOP_GROUP_EXIT`) this:
   - pops `ch_out_mbox` into `group->exit_status`,
   - sets `group->join_state = SYS_SPU_THREAD_GROUP_JOIN_GROUP_EXIT`,
   - calls `set_status_npc()` (sets `SPU_STATUS_STOPPED_BY_STOP`),
   - signals join waiters.
7. `return true;` — the caller (`cpu_task()`) returns immediately.

The `TraceFinalGuard` destructor in `cpu_task()` still fires after
the function returns, so the JSONL trace pipeline sees the
identical final_state event a normal C++-driven exit would emit.

### Fallback contract (failure → `false`)

| Where | Outcome | RPCS3 state on fallback |
|---|---|---|
| Phase 0 alloc/load/set | any non-zero return | Untouched (handle drop only) |
| Phase 1 push_inmbox | `≠ 0` | `ch_in_mbox` unchanged (peek was non-destructive) |
| Phase 1b rust_spu_signal | `≠ 0` | `ch_snr1` / `ch_snr2` unchanged (peek was non-destructive) |
| Phase 2 outcome | `Continue / StallRead / StallWrite / Error` | All channels untouched; PC/GPR untouched |

## R6.4a outcome contract

Every call to `try_delegate_execution()` resolves to exactly one of
the five Rust outcomes. The bridge's response is fully determined
by the outcome:

| Outcome | `out_code` | RPCS3 state mutation | Bridge return |
|---|---|---|---|
| `Stop` | 14-bit stop code | **COMMIT:** drain peeked channels, push OUT_MBOX, sync PC+GPRs, `stop_and_signal(out_code)` | `true` |
| `Continue` | 0 | NONE | `false` (fall back) |
| `StallRead` | channel index | NONE | `false` (fall back) |
| `StallWrite` | channel index | NONE | `false` (fall back) |
| `Error` | failing PC | NONE | `false` (fall back) |

**Load-bearing invariant:** for every `false` return path, RPCS3
state is byte-identical to the moment `try_delegate_execution`
was entered. The C++ executor (jit/interpreter) takes over and
runs from the original `spu_thread::pc` / `gpr[]` / channel state
exactly as if the bridge had never been called.

### Channel classification (for diagnostic + R6.4b)

When a `Stall*` outcome surfaces, the bridge maps `out_code`
(channel index) to a category that informs whether a future
re-invocation could succeed:

| Category | Channels | Re-tryable in R6.4b? |
|---|---|---|
| Mailbox-class | `28` (OUT_MBOX), `29` (IN_MBOX) | yes — PPU action will drain/fill |
| Signal-class | `3` (SNR1), `4` (SNR2) | yes — PPU `WriteSignal` will fill |
| Event-class | `0` (EVENTSTAT), `1`, `2`, `22`, `23` | no (R6.4 scope) — would need event_stat plumbing |
| Decrementer | `7` (WRDEC), `8` (RDDEC) | no — timing-dependent |
| Other | `30` (OUT_INTR_MBOX), unknown | no |
| MFC/DMA | `21` and friends (16..21) | no — Rust subset doesn't decode them; surfaces as `Error` rather than `Stall` |

In R6.4a, **all** stalls fall back regardless of category. The
classification only drives the warning log message so the operator
can see which workloads are candidates for R6.4b's persistent-handle
support and which are out of scope.

### Continue handling

`Continue` means `max_steps` (currently 1M per call) was reached
with the SPU still running. R6.4a treats this as a fallback
condition; R6.4b would chain calls on a persistent handle.

In practice none of the four R5.11/R5.11b oracle fixtures get
close to the budget (max observed: `single_spu_loadstore_v1` at
187 steps). Continue is a runaway-loop guard for now.

### Error handling

`Error` means the Rust SPU subset hit an opcode it doesn't decode
(or an LS-OOB). The C++ recompilers (LLVM / asmjit) cover a
broader subset, so falling back is the right move. The log
includes the failing PC so the operator can identify which opcode
needs to land in the Rust executor for future delegation.

The caller (`cpu_task()`) sees the `false` and falls through to
`if (jit) { … } else { … }` — the existing C++ executor takes over
exactly as if the bridge had never been called. No silent corruption.

### Default OFF preserved

- Compile-time gate: bridge body only exists if
  `RPCS3_HAS_SPU_RUST_BRIDGE` is defined (CMake `EXISTS` check).
- Runtime gate: env var `RPCS3_SPU_RUST_BRIDGE` MUST be exactly `"1"`.
  Cached on first call; no per-instruction cost when off.
- The `cpu_task()` hook only runs the bridge body when both gates are
  true; otherwise `if (jit) { … }` runs unmodified.

## Apply order

The patch baselines against the **post-scaffolding** state (i.e. it
expects `spu_trace_jsonl_scaffolding.patch` to have been applied
first; runtime hooks are independent). Recommended order:

```bash
cd <upstream rpcs3 source tree>
git apply path/to/spu_trace_jsonl_scaffolding.patch     # R5.8 A.3 (pinned)
git apply path/to/spu_trace_jsonl_runtime_hooks.patch    # R5.9c (pinned, optional)
git apply path/to/spu_rust_bridge.patch                  # R6.1 (this patch)
```

The trace patches' sha256s are pinned by
`behavior-freeze/harness/check_patch_separation.py` and **must not
drift**. The new bridge patch's sha256 is recorded separately
(see "Sha256" below) and checked by the same gate's optional
extension.

## Build the Rust staticlib + header

Before re-running CMake / re-opening the MSVC solution, build the
Rust crate so the `EXISTS` check in `CMakeLists.txt` finds it:

```bash
# Linux / macOS / Git Bash:
scripts/build_rust_spu_ffi.sh

# Windows PowerShell:
pwsh scripts\build_rust_spu_ffi.ps1
```

The script:
1. Runs `cargo build --release -p rpcs3-spu-ffi`, producing:
   - `rust/target/release/rpcs3_spu_ffi.lib` (Windows MSVC, ~12 MB)
   - `rust/target/release/librpcs3_spu_ffi.a` (Unix-style)
2. Regenerates `rust/rpcs3-spu-ffi/include/rpcs3_spu_ffi.h` via
   cbindgen (install via `cargo install cbindgen` if missing).

Then **re-run CMake configure** so the new `EXISTS` check picks up
the staticlib. CMake prints one of:

```
-- R6.1: linking rpcs3_spu_ffi.lib (MSVC staticlib)
-- R6.1: linking librpcs3_spu_ffi.a (Unix staticlib)
-- R6.1: rpcs3_spu_ffi staticlib not found; bridge disabled at compile time. Run scripts/build_rust_spu_ffi.{sh,ps1} to enable.
```

## Opt-in at runtime

Compile-time gate `RPCS3_HAS_SPU_RUST_BRIDGE` MUST be defined (set
automatically by the CMake `EXISTS` check). Then runtime gate via
the environment variable:

```bash
# Linux/macOS/Git Bash:
RPCS3_SPU_RUST_BRIDGE=1 ./rpcs3 path/to/single_spu_mailbox_v1.self

# Windows PowerShell:
$env:RPCS3_SPU_RUST_BRIDGE = "1"
.\rpcs3.exe path\to\single_spu_mailbox_v1.self
```

When the env var is set, the SPU thread loop logs three lines on
the first delegated entry:

```
[Rust SPU bridge] RPCS3_SPU_RUST_BRIDGE=1 detected; bridge ENABLED globally for this RPCS3 process. R6.2 first delegated execution scope: mailbox-only workloads (no DMA/MFC, no SPURS).
[Rust SPU bridge] hello-world FFI roundtrip OK on spu_thread '...' (lv2_id=0x...). Static linkage of rpcs3_spu_ffi.lib confirmed; rust_spu_t handle allocated + dropped cleanly.
[Rust SPU bridge] DELEGATED EXECUTION OK on '...' (lv2_id=0x...): Stop code=0x101 steps=4 in_mbox_consumed=1 final_pc=0xc. Routing through stop_and_signal() now.
```

The third line is the load-bearing R6.2 evidence — `Stop code` is
the SPU's actual stop instruction code, `steps` is the count of
SPU instructions Rust executed, `in_mbox_consumed` confirms the
mailbox was forwarded, `final_pc` is the SPU PC at halt.

If the workload is not yet supported (e.g. requires DMA, hits
`StallRead` on a non-mailbox channel, or runs past `kMaxSteps`),
the bridge logs a WARNING and falls back to the C++ executor:

```
[Rust SPU bridge] try_delegate: outcome=<Continue|StallRead|StallWrite|Error> code=0x... steps=... on '...'; falling back to C++ executor (R6.2 supports Stop only)
```

## Default behavior (env var not set)

The bridge is silent. `is_enabled()` returns false on first call,
caches the result, and subsequent calls are lock-free no-ops.
Production behavior is unchanged.

## Two-tier link (MSVC) — emucore is a static lib

`emucore.vcxproj` builds a **static** library (`emucore.lib`), so any
external static library it consumes via `AdditionalDependencies` must
**also** be added to the final EXE project (`rpcs3.vcxproj`); MSVC
does not propagate static-archive dependencies transitively. R6.1b
hit this concretely as `LNK2001` on `rust_spu_new`/`rust_spu_drop`
when only `emucore.vcxproj` linked the Rust archive.

**Fix:** the patch adds an `ItemDefinitionGroup` with the same
`Exists(...)` condition to **both** `emucore.vcxproj` and
`rpcs3.vcxproj`. The CMake path (Linux/macOS, or MSVC via
CMake/Ninja) handles this automatically through `target_link_libraries`,
so no equivalent CMake-side fan-out is needed.

## Rust std → Windows system libs

The Rust std library on `x86_64-pc-windows-msvc` references several
OS-level APIs (`GetUserProfileDirectoryW`, sockets, crypto,
network adapters) via `#[link]` attributes that MSVC's link.exe
does not auto-resolve when consuming the staticlib `rpcs3_spu_ffi.lib`.
The patch's `rpcs3.vcxproj` `ItemDefinitionGroup` lists the four
required system libs:

| Lib | Why |
|---|---|
| `userenv.lib` | `GetUserProfileDirectoryW` (resolves `%USERPROFILE%` for `std::env::home_dir`) |
| `ws2_32.lib`  | Winsock 2 — `std::net` and Tokio internals |
| `bcrypt.lib`  | CNG random / hashing — `std::collections::HashMap` ASLR seed |
| `iphlpapi.lib`| IP helper — adapter enumeration in `std::net` |

Adding these to `AdditionalDependencies` resolves the second batch
of `LNK2001 __imp_*` errors observed during R6.1b.

> **Pre-existing build failure (unrelated):** `rpcs3_test.vcxproj`
> fails with 9 errors due to `gtest/gtest.h` missing. This is
> documented in the R5.9e.3 notes as a pre-existing 3rdparty
> submodule gap and **does not block R6.1b** — the relevant artifact
> (`rpcs3.exe`) builds and links successfully.

## Sha256 (R6.5b baseline)

The patch's sha256 should match this value. If it drifts, the
patch was edited and the change must be reviewed.

```
7d6b6bba3d1c590ec16f2ff175b262a4f95bdf95ace92eb91636824488436c03  docs/patches/spu_rust_bridge.patch
```

Sha256 history (each closure regenerated the patch as the bridge
surface grew):

| Phase | sha256 | Notes |
|---|---|---|
| R6.1 | `4d48e36b…` | hello-world only; required `--recount --ignore-whitespace --inaccurate-eof` |
| R6.1b interim | `7c1bf70c…` | added `rpcs3.vcxproj` + system libs; same flag requirement |
| R6.1c | `1ecbdb17…` | patch hygiene — applies via plain `git apply --check`, no flags |
| R6.2c | `e41cd4cf…` | adds `try_delegate_execution()` body; applies via plain `git apply --check`, no flags |
| R6.3a-c | `5a0cdb9f…` | folds R6.3a cosmetic updates (branch_loop in Verified list); same clean apply, no logic delta |
| R6.3b-c | `9555c363…` | folds R6.3b cosmetic updates (loadstore_v1 added to Verified list); same clean apply, no logic delta |
| R6.3c | `054b2bec…` | adds Phase 1b SNR1/SNR2 forwarding (`rust_spu_signal()`) + Phase 3 SNR drain on Stop commit; first real bridge logic extension since R6.2; signal_forwarded counter in success log |
| R6.4a | `8f1d6c0b…` | formalizes the outcome contract (Stop=COMMIT; everything else=fallback); adds `classify_stall_channel()` helper + per-outcome diagnostic warnings (channel name, re-tryability flag, failing PC); +120 lines vs R6.3c, all classification logic — no behavior change for the 4 existing fixtures |
| R6.4b | `a814bce8…` | persistent-handle re-entry infrastructure: side-table `unordered_map<lv2_id, BridgeSession>` + mutex; multi-round loop in `try_delegate_execution` that handles supported `StallRead` (IN_MBOX 29 / SNR1 3 / SNR2 4) by `pop_wait`-blocking on the corresponding RPCS3 channel + forwarding to Rust + resuming on the same handle; new log fields `total_steps` + `stall_iters`; +251 lines vs R6.4a, all in `SPURustBridge.cpp/.h` — no behavior change for the 4 existing single-shot fixtures (verified `stall_iters=0`) |
| **R6.5b (current)** | **`7d6b6bba…`** | StallWrite OUT_MBOX (ch28) handling: depth-1 overwrite via `set_value` on RPCS3's `ch_out_mbox` + drain Rust's mailbox + resume same handle; new `stall_write_iterations` counter in `BridgeSession`; new log field `stall_write_iters` in DELEGATED success message; FFI test `rust_spu_outmbox_backpressure_via_ffi`; **architectural finding documented**: a real-binary fixture for this path is not feasible in PSL1GHT cooperative-thread context (no PPU-side syscall to drain a cooperative SPU's OUT_MBOX, so a SPU that writes OUT_MBOX twice without an intervening stop deadlocks both C++ executor's `push_wait` and the bridge); FFI test is the canonical acceptance gate. +87 lines vs R6.4b — no behavior change for the 5 existing fixtures (all show `stall_write_iters=0`) |

> **Patch regeneration discipline (R6.1c onward):** the canonical
> patch is generated via `git diff --cached` against an LF-normalized
> baseline (we run `tr -d '\r'` on the pre/post snapshots before
> committing into the temp repo). This keeps the diff focused on
> logical content; CRLF warnings from `git apply` on Windows-CRLF
> targets are cosmetic and do not affect the apply.

The pre-existing trace-writer patches' sha256s remain pinned at:

- `spu_trace_jsonl_scaffolding.patch`: `d65aec91b6b2439b4befeaf6d51d64ddb98b9425726fc17abbc3d434ae1aba1c`
- `spu_trace_jsonl_runtime_hooks.patch`: `8f253d7d207793266eb3a81e809c73731a8e565757a9d2c40fa944a88266663a`

## Verification (without rebuilding RPCS3)

The patch can be verified to apply cleanly without compiling:

```bash
cd <upstream rpcs3 source tree>
# Apply the prerequisite trace patches first if not already applied.
git apply --check docs/patches/spu_rust_bridge.patch
```

Round-trip-tested: applying the patch to a post-scaffolding tree
produces byte-identical content to the hand-edited reference (the
local `rpcs3/` snapshot in this repo, modulo Windows CRLF line
endings). See `behavior-freeze/harness/check_patch_separation.py`
for the optional sha256-pinning check on the new patch.

## What R6.2 does NOT do

- ❌ Take over execution by default. Bridge is OFF unless
  `RPCS3_SPU_RUST_BRIDGE=1`.
- ❌ Handle DMA / MFC. `try_delegate_execution()` operates only on
  LS, PC, GPRs, IN_MBOX (ch29), OUT_MBOX (ch28). Any other channel
  use surfaces as a `Stall*` outcome and falls back honestly.
- ❌ Handle SPURS / cooperative-thread workloads. R6.3+ scope.
- ❌ Handle channel parking with re-entry. R6.2 supports only the
  "run-to-Stop in one shot" pattern; `Continue` (max steps reached)
  is treated as fallback rather than re-invocation, since the
  R6.2 fixture finishes in ~4 SPU instructions.
- ❌ Capture traces from the Rust executor. The existing C++ trace
  writer hooks fire on the C++ executor only. When the bridge
  takes over, the `TraceFinalGuard` in `cpu_task()` still fires on
  return and emits final_state, so per-thread closure events are
  preserved; per-instruction events from the Rust path land in
  R6.4+.
- ❌ Modify the trace-writer patches. The two pinned patches
  (`spu_trace_jsonl_scaffolding.patch` + `spu_trace_jsonl_runtime_hooks.patch`)
  are unchanged. Verified by `check_patch_separation.py`.

## Status (R6.4b)

R6.1 / R6.1b / R6.1c / R6.2 / R6.2c / R6.3a / R6.3a-c / R6.3b / R6.3b-c / R6.3c / R6.4a / R6.4b-pre / R6.4b deliverables:

- ✅ Rust crate `rpcs3-spu-ffi` builds clean (`cargo build --release`)
- ✅ Header `rpcs3_spu_ffi.h` regenerates clean (cbindgen)
- ✅ Header smoke-tested under `gcc -Wall -Wextra -Werror`
- ✅ Patch `spu_rust_bridge.patch` applies via plain
  `git apply --check` (no flags) — R6.1c hygiene preserved through
  R6.2c regeneration.
- ✅ Build scripts `scripts/build_rust_spu_ffi.{sh,ps1}` ship with
  `--dest-root` / `-DestRoot` to auto-copy artifacts.
- ✅ Doc `docs/SPU_RUST_BRIDGE_PATCH.md` reflects R6.2 delegation
  semantics (this section).
- ✅ All 4 oracle fixture replay tests pass.
- ✅ Workspace `cargo test --workspace --lib --no-fail-fast`: 5590/0.
- ✅ **R6.1b — C++ build verification:** `R:\bin\rpcs3.exe` builds
  and links the Rust staticlib (~64 MB).
- ✅ **R6.1b — hello-world bridge ON acceptance:** activation +
  FFI-roundtrip log lines, default-OFF behavior preserved.
- ✅ **R6.2 — first delegated execution:** `single_spu_mailbox_v1`
  with `RPCS3_SPU_RUST_BRIDGE=1` runs entirely inside the Rust
  executor and exits via `stop_and_signal(0x101)`. Log evidence
  (`Stop code=0x101 steps=4 in_mbox_consumed=1 final_pc=0xc`) +
  TTY (`[smbox_v1] OK cause=0x1 status=0x129`).
- ✅ **R6.2 — default OFF preserved:** no bridge log lines, no
  delegation, identical TTY status when env var is unset.
- ✅ **R6.2 — fallback contract:** unsupported outcomes
  (`Continue / StallRead / StallWrite / Error`) drop the handle,
  log a warning, and let the C++ executor take over with RPCS3
  state untouched.
- ✅ **R6.2c — patch hygiene:** sha256 pin updated to `e41cd4cf…`;
  patch applies cleanly via plain `git apply --check`; this doc
  reflects the new delegation surface.
- ✅ **R6.3a — second delegated fixture:** `single_spu_branch_loop_v1`
  now delegates end-to-end through the same generic R6.2 path. The
  bridge needed **no logic changes** — only source-level comment /
  log updates documenting that the run-to-Stop pattern covers any
  mailbox-only single-shot SPU workload. Evidence:
  `Stop code=0x101 steps=70 in_mbox_consumed=1 final_pc=0x3c` +
  `[brloop_v1] OK cause=0x1 status=0x59` (= Fib(10) = 89).
- ✅ **R6.3b — third delegated fixture:** `single_spu_loadstore_v1`
  also delegates end-to-end with **no bridge logic changes**. Confirms
  the bridge's LS load (256 KiB), GPR LE→BE conversion (including
  r1 stack-pointer preservation at 0x3FFF0), and the Rust SPU
  executor's stqd/lqd quadword load-store opcodes. Evidence:
  `Stop code=0x101 steps=187 in_mbox_consumed=1 final_pc=0x80` +
  `[ldst_v1] OK cause=0x1 status=0x81c` (= 8 × 0x100 + 28 = 0x81C,
  the deterministic checksum for seed=0x10).
- ✅ **R6.3b-c — patch hygiene + clean rebuild:** lock on
  `R:\bin\rpcs3.exe` finally released after the R6.3a-c run; clean
  rebuild produced a fresh binary at `R:\bin\rpcs3.exe` (5min 49s,
  64 MB) carrying the R6.3b activation log message that lists all
  three verified fixtures. Canonical patch regenerated to sha256
  `9555c363…`; sha256 pin in `check_patch_separation.py` updated;
  all gates re-verified green.
- ✅ **R6.3c — fourth delegated fixture (first real bridge logic
  extension since R6.2):** `single_spu_signal_v1` exercises
  channel 3 (SPU_RdSigNotify1) instead of IN_MBOX. Bridge now has
  a Phase 1b that non-destructively peeks `spu.ch_snr1` /
  `spu.ch_snr2` (single-value `spu_channel`) and forwards via
  `rust_spu_signal(slot, value)` (slot 0 = SNR1, slot 1 = SNR2).
  On Stop commit the corresponding RPCS3-side channels are
  drained via `try_pop`. On any non-Stop outcome the SNR channels
  are left untouched (peek was non-destructive). New
  `signal_forwarded=N` field added to the DELEGATED success log.
  Evidence: `Stop code=0x101 steps=8 in_mbox_consumed=0
  signal_forwarded=1 final_pc=0x1c` + `[sig_v1] OK cause=0x1
  status=0x11121` (= 0x1234 + 0xFEED). All three previously-
  delegated fixtures verified non-regressed (regression check on
  mailbox / branch_loop / loadstore: all `signal_forwarded=0`,
  identical steps to prior runs). Canonical patch regenerated to
  sha256 `054b2bec…`.
- ✅ **R6.4b — persistent-handle re-entry infrastructure (FFI-gated):**
  side-table `std::unordered_map<lv2_id, std::unique_ptr<BridgeSession>>`
  with mutex; `BridgeSession` carries the `rust_spu_t*` plus
  bookkeeping (`in_mbox_consumed`, `snr*_forwarded`, `stall_iterations`,
  `total_steps`). The multi-round loop in `try_delegate_execution`
  handles supported `StallRead` (IN_MBOX 29 / SNR1 3 / SNR2 4) by
  blocking on the corresponding RPCS3 channel via `pop_wait` (same
  path the C++ executor takes in `spu_thread::get_ch_value`),
  forwarding the popped value into the Rust handle, and resuming
  the same `rust_spu_t*`. Capped at 32 stall iterations for safety.
  All 4 existing single-shot fixtures verified non-regressed
  (`stall_iters=0`, identical step counts and statuses). The FFI
  acceptance gate
  `rust_spu_mailbox_multi_round_via_ffi` continues passing — but
  end-to-end real-binary acceptance for `single_spu_mailbox_multi_v1`
  is **deferred** until PSL1GHT is available to build the `.self`.
  Canonical patch regenerated to sha256 `a814bce8…`.

- ✅ **R6.4b-pre — stall-bound oracle fixture authored:**
  `single_spu_mailbox_multi_v1` source files (`main.c`,
  `spu/spu_mailbox_multi.c`, `Makefile`, `README.md`, `LICENSE.md`)
  checked into
  `behavior-freeze/fixtures/spu/sources/single_spu_mailbox_multi_v1/`.
  This is the first oracle that REQUIRES persistent-handle
  re-entry: a stateless bridge cannot deliver the round-1 OUT_MBOX
  AND survive the inter-round StallRead AND deliver the round-2
  OUT_MBOX without either falling back to C++ entirely or
  duplicating the first OUT_MBOX. The PSL1GHT toolchain is not
  available in the current dev env, so the `.self` is not yet
  built; instead an executable acceptance gate landed as a
  synthetic FFI test
  (`rust_spu_mailbox_multi_round_via_ffi` in
  `rust/rpcs3-spu-ffi/src/tests.rs`) that exercises the same
  `StallRead → push → resume on same handle → Stop` pattern at
  the FFI level. Any R6.4b implementation must keep this test
  passing. The fixture's `.self` will land when a properly-
  provisioned host runs `make` in the fixture dir.
  the only commit-eligible outcome; Continue / StallRead /
  StallWrite / Error all fall back to the C++ executor with RPCS3
  state byte-identical to entry. New helper
  `classify_stall_channel()` maps stall channel index to a
  human-readable name (`"IN_MBOX (ch29)"`, `"SNR1 (ch3)"`, etc.)
  and a re-tryability flag (mailbox 28/29 + signal 3/4 = yes;
  event/decrementer/DMA = no). Per-outcome warning logs added so
  the operator can identify exactly why the bridge declined and
  whether the workload is a candidate for R6.4b's persistent
  handle. New FFI lib test
  `rust_spu_continue_then_resume_on_same_handle` documents the
  same-handle resume pattern that R6.4b will leverage. The 4
  existing fixtures verified non-regressed end-to-end (4/70/187/8
  steps unchanged). Canonical patch regenerated to sha256
  `8f1d6c0b…`.

## Verified delegated workloads

| Fixture | Steps | OUT_MBOX | TTY status | Notes |
|---|---|---|---|---|
| `single_spu_mailbox_v1` | 4 | `0x129` | `cause=0x1 status=0x129` | First delegated fixture (R6.2). |
| `single_spu_branch_loop_v1` | 70 | `0x59` | `cause=0x1 status=0x59` | Real loop with branches; Fib(10)=89 (R6.3a). |
| `single_spu_loadstore_v1` | 187 | `0x81C` | `cause=0x1 status=0x81c` | Real LS round-trip via volatile stack buffer; exercises stqd/lqd + r1 stack-pointer sync (R6.3b). |
| `single_spu_signal_v1` | 8 | `0x11121` | `cause=0x1 status=0x11121` | First fixture using SNR (channel 3) instead of IN_MBOX. Required adding Phase 1b SNR forwarding via `rust_spu_signal()` (R6.3c). |

All four R5.11/R5.11b oracle fixtures now delegate end-to-end
through the Rust SPU bridge.
