# Patches — RPCS3 fork integration artifacts

Tracked patches that capture C++ work the Rust port needs applied to a tracked RPCS3 fork. The workspace's `.gitignore` excludes `/rpcs3/`, `/3rdparty/`, `/Utilities/`, `/bin/`, `/buildfiles/`, `/darwin/` per the project's tracking strategy ("Rust port + behavior-freeze docs only; RPCS3 upstream source kept locally as reference snapshot"), so any C++ artifact is committed here as a reusable, replayable patch rather than as an in-tree edit.

Patches in this directory MUST satisfy:

1. **Replayable.** Apply cleanly via `git apply` or `git am` against an upstream RPCS3 working tree.
2. **Self-describing.** Each `.patch` file has a paired explanation (`<patch>.notes.md` or covered by this README) documenting scope, prerequisites, validation steps.
3. **Scoped narrowly.** No patch should mix scaffolding with runtime hooks; no patch should bundle unrelated changes. One patch = one well-defined boundary.
4. **No fabricated content.** Patches encode actual local changes; they do not invent code that wasn't tested elsewhere or claim to come from a real capture when they don't.

## Index

| Patch | Scope | Status | Companion |
|---|---|---|---|
| [`spu_trace_jsonl_scaffolding.patch`](./spu_trace_jsonl_scaffolding.patch) | C++ trace-writer scaffolding — **R5.9e.3-fix** (target_spu emission + record_spu_image with bool-return write_image_side_file + post-write verification) | build-validated against `7028e85fa`. **Current sha256 `d65aec91b6b2439b4befeaf6d51d64ddb98b9425726fc17abbc3d434ae1aba1c`** (32,957 bytes). Chronological history: `8525caea…` v1 → `a8baa1a7…b8dbe7b` v2 (R5.8 hardening, seq race fix) → `2baebca5…91149` (R5.9c, target_spu on 7 SPU-side recorders) → `d4873c358d…509a09ac` (R5.9e.3, +`record_spu_image` + `mbedtls_sha256` + `.spuimg` side-file write) → current (R5.9e.3-fix, write_image_side_file returns bool + post-write file_size verification + record_spu_image bails before JSONL on failure; trace v4 capture validated end-to-end with `.spuimg` SHA-matching the JSONL `image_sha256`). | [details below](#spu_trace_jsonl_scaffolding) |
| [`spu_trace_jsonl_runtime_hooks.patch`](./spu_trace_jsonl_runtime_hooks.patch) | Runtime hooks at SPU/PPU hot-paths — **R5.9e.3** (cpu_task spu_image emission) | build + smoke validated through R5.9d real-trace v3. **Current sha256 `8f253d7d207793266eb3a81e809c73731a8e565757a9d2c40fa944a88266663a`**. Chronological history: `1b69f107…b28694` (R5.8 A.3 final) → `3ee7a861…2bed39` (R5.9c, `lv2_id` threaded through 11 SPU-side call sites + 2 lambda-scope snapshots) → current (R5.9e.3, +new cpu_task hunk emitting `spu_image` after `pc &= 0x3fffc;` so entry_pc is the clean instruction address; +6 hunk-header offset adjustments). | [details below](#spu_trace_jsonl_runtime_hooks) |

## Apply order (mandatory)

These two patches MUST be applied in order. The scaffolding patch creates `rpcs3/Emu/Cell/SPUTraceJsonl.{h,cpp}` and wires them into the build; the runtime hooks patch then references `rpcs3::spu_trace::TraceWriter` from the SPU/PPU hot-paths. Applying the runtime hooks patch first will fail to compile (missing header).

```bash
# 1. Apply scaffolding (creates writer files, edits build files only).
git apply docs/patches/spu_trace_jsonl_scaffolding.patch

# 2. Build, verify rpcs3.exe is produced, smoke without/with env var (writer dormant).
msbuild rpcs3.sln /p:Configuration=Release /p:Platform=x64 /m

# 3. Apply runtime hooks (edits hot-path SPU/PPU sources to invoke record_*).
git apply docs/patches/spu_trace_jsonl_runtime_hooks.patch

# 4. Rebuild, verify rpcs3.exe still produced, smoke with env var.
msbuild rpcs3.sln /p:Configuration=Release /p:Platform=x64 /m

# 5. Capture a real .jsonl by running a deterministic homebrew with
#    RPCS3_SPU_TRACE_JSONL=/path/to/out.jsonl set. Commit the resulting
#    trace to behavior-freeze/fixtures/spu/traces/ alongside its .notes.md.

# 6. Run Rust replay tests against the captured trace
#    (parse_jsonl_trace + captured_events_to_trace + replay_trace
#    against InterpreterExecutor and RecompilerExecutor).
```

---

## `spu_trace_jsonl_scaffolding`

**What it does:**
- Adds two new files: `rpcs3/Emu/Cell/SPUTraceJsonl.h` and `rpcs3/Emu/Cell/SPUTraceJsonl.cpp`. Self-contained env-var-gated JSONL trace writer that emits events conforming to [`docs/SPU_TRACE_CAPTURE.md`](../SPU_TRACE_CAPTURE.md). Disabled by default (zero overhead when `RPCS3_SPU_TRACE_JSONL` env var is unset).
- Wires the new files into the build: one line in `rpcs3/Emu/CMakeLists.txt`, one `<ClCompile>` + one `<ClInclude>` in `rpcs3/emucore.vcxproj`, equivalent entries in `rpcs3/emucore.vcxproj.filters`. Additive, alphabetically positioned after the existing `SPUThread.cpp/h` entries.

**What it does NOT do:**
- No edits to `SPUThread.cpp`, `SPUCommonRecompiler.cpp`, `SPUInterpreter.cpp`, `SPULLVMRecompiler.cpp`, `SPUASMJITRecompiler.cpp`, `RawSPUThread.cpp`, `lv2/sys_spu.cpp`. **Zero runtime hooks.**
- No `.jsonl` trace fixtures committed.
- No Rust code changes (the Rust pipeline that consumes the JSONL output is independently committed under `rust/rpcs3-spu-differential/src/trace_fmt.rs`).

**Apply procedure (in a tracked RPCS3 fork):**

```bash
# Verify the patch applies cleanly first.
git apply --check docs/patches/spu_trace_jsonl_scaffolding.patch

# If clean, apply.
git apply docs/patches/spu_trace_jsonl_scaffolding.patch

# Or apply as a commit:
git am < docs/patches/spu_trace_jsonl_scaffolding.patch
```

**Build validation (the gate before runtime hooks):**

```bash
# Linux / macOS (CMake).
cmake -B build -S . -DCMAKE_BUILD_TYPE=Release
cmake --build build --target rpcs3 -j$(nproc)

# Windows (MSBuild via VS 2022).
msbuild rpcs3.sln /p:Configuration=Release /p:Platform=x64 /m
```

The build MUST complete without errors. Specifically, no missing-symbol / undefined-reference errors should originate from `SPUTraceJsonl.{cpp,h}` — the writer is self-contained C++17 + standard library only and should compile against any host that already builds RPCS3.

**Sanity check post-build (writer is a true noop when disabled):**

Launch RPCS3 WITHOUT setting `RPCS3_SPU_TRACE_JSONL`, run a known-good homebrew, confirm behavior is unchanged versus an unpatched build (same exit code, same console log, same framebuffer hash if applicable). The writer's `enabled()` atomic-load short-circuit must keep every `record_*` invocation a single load+compare when the env var is absent.

**Smoke check writer is reachable when enabled:**

```bash
export RPCS3_SPU_TRACE_JSONL=/tmp/smoke.jsonl
./build/bin/rpcs3 --headless ./behavior-freeze/fixtures/spu/synthetic_il_stop.elf
test -s /tmp/smoke.jsonl && echo "trace produced"
```

**At this stage no events fire** — the writer is initialized but no runtime hooks call its `record_*` methods. The file should be created (because the writer's constructor opens it) but stay empty or near-empty. That confirms the writer can open the file and is ready to receive events.

**Only AFTER the build + sanity + smoke pass:** apply the runtime-hook integration patch documented in [`docs/SPU_TRACE_CAPTURE_PATCH.md`](../SPU_TRACE_CAPTURE_PATCH.md). That patch is intentionally separate so a maintainer who hits a build issue with the scaffolding can fix it (in `SPUTraceJsonl.{h,cpp}` or in the build files) without rolling back any runtime-hook changes.

**If the scaffolding build fails:**
- Diagnose only inside `SPUTraceJsonl.{h,cpp}` or the three build-system files.
- Do NOT modify hot-path runtime files (`SPUThread.cpp`, `RawSPUThread.cpp`, `lv2/sys_spu.cpp`, etc.) — those stay untouched until runtime hooks are intentionally applied.
- Do NOT add runtime hooks "to test" — runtime hooks land only after the build is green and is a separate, scope-isolated step.

**Where the patch came from:**
- The scaffolding files exist on local disk in this workspace at `rpcs3/Emu/Cell/SPUTraceJsonl.{h,cpp}` and the build edits are present in `rpcs3/Emu/CMakeLists.txt` + `rpcs3/emucore.vcxproj{,.filters}`. Because `/rpcs3/` is gitignored, those edits are NOT version-controlled here. This patch is the version-controlled artifact that captures them.
- Regenerated against upstream RPCS3 master commit `7028e85fa` (Apr 2026) by cloning a clean tracked tree, copying the scaffolding files in, applying byte-level inserts to the three build files (preserving CRLF line endings), then running `git diff` (no `--no-index`, no autocrlf override, stderr suppressed to keep warnings out of the patch body). The previous generation method using `grep -v` / `awk` left a malformed `.filters` hunk because the SPUTraceJsonl 3-line block in the local copy was only partially matched by the line-grep, producing an invalid "before" state. The current patch is generated from the actual tracked baseline and round-trips byte-exact under `git apply --check` + `git apply` against `7028e85fa`.

**Target upstream baseline:**
- Commit: `7028e85fa` (https://github.com/RPCS3/rpcs3)
- Branch: `master`
- Verified: `git apply --check docs/patches/spu_trace_jsonl_scaffolding.patch` exits 0 against a fresh clone at this commit.
- Drift caveat: as upstream evolves, the line numbers in the `emucore.vcxproj{,.filters}` hunks may drift. The CMakeLists.txt and `/dev/null`-anchored new-file hunks tolerate drift better. If the patch goes stale again, regenerate against the new HEAD using the same procedure.

**Cross-references:**
- Wire format: [`docs/SPU_TRACE_CAPTURE.md`](../SPU_TRACE_CAPTURE.md).
- Runtime-hook integration patch (apply ONLY after build is green): [`docs/SPU_TRACE_CAPTURE_PATCH.md`](../SPU_TRACE_CAPTURE_PATCH.md).
- Runtime-hook application guide + validation checklist: [`docs/SPU_TRACE_CAPTURE_RUNTIME_HOOKS.md`](../SPU_TRACE_CAPTURE_RUNTIME_HOOKS.md).
- Rust pipeline that consumes JSONL output: [`rust/rpcs3-spu-differential/src/trace_fmt.rs`](../../rust/rpcs3-spu-differential/src/trace_fmt.rs).
- Trace fixture destination (currently empty by design): [`behavior-freeze/fixtures/spu/traces/README.md`](../../behavior-freeze/fixtures/spu/traces/README.md).
- Project status: [`docs/PROJECT_STATUS.md`](../PROJECT_STATUS.md).

---

## `spu_trace_jsonl_runtime_hooks`

**What it does:**
- Wires the SPU/PPU hot-paths to invoke `rpcs3::spu_trace::TraceWriter::record_*` methods so the dormant scaffolding from [`spu_trace_jsonl_scaffolding.patch`](./spu_trace_jsonl_scaffolding.patch) starts emitting JSONL events when the env var `RPCS3_SPU_TRACE_JSONL` is set.
- Touches exactly three files: `rpcs3/Emu/Cell/SPUThread.cpp` (5 hooks: `cpu_task` / `get_ch_count` / `get_ch_value` / `set_ch_value` SPU_WrOutMbox arm only / `stop_and_signal`), `rpcs3/Emu/Cell/RawSPUThread.cpp` (1 hook: `SPU_Out_MBox_offs` PPU read post-`pop`), and `rpcs3/Emu/Cell/lv2/sys_spu.cpp` (2 hooks: `sys_spu_thread_write_spu_mb` post-commit + `sys_spu_thread_write_snr` post-`push_snr`).
- Adds `#include "Emu/Cell/SPUTraceJsonl.h"` (or same-dir `#include "SPUTraceJsonl.h"` in `RawSPUThread.cpp`) to all three files.
- All hot-path emit sites use `tracer.enabled()` short-circuit so the writer-disabled path is one atomic load + branch — no behavior change when `RPCS3_SPU_TRACE_JSONL` is unset.

**What it does NOT do:**
- Does NOT touch `RawSPUThread.cpp`'s `SPU_In_MBox_offs` write arm (line 289). PPU push via raw MMIO is deferred — the threaded-SPU equivalent is already covered by `sys_spu_thread_write_spu_mb` (hook 6a), so the raw MMIO path is redundant for first-pass capture; revisit when raw-SPU homebrews exercise it.
- Does NOT touch `set_ch_value`'s `SPU_WrOutIntrMbox` arm. That arm has a non-raw-SPU path that routes to `sys_spu_thread_send_event` instead of pushing the channel directly; the doc's "wrch + park + wake" pattern doesn't fit that fork. Documented as deferred for first capture.
- Does NOT modify `docs/patches/spu_trace_jsonl_scaffolding.patch` — the scaffolding patch is closed and runtime hooks are a separate, sequenced artifact.
- Does NOT create any `.jsonl` file; that comes from a real homebrew run with the env var set.

**Hooks delivered (8 of 10 documented events):**

| # | Event | Site | Status |
|---|---|---|---|
| 1 | `spu_rdch` + `spu_park`(ChannelRead) + `spu_wake` | `SPUThread.cpp:5335` `get_ch_value` `read_channel` lambda | ✅ |
| 2 | `spu_wrch` + `spu_park`(ChannelWrite) + `spu_wake` | `SPUThread.cpp:6258` `set_ch_value` SPU_WrOutMbox arm | ✅ partial |
| 3 | `spu_rchcnt` | `SPUThread.cpp:5288` `get_ch_count` (restructured with `result` accumulator) | ✅ |
| 4 | `spu_stop` | `SPUThread.cpp:6431` `stop_and_signal` top-of-function | ✅ |
| 5 | `final_state` | `SPUThread.cpp:1442` `cpu_task` via `TraceFinalGuard` destructor | ✅ |
| 6a | `ppu_push_inmbox` | `lv2/sys_spu.cpp:1939` `sys_spu_thread_write_spu_mb` post-commit | ✅ |
| 6b | `ppu_pop_outmbox` | `RawSPUThread.cpp:145` `SPU_Out_MBox_offs` post-pop with pre-pop count check | ✅ |
| 6c | `ppu_signal` | `lv2/sys_spu.cpp:2015` `sys_spu_thread_write_snr` post-`push_snr` | ✅ |
| 6d | `ppu_push_inmbox` (raw MMIO) | `RawSPUThread.cpp:289` `SPU_In_MBox_offs` | ❌ deferred (redundant with 6a for first-pass) |
| 2-extra | `spu_wrch` SPU_WrOutIntrMbox | `SPUThread.cpp:6059` (raw arm only fits doc pattern) | ❌ deferred |

**Documented edge cases the patch addresses:**
- `ch_in_mbox` is `spu_channel_4_t` (4-deep queue) which exposes only destructive `try_pop()` / `pop()` — no non-destructive peek. Per the rule "do not pop just to log", `final_state` emits `in_mbox: nullopt` rather than fabricating a value or causing side effects. Other channels (`ch_out_mbox`, `ch_out_intr_mbox`, `ch_snr1`, `ch_snr2`) are single-slot `spu_channel` with non-destructive `get_value()` and are captured byte-faithful.
- Hook 6b (`ppu_pop_outmbox` at `RawSPUThread.cpp:145`) uses a non-destructive `ch_out_mbox.get_count() > 0` check BEFORE the existing `pop()`, then reads `value` from the unchanged pop result — no extra pop, no value mutation. The pre-pop count distinguishes a real value pop from an empty-mbox pop (which returns 0 by RPCS3 convention). Acceptable race window: between `get_count()` and `pop()` the SPU could push, in which case the trace records `nullopt` while pop returns the new value. The numeric value reported is always the actual pop result; only the empty-vs-Some discrimination has the documented race. `target_spu` uses `lv2_id` (the syscall-visible id field, consistent with hooks 6a and 6c).
- SPU lane-0 GPR maps to `v128::_u32[3]` on RPCS3's little-endian builds (verified against existing GPR access patterns).
- `TraceFinalGuard` destructor wraps emit in `try {} catch (...) {}` — never propagates from a destructor, never perturbs SPU shutdown.
- `get_ch_value`'s post-resume `record_spu_rdch(would_stall=false, value=static_cast<u32>(out))` truncates the 64-bit `pop_wait` return to its low 32 bits. Schema permits any `u32`; if `out` carries an error sentinel in the high bits, the truncated value is still a valid trace observation.
- `set_ch_value` SPU_WrOutMbox: `would_stall` captured at function entry (pre-mfc). If `do_mfc()` then drains the mbox, the trace records a balanced `park`/`wake` pair around a no-op stall. Acceptable imprecision documented in the patch's inline comment.

**Apply procedure (post-scaffolding):**

```bash
# Prerequisite: scaffolding patch already applied + rpcs3.exe builds.
git apply --check docs/patches/spu_trace_jsonl_runtime_hooks.patch
git apply docs/patches/spu_trace_jsonl_runtime_hooks.patch

# Rebuild — only emucore.lib (and downstream) needs to relink.
msbuild rpcs3.sln /p:Configuration=Release /p:Platform=x64 /m
```

**Validation:**
- `git apply --check` exit 0 on clean upstream `7028e85fa` (with scaffolding applied) ✅ verified.
- `git apply` exit 0 ✅ verified.
- `git apply --check --reverse` exit 0 ✅ verified.
- Build with hooks succeeds (rpcs3.exe rebuilt at `R:\bin\rpcs3.exe` 64 MB; remaining 9 errors are gtest-suite-only in `rpcs3_test.vcxproj`, unrelated to runtime hooks).
- Smoke without `RPCS3_SPU_TRACE_JSONL`: rpcs3.exe runs 8s, no crash, no spurious `.jsonl`.
- Smoke with `RPCS3_SPU_TRACE_JSONL=/tmp/smoke.jsonl`: rpcs3.exe runs 12s, no crash, no `.jsonl` created (correct — no SPU thread activity without homebrew loaded; writer's lazy `open()` never fired).

**Pending (real-trace capture, requires deterministic homebrew):**
- Run a deterministic SPU/PPU homebrew under patched RPCS3 with `RPCS3_SPU_TRACE_JSONL` set, observe events in the `.jsonl`.
- Validate via Rust pipeline at [`rust/rpcs3-spu-differential/src/trace_fmt.rs`](../../rust/rpcs3-spu-differential/src/trace_fmt.rs): `parse_jsonl_trace()` then `captured_events_to_trace()`, then `replay_trace()` against both `InterpreterExecutor` and `RecompilerExecutor`.
- If parse / transform / replay fails, preserve the trace as-is — don't weaken the Rust-side oracle.
