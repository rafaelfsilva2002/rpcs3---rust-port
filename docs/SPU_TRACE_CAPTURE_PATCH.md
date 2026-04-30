# SPU Trace Capture — RPCS3 Integration Patch (R5.8 A.3)

**Status:** Partial. Trace-writer infrastructure (`rpcs3/Emu/Cell/SPUTraceJsonl.{h,cpp}`) is shipped and now wired into the build (CMake + vcxproj + filters, additive only); integration call-site insertions documented below; **runtime hooks NOT applied** in any hot-path C++ source file; **trace capture itself is environmentally deferred** — it requires building and running RPCS3 with the runtime hooks applied, which this Rust-focused workflow cannot do. A future iteration with C++ build access applies the diffs in this doc, runs a homebrew, commits the resulting `.jsonl` to `behavior-freeze/fixtures/spu/traces/`, and adds a Rust replay test against the captured trace.

**See also:** [`SPU_TRACE_CAPTURE_RUNTIME_HOOKS.md`](./SPU_TRACE_CAPTURE_RUNTIME_HOOKS.md) — companion document that frames the current state ("scaffolding exists, runtime hooks not applied, real trace not captured"), explains why hooks are deferred, lists the exact validation checklist a maintainer with build access should follow, and documents the field-naming mapping between the schema in `SPU_TRACE_CAPTURE.md` and the user-facing minimum-suggested fields (`step`/`event`/`direction`/`spu_id`/`schema`/`note`).

This document is the precise, file:line-level integration plan for the C++ patch, complementing the wire-format spec in [`SPU_TRACE_CAPTURE.md`](./SPU_TRACE_CAPTURE.md) and the deferred-capture rationale in [`PROJECT_STATUS.md`](./PROJECT_STATUS.md).

---

## What's already shipped

Two new self-contained C++ files, requiring no integration changes to compile in isolation:

| File | Purpose |
|---|---|
| [`rpcs3/Emu/Cell/SPUTraceJsonl.h`](../rpcs3/Emu/Cell/SPUTraceJsonl.h) | Public surface: `TraceWriter` singleton, `EventKind`, `ParkReason`, `ChannelsSnapshot`, `GprEntry`. |
| [`rpcs3/Emu/Cell/SPUTraceJsonl.cpp`](../rpcs3/Emu/Cell/SPUTraceJsonl.cpp) | Implementation: env-var-gated singleton, monotonic seq, hand-rolled JSON serializer (no new deps), mutex-protected file write. |

These files have **zero dependency** on RPCS3-internal types in their public API — every input is a plain C++17 scalar. They could compile against any host with C++17 + the standard library.

**Activation:** set `RPCS3_SPU_TRACE_JSONL=/abs/path/to/out.jsonl` before launching the patched RPCS3. Unset (or empty) → every emit is a fast atomic-load short-circuit, zero file I/O.

**Schema mirror:** the writer's output is byte-equivalent to a manually authored JSONL trace conforming to [`SPU_TRACE_CAPTURE.md`](./SPU_TRACE_CAPTURE.md). The Rust parser+transformer in [`rust/rpcs3-spu-differential/src/trace_fmt.rs`](../rust/rpcs3-spu-differential/src/trace_fmt.rs) consumes it directly.

---

## What requires the C++ patch (deferred)

The trace writer is dormant until call-site insertions in existing RPCS3 source files invoke its `record_*` methods. The call sites are conceptually identified in [`SPU_TRACE_CAPTURE.md`](./SPU_TRACE_CAPTURE.md) § "Conceptual instrumentation hooks"; this section gives them at file:line precision against the current upstream `master` snapshot in this workspace.

**Why this is documented as a diff rather than committed:** the Rust workflow this PR ladder is grounded in cannot build/run RPCS3 to verify the C++ patch compiles cleanly, links without unresolved-symbol errors, or produces correct trace output on a real homebrew run. Modifying upstream-tracked C++ source files without that verification carries a real risk of subtle regressions (off-by-one in PC capture, lock contention under SMP, channel-state read-after-write hazards) that the maintainer running RPCS3 must catch. A documented diff is a safer handoff than blind edits.

### Build integration

Add the two new files to the existing CMake list. Search for the line that lists `SPUThread.cpp` in `rpcs3/Emu/CMakeLists.txt` (or equivalent VS project file) and add adjacent entries:

```cmake
# After: rpcs3/Emu/Cell/SPUThread.cpp
rpcs3/Emu/Cell/SPUTraceJsonl.cpp
rpcs3/Emu/Cell/SPUTraceJsonl.h
```

For `rpcs3.vcxproj` (Visual Studio direct), add `<ClInclude Include="Emu\Cell\SPUTraceJsonl.h" />` and `<ClCompile Include="Emu\Cell\SPUTraceJsonl.cpp" />` next to the existing `SPUThread` entries.

### Hook 1: `spu_thread::get_ch_value` — `spu_rdch` + `spu_park` + `spu_wake`

**File:** `rpcs3/Emu/Cell/SPUThread.cpp` line 5335 (`s64 spu_thread::get_ch_value(u32 ch)`).

The function dispatches per-channel. The capture must:
1. Emit `spu_rdch` with `would_stall=false, value=<consumed>` on successful immediate read.
2. Emit `spu_rdch` with `would_stall=true, value=null` on the stall path (channel empty), then immediately emit `spu_park`.
3. After the stall completes (channel becomes ready and the SPU resumes), emit `spu_wake` followed by another `spu_rdch` with `would_stall=false, value=<consumed>`.

The cleanest insertion approach: wrap the existing `read_channel` lambda to capture stall-decision state. Suggested skeleton (insert near line 5340, before the existing `auto read_channel = [&](spu_channel& channel)` definition):

```cpp
#include "SPUTraceJsonl.h"

// ... existing code ...

s64 spu_thread::get_ch_value(u32 ch)
{
    if (ch < 128) spu_log.trace("get_ch_value(ch=%s)", spu_ch_name[ch]);

    // R5.8 A.3 trace: capture pre-state for park decision.
    auto& tracer = rpcs3::spu_trace::TraceWriter::instance();
    const u32 trace_pc = pc;
    bool trace_did_stall = false;

    auto read_channel = [&](spu_channel& channel) -> s64
    {
        const bool empty_at_entry = channel.get_count() == 0;
        if (empty_at_entry)
        {
            // SPU is about to park.
            if (tracer.enabled())
            {
                tracer.record_spu_rdch(trace_pc, ch, std::nullopt, /*would_stall=*/true);
                rpcs3::spu_trace::ParkReason r = rpcs3::spu_trace::ParkReason::ChannelRead;
                tracer.record_spu_park(trace_pc, r, ch, std::nullopt);
                trace_did_stall = true;
            }
            state += cpu_flag::wait + cpu_flag::temp;
        }

        if (state & cpu_flag::pending) { do_mfc(); }
        last_getllar_addr = umax;

        const s64 out = channel.pop_wait(*this);

        if (state & cpu_flag::wait) { wakeup_delay(); }

        // After pop_wait returns, if we stalled we are now resumed.
        if (tracer.enabled())
        {
            if (trace_did_stall)
            {
                tracer.record_spu_wake(trace_pc);
            }
            tracer.record_spu_rdch(trace_pc, ch, static_cast<u32>(out), /*would_stall=*/false);
        }
        return out;
    };

    // ... rest of original switch unchanged ...
}
```

**Edge case:** `pop_wait` may return failure / `error_code`-style values via the high bits of the `s64` return. The trace should record the `value` only when the read succeeded. Implementer to add a guard before the `record_spu_rdch` post-resume call if `out` carries an error sentinel — the schema spec already permits `value: null` for the stalling case but does NOT permit it for `would_stall: false`.

### Hook 2: `spu_thread::set_ch_value` — `spu_wrch` + `spu_park` + `spu_wake`

**File:** `rpcs3/Emu/Cell/SPUThread.cpp` line 5957 (`bool spu_thread::set_ch_value(u32 ch, u32 value)`).

The function has many per-channel arms. The two channels relevant to the schema are `SPU_WrOutMbox` (28) and `SPU_WrOutIntrMbox` (30); both can stall when the mailbox is full. Insert tracer calls in those arms:

```cpp
case SPU_WrOutMbox:
{
    auto& tracer = rpcs3::spu_trace::TraceWriter::instance();
    const u32 trace_pc = pc;
    bool would_stall = ch_out_mbox.get_count() != 0;

    if (tracer.enabled())
    {
        tracer.record_spu_wrch(trace_pc, ch, value, would_stall);
        if (would_stall)
        {
            tracer.record_spu_park(trace_pc,
                rpcs3::spu_trace::ParkReason::ChannelWrite, ch, std::nullopt);
        }
    }

    // ... existing push_wait logic unchanged ...

    if (would_stall && tracer.enabled())
    {
        tracer.record_spu_wake(trace_pc);
        tracer.record_spu_wrch(trace_pc, ch, value, /*would_stall=*/false);
    }

    return true;
}
```

Same pattern applies to `SPU_WrOutIntrMbox` (channel 30). Channels that never stall (`SPU_WrEventMask`, `SPU_WrDec`, etc.) can emit a single `spu_wrch` with `would_stall=false` if desired — the Rust transformer ignores those events as state-machine context only.

### Hook 3: `spu_thread::get_ch_count` — `spu_rchcnt`

**File:** `rpcs3/Emu/Cell/SPUThread.cpp` line 5288 (`u32 spu_thread::get_ch_count(u32 ch)`).

The function returns immediately with a count. No stall path. Insert at the end, just before each `return`:

```cpp
u32 spu_thread::get_ch_count(u32 ch)
{
    if (ch < 128) spu_log.trace("get_ch_count(ch=%s)", spu_ch_name[ch]);

    u32 result;
    switch (ch)
    {
    case SPU_WrOutMbox:    result = ch_out_mbox.get_count() ^ 1;  break;
    case SPU_WrOutIntrMbox: result = ch_out_intr_mbox.get_count() ^ 1; break;
    // ... other arms compute result similarly ...
    default:
        ensure(ch < 128u);
        spu_log.error("Unknown/illegal channel in RCHCNT (ch=%s)", spu_ch_name[ch]);
        return 0;
    }

    auto& tracer = rpcs3::spu_trace::TraceWriter::instance();
    if (tracer.enabled())
    {
        tracer.record_spu_rchcnt(pc, ch, result);
    }
    return result;
}
```

This is a structural rewrite (single return point with `result` accumulator) — implementer can decide whether to apply this pattern or insert a `record_spu_rchcnt` call at every existing `return` site (more lines but smaller diff).

### Hook 4: `spu_thread::stop_and_signal` — `spu_stop`

**File:** `rpcs3/Emu/Cell/SPUThread.cpp` line 6431 (`bool spu_thread::stop_and_signal(u32 code)`).

`stop_and_signal` is invoked for both `stop` and `stopd`. Insert at the top:

```cpp
bool spu_thread::stop_and_signal(u32 code)
{
    auto& tracer = rpcs3::spu_trace::TraceWriter::instance();
    if (tracer.enabled())
    {
        tracer.record_spu_stop(pc, code);
    }

    // ... rest of function unchanged ...
}
```

### Hook 5: `spu_thread::cpu_task` — terminal `final_state`

**File:** `rpcs3/Emu/Cell/SPUThread.cpp` line 1442 (`void spu_thread::cpu_task()`).

The terminal hook needs to fire exactly once per SPU thread, after `stop_and_signal` and before the function returns. Two options:

1. **Wrap the function body in a try/finally idiom** using a destructor on a local guard object — fires regardless of how `cpu_task` exits.

2. **Explicit emit at every return site** — works but more error-prone.

Suggested guard pattern (insert near the top of `cpu_task`):

```cpp
void spu_thread::cpu_task()
{
#ifdef __APPLE__
    pthread_jit_write_protect_np(true);
#endif
    start_time = 0;

    // R5.8 A.3 trace: emit final_state on scope exit (any path).
    struct TraceFinalGuard
    {
        spu_thread* spu;
        ~TraceFinalGuard()
        {
            auto& tracer = rpcs3::spu_trace::TraceWriter::instance();
            if (!tracer.enabled()) return;

            // Capture policy: emit only the registers the workload's
            // contract uses. For the first-pass capture, emit ALL
            // non-zero lane-0 GPRs; future captures can narrow.
            std::vector<rpcs3::spu_trace::GprEntry> gprs;
            for (u32 reg = 0; reg < 128; ++reg)
            {
                const u32 lane0 = spu->gpr[reg]._u32[3];  // SPU lane 0 = preferred slot = u32[3] in RPCS3
                if (lane0 != 0)
                {
                    gprs.push_back({reg, lane0});
                }
            }

            rpcs3::spu_trace::ChannelsSnapshot ch{};
            ch.in_mbox       = spu->ch_in_mbox.get_count()       ? std::optional<u32>(spu->ch_in_mbox.get_value())       : std::nullopt;
            ch.out_mbox      = spu->ch_out_mbox.get_count()      ? std::optional<u32>(spu->ch_out_mbox.get_value())      : std::nullopt;
            ch.out_intr_mbox = spu->ch_out_intr_mbox.get_count() ? std::optional<u32>(spu->ch_out_intr_mbox.get_value()) : std::nullopt;
            ch.snr1 = spu->ch_snr1.get_count() ? spu->ch_snr1.get_value() : 0;
            ch.snr2 = spu->ch_snr2.get_count() ? spu->ch_snr2.get_value() : 0;

            tracer.record_final_state(gprs, ch);
        }
    } _trace_final_guard{this};

    // ... rest of cpu_task unchanged ...
}
```

**Edge case 1:** `spu_channel::get_value()` — verify this method exists and is non-destructive (does NOT pop the channel). If RPCS3's `spu_channel` only exposes destructive `pop()` for value access, the implementer must add a `peek()` method or capture the value before pop in the channel's own access path. Falling back to `get_count()`-only capture is acceptable but loses payload values.

**Edge case 2:** the GPR layout. RPCS3's SPU register is `v128`, and lane-0 (preferred slot) maps to `_u32[3]` on little-endian builds (the "high" slot when interpreted as a big-endian SPU register). Verify against `SPUInterpreter.cpp`'s existing GPR access patterns before committing.

**Edge case 3:** SPU thread destruction may bypass `cpu_task` cleanup on certain shutdown paths (force-exit, error termination). The guard above fires only when `cpu_task` returns normally. For complete coverage, also place a `TraceFinalGuard`-equivalent in `~spu_thread()` or ensure all exit paths route through `cpu_task`.

### Hook 6: PPU-side mailbox / signal helpers

**Files:**
- `rpcs3/Emu/Cell/lv2/sys_spu.cpp:1913` — `sys_spu_thread_write_spu_mb` (PPU pushes to SPU's in_mbox).
- `rpcs3/Emu/Cell/lv2/sys_spu.cpp:1989` — `sys_spu_thread_write_snr` (PPU writes SNR1/SNR2).
- `rpcs3/Emu/Cell/RawSPUThread.cpp:147` — Raw SPU `SPU_Out_MBox_offs` read (PPU pops out_mbox).
- `rpcs3/Emu/Cell/RawSPUThread.cpp:289` — Raw SPU `SPU_In_MBox_offs` write (PPU pushes in_mbox).
- `rpcs3/Emu/Cell/lv2/sys_spu.cpp:1930` — `sys_spu_thread_write_spu_mb` body (alternative push path).

Patterns:

**`sys_spu_thread_write_spu_mb`** (insert at line ~1930, after the `state = thread->ch_in_mbox.push(...)` call):

```cpp
auto& tracer = rpcs3::spu_trace::TraceWriter::instance();
if (tracer.enabled())
{
    tracer.record_ppu_push_inmbox(/*target_spu=*/id, value);
}
```

**`sys_spu_thread_write_snr`** (insert at the start of the function):

```cpp
auto& tracer = rpcs3::spu_trace::TraceWriter::instance();
if (tracer.enabled())
{
    // schema slot: 0 = SNR1 (channel 3), 1 = SNR2 (channel 4)
    tracer.record_ppu_signal(/*target_spu=*/id, /*slot=*/number, value);
}
```

**`RawSPUThread.cpp:147` (PPU pops out_mbox)** — the `value = ch_out_mbox.pop();` line. Wrap to capture:

```cpp
case SPU_Out_MBox_offs:
{
    value = ch_out_mbox.pop();
    auto& tracer = rpcs3::spu_trace::TraceWriter::instance();
    if (tracer.enabled())
    {
        // pop() returns 0 on empty mailbox per RPCS3 convention; capture
        // distinguishes empty via `count`-was-zero check before pop.
        // For accurate capture, peek count before pop; defer that
        // refinement to the implementer.
        tracer.record_ppu_pop_outmbox(/*target_spu=*/id, std::optional<u32>(value));
    }
    return true;
}
```

**`RawSPUThread.cpp:289` (PPU pushes in_mbox via raw SPU MMIO)** — the `case SPU_In_MBox_offs:` block. Similar shape to `sys_spu_thread_write_spu_mb`.

### Optional: thread-id discriminator

The schema reserves `target_spu` as `u32` for forward compatibility but the parser/transformer assumes single-SPU. For the first capture pass, set `target_spu = thread_id` (or any stable identifier) and capture only one SPU at a time. Multi-SPU capture (with per-SPU JSONL files or interleaved single file with `target_spu` discrimination) is R5.9+ scope.

---

## Capture procedure (post-build)

Once the patch is applied and RPCS3 is built:

```bash
# Linux / macOS
export RPCS3_SPU_TRACE_JSONL=/tmp/spu_trace.jsonl
./rpcs3 --headless /path/to/homebrew.elf

# Windows (PowerShell)
$env:RPCS3_SPU_TRACE_JSONL = "C:\tmp\spu_trace.jsonl"
.\rpcs3.exe --headless C:\path\to\homebrew.elf
```

**Verification:**

```bash
head -5 /tmp/spu_trace.jsonl    # Eyeball the first few events.
wc -l /tmp/spu_trace.jsonl      # Event count.
```

Apply the Rust parser as a syntax check:

```bash
cd rust/rpcs3-spu-differential
cargo run --example parse_jsonl_trace -- /tmp/spu_trace.jsonl
# (example program TBD — parses and prints summary; trivial wrapper around
#  parse_jsonl_trace + captured_events_to_trace + summary())
```

---

## Validation strategy after capture

1. **Syntax:** `parse_jsonl_trace()` succeeds with `Ok(events)`. Failure here = malformed JSON or schema violation in the C++ patch.
2. **Transformation:** `captured_events_to_trace()` succeeds with `Ok(trace)`. Failure here = state-machine boundary violated (e.g., `final_state` before `spu_stop`).
3. **Replay:** `replay_trace()` against both `InterpreterExecutor` and `RecompilerExecutor` produces matching `Finished` reports. Failure here = real correctness gap between Rust SPU stack and C++ — that's the whole point of the harness.
4. **Mutation:** poke an `expect: Some(v)` value in the transformed trace, re-run, assert `OutMboxValueMismatch` keyed at the right event index. Confirms the captured trace wasn't accidentally satisfying weakened assertions.

---

## Out of scope

- Building / running RPCS3 to produce the actual capture (deferred — environmental gating).
- Multi-SPU traces — schema and writer are single-SPU-only.
- Timing / performance fields — R5.5 replay is determinism-driven.
- DMA / memory traffic — not in this version.
- Schema evolution / versioning — defer until needed.
- C++ unit tests for `TraceWriter` — desirable but requires RPCS3 to have a working test harness; not present in this workspace.

---

## Cross-references

- Wire format: [`SPU_TRACE_CAPTURE.md`](./SPU_TRACE_CAPTURE.md).
- Trace writer header: [`rpcs3/Emu/Cell/SPUTraceJsonl.h`](../rpcs3/Emu/Cell/SPUTraceJsonl.h).
- Trace writer impl: [`rpcs3/Emu/Cell/SPUTraceJsonl.cpp`](../rpcs3/Emu/Cell/SPUTraceJsonl.cpp).
- Rust parser+transformer: [`rust/rpcs3-spu-differential/src/trace_fmt.rs`](../rust/rpcs3-spu-differential/src/trace_fmt.rs).
- R5.5 replay engine: [`rust/rpcs3-spu-differential/src/lib.rs`](../rust/rpcs3-spu-differential/src/lib.rs) — search for `pub fn replay_trace`.
- Status / next-phase: [`PROJECT_STATUS.md`](./PROJECT_STATUS.md).
