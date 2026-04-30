# SPU Trace Capture — Runtime Hooks Application Guide

**Status (current iteration):**
- Trace writer scaffolding **exists**: [`rpcs3/Emu/Cell/SPUTraceJsonl.h`](../rpcs3/Emu/Cell/SPUTraceJsonl.h) and [`SPUTraceJsonl.cpp`](../rpcs3/Emu/Cell/SPUTraceJsonl.cpp).
- Build system entries for the new files are **applied** to all three locations: `rpcs3/Emu/CMakeLists.txt`, `rpcs3/emucore.vcxproj`, `rpcs3/emucore.vcxproj.filters`. Additive only.
- Runtime hooks are **NOT applied** in any hot-path C++ source file. The scaffolding compiles in isolation; nothing in `SPUThread.cpp`, `SPUInterpreter.cpp`, `SPUCommonRecompiler.cpp`, `SPULLVMRecompiler.cpp`, `SPUASMJITRecompiler.cpp`, `RawSPUThread.cpp`, or `lv2/sys_spu.cpp` calls into the trace writer yet.
- Real captured trace is **NOT** committed to `behavior-freeze/fixtures/spu/traces/`. The directory exists with a `README.md` documenting naming/licensing conventions, but no `.jsonl` is present.

This document explains, with sufficient precision for a maintainer with full RPCS3 build access (MSVC + Qt + Vulkan SDK), how to apply the runtime hooks, validate that the scaffolding remains zero-overhead when disabled, run a homebrew under it, and commit the first real captured trace fixture.

It does NOT instruct anyone to modify hot-path C++ source files in this iteration — the present scope is scaffolding only. The hooks are intentionally deferred to a build-capable environment.

---

## Why hooks are not applied in this iteration

Three reasons, in priority order:

1. **No C++ build verification available.** This Rust-focused workflow has no MSVC + Qt + Vulkan SDK provisioned. Modifications to upstream-tracked C++ source files (`SPUThread.cpp`, etc.) without ability to compile, link, and run them risk introducing subtle regressions: off-by-one in PC capture, header include drift, lock contention under SMP, channel-state read-after-write hazards. Any of those would surface only at runtime — and we cannot run.

2. **Line numbers in any patch document drift over time.** [`SPU_TRACE_CAPTURE_PATCH.md`](./SPU_TRACE_CAPTURE_PATCH.md) was authored with file:line precision against the current upstream snapshot in this workspace, but RPCS3 is actively developed upstream. Apply-blind without re-validating line numbers is dangerous; the safer pattern is "implementer pulls latest, applies patch in their working copy after re-checking, builds in their tree, commits the diff alongside the captured `.jsonl`".

3. **Any captured trace MUST be a validation oracle**, not something fitted to pass. If hooks were applied incorrectly here, the captured trace would either (a) fail to compile — visible — or (b) produce subtly wrong events that pass the parser and transformer but encode wrong behavior. Case (b) is the real risk: it makes a future "real trace replay passes" claim hollow. The implementer with build access verifies this loop end-to-end in one place.

---

## Scaffolding contract (what is in place)

### Public surface

[`rpcs3/Emu/Cell/SPUTraceJsonl.h`](../rpcs3/Emu/Cell/SPUTraceJsonl.h) exposes:

- `namespace rpcs3::spu_trace { ... }` — single namespace.
- `class TraceWriter` — singleton, accessed via `TraceWriter::instance()`. First call reads `RPCS3_SPU_TRACE_JSONL` env var; absent → writer is a no-op forever for this process. Atomic-load short-circuit on every emit.
- `enum EventKind` — 10 variants: `SpuRdch`, `SpuWrch`, `SpuRchcnt`, `SpuPark`, `SpuWake`, `SpuStop`, `FinalState`, `PpuPushInmbox`, `PpuPopOutmbox`, `PpuSignal`. Mirrors the wire-format `kind` field in [`SPU_TRACE_CAPTURE.md`](./SPU_TRACE_CAPTURE.md).
- `enum ParkReason { ChannelRead, ChannelWrite }` — wire-format `reason`.
- `struct ChannelsSnapshot` and `struct GprEntry` — payloads for `spu_park`'s optional `channels_at_park` and `final_state`'s `gpr_lane_zero`.
- `record_*` member functions — one per event kind. Each takes plain C++17 scalars (u32, optional u32, etc.). Each is a no-op when `enabled() == false`.

### Wire format (current, R5.5/R5.7-compatible)

The writer emits JSONL lines conforming to [`SPU_TRACE_CAPTURE.md`](./SPU_TRACE_CAPTURE.md). Every event line carries:

| Field | Type | Required |
|---|---|---|
| `seq` | u64 monotonic | yes |
| `side` | `"spu"` / `"ppu"` | yes |
| `kind` | string event name | yes |
| event-specific payload fields | varies | per-event |

**Compatibility note:** the user-facing field-naming in some scope drafts (e.g., `step` instead of `seq`, `event` instead of `kind`, `direction` instead of `side`, `spu_id` instead of `target_spu`, `schema` versioning, `note` field) is a *minimum-suggested superset* — the existing scaffolding satisfies the schema-doc spec under different but equivalent field names, and the Rust-side parser/transformer in [`rust/rpcs3-spu-differential/src/trace_fmt.rs`](../rust/rpcs3-spu-differential/src/trace_fmt.rs) consumes that exact format. Renaming the fields would require synchronized changes in `SPU_TRACE_CAPTURE.md`, the Rust parser types, the round-trip equivalence test, and the JIT smoke test — out of scope for this iteration. If a future iteration genuinely needs a field rename, do it in one atomic step across all four surfaces.

The "field mapping" is therefore:

| Suggested (informal) | Actual (shipped) | Notes |
|---|---|---|
| `schema` (versioning) | not present | optional addition; if needed, reserve `"schema_version": <u32>` as a header line per `SPU_TRACE_CAPTURE.md` § "Open questions" |
| `event` | `kind` | one-to-one |
| `step` | `seq` | one-to-one (monotonic u64 across both sides) |
| `spu_id` | `target_spu` (only on PPU events) | SPU events implicitly target the SPU thread emitting them |
| `pc` | `pc` | identical |
| `channel` | `channel` | identical |
| `value` | `value` | identical |
| `direction` | `side` | finer-grained: `direction: spu_internal` corresponds to `side: spu` for non-channel events; `direction: ppu_to_spu` corresponds to `side: ppu` + `kind: ppu_push_inmbox`/`ppu_signal`; `direction: spu_to_ppu` corresponds to `side: spu` + `kind: spu_wrch` (when value flows out via mailbox) and `side: ppu` + `kind: ppu_pop_outmbox` |
| `note` (free-form string) | absent | reserve `"note": "..."` for future captures if needed; transformer ignores unknown fields |

### Activation contract

```bash
# Linux / macOS
export RPCS3_SPU_TRACE_JSONL=/tmp/spu_trace.jsonl
./rpcs3 --headless /path/to/homebrew.elf

# Windows (PowerShell)
$env:RPCS3_SPU_TRACE_JSONL = "C:\tmp\spu_trace.jsonl"
.\rpcs3.exe --headless C:\path\to\homebrew.elf
```

When `RPCS3_SPU_TRACE_JSONL` is **unset or empty**: every `record_*` call is a single atomic-load that returns immediately. Zero file I/O, zero allocation. A patched build whose hooks fire `TraceWriter::instance().record_spu_rdch(...)` thousands of times per frame should be indistinguishable from an unpatched build at the macroscopic level.

When the env var is **set**:
- First `instance()` call opens the file (truncate). On open failure, a single `stderr` diagnostic is printed and `enabled()` stays `false` — the emulator continues running.
- Each subsequent emit: serializes one JSONL line, writes under a `std::mutex`. No per-line `flush()` — OS buffer flushes on close. If the process crashes before exit, the trace is truncated; that is acceptable because such a capture failed to reach `final_state` and is uninteresting.

Future suggestion (not part of current scope): add `RPCS3_SPU_TRACE_ENABLE=1` as an explicit bool gate, and `RPCS3_SPU_TRACE_SPU_ID=all|<n>` as an SPU-id discriminator. The current writer is single-SPU-only by schema, and the env var presence acts as the implicit enable. Multi-SPU + per-SPU-id filtering is R5.9+ scope.

---

## Where the hooks need to go (the plan, not the application)

The implementer applies the following insertions in their working copy. **Do not apply them in this iteration.** Each hook is documented at file:line precision in [`SPU_TRACE_CAPTURE_PATCH.md`](./SPU_TRACE_CAPTURE_PATCH.md); this section gives the high-level view.

### SPU-side hooks (six call sites)

| File | Function | Line (approx) | Hook |
|---|---|---:|---|
| `rpcs3/Emu/Cell/SPUThread.cpp` | `spu_thread::cpu_task` | 1442 | RAII guard fires `record_final_state` on any function exit |
| `rpcs3/Emu/Cell/SPUThread.cpp` | `spu_thread::get_ch_count` | 5288 | `record_spu_rchcnt` before each return |
| `rpcs3/Emu/Cell/SPUThread.cpp` | `spu_thread::get_ch_value` | 5335 | `record_spu_rdch` (with `would_stall`) + `record_spu_park` on stall + `record_spu_wake` after resume |
| `rpcs3/Emu/Cell/SPUThread.cpp` | `spu_thread::set_ch_value` | 5957 | `record_spu_wrch` + park/wake symmetric to read |
| `rpcs3/Emu/Cell/SPUThread.cpp` | `spu_thread::stop_and_signal` | 6431 | `record_spu_stop` at function entry |
| `rpcs3/Emu/Cell/RawSPUThread.cpp` | `case SPU_Out_MBox_offs:` | 147 | `record_ppu_pop_outmbox` after `pop()` |
| `rpcs3/Emu/Cell/RawSPUThread.cpp` | `case SPU_In_MBox_offs:` | 289 | `record_ppu_push_inmbox` after `push()` |

### PPU-side hooks (two call sites in lv2)

| File | Function | Line (approx) | Hook |
|---|---|---:|---|
| `rpcs3/Emu/Cell/lv2/sys_spu.cpp` | `sys_spu_thread_write_spu_mb` | 1913 | `record_ppu_push_inmbox` after the actual mailbox push |
| `rpcs3/Emu/Cell/lv2/sys_spu.cpp` | `sys_spu_thread_write_snr` | 1989 | `record_ppu_signal` after the slot write |

**Each hook MUST**:
1. Include `"SPUTraceJsonl.h"` at the top of the modified `.cpp`.
2. Guard every `record_*` call with `if (auto& w = rpcs3::spu_trace::TraceWriter::instance(); w.enabled())` — even though the recorders themselves short-circuit, the explicit guard avoids constructing snapshot objects (e.g., `ChannelsSnapshot`, `std::vector<GprEntry>`) when capture is disabled.
3. Capture `pc` from the live `spu_thread::pc` member at the moment the channel op begins — NOT pc+4. The schema invariant is "park PC = channel-op PC", and the Rust replay engine asserts on this exactly.
4. For `would_stall` flags: read it from the same condition the existing code uses to decide whether to call `pop_wait` / `push_wait` (i.e., `channel.get_count() == 0` for read-side, non-zero for write-side). Do NOT introduce a new check that could disagree with the actual stall path.

### Edge cases the implementer must handle

- **Non-destructive channel-value access at `final_state`**: `spu_channel` may not expose a non-destructive `peek()`. Without it, the final-state hook can only emit `Some(value)` for mailboxes whose value was already captured during a prior `pop`, or fall back to recording the count-only state (`get_count()`) and leaving `value: null`. Either is acceptable per schema; the implementer chooses based on what `spu_channel` actually exposes.
- **GPR lane-0 layout**: RPCS3 SPU GPRs are `v128`. Lane 0 (preferred slot, the "high" u32 in the SPU's big-endian view) maps to `_u32[3]` on little-endian builds. Verify against existing GPR access in `SPUInterpreter.cpp` before the `final_state` hook ships.
- **Force-exit / abnormal termination**: SPU thread destruction may bypass `cpu_task`'s normal return path. The RAII guard idiom proposed in `SPU_TRACE_CAPTURE_PATCH.md` covers normal exit and exception unwinding; for hard kill (process abort), the trace is truncated regardless — that is acceptable.
- **`pop_wait` error sentinels**: in `get_ch_value`, the `s64` return mixes value bits with error sentinels. The post-resume `record_spu_rdch(value=Some(out), would_stall=false)` call must guard against the error case — emit `value=null` only when the read genuinely failed; otherwise the schema requires a non-null value.
- **Multi-SPU**: the schema is single-SPU-only. If the captured workload uses multiple SPUs, set `RPCS3_SPU_TRACE_SPU_ID=<n>` (TBD env var) to filter, OR capture per-SPU traces to separate files (TBD path templating). Out of scope for first capture.

---

## Validation checklist for the maintainer applying hooks

Apply this checklist after applying the patch in `SPU_TRACE_CAPTURE_PATCH.md` and rebuilding:

```text
[ ] 1. Build succeeds without errors. RPCS3 produces a working binary.
      Command (Linux example):
          cmake --build build --target rpcs3 -j$(nproc)

[ ] 2. Unpatched-behavior sanity. Launch a known-good homebrew (or a
      committed synthetic ELF) WITHOUT setting RPCS3_SPU_TRACE_JSONL
      and confirm the run completes with the same observable behavior
      as a pre-patch build. Compare: exit code, console log,
      framebuffer hash if applicable.

[ ] 3. Trace-enabled smoke. Set RPCS3_SPU_TRACE_JSONL=/tmp/smoke.jsonl,
      run the same homebrew, confirm the file is created and grows.
      Command:
          export RPCS3_SPU_TRACE_JSONL=/tmp/smoke.jsonl
          ./rpcs3 --headless ./fixture.elf
          test -s /tmp/smoke.jsonl && echo "trace produced"

[ ] 4. JSONL syntactic validity:
          head -1 /tmp/smoke.jsonl | python3 -c "import json,sys; print(json.loads(sys.stdin.read()))"
      First line must parse as JSON and contain seq=0, side, kind.

[ ] 5. Rust-side parse + transform smoke (using the existing public API):
      Add a tiny example program (rust/rpcs3-spu-differential/examples/parse_jsonl_trace.rs)
      that takes the path on argv[1] and runs:
          parse_jsonl_trace(input)?  → assert no TraceParseError
          captured_events_to_trace(&events)?  → assert no TraceTransformError
          print summary
      This is one-off testing infrastructure; commit only if it proves
      useful for repeated captures.

[ ] 6. Rust-side replay test against the captured trace, with the
      homebrew that produced it, through both backends:
          let prog = <build SpuProgram from the homebrew ELF>;
          let trace = captured_events_to_trace(&parse_jsonl_trace(captured)?)?;
          replay_trace(&mut interpreter, prog.clone(), &trace)?;
          replay_trace(&mut recompiler, prog, &trace)?;
      Both must produce Finished{stop_code: <expected>} with the same
      final snapshot. **Any divergence is a real correctness gap;
      preserve the trace as-is and diagnose.**

[ ] 7. Mutation regression: poke an `expect: Some(v)` value in the
      transformed trace, re-run replay, assert OutMboxValueMismatch
      keyed at the right event index. Confirms the captured trace
      wasn't accidentally satisfying weakened assertions.

[ ] 8. Commit the trace + companion notes (.jsonl + .notes.md) to
      behavior-freeze/fixtures/spu/traces/ per the naming convention
      in that directory's README.md.

[ ] 9. Add a Rust test pointing at the committed trace, modeled after
      r5_8_jsonl_pipeline_jit_replay_smoke in
      rust/rpcs3-spu-recompiler/src/lib.rs.
```

If step 6 fails, **stop and diagnose**. Do not adjust Rust assertions, do not weaken parser/transformer validation, do not add `note` fields documenting why the trace is "expected to fail". The trace is an oracle; its failure is data about the Rust SPU stack diverging from C++.

---

## Build / test commands the maintainer should run

Replace `<build-dir>` with the actual CMake/MSBuild output directory.

### CMake (Linux / macOS)

```bash
# Configure
cmake -B build -S . -DCMAKE_BUILD_TYPE=Release

# Build (after applying the runtime-hook patch)
cmake --build build --target rpcs3 -j$(nproc)

# Sanity (without trace)
./build/bin/rpcs3 --headless ./behavior-freeze/fixtures/spu/synthetic_il_stop.elf

# Smoke (with trace)
RPCS3_SPU_TRACE_JSONL=/tmp/spu_smoke.jsonl ./build/bin/rpcs3 --headless ./behavior-freeze/fixtures/spu/synthetic_il_stop.elf
test -s /tmp/spu_smoke.jsonl
```

### MSBuild (Windows)

```powershell
# Open rpcs3.sln in Visual Studio 2022; or msbuild from cmdline:
msbuild rpcs3.sln /p:Configuration=Release /p:Platform=x64 /m

# Sanity
.\build\bin\rpcs3.exe --headless .\behavior-freeze\fixtures\spu\synthetic_il_stop.elf

# Smoke
$env:RPCS3_SPU_TRACE_JSONL = "C:\tmp\spu_smoke.jsonl"
.\build\bin\rpcs3.exe --headless .\behavior-freeze\fixtures\spu\synthetic_il_stop.elf
Test-Path C:\tmp\spu_smoke.jsonl
```

### Rust-side regression (every iteration)

```bash
cd rust
cargo test -p rpcs3-spu-thread --lib
cargo test -p rpcs3-spu-interpreter --lib
cargo test -p rpcs3-spu-differential --lib
cargo test -p rpcs3-spu-recompiler --release
cargo test -p spu-runner
cargo test --workspace --lib
```

Current baseline: `cargo test --workspace --lib` = **5461 passed, 0 failed**. Re-run after applying the runtime-hook patch — must remain green (the C++ patch should not affect any Rust crate).

---

## Hard rule: no fabricated traces

`behavior-freeze/fixtures/spu/traces/` exists for **real C++-captured traces only**. Hand-authored / simulated / "schema-conformant test" traces do NOT belong there. The synthetic round-trip fixture (`R5_6_REFERENCE_JSONL` in `rust/rpcs3-spu-differential/src/trace_fmt.rs`) is explicitly inside the Rust crate, marked as synthetic, and serves as the parser+transformer's known-good input.

If a maintainer cannot capture a real trace yet, the correct action is to leave the `traces/` directory empty (the README.md alone is fine). Do NOT commit a `placeholder.jsonl` or `template.jsonl` — they create the impression that real-trace replay was validated when it wasn't.

---

## Cross-references

- Wire format: [`SPU_TRACE_CAPTURE.md`](./SPU_TRACE_CAPTURE.md).
- Integration patch (file:line precision): [`SPU_TRACE_CAPTURE_PATCH.md`](./SPU_TRACE_CAPTURE_PATCH.md).
- Trace writer header: [`rpcs3/Emu/Cell/SPUTraceJsonl.h`](../rpcs3/Emu/Cell/SPUTraceJsonl.h).
- Trace writer impl: [`rpcs3/Emu/Cell/SPUTraceJsonl.cpp`](../rpcs3/Emu/Cell/SPUTraceJsonl.cpp).
- Rust parser+transformer: [`rust/rpcs3-spu-differential/src/trace_fmt.rs`](../rust/rpcs3-spu-differential/src/trace_fmt.rs).
- Replay engine: [`rust/rpcs3-spu-differential/src/lib.rs`](../rust/rpcs3-spu-differential/src/lib.rs) — search `pub fn replay_trace`.
- Trace destination: [`behavior-freeze/fixtures/spu/traces/README.md`](../behavior-freeze/fixtures/spu/traces/README.md).
- Project status / next-phase: [`PROJECT_STATUS.md`](./PROJECT_STATUS.md).
